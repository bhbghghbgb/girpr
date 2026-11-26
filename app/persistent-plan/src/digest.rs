//! The digest gate, as a pure decision over values.
//!
//! Collapse prefers the manifest's XXH64 `hash` over `md5`, and
//! girpr checks the **size first**: a short body is a size failure even when a
//! digest happens to agree. The same verdict is used for the download gate and
//! for the mandatory re-read after the atomic publish, so the two cannot drift.
//!
//! The caller supplies the observed size and digests from its streaming sink;
//! [`verify_bytes`] is the convenience form for in-memory bytes (manifests and
//! unit tests).

use girpr_digest::{Md5, md5_hex, xxhash64_hex};

use crate::value::PersistentRow;

/// Which digest a row requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpectedDigest {
    /// XXH64 (seed 0) — the `hash` field wins when present.
    Xxh64(u64),
    /// MD5 fallback.
    Md5(String),
}

/// What a byte stream actually produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedDigest {
    pub size: u64,
    pub md5: Md5,
    pub xxhash64: u64,
}

impl ObservedDigest {
    /// Hash in-memory bytes with both algorithms.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self {
            size: bytes.len() as u64,
            md5: Md5::try_parse_hex(&md5_hex(bytes)).unwrap_or(Md5::EMPTY),
            xxhash64: xxhash64_u64(bytes),
        }
    }

    /// Build the observation from a streaming sink's four values.
    pub fn of_stream(size: u64, md5: Md5, xxhash64: u64) -> Self {
        Self {
            size,
            md5,
            xxhash64,
        }
    }
}

/// `hash` wins over `md5`, exactly as Collapse's HTTP path does.
pub fn expected_digest(row: &PersistentRow) -> ExpectedDigest {
    match row.hash {
        Some(hash) => ExpectedDigest::Xxh64(hash),
        None => ExpectedDigest::Md5(row.md5.clone()),
    }
}

/// The result of a size-first verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DigestVerdict {
    Ok,
    SizeMismatch {
        expected: u64,
        actual: u64,
    },
    Mismatch {
        algorithm: &'static str,
        expected: String,
        actual: String,
    },
}

impl DigestVerdict {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }
}

impl std::fmt::Display for DigestVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ok => write!(f, "digest ok"),
            Self::SizeMismatch { expected, actual } => {
                write!(f, "size mismatch: expected {expected}, got {actual}")
            }
            Self::Mismatch {
                algorithm,
                expected,
                actual,
            } => write!(f, "{algorithm} mismatch: expected {expected}, got {actual}"),
        }
    }
}

/// Size first, then the preferred digest. The other digest is never consulted
/// when `hash` is present: a row that carries XXH64 does not also require MD5
/// to agree (the two are recorded together but the server can spell either one
/// from a different byte set).
pub fn verify_observed(row: &PersistentRow, observed: &ObservedDigest) -> DigestVerdict {
    if observed.size != row.file_size {
        return DigestVerdict::SizeMismatch {
            expected: row.file_size,
            actual: observed.size,
        };
    }
    match expected_digest(row) {
        ExpectedDigest::Xxh64(want) => {
            if observed.xxhash64 == want {
                DigestVerdict::Ok
            } else {
                DigestVerdict::Mismatch {
                    algorithm: "xxh64",
                    expected: format!("{want:016x}"),
                    actual: format!("{:016x}", observed.xxhash64),
                }
            }
        }
        ExpectedDigest::Md5(want) => {
            if observed.md5.matches_hex(&want) {
                DigestVerdict::Ok
            } else {
                DigestVerdict::Mismatch {
                    algorithm: "md5",
                    expected: want,
                    actual: observed.md5.to_hex(),
                }
            }
        }
    }
}

/// Verify in-memory bytes against a row.
pub fn verify_bytes(row: &PersistentRow, bytes: &[u8]) -> DigestVerdict {
    verify_observed(row, &ObservedDigest::of_bytes(bytes))
}

/// The numeric XXH64 a hex spelling denotes. `xxhash64_hex` emits exactly 16
/// hex digits, so the parse cannot fail for our own output; a zero fallback is
/// only there to avoid a panic in a leaf with no error policy of its own.
fn xxhash64_u64(bytes: &[u8]) -> u64 {
    u64::from_str_radix(&xxhash64_hex(bytes), 16).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOB: &[u8] =
        include_bytes!("../../../tests/fixtures/persistent_inputs/samples/fixtures/hotfix_a.blk");

    fn row(hash: Option<u64>, md5: &str, size: u64) -> PersistentRow {
        PersistentRow {
            remote_name: "blocks/00/11111111.blk".into(),
            md5: md5.into(),
            hash,
            file_size: size,
        }
    }

    #[test]
    fn hash_is_preferred_over_md5() {
        let good_hash = 0xf7c6_7301_db67_13f0;
        let good_md5 = "b2d3f56bc197fd985d5965079b5e7148";
        // Both digests agree with the blob: either rule passes.
        assert_eq!(
            expected_digest(&row(Some(good_hash), good_md5, 64)),
            ExpectedDigest::Xxh64(good_hash)
        );
        assert_eq!(
            verify_bytes(&row(Some(good_hash), good_md5, 64), BLOB),
            DigestVerdict::Ok
        );
        assert_eq!(
            verify_bytes(&row(None, good_md5, 64), BLOB),
            DigestVerdict::Ok
        );

        // With a hash present, a wrong MD5 is ignored; with no hash, the MD5 is
        // the only gate. This is the Collapse preference, not a belt-and-braces
        // "both must match" check.
        assert_eq!(
            verify_bytes(
                &row(Some(good_hash), "00000000000000000000000000000000", 64),
                BLOB
            ),
            DigestVerdict::Ok
        );
        assert_eq!(
            verify_bytes(&row(None, "00000000000000000000000000000000", 64), BLOB),
            DigestVerdict::Mismatch {
                algorithm: "md5",
                expected: "00000000000000000000000000000000".into(),
                actual: good_md5.into(),
            }
        );
        assert_eq!(
            verify_bytes(&row(Some(1), good_md5, 64), BLOB),
            DigestVerdict::Mismatch {
                algorithm: "xxh64",
                expected: "0000000000000001".into(),
                actual: "f7c67301db6713f0".into(),
            }
        );
    }

    #[test]
    fn size_is_checked_before_either_digest() {
        let good_hash = Some(0xf7c6_7301_db67_13f0);
        let verdict = verify_bytes(
            &row(good_hash, "b2d3f56bc197fd985d5965079b5e7148", 65),
            BLOB,
        );
        assert_eq!(
            verdict,
            DigestVerdict::SizeMismatch {
                expected: 65,
                actual: 64
            }
        );
        assert!(!verdict.is_ok());
        // A row whose MD5 is malformed still reports the size failure first.
        assert!(matches!(
            verify_bytes(&row(None, "not a digest", 65), BLOB),
            DigestVerdict::SizeMismatch { .. }
        ));
    }

    #[test]
    fn observed_stream_and_in_memory_bytes_agree() {
        let observed = ObservedDigest::of_bytes(BLOB);
        let from_stream = ObservedDigest::of_stream(64, observed.md5, observed.xxhash64);
        assert_eq!(observed, from_stream);
        assert_eq!(observed.xxhash64, 0xf7c6_7301_db67_13f0);
    }
}
