//! Digest spelling shared by the integration test targets.
//!
//! Fixture builders call this from the library; fixture-free targets reach the
//! same function through the thin shim at `tests/common/digest.rs`. Both spell
//! the digest exactly as the repair path does, because the bytes come from the
//! same leaf crate the programs use.

/// Lowercase hex MD5 of a byte slice.
pub fn md5hex(data: &[u8]) -> String {
    girpr_digest::md5_hex(data)
}
