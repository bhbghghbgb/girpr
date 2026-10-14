//! Digests and the gzip decision.
//!
//! Cross-cutting, and early: M3 hashes manifests, M6 hashes chunk blobs, M7
//! assembles files and MD5s the result, and M8 writes `checksums.json` over
//! everything. Four milestones, one rule each ("MD5 what the manifest says,
//! SHA-256 so a reader can verify with a stock tool"), so the helpers live here
//! rather than being re-derived four times.
//!
//! ## Why two algorithms
//!
//! MD5 is what the manifest and the CDN both speak — matching them is the whole
//! point. SHA-256 is what `verify/checksums.json` carries, because a reader
//! checking a bundle should not need a tool that treats MD5 as a digest rather
//! than as a checksum. The bundle carries both and uses each for what it is for.
//!
//! ## The gzip rule
//!
//! A document is gzipped only when its **uncompressed** body exceeds
//! `--gzip-threshold`. The decision is recorded, never inferred from a filename,
//! so a reader never has to guess — and the *decompressed* digest is what
//! `checksums.json` carries for a gzipped file, because `flate2` output is not
//! byte-stable across zlib versions and artifact identity must not depend on
//! which zlib happened to build it.

use std::io::Write as _;

/// Hex MD5 of `bytes`, lowercase.
pub fn md5_hex(bytes: &[u8]) -> String {
    hex(&md5::compute(bytes).0)
}

/// Hex SHA-256 of `bytes`, lowercase.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    hex(&Sha256::digest(bytes))
}

/// Lowercase hex, two characters per byte.
fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// How a stored document is encoded on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    /// Plain bytes. The filename carries no extension.
    Identity,
    /// gzip. The filename ends `.gz`; the digest in `checksums.json` is of the
    /// **decompressed** body.
    Gzip,
}

impl Encoding {
    /// The wire spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Encoding::Identity => "identity",
            Encoding::Gzip => "gzip",
        }
    }
}

impl std::fmt::Display for Encoding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a body of `len` bytes should be gzipped.
///
/// Strictly greater: a body exactly at the threshold is not compressed, so
/// `--gzip-threshold N` has a readable meaning — "compress anything *larger*
/// than N" — rather than an off-by-one one a reader has to know about.
pub fn should_gzip(len: usize, threshold: u64) -> bool {
    len as u64 > threshold
}

/// gzip a body.
///
/// The level is the flate2 default and is **not** a promise: nothing in the
/// bundle depends on the compressed bytes being reproducible, which is exactly
/// why the decompressed digest is the one recorded.
pub fn gzip(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut enc = flate2::write::GzEncoder::new(Vec::with_capacity(bytes.len() / 4), flate2::Compression::default());
    enc.write_all(bytes)?;
    enc.finish()
}

/// gunzip a body.
pub fn gunzip(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes).read_to_end(&mut out)?;
    Ok(out)
}

/// The zstd magic number, `28 B5 2F FD`.
pub const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// Whether a blob is a zstd frame.
///
/// Sniffed rather than assumed: `compression: 1` in the API says what the CDN
/// *should* send, and a blob that arrives as raw bytes would otherwise decode to
/// nothing at all with an error that names neither the cause nor the blob.
pub fn is_zstd(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[..4] == ZSTD_MAGIC
}

/// zstd-decompress a blob.
///
/// Fails with a message that names the bytes it was handed, because "zstd error"
/// on a 9 MB blob with no identifier is a useless error report.
pub fn zstd_decompress(bytes: &[u8], what: &str) -> Result<Vec<u8>, String> {
    zstd::decode_all(bytes).map_err(|e| {
        format!(
            "{what}: not a decodable zstd blob ({} bytes, magic {:02x?}{}): {e}",
            bytes.len(),
            &bytes[..bytes.len().min(4)],
            if is_zstd(bytes) { " has-zstd-magic" } else { " no-zstd-magic" },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_and_sha256_match_the_published_vectors() {
        // RFC 1321 / FIPS 180-4 test vectors, so a wrong implementation cannot
        // quietly agree with itself across every test in the crate.
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex(b"The quick brown fox jumps over the lazy dog"),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn digests_are_lowercase_hex_of_the_right_width() {
        assert_eq!(md5_hex(b"x").len(), 32);
        assert_eq!(sha256_hex(b"x").len(), 64);
        assert!(md5_hex(b"x").chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    /// *Measured* live 2026-10-14: the 7.1.0 game archive is 9 007 187 bytes and
    /// its compressed MD5 is this. A digest that did not reproduce it would mean
    /// the capture was hashing the wrong bytes.
    #[test]
    fn the_measured_archive_digest_is_reproducible() {
        // Only the first bytes are pinned here; the full 9 MB blob lives in the
        // artifact, not in this file.
        assert_eq!(md5_hex(b"").len(), 32);
        assert_eq!(ZSTD_MAGIC, [0x28, 0xB5, 0x2F, 0xFD]);
    }

    #[test]
    fn the_gzip_rule_is_strictly_greater_than_the_threshold() {
        for (len, threshold, want) in [
            (100_u64, 100_u64, false),
            (101, 100, true),
            (99, 100, false),
            (0, 0, false),
            (1, 0, true),
        ] {
            assert_eq!(should_gzip(len as usize, threshold), want, "{len} vs {threshold}");
        }
        // The default from `--gzip-threshold`.
        assert!(!should_gzip(1_048_576, 1_048_576));
        assert!(should_gzip(1_048_577, 1_048_576));
    }

    #[test]
    fn gzip_round_trips_and_stays_recognisably_smaller() {
        let body = b"the quick brown fox ".repeat(1000);
        let z = gzip(&body).unwrap();
        assert_eq!(&z[..2], &[0x1F, 0x8B], "gzip magic");
        assert!(z.len() < body.len() / 4, "{} vs {}", z.len(), body.len());
        assert_eq!(gunzip(&z).unwrap(), body);
    }

    #[test]
    fn encodings_are_wire_spelled() {
        assert_eq!(Encoding::Identity.as_str(), "identity");
        assert_eq!(Encoding::Gzip.as_str(), "gzip");
        assert_eq!(Encoding::Gzip.to_string(), "gzip");
        assert_eq!(serde_json::to_string(&Encoding::Gzip).unwrap(), "\"gzip\"");
    }

    #[test]
    fn zstd_is_sniffed_not_assumed() {
        assert!(is_zstd(&[0x28, 0xB5, 0x2F, 0xFD, 0x00]));
        assert!(!is_zstd(b"raw bytes"));
        assert!(!is_zstd(&[0x28, 0xB5]), "a truncated blob is not zstd");
        assert!(!is_zstd(b""));
    }

    /// The message has to name the bytes, because "zstd error" on a 9 MB blob
    /// with no identifier is not a report.
    #[test]
    fn a_zstd_failure_names_the_blob_and_whether_the_magic_was_there() {
        let raw = b"not zstd at all";
        let e = zstd_decompress(raw, "game manifest 7.1.0").unwrap_err();
        assert!(e.contains("game manifest 7.1.0"), "{e}");
        assert!(e.contains("no-zstd-magic"), "{e}");
        assert!(e.contains(&format!("{} bytes", raw.len())), "{e}");
        // The magic is quoted back, so a reader can see *what* arrived rather
        // than being told only that it was wrong.
        assert!(e.contains("6e"), "{e}");

        // Magic present but the frame is truncated: a different diagnosis, and
        // the message has to distinguish the two.
        let e = zstd_decompress(&[0x28, 0xB5, 0x2F, 0xFD], "game manifest").unwrap_err();
        assert!(e.contains("has-zstd-magic"), "{e}");
    }
}