//! The `GenshinGateway` protobuf, typed where it is consumed and generic where
//! it is merely observed.
//!
//! `prost` decodes the fields that become `DispatchInfo`. A small wire walker
//! additionally sees the fields the checked-in `.proto` omits (root 11/12/14
//! and properties 38–47). Those are collected as opaque lengths and logged;
//! no file placement decision can read them.

use crate::{DispatchError, Result};
use girpr_digest::Md5;
use prost::Message as _;

/// Root of the dispatcher protobuf. Only tag 3 is consumed; the other root
/// tags are opaque to this crate.
#[derive(Clone, PartialEq, prost::Message)]
pub struct DispatchResponse {
    #[prost(message, optional, tag = "3")]
    pub properties: Option<GatewayProperties>,
}

/// `GatewayProperties` (root tag 3). The tags mirror `GenshinGateway.proto`;
/// the names are the ones `docs/08` Step 3 uses.
#[derive(Clone, PartialEq, prost::Message)]
pub struct GatewayProperties {
    /// Channel discriminator: 22101 (global/CN), 22102 (Bilibili).
    #[prost(uint64, tag = "2")]
    pub properties_2: u64,
    #[prost(string, tag = "7")]
    pub server_name: String,
    #[prost(string, tag = "8")]
    pub repo_res_version_url: String,
    #[prost(string, tag = "9")]
    pub repo_design_data_url: String,
    /// `7.2_live` — compared by minor version, not literal equality.
    #[prost(string, tag = "12")]
    pub game_version: String,
    #[prost(uint64, tag = "14")]
    pub data_revision: u64,
    #[prost(uint64, tag = "18")]
    pub silence_revision: u64,
    #[prost(string, tag = "19")]
    pub data_descriptor: String,
    #[prost(string, tag = "20")]
    pub silence_descriptor: String,
    #[prost(message, optional, tag = "22")]
    pub res_version_properties: Option<RepoResVersionProperties>,
    #[prost(string, tag = "26")]
    pub data_hash: String,
    #[prost(string, tag = "27")]
    pub silence_hash: String,
    #[prost(string, tag = "36")]
    pub codename: String,
}

/// `RepoResVersionProperties` (properties tag 22).
#[derive(Clone, PartialEq, prost::Message)]
pub struct RepoResVersionProperties {
    #[prost(uint64, tag = "1")]
    pub res_revision: u64,
    #[prost(string, tag = "3")]
    pub res_descriptor: String,
    #[prost(string, tag = "5")]
    pub res_hash: String,
    #[prost(string, tag = "6")]
    pub version: String,
}

/// A protobuf scalar as seen by the generic walker. Only `Bytes` values are
/// interpreted; varints are kept for the channel discriminator check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireValue<'a> {
    Varint(u64),
    Fixed64(u64),
    Bytes(&'a [u8]),
    Fixed32(u32),
}

impl WireValue<'_> {
    fn opaque_len(&self) -> usize {
        match self {
            Self::Varint(_) => 1,
            Self::Fixed64(_) => 8,
            Self::Bytes(bytes) => bytes.len(),
            Self::Fixed32(_) => 4,
        }
    }
}

fn read_varint(bytes: &[u8]) -> Result<(u64, &[u8])> {
    let mut value = 0u64;
    for (index, byte) in bytes.iter().take(10).enumerate() {
        value |= u64::from(byte & 0x7f) << (7 * index);
        if byte & 0x80 == 0 {
            return Ok((value, &bytes[index + 1..]));
        }
    }
    Err(DispatchError::Proto("unterminated protobuf varint".into()))
}

/// Walk every field in a message without a schema. Unknown wire types are an
/// error so a corrupt body cannot be silently "decoded" into a wrong revision.
pub fn walk_fields(mut bytes: &[u8]) -> Result<Vec<(u32, WireValue<'_>)>> {
    let mut out = Vec::new();
    while !bytes.is_empty() {
        let (tag, rest) = read_varint(bytes)?;
        let field = u32::try_from(tag >> 3)
            .map_err(|_| DispatchError::Proto("protobuf field number overflow".into()))?;
        if field == 0 {
            return Err(DispatchError::Proto("protobuf field 0 is invalid".into()));
        }
        match tag & 7 {
            0 => {
                let (value, rest) = read_varint(rest)?;
                out.push((field, WireValue::Varint(value)));
                bytes = rest;
            }
            1 => {
                let (lo, rest) = rest.split_at_checked(8).ok_or_else(|| {
                    DispatchError::Proto(format!("truncated fixed64 for field {field}"))
                })?;
                let value =
                    u64::from_le_bytes(lo.try_into().map_err(|_| {
                        DispatchError::Proto(format!("bad fixed64 for field {field}"))
                    })?);
                out.push((field, WireValue::Fixed64(value)));
                bytes = rest;
            }
            2 => {
                let (len, rest) = read_varint(rest)?;
                let len = usize::try_from(len)
                    .map_err(|_| DispatchError::Proto(format!("field {field} length overflow")))?;
                let (payload, rest) = rest.split_at_checked(len).ok_or_else(|| {
                    DispatchError::Proto(format!("field {field} is truncated ({len} bytes)"))
                })?;
                out.push((field, WireValue::Bytes(payload)));
                bytes = rest;
            }
            5 => {
                let (lo, rest) = rest.split_at_checked(4).ok_or_else(|| {
                    DispatchError::Proto(format!("truncated fixed32 for field {field}"))
                })?;
                let value =
                    u32::from_le_bytes(lo.try_into().map_err(|_| {
                        DispatchError::Proto(format!("bad fixed32 for field {field}"))
                    })?);
                out.push((field, WireValue::Fixed32(value)));
                bytes = rest;
            }
            other => {
                return Err(DispatchError::Proto(format!(
                    "unsupported protobuf wire type {other} on field {field}"
                )));
            }
        }
    }
    Ok(out)
}

/// The fields the checked-in `.proto` does not name. Kept as `(tag, byte len)`
/// so they can be logged but never used for placement.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OpaqueFields {
    pub root: Vec<(u32, usize)>,
    pub properties: Vec<(u32, usize)>,
}

/// Decode the typed messages and collect the opaque fields.
pub fn parse_dispatch_protobuf(bytes: &[u8]) -> Result<(DispatchResponse, OpaqueFields)> {
    let response = DispatchResponse::decode(bytes)
        .map_err(|e| DispatchError::Proto(format!("gateway protobuf: {e}")))?;
    let root = walk_fields(bytes)?;
    let mut opaque = OpaqueFields::default();
    for (field, value) in &root {
        if matches!(field, 11 | 12 | 14) {
            opaque.root.push((*field, value.opaque_len()));
        }
    }
    if let Some((_, WireValue::Bytes(props))) = root.iter().find(|(field, _)| *field == 3) {
        let fields = walk_fields(props)?;
        for (field, value) in &fields {
            if (38..=47).contains(field) {
                opaque.properties.push((*field, value.opaque_len()));
            }
        }
    }
    if !opaque.root.is_empty() || !opaque.properties.is_empty() {
        tracing::debug!(
            ?opaque,
            "opaque dispatcher fields (never used for placement)"
        );
    }
    Ok((response, opaque))
}

/// One recognised line of a descriptor NDJSON body. Extra keys (`isPatch`,
/// `category`, `localName`, `hash`) are parsed-but-ignored rather than fatal.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct DescriptorRow {
    #[serde(rename = "remoteName")]
    pub remote_name: String,
    pub md5: String,
    #[serde(rename = "fileSize")]
    pub file_size: u64,
    #[serde(default)]
    pub hash: Option<String>,
}

/// The descriptor bodies carry several rows; the first non-blank row is the
/// full manifest the query path mirrors. A blank line is skipped (Collapse's
/// own parser throws on it).
pub fn parse_descriptor(text: &str) -> Result<DescriptorRow> {
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let row: DescriptorRow = serde_json::from_str(line)
            .map_err(|e| DispatchError::Proto(format!("bad descriptor row: {e}")))?;
        if Md5::try_parse_hex(&row.md5).is_none() {
            return Err(DispatchError::Proto(format!(
                "descriptor md5 is not 32 hex digits: {:?}",
                row.md5
            )));
        }
        if row.remote_name.is_empty() || row.file_size == 0 {
            return Err(DispatchError::Proto(
                "descriptor has no remoteName or fileSize".into(),
            ));
        }
        return Ok(row);
    }
    Err(DispatchError::Proto("empty descriptor body".into()))
}

/// Parse the second GET's `"<revision> <hash>"` body.
pub fn parse_base_revision(body: &str) -> Result<(u64, String)> {
    let body = body.trim();
    let (revision, hash) = body
        .split_once(' ')
        .ok_or_else(|| DispatchError::Metadata(format!("base_revision has no space: {body:?}")))?;
    let revision = revision.parse::<u64>().map_err(|_| {
        DispatchError::Metadata(format!("base_revision is not an integer: {revision:?}"))
    })?;
    if hash.is_empty()
        || hash.contains(char::is_whitespace)
        || !hash.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(DispatchError::Metadata(format!(
            "base_revision hash is not a hex token: {hash:?}"
        )));
    }
    Ok((revision, hash.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walker_reads_every_wire_type_and_rejects_a_group() {
        let bytes = [
            0x08, 0x96, 0x01, // 1 varint 150
            0x11, 1, 0, 0, 0, 0, 0, 0, 0, // 2 fixed64 1
            0x1a, 0x03, b'a', b'b', b'c', // 3 bytes
            0x25, 7, 0, 0, 0, // 4 fixed32 7
        ];
        let fields = walk_fields(&bytes).unwrap();
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[0], (1, WireValue::Varint(150)));
        assert_eq!(fields[1], (2, WireValue::Fixed64(1)));
        assert_eq!(fields[2], (3, WireValue::Bytes(b"abc")));
        assert_eq!(fields[3], (4, WireValue::Fixed32(7)));
        assert!(walk_fields(&[0x0b]).is_err()); // wire type 3 (group)
        assert!(walk_fields(&[0x80]).is_err()); // unterminated varint
        assert!(walk_fields(&[0x1a, 0x05, b'a']).is_err()); // truncated bytes
    }

    #[test]
    fn descriptor_skips_blank_lines_and_checks_md5() {
        let text = "\n{\"remoteName\":\"data_versions\",\"md5\":\"09f2c36615dc806f67f55e3d2c8a0c8d\",\"fileSize\":71416,\"hash\":\"a5c0e5e8d7899cd3\"}\n\n";
        let row = parse_descriptor(text).unwrap();
        assert_eq!(row.remote_name, "data_versions");
        assert_eq!(row.file_size, 71416);
        assert_eq!(row.hash.as_deref(), Some("a5c0e5e8d7899cd3"));
        assert!(parse_descriptor("\n\n").is_err());
        assert!(parse_descriptor("{\"remoteName\":\"x\",\"md5\":\"zz\",\"fileSize\":1}").is_err());
    }

    #[test]
    fn base_revision_body_is_two_tokens() {
        assert_eq!(
            parse_base_revision("49008628 644b3d946e\n").unwrap(),
            (49_008_628, "644b3d946e".to_owned())
        );
        assert!(parse_base_revision("49008628").is_err());
        assert!(parse_base_revision("49a 644b").is_err());
        assert!(parse_base_revision("49008628 644b3d946e extra").is_err());
    }
}
