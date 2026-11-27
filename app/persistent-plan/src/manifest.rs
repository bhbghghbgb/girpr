//! Pure NDJSON row parsing.
//!
//! The wire shape is Collapse's persistent manifest: one JSON object per line,
//! with `remoteName`, `md5`, `fileSize` and an optional XXH64 `hash`. Extra keys
//! (`isPatch`, `category`, `localName`, …) are parsed-but-ignored — the server
//! does send `isPatch` on some rows — and **blank lines are skipped**, unlike
//! Collapse's own parser, which throws on them.
//!
//! Nothing here touches the network or the filesystem: the fetch module
//! supplies verified bytes, and this module turns them into values or a typed
//! [`ParseError`] naming the offending line.

use girpr_digest::{Md5, parse_xxhash64_hex};
use girpr_line_reader::{BomPolicy, non_blank_lines};

use crate::value::PersistentRow;

/// A malformed manifest body or row. Never an exit code — the caller maps it
/// onto its own failure envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The body was not UTF-8.
    NotUtf8(String),
    /// Every line was blank.
    Empty,
    /// A non-blank line was not a valid recognised row.
    Row { line: usize, message: String },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotUtf8(s) => write!(f, "manifest is not UTF-8: {s}"),
            Self::Empty => write!(f, "manifest has no rows"),
            Self::Row { line, message } => write!(f, "manifest line {line}: {message}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// The recognised keys. Serde ignores any key not named here, which is the
/// parsed-but-ignored rule for the server's extra fields.
#[derive(serde::Deserialize)]
struct WireRow {
    #[serde(rename = "remoteName")]
    remote_name: String,
    md5: String,
    #[serde(rename = "fileSize")]
    file_size: u64,
    #[serde(default)]
    hash: Option<String>,
}

/// Parse every non-blank line, returning a typed error for the first bad one.
pub fn parse_rows(bytes: &[u8]) -> Result<Vec<PersistentRow>, ParseError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ParseError::NotUtf8(e.to_string()))?;
    let mut rows = Vec::new();
    // The manifest wire shape has no BOM convention, so `Keep` is deliberate:
    // a leading U+FEFF is part of the first row and fails it, as before.
    for (line_number, line) in non_blank_lines(text, BomPolicy::Keep) {
        let line = line.trim();
        let wire: WireRow = serde_json::from_str(line).map_err(|e| ParseError::Row {
            line: line_number,
            message: format!("invalid JSON row: {e}"),
        })?;
        let row = into_row(wire, line_number)?;
        rows.push(row);
    }
    if rows.is_empty() {
        return Err(ParseError::Empty);
    }
    Ok(rows)
}

fn into_row(wire: WireRow, line: usize) -> Result<PersistentRow, ParseError> {
    let bad = |message: String| ParseError::Row { line, message };
    if wire.remote_name.is_empty() {
        return Err(bad("empty remoteName".into()));
    }
    if Md5::try_parse_hex(&wire.md5).is_none() {
        return Err(bad(format!("md5 is not 32 hex digits: {:?}", wire.md5)));
    }
    if wire.file_size == 0 {
        return Err(bad("fileSize is zero".into()));
    }
    let hash = match wire.hash {
        Some(hex) => Some(parse_hash_hex(&hex).map_err(bad)?),
        None => None,
    };
    Ok(PersistentRow {
        remote_name: wire.remote_name,
        md5: wire.md5,
        hash,
        file_size: wire.file_size,
    })
}

/// The manifest spells XXH64 as 16 hex digits: the big-endian bytes of the
/// 64-bit value, the same spelling `girpr_digest::xxhash64_hex` emits. The leaf
/// parses either case and enforces the width; this adapter keeps the manifest's
/// own error wording.
fn parse_hash_hex(text: &str) -> Result<u64, String> {
    parse_xxhash64_hex(text).map_err(|_| format!("hash is not 16 hex digits: {text:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_lines_are_skipped_and_extra_keys_are_ignored() {
        // The shipped synthetic fixture's shape: a blank line in the middle,
        // one extra key per row. No production bytes are used.
        let bytes = b"\n{\"remoteName\":\"blocks/00/11111111.blk\",\"md5\":\"b2d3f56bc197fd985d5965079b5e7148\",\"hash\":\"f7c67301db6713f0\",\"fileSize\":64,\"isPatch\":true}\n   \n{\"remoteName\":\"English(US)/VO_TEST.pck\",\"md5\":\"5117f4269ca5456f8dd0e47ddd3e0832\",\"fileSize\":48,\"category\":\"Audio\",\"localName\":\"x\"}\n";
        let rows = parse_rows(bytes).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].remote_name, "blocks/00/11111111.blk");
        assert_eq!(rows[0].hash, Some(0xf7c6_7301_db67_13f0));
        assert_eq!(rows[1].remote_name, "English(US)/VO_TEST.pck");
        assert_eq!(rows[1].hash, None);
        assert_eq!(rows[1].md5, "5117f4269ca5456f8dd0e47ddd3e0832");
    }

    #[test]
    fn upper_case_hex_and_md5_are_accepted() {
        let bytes = b"{\"remoteName\":\"a.blk\",\"md5\":\"B2D3F56BC197FD985D5965079B5E7148\",\"hash\":\"F7C67301DB6713F0\",\"fileSize\":64}";
        let rows = parse_rows(bytes).unwrap();
        assert_eq!(rows[0].hash, Some(0xf7c6_7301_db67_13f0));
        assert_eq!(rows[0].md5, "B2D3F56BC197FD985D5965079B5E7148");
    }

    #[test]
    fn malformed_rows_produce_a_typed_error_with_the_line_number() {
        let cases: &[(&str, &str)] = &[
            ("not json", "invalid JSON row"),
            (
                "{\"remoteName\":\"\",\"md5\":\"00\",\"fileSize\":1}",
                "empty remoteName",
            ),
            (
                "{\"remoteName\":\"a\",\"md5\":\"zz\",\"fileSize\":1}",
                "md5 is not 32 hex digits",
            ),
            (
                "{\"remoteName\":\"a\",\"md5\":\"d41d8cd98f00b204e9800998ecf8427e\",\"fileSize\":0}",
                "fileSize is zero",
            ),
            (
                "{\"remoteName\":\"a\",\"md5\":\"d41d8cd98f00b204e9800998ecf8427e\",\"fileSize\":1,\"hash\":\"xyz\"}",
                "hash is not 16 hex digits",
            ),
            (
                "{\"remoteName\":\"a\",\"md5\":\"d41d8cd98f00b204e9800998ecf8427e\"}",
                "invalid JSON row",
            ),
        ];
        for (body, fragment) in cases {
            let body = format!("\n{body}\n");
            let err = parse_rows(body.as_bytes()).unwrap_err();
            assert_eq!(
                err,
                ParseError::Row {
                    line: 2,
                    message: match &err {
                        ParseError::Row { message, .. } => message.clone(),
                        _ => String::new(),
                    }
                },
                "the error carries the 1-based line: {body}"
            );
            assert!(err.to_string().contains("manifest line 2"), "{err}");
            assert!(
                err.to_string().contains(fragment),
                "expected {fragment:?}: {err}"
            );
        }
    }

    #[test]
    fn an_all_blank_or_non_utf8_body_is_rejected() {
        assert_eq!(parse_rows(b"\n\n  \n").unwrap_err(), ParseError::Empty);
        assert!(matches!(
            parse_rows(&[0xff, 0xfe]).unwrap_err(),
            ParseError::NotUtf8(_)
        ));
    }
}
