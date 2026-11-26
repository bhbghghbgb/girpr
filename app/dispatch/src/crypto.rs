//! Base64 and PKCS#1 v1.5 mechanics shared by the two RSA steps.
//!
//! Both the Collapse metadata blobs and the dispatcher `content` field use
//! block-aligned PKCS#1 v1.5 private-key decryption. The block length is
//! metadata-driven: 128 bytes for the RSA-1024 master key, 256 for the
//! dispatcher key (`DispatcherKeyBitLength`, despite the byte-vs-bit name).

use crate::{DispatchError, Result};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use rsa::{PrivateKey, pkcs1v15::Pkcs1v15Encrypt, traits::PublicKeyParts};

/// Decode standard base64, tolerating both padding shapes: some Collapse
/// fields and mock bodies omit `=`, while the XML key components keep it.
/// Whitespace is dropped before decoding either shape.
pub fn decode_base64(text: &str) -> Result<Vec<u8>> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    STANDARD_NO_PAD
        .decode(compact.as_bytes())
        .or_else(|_| STANDARD.decode(compact.as_bytes()))
        .map_err(|e| DispatchError::Crypto(format!("invalid base64: {e}")))
}

/// Decode a whole ciphertext as `block_bytes`-sized PKCS#1 v1.5 blocks.
///
/// A misaligned body or a bad block is an error, never a partial plaintext: a
/// truncated protobuf would otherwise be decoded into plausible-looking but
/// wrong revisions.
pub fn decrypt_pkcs1_blocks(
    key: &PrivateKey,
    ciphertext: &[u8],
    block_bytes: usize,
) -> Result<Vec<u8>> {
    if block_bytes == 0 {
        return Err(DispatchError::Crypto("zero RSA block size".into()));
    }
    if key.size() != block_bytes {
        return Err(DispatchError::Crypto(format!(
            "key is {} bytes, expected {block_bytes}-byte blocks",
            key.size()
        )));
    }
    if ciphertext.is_empty() || !ciphertext.len().is_multiple_of(block_bytes) {
        return Err(DispatchError::Crypto(format!(
            "ciphertext {} bytes is not aligned to {block_bytes}-byte blocks",
            ciphertext.len()
        )));
    }
    let mut out = Vec::with_capacity(ciphertext.len());
    for (index, block) in ciphertext.chunks(block_bytes).enumerate() {
        out.extend(key.decrypt(Pkcs1v15Encrypt, block).map_err(|e| {
            DispatchError::Crypto(format!("RSA block {index} decrypt failed: {e}"))
        })?);
    }
    Ok(out)
}
