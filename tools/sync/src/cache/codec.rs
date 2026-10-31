//! The binary codec: [`FileRec`] and [`Meta`] to bytes and back.
//!
//! No JSON, no hex, and no fixed id registry for algorithm names — so the bytes
//! are small, deterministic, and forward compatible.
//!
//! Entry value layout (all integers little-endian):
//! ```text
//! [u8 fmt=1][u8 kind: 0=file, 1=dir][u64 size][i64 mtime_ns]
//! [u8 n_hashes][ per hash: u8 name_len, name bytes, u8 digest_len, digest ]
//! ```
//!
//! Algorithm names are stored inline (not a fixed id registry), so a cache
//! written with a new algorithm stays readable by older binaries: unknown
//! names decode into the map and are simply ignored unless requested.
//! Meta value: `[u32 version][u8 case_sensitive]`.

use anyhow::{Context, Result, bail};

use crate::hash::DigestSet;

use super::schema::{FileRec, Meta};

const FMT_V1: u8 = 1;
const KIND_FILE: u8 = 0;
const KIND_DIR: u8 = 1;

fn kind_to_byte(kind: &str) -> Result<u8> {
    match kind {
        "file" => Ok(KIND_FILE),
        "dir" => Ok(KIND_DIR),
        other => bail!("invalid record kind {other:?}"),
    }
}

/// A closed registry, so an unknown value is an error.
///
/// **This is the opposite of what an algorithm name gets, and the difference is
/// deliberate.** A third `kind` would break older binaries that cannot represent it,
/// so refusing it loudly is correct. An unknown *algorithm* must not be refused at
/// all — see [`decode_rec`] — because a row written by a newer build has to stay
/// readable here. Do not "fix" the asymmetry.
fn kind_from_byte(b: u8) -> Result<String> {
    match b {
        KIND_FILE => Ok("file".into()),
        KIND_DIR => Ok("dir".into()),
        other => bail!("invalid record kind byte {other}"),
    }
}

pub(super) fn encode_rec(rec: &FileRec) -> Result<Vec<u8>> {
    let mut v = Vec::with_capacity(19 + rec.hashes.len() * 24);
    v.push(FMT_V1);
    v.push(kind_to_byte(&rec.kind)?);
    v.extend_from_slice(&rec.size.to_le_bytes());
    v.extend_from_slice(&rec.mtime_ns.to_le_bytes());
    if rec.hashes.len() > 255 {
        bail!("too many hashes for {}", rec.hashes.len());
    }
    // Known algorithms in canonical order, then foreign names sorted by name.
    //
    // Both halves are deterministic, which is the point: the bytes of a row are
    // reproducible for the same content. Known algorithms are emitted in canonical
    // order (`md5`, `sha256`), which is also alphabetical order. A row carrying an
    // unknown name moves that name after the known ones instead of interleaving it
    // — the meaning does not change, and nothing compares encoded values for
    // equality (`CacheWrite::put` only inserts).
    let mut foreign: Vec<(&str, &[u8])> = rec.hashes.foreign_iter().collect();
    foreign.sort_by_key(|(n, _)| *n);
    v.push(rec.hashes.len() as u8);
    for (algo, digest) in rec.hashes.iter() {
        push_hash(&mut v, algo.as_str(), digest)?;
    }
    for (name, digest) in foreign {
        push_hash(&mut v, name, digest)?;
    }
    Ok(v)
}

/// One `name, digest` pair, length-prefixed.
fn push_hash(v: &mut Vec<u8>, name: &str, digest: &[u8]) -> Result<()> {
    if name.len() > 255 || digest.len() > 255 {
        bail!("hash entry too large for {name:?}");
    }
    v.push(name.len() as u8);
    v.extend_from_slice(name.as_bytes());
    v.push(digest.len() as u8);
    v.extend_from_slice(digest);
    Ok(())
}

pub(super) fn decode_rec(bytes: &[u8]) -> Result<FileRec> {
    let mut cur = bytes;
    // Borrow each field from `bytes`; only the destinations that own data copy.
    fn take<'a>(cur: &mut &'a [u8], n: usize) -> Result<&'a [u8]> {
        if cur.len() < n {
            bail!("truncated cache entry");
        }
        let (head, tail) = cur.split_at(n);
        *cur = tail;
        Ok(head)
    }
    let fmt = take(&mut cur, 1)?[0];
    if fmt != FMT_V1 {
        bail!("unsupported entry format {fmt}");
    }
    let kind = kind_from_byte(take(&mut cur, 1)?[0])?;
    let size = u64::from_le_bytes(take(&mut cur, 8)?.try_into().unwrap());
    let mtime_ns = i64::from_le_bytes(take(&mut cur, 8)?.try_into().unwrap());
    let n = take(&mut cur, 1)?[0] as usize;
    let mut hashes = DigestSet::default();
    for _ in 0..n {
        let nl = take(&mut cur, 1)?[0] as usize;
        let name = std::str::from_utf8(take(&mut cur, nl)?).context("non-UTF8 algo name")?;
        let dl = take(&mut cur, 1)?[0] as usize;
        let digest = take(&mut cur, dl)?;
        // **Never `bail!` on an algorithm name.** This is the whole forward-compat
        // guarantee: a row written by a build that knows `blake3` has to stay
        // readable by this one, so an unrecognised name decodes and is carried. The
        // routing is `DigestSet::insert_named`'s, and it is the only place that rule
        // is written — including the case of a *known* name with an unexpected length,
        // which is carried rather than refused because that is what a map did.
        hashes.insert_named(name, digest);
    }
    if !cur.is_empty() {
        bail!("trailing bytes in cache entry");
    }
    Ok(FileRec {
        kind,
        size,
        mtime_ns,
        hashes,
    })
}

pub(super) fn encode_meta(m: &Meta) -> Vec<u8> {
    let mut v = Vec::with_capacity(5);
    v.extend_from_slice(&m.version.to_le_bytes());
    v.push(u8::from(m.case_sensitive));
    v
}

pub(super) fn decode_meta(bytes: &[u8]) -> Result<Meta> {
    if bytes.len() != 5 {
        bail!("corrupt cache meta (len {})", bytes.len());
    }
    let version = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    Ok(Meta {
        version,
        case_sensitive: bytes[4] != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::schema::CACHE_VERSION;
    use crate::hash::HashAlg;

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
    /// matters here.
    ///
    /// Names are sorted, which for `md5` and `sha256` is also canonical order, so
    /// the layouts coincide and **every cache written by an earlier build stays
    /// readable**. If this ever changes, that stops being true, and nothing else
    /// in the suite would notice.
    #[test]
    fn the_wire_format_is_byte_identical_to_what_a_map_produced() {
        let mut want: Vec<u8> = vec![FMT_V1, KIND_FILE];
        want.extend_from_slice(&42u64.to_le_bytes());
        want.extend_from_slice(&123_456_789i64.to_le_bytes());
        want.push(2); // two digests
        want.push(3);
        want.extend_from_slice(b"md5");
        want.push(16);
        want.extend_from_slice(&[1u8; 16]);
        want.push(6);
        want.extend_from_slice(b"sha256");
        want.push(32);
        want.extend_from_slice(&[2u8; 32]);
        assert_eq!(encode_rec(&file_rec()).unwrap(), want);
    }

    /// An unknown name is carried, and it lands **after** the known ones rather
    /// than interleaving them — the one place this layout reorders entries. The
    /// meaning does not change, the bytes stay deterministic, and nothing compares
    /// encoded values for equality; this says so out loud rather than leaving a
    /// reader to wonder whether the reordering was intended.
    #[test]
    fn an_unknown_name_is_carried_and_lands_after_the_known_ones() {
        let mut rec = file_rec();
        rec.hashes.insert_named("blake3", &[7u8; 32]);
        let bytes = encode_rec(&rec).unwrap();

        assert_eq!(bytes[18], 3, "the count is every digest, foreign included");
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

    /// Byte offset of the first occurrence of `needle` in `hay`.
    fn find(hay: &[u8], needle: &[u8]) -> usize {
        hay.windows(needle.len())
            .position(|w| w == needle)
            .unwrap_or_else(|| panic!("{needle:?} not found"))
    }

    #[test]
    fn codec_preserves_unknown_algorithms() {
        // Forward compat: a future `--hash blake3` row must survive a
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

    #[test]
    fn codec_rejects_truncated_and_bad_version() {
        let good = encode_rec(&file_rec()).unwrap();
        assert!(decode_rec(&good[..good.len() - 2]).is_err());
        let mut bad = good.clone();
        bad[0] = 99;
        assert!(decode_rec(&bad).is_err());
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
}
