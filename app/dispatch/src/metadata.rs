//! Collapse release metadata: magic blobs, the master key and per-biz keys.
//!
//! The release repo serves `config_master.json` (a Brotli-wrapped RSA-1024
//! PKCS#1 DER key) and a stamp table that resolves the per-biz preset file.
//! That preset carries base64 "Collapse-magic" blobs for the dispatcher key,
//! URL template, seed and channel tuple. Everything here is pure except the
//! injected GET, so a test can feed recorded/injected bytes and never touch
//! the release repo.

use crate::crypto::decode_base64;
use crate::dispatch::BytesFetcher;
use crate::url::{
    channel_id, channel_prefix, dispatch_host, region_id, sub_channel_id, template_for,
};
use crate::{DispatchError, Result};
use brotli::Decompressor;
use girpr_digest::sha256_hex;
use girpr_model::Region;
use rsa::{BoxedUint, PrivateKey, pkcs1::DecodeRsaPrivateKey};
use serde_json::Value;
use std::io::Read;

/// `CollapseLauncher-ReleaseRepo` metadata root used by `fetch_metadata.py`.
pub const METADATA_BASE: &str = "https://raw.githubusercontent.com/CollapseLauncher/CollapseLauncher-ReleaseRepo/main/metadata/v3/stable";
pub const MASTER_URL: &str = "https://raw.githubusercontent.com/CollapseLauncher/CollapseLauncher-ReleaseRepo/main/metadata/v3/stable/config_master.json";
pub const STAMP_URL: &str = "https://raw.githubusercontent.com/CollapseLauncher/CollapseLauncher-ReleaseRepo/main/metadata/v3/stable/stamp.json";

/// `DataCooker`'s "Collapse-magic" signature, as a signed little-endian i64.
pub const MAGIC_SIGNATURE: i64 = 7310310183885631299;
const HEADER_LEN: usize = 32;

/// Pin for the *raw* `config_master.json` `Key` blob (the decoded base64
/// bytes). From `samples/metadata_summary.json`; an upstream change must be
/// reviewed rather than silently accepted as a new key.
pub const MASTER_KEY_BLOB_SHA256: &str =
    "d69fe7224adfb8afe1bc8b61fcc2dc1fe5e5f94628c60d26cbb07537753bde09";
pub const MASTER_KEY_BLOB_LEN: usize = 642;

/// A parsed 32-byte magic header plus the still-compressed/encrypted payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagicBlob {
    pub flags: i64,
    pub compression: u8,
    pub encryption: u8,
    pub compressed_size: usize,
    pub decompressed_size: usize,
    pub payload: Vec<u8>,
}

fn read_i64_le(bytes: &[u8], offset: usize) -> Result<i64> {
    let slice: [u8; 8] = bytes
        .get(offset..offset + 8)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| DispatchError::Metadata("truncated Collapse-magic header".into()))?;
    Ok(i64::from_le_bytes(slice))
}

/// Parse and validate the magic header before touching the payload.
pub fn parse_magic_blob(raw: &[u8]) -> Result<MagicBlob> {
    if raw.len() < HEADER_LEN {
        return Err(DispatchError::Metadata(format!(
            "Collapse-magic blob is {} bytes, header needs {HEADER_LEN}",
            raw.len()
        )));
    }
    let signature = read_i64_le(raw, 0)?;
    if signature != MAGIC_SIGNATURE {
        return Err(DispatchError::Metadata(format!(
            "not a Collapse-magic blob: signature {signature:#x}"
        )));
    }
    let flags = read_i64_le(raw, 8)?;
    let compressed = read_i64_le(raw, 16)?;
    let decompressed = read_i64_le(raw, 24)?;
    let to_usize = |name: &str, value: i64| -> Result<usize> {
        usize::try_from(value)
            .map_err(|_| DispatchError::Metadata(format!("negative {name} size: {value}")))
    };
    Ok(MagicBlob {
        flags,
        compression: (flags & 0xff) as u8,
        encryption: ((flags >> 8) & 0xff) as u8,
        compressed_size: to_usize("compressed", compressed)?,
        decompressed_size: to_usize("decompressed", decompressed)?,
        payload: raw[HEADER_LEN..].to_vec(),
    })
}

/// Decrypt (optionally) and decompress a magic blob's payload.
///
/// `master` is `None` only for the master key's own flags-`0x1` blob; a flags-
/// `0x101` field without a master key is an error. Both the compressed and the
/// decompressed sizes from the header are verified.
pub fn unwrap_magic_blob(
    raw: &[u8],
    master: Option<&PrivateKey>,
    block_bytes: usize,
) -> Result<Vec<u8>> {
    let blob = parse_magic_blob(raw)?;
    let mut payload = blob.payload;
    if blob.encryption != 0 {
        let key = master.ok_or_else(|| {
            DispatchError::Metadata(format!(
                "encrypted magic blob (flags {:#x}) has no master key",
                blob.flags
            ))
        })?;
        payload = crate::decrypt_pkcs1_blocks(key, &payload, block_bytes)?;
    }
    if payload.len() != blob.compressed_size {
        return Err(DispatchError::Metadata(format!(
            "blob payload {} bytes != compressed size {}",
            payload.len(),
            blob.compressed_size
        )));
    }
    let out = match blob.compression {
        0 => payload,
        1 => {
            let mut out = Vec::with_capacity(blob.decompressed_size);
            Decompressor::new(payload.as_slice(), 4096)
                .read_to_end(&mut out)
                .map_err(|e| DispatchError::Metadata(format!("Brotli decompress failed: {e}")))?;
            out
        }
        other => {
            return Err(DispatchError::Metadata(format!(
                "unsupported Collapse compression {other}"
            )));
        }
    };
    if out.len() != blob.decompressed_size {
        return Err(DispatchError::Metadata(format!(
            "blob decompressed to {} bytes, header says {}",
            out.len(),
            blob.decompressed_size
        )));
    }
    Ok(out)
}

fn blob_bytes(value: &Value) -> Result<Vec<u8>> {
    match value {
        Value::String(s) => decode_base64(s),
        Value::Array(items) => items
            .iter()
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| DispatchError::Metadata("Key array has a non-byte".into()))
            })
            .collect(),
        other => Err(DispatchError::Metadata(format!(
            "master Key is neither base64 nor an array: {other}"
        ))),
    }
}

/// Build the metadata master key from `config_master.json`.
///
/// When `pin` is true (the product path), the raw key blob must match the
/// recorded digest and length exactly. Tests with a synthetic master key pass
/// `false` and supply their own expected digest in the test itself.
pub fn master_key_from_json(json: &[u8], pin: bool) -> Result<(PrivateKey, usize)> {
    let value: Value = serde_json::from_slice(json)
        .map_err(|e| DispatchError::Metadata(format!("bad config_master.json: {e}")))?;
    let raw = blob_bytes(
        value
            .get("Key")
            .ok_or_else(|| DispatchError::Metadata("config_master.json has no Key".into()))?,
    )?;
    if pin {
        if raw.len() != MASTER_KEY_BLOB_LEN {
            return Err(DispatchError::Metadata(format!(
                "master key blob is {} bytes, pinned length is {MASTER_KEY_BLOB_LEN}",
                raw.len()
            )));
        }
        let digest = sha256_hex(&raw);
        if digest != MASTER_KEY_BLOB_SHA256 {
            return Err(DispatchError::Metadata(format!(
                "master key blob sha256 {digest} != pinned {MASTER_KEY_BLOB_SHA256}"
            )));
        }
    }
    let bit_size = value
        .get("BitSize")
        .and_then(Value::as_u64)
        .ok_or_else(|| DispatchError::Metadata("config_master.json has no BitSize".into()))?;
    let block_bytes = usize::try_from(bit_size)
        .map_err(|_| DispatchError::Metadata(format!("BitSize {bit_size} does not fit")))?;
    let der = unwrap_magic_blob(&raw, None, block_bytes)?;
    let key = PrivateKey::from_pkcs1_der(&der)
        .map_err(|e| DispatchError::Metadata(format!("bad master PKCS#1 DER: {e}")))?;
    Ok((key, block_bytes))
}

/// Resolve the per-biz preset file from `stamp.json` (the release table), not
/// by guessing a file name.
pub fn config_file_from_stamp(json: &[u8], biz: Region) -> Result<String> {
    let want_region = match biz {
        Region::Hk4eGlobal => "Global",
        Region::Hk4eCn => "Mainland China",
        Region::Hk4eBilibili => "Bilibili",
    };
    let rows: Vec<Value> = serde_json::from_slice(json)
        .map_err(|e| DispatchError::Metadata(format!("bad stamp.json: {e}")))?;
    let mut matches = rows
        .iter()
        .filter(|row| {
            row.get("GameName").and_then(Value::as_str) == Some("Genshin Impact")
                && row.get("GameRegion").and_then(Value::as_str) == Some(want_region)
                && row.get("MetadataType").and_then(Value::as_str) == Some("PresetConfigV2")
        })
        .filter_map(|row| row.get("MetadataPath").and_then(Value::as_str))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    matches.dedup();
    match matches.len() {
        1 => Ok(matches.remove(0)),
        n => Err(DispatchError::Metadata(format!(
            "stamp.json has {n} preset entries for {want_region}/{biz}"
        ))),
    }
}

/// Unwrap one metadata field. A plain JSON scalar is returned unchanged; only
/// strings beginning with base64("Collapse") are magic blobs.
pub fn unwrap_metadata_field(
    value: &Value,
    master: &PrivateKey,
    block_bytes: usize,
) -> Result<Value> {
    let Some(text) = value.as_str() else {
        return Ok(value.clone());
    };
    if !text.starts_with("Q29sbGFwc2U") {
        return Ok(value.clone());
    }
    let raw = decode_base64(text)?;
    let out = unwrap_magic_blob(&raw, Some(master), block_bytes)?;
    let string = String::from_utf8(out)
        .map_err(|e| DispatchError::Metadata(format!("unwrapped field is not UTF-8: {e}")))?;
    Ok(Value::String(string))
}

/// Load a .NET `RSAKeyValue` as `(n, e, d, p, q)` only.
///
/// .NET's `InverseQ` is `q^-1 mod p`, while a seventh CRT component in several
/// libraries is `p^-1 mod q`. Passing it would fail validation, so DP/DQ and
/// InverseQ are deliberately not read: the key derives its own CRT values from
/// the five components it does need.
pub fn rsa_key_from_xml(xml: &str) -> Result<PrivateKey> {
    fn component(xml: &str, tag: &str) -> Result<Vec<u8>> {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        let start = xml
            .find(&open)
            .ok_or_else(|| DispatchError::Metadata(format!("key XML has no {tag}")))?
            + open.len();
        let end = xml[start..]
            .find(&close)
            .ok_or_else(|| DispatchError::Metadata(format!("key XML has no closing {tag}")))?
            + start;
        decode_base64(xml[start..end].trim())
    }

    fn uint(bytes: &[u8], name: &str) -> Result<BoxedUint> {
        if bytes.is_empty() {
            return Err(DispatchError::Metadata(format!("empty {name} in key XML")));
        }
        let bits = u32::try_from(bytes.len())
            .ok()
            .and_then(|n| n.checked_mul(8))
            .ok_or_else(|| DispatchError::Metadata(format!("{name} is too large")))?;
        BoxedUint::from_be_slice(bytes, bits)
            .map_err(|e| DispatchError::Metadata(format!("bad {name} in key XML: {e}")))
    }

    let n_bytes = component(xml, "Modulus")?;
    let e_bytes = component(xml, "Exponent")?;
    let d_bytes = component(xml, "D")?;
    let p_bytes = component(xml, "P")?;
    let q_bytes = component(xml, "Q")?;
    let key = PrivateKey::from_components(
        uint(&n_bytes, "Modulus")?,
        uint(&e_bytes, "Exponent")?,
        uint(&d_bytes, "D")?,
        vec![uint(&p_bytes, "P")?, uint(&q_bytes, "Q")?],
    )
    .map_err(|e| DispatchError::Metadata(format!("invalid dispatcher key: {e}")))?;
    Ok(key)
}

/// Versioned seed table (non-user-facing fallback / pin). Both keys recorded
/// in docs/03 §7.4.4 and the three live seeds from `CHANNELS.md` are here; a
/// version not in the table fails closed, never reuses an older seed.
pub fn pinned_seed(biz: Region, version: &str) -> Result<&'static str> {
    let components: Vec<&str> = version
        .split(['.', '_', '-'])
        .take(3)
        .filter(|s| !s.is_empty())
        .collect();
    let key = if components.len() >= 2 {
        format!("{}.{}", components[0], components[1])
    } else {
        version.to_owned()
    };
    let seed = match (biz, key.as_str()) {
        (Region::Hk4eGlobal, "7.2") => "018bc9560abc707f",
        (Region::Hk4eGlobal, "7.0") => "d9f572e5464f160f",
        (Region::Hk4eCn, "7.2") | (Region::Hk4eBilibili, "7.2") => "e17f04ce947d859a",
        _ => {
            return Err(DispatchError::Metadata(format!(
                "no pinned dispatcher seed for {biz} version {version:?}"
            )));
        }
    };
    Ok(seed)
}

/// The unwrapped per-biz key material plus the derived URL/channel values.
pub struct DispatcherKeys {
    pub biz: Region,
    pub host: &'static str,
    pub channel_prefix: &'static str,
    pub region_id: u8,
    pub template: String,
    pub seed: String,
    pub key: PrivateKey,
    pub block_bytes: usize,
    pub exe_stem: String,
}

fn as_u64(value: &Value, name: &str) -> Result<u64> {
    value
        .as_u64()
        .ok_or_else(|| DispatchError::Metadata(format!("{name} is not an integer")))
}

fn as_string(value: &Value, name: &str) -> Result<String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| DispatchError::Metadata(format!("{name} is not a string")))
}

/// Fetch and unwrap the master key, the stamp table and the per-biz preset.
pub fn load_dispatch_keys<F: BytesFetcher>(
    biz: Region,
    latest: &str,
    fetcher: &F,
) -> Result<DispatcherKeys> {
    load_dispatch_keys_with_pin(biz, latest, fetcher, true)
}

/// The injectable seam behind [`load_dispatch_keys`]. `pin` is always true in
/// the product path; offline tests with a synthetic master key pass `false`
/// and pin their own expected digest separately.
pub fn load_dispatch_keys_with_pin<F: BytesFetcher>(
    biz: Region,
    latest: &str,
    fetcher: &F,
    pin: bool,
) -> Result<DispatcherKeys> {
    let master_json = fetcher
        .get(MASTER_URL)
        .map_err(|e| DispatchError::Transport(format!("config_master.json: {e}")))?;
    let stamp_json = fetcher
        .get(STAMP_URL)
        .map_err(|e| DispatchError::Transport(format!("stamp.json: {e}")))?;
    let (master, master_block) = master_key_from_json(&master_json, pin)?;
    let config_file = config_file_from_stamp(&stamp_json, biz)?;
    let config_url = format!("{METADATA_BASE}/{config_file}");
    let config_json = fetcher
        .get(&config_url)
        .map_err(|e| DispatchError::Transport(format!("{config_file}: {e}")))?;
    let config: Value = serde_json::from_slice(&config_json)
        .map_err(|e| DispatchError::Metadata(format!("bad {config_file}: {e}")))?;

    let field = |name: &'static str| -> Result<Value> {
        let raw = config
            .get(name)
            .ok_or_else(|| DispatchError::Metadata(format!("{config_file} has no {name}")))?;
        unwrap_metadata_field(raw, &master, master_block)
    };

    let template = match config.get("GameDispatchURL") {
        Some(raw) => as_string(
            &unwrap_metadata_field(raw, &master, master_block)?,
            "GameDispatchURL",
        )?,
        None => template_for(biz).to_owned(),
    };
    let seed = as_string(&field("ProtoDispatchKey")?, "ProtoDispatchKey")?;
    let expected_seed = pinned_seed(biz, latest)?;
    if seed != expected_seed {
        return Err(DispatchError::Metadata(format!(
            "metadata seed {seed} != pinned {expected_seed} for {biz} {latest}"
        )));
    }
    let block_bytes = as_u64(&field("DispatcherKeyBitLength")?, "DispatcherKeyBitLength")?;
    let block_bytes = usize::try_from(block_bytes)
        .map_err(|_| DispatchError::Metadata("DispatcherKeyBitLength does not fit".into()))?;
    if block_bytes != 256 {
        return Err(DispatchError::Metadata(format!(
            "dispatcher block length {block_bytes} != 256"
        )));
    }
    let key_xml = as_string(&field("DispatcherKey")?, "DispatcherKey")?;
    let key = rsa_key_from_xml(&key_xml)?;
    let channel = as_u64(&field("ChannelID")?, "ChannelID")?;
    let sub_channel = as_u64(&field("SubChannelID")?, "SubChannelID")?;
    if channel != u64::from(channel_id(biz)) || sub_channel != u64::from(sub_channel_id(biz)) {
        return Err(DispatchError::Metadata(format!(
            "{config_file} channel tuple ({channel}, {sub_channel}) != ({}, {}) for {biz}",
            channel_id(biz),
            sub_channel_id(biz)
        )));
    }
    let exe = as_string(&field("GameExecutableName")?, "GameExecutableName")?;
    let exe_stem = exe
        .strip_suffix(".exe")
        .or_else(|| exe.strip_suffix(".EXE"))
        .unwrap_or(&exe)
        .to_owned();
    Ok(DispatcherKeys {
        biz,
        host: dispatch_host(biz),
        channel_prefix: channel_prefix(biz),
        region_id: region_id(biz),
        template,
        seed,
        key,
        block_bytes,
        exe_stem,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_header_is_32_little_endian_bytes() {
        let mut raw = vec![0u8; HEADER_LEN + 3];
        raw[0..8].copy_from_slice(&MAGIC_SIGNATURE.to_le_bytes());
        raw[8..16].copy_from_slice(&0x101i64.to_le_bytes());
        raw[16..24].copy_from_slice(&3i64.to_le_bytes());
        raw[24..32].copy_from_slice(&7i64.to_le_bytes());
        raw[32..].copy_from_slice(b"abc");
        let blob = parse_magic_blob(&raw).unwrap();
        assert_eq!(blob.flags, 0x101);
        assert_eq!((blob.compression, blob.encryption), (1, 1));
        assert_eq!((blob.compressed_size, blob.decompressed_size), (3, 7));
        assert_eq!(blob.payload, b"abc");
        let mut bad = raw.clone();
        bad[0] ^= 1;
        assert!(parse_magic_blob(&bad).is_err());
        assert!(parse_magic_blob(&raw[..HEADER_LEN - 1]).is_err());
    }
}
