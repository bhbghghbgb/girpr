//! The binary wire codec: [`FileRec`] and [`Meta`] to bytes and back, via
//! `postcard`.
//!
//! No JSON, no hex, and no fixed id registry for algorithm names — so the bytes
//! are small, deterministic, and forward compatible.
//!
//! Entry value layout:
//! ```text
//! [u8 fmt][ postcard FileRec: kind, size, mtime_ns, hashes ]
//! ```
//! `postcard` writes integers as varints and struct fields positionally (no
//! field names), and [`crate::hash::DigestSet`]'s serde impl writes `hashes` as
//! a sequence of inline `(name, digest)` pairs — known algorithms in canonical
//! order, then foreign names sorted by name. The format marker lets a foreign
//! or older record be refused before decoding, and `Meta` is likewise a
//! positional two-field `postcard` struct: `[version][case_sensitive]`.
//!
//! Algorithm names stay inline rather than behind an id registry, so a cache
//! written with a new algorithm stays readable by older binaries: an unknown
//! name decodes into the set and is carried, not dropped.

use anyhow::{Context, Result, bail};

use super::schema::{FileRec, Meta};

/// The record format marker, bumped when the `postcard` layout changes. A
/// different value is refused rather than decoded under the wrong assumptions.
const FMT_V2: u8 = 2;

/// Encode one record: the format marker, then the positional fields.
pub(super) fn encode_rec(rec: &FileRec) -> Result<Vec<u8>> {
    let mut v = Vec::with_capacity(19 + rec.hashes.len() * 24);
    v.push(FMT_V2);
    // `to_extend` appends to the Vec, so the marker stays at byte zero and the
    // serde fields follow it in declaration order.
    postcard::to_extend(rec, v).context("encode cache record")
}

/// Decode one record, refusing a foreign format, truncation and trailing bytes.
pub(super) fn decode_rec(bytes: &[u8]) -> Result<FileRec> {
    let (fmt, rest) = bytes.split_first().context("truncated cache entry")?;
    if *fmt != FMT_V2 {
        bail!("unsupported entry format {fmt}");
    }
    let (rec, trailing) = postcard::take_from_bytes(rest).context("decode cache record")?;
    if !trailing.is_empty() {
        bail!("trailing bytes in cache entry");
    }
    Ok(rec)
}

/// Encode the `meta` row. Serializing these two fixed fields into a growable
/// `Vec` cannot fail.
#[allow(clippy::expect_used)] // serializing two fixed fields into a Vec cannot fail
pub(super) fn encode_meta(m: &Meta) -> Vec<u8> {
    postcard::to_extend(m, Vec::with_capacity(8)).expect("serialize cache meta")
}

/// Decode the `meta` row, refusing truncation and trailing bytes.
pub(super) fn decode_meta(bytes: &[u8]) -> Result<Meta> {
    let (m, trailing) = postcard::take_from_bytes(bytes).context("decode cache meta")?;
    if !trailing.is_empty() {
        bail!("corrupt cache meta ({} trailing bytes)", trailing.len());
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::{DigestSet, HashAlg};
    use crate::schema::CACHE_VERSION;

    /// One digest per name: `byte` repeated to that algorithm's length, or to 32
    /// for a name this build does not know.
    fn digests(pairs: &[(&str, u8)]) -> DigestSet {
        let mut set = DigestSet::default();
        for (name, byte) in pairs {
            let n = name
                .parse::<HashAlg>()
                .map(|a| a.digest_len())
                .unwrap_or(32);
            set.insert_named(name, &vec![*byte; n]);
        }
        set
    }

    fn file_rec() -> FileRec {
        FileRec {
            kind: "file".into(),
            size: 42,
            mtime_ns: 123456789,
            hashes: digests(&[("md5", 1), ("sha256", 2)]),
        }
    }

    /// Byte offset of the first occurrence of `needle` in `hay`.
    fn find(hay: &[u8], needle: &[u8]) -> usize {
        hay.windows(needle.len())
            .position(|w| w == needle)
            .unwrap_or_else(|| panic!("{needle:?} not found"))
    }

    #[test]
    fn codec_roundtrips_file_with_hashes() {
        let rec = file_rec();
        let back = decode_rec(&encode_rec(&rec).unwrap()).unwrap();
        assert_eq!(rec, back);
    }

    #[test]
    fn codec_roundtrips_dir_without_hashes() {
        let rec = FileRec::dir();
        let back = decode_rec(&encode_rec(&rec).unwrap()).unwrap();
        assert_eq!(rec, back);
    }

    /// **The wire format is a contract between builds**, so it is pinned as
    /// literal bytes rather than as a round trip — a round trip passes for any
    /// two encodings that agree with each other, which is not the property that
    /// matters here. The expected bytes are built from the same positional
    /// shape a `postcard` struct has: the marker, the fixed fields, and a
    /// map-like sequence of `(name, digest)` pairs.
    #[test]
    fn the_wire_format_is_byte_identical_to_what_a_map_produced() {
        let md5 = [1u8; 16];
        let sha256 = [2u8; 32];
        let mut want = vec![FMT_V2];
        want = postcard::to_extend(
            &(
                "file",
                42u64,
                123_456_789i64,
                vec![("md5", &md5[..]), ("sha256", &sha256[..])],
            ),
            want,
        )
        .unwrap();
        assert_eq!(encode_rec(&file_rec()).unwrap(), want);
    }

    /// An unknown name is carried, and it lands **after** the known ones rather
    /// than interleaving them — the one place this layout reorders entries. The
    /// meaning does not change, the bytes stay deterministic, and nothing
    /// compares encoded values for equality; this says so out loud rather than
    /// leaving a reader to wonder whether the reordering was intended.
    #[test]
    fn an_unknown_name_is_carried_and_lands_after_the_known_ones() {
        let mut rec = file_rec();
        rec.hashes.insert_named("blake3", &[7u8; 32]);
        let bytes = encode_rec(&rec).unwrap();

        // The pair-count varint sits right after the fixed fields and counts
        // every digest, foreign ones included.
        let mut prefix = vec![FMT_V2];
        prefix = postcard::to_extend(&("file", 42u64, 123_456_789i64), prefix).unwrap();
        assert_eq!(
            bytes[prefix.len()],
            3,
            "the count is every digest, foreign included"
        );

        let back = decode_rec(&bytes).unwrap();
        assert_eq!(back.hashes.get_named("blake3"), Some(&[7u8; 32][..]));
        assert_eq!(
            back.hashes.get(HashAlg::Md5),
            Some(&[1u8; 16][..]),
            "and the known ones are still known ones"
        );

        let (md5_at, sha_at, blake_at) = (
            find(&bytes, b"md5"),
            find(&bytes, b"sha256"),
            find(&bytes, b"blake3"),
        );
        assert!(
            md5_at < sha_at && sha_at < blake_at,
            "canonical order first, then foreign: {md5_at} {sha_at} {blake_at}"
        );
    }

    /// **A decoded row holds no room beyond its entries.** A vector grows in
    /// steps, so a row that carries one unknown name would hold room for
    /// four — and the rows a record keeps are decoded rows, so the room is
    /// worth giving back where the row is finished.
    #[test]
    fn a_decoded_row_holds_no_spare_capacity() {
        let mut rec = file_rec();
        rec.hashes.insert_named("blake3", &[7u8; 32]);
        let back = decode_rec(&encode_rec(&rec).unwrap()).unwrap();
        assert_eq!(
            back.hashes.foreign_capacity(),
            back.hashes.foreign_iter().count(),
            "the decoded row holds no room beyond its entries"
        );
        assert_eq!(
            back.hashes.get_named("blake3"),
            Some(&[7u8; 32][..]),
            "and the name it was carried under is still there"
        );
    }

    #[test]
    fn codec_preserves_unknown_algorithms() {
        // Forward compat: a future `--hash-all-of blake3` row must survive a
        // roundtrip through this binary, which never drops opaque entries.
        let mut rec = FileRec {
            kind: "file".into(),
            size: 7,
            mtime_ns: 9,
            hashes: digests(&[("blake3", 7)]),
        };
        rec.hashes.insert_named("md5", &[3u8; 16]);
        let back = decode_rec(&encode_rec(&rec).unwrap()).unwrap();
        assert_eq!(rec, back);
    }

    /// The record's `kind` is a closed registry: an unknown spelling is refused,
    /// the opposite of an algorithm name. This pins that asymmetry across the
    /// switch to a serde-derived record.
    #[test]
    fn a_closed_record_kind_is_rejected() {
        let mut bytes = vec![FMT_V2];
        bytes = postcard::to_extend(&("other", 0u64, 0i64, Vec::<(&str, &[u8])>::new()), bytes)
            .unwrap();
        assert!(decode_rec(&bytes).is_err());
    }

    #[test]
    fn codec_rejects_truncated_and_bad_version() {
        let good = encode_rec(&file_rec()).unwrap();
        assert!(decode_rec(&good[..good.len() - 2]).is_err());
        let mut bad = good.clone();
        bad[0] = 99;
        assert!(decode_rec(&bad).is_err());
        // A record with no payload, or a marker left over from an older format,
        // is refused rather than decoded under the new assumptions.
        assert!(decode_rec(&[FMT_V2]).is_err());
        assert!(decode_rec(&[1u8]).is_err());
    }

    #[test]
    fn meta_codec_roundtrips() {
        for cs in [true, false] {
            let m = Meta {
                version: CACHE_VERSION,
                case_sensitive: cs,
            };
            let back = decode_meta(&encode_meta(&m)).unwrap();
            assert_eq!(back.version, CACHE_VERSION);
            assert_eq!(back.case_sensitive, cs);
        }
    }

    #[test]
    fn meta_codec_rejects_truncation_and_trailing_bytes() {
        let good = encode_meta(&Meta {
            version: CACHE_VERSION,
            case_sensitive: true,
        });
        assert!(decode_meta(&good[..good.len() - 1]).is_err());
        let mut extra = good.clone();
        extra.push(0);
        assert!(decode_meta(&extra).is_err());
    }
}
