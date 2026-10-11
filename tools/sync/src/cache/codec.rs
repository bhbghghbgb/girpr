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
use std::collections::HashMap;

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
    // Sorted names for deterministic bytes.
    let mut names: Vec<&String> = rec.hashes.keys().collect();
    names.sort();
    v.push(names.len() as u8);
    for name in names {
        let digest = &rec.hashes[name];
        if name.len() > 255 || digest.len() > 255 {
            bail!("hash entry too large for {name:?}");
        }
        v.push(name.len() as u8);
        v.extend_from_slice(name.as_bytes());
        v.push(digest.len() as u8);
        v.extend_from_slice(digest);
    }
    Ok(v)
}

pub(super) fn decode_rec(bytes: &[u8]) -> Result<FileRec> {
    let mut cur = bytes;
    let take = |cur: &mut &[u8], n: usize| -> Result<Vec<u8>> {
        if cur.len() < n {
            bail!("truncated cache entry");
        }
        let (h, t) = cur.split_at(n);
        *cur = t;
        Ok(h.to_vec())
    };
    let fmt = take(&mut cur, 1)?[0];
    if fmt != FMT_V1 {
        bail!("unsupported entry format {fmt}");
    }
    let kind = kind_from_byte(take(&mut cur, 1)?[0])?;
    let size = u64::from_le_bytes(take(&mut cur, 8)?.try_into().unwrap());
    let mtime_ns = i64::from_le_bytes(take(&mut cur, 8)?.try_into().unwrap());
    let n = take(&mut cur, 1)?[0] as usize;
    let mut hashes = HashMap::with_capacity(n);
    for _ in 0..n {
        let nl = take(&mut cur, 1)?[0] as usize;
        let name = String::from_utf8(take(&mut cur, nl)?).context("non-UTF8 algo name")?;
        let dl = take(&mut cur, 1)?[0] as usize;
        let digest = take(&mut cur, dl)?;
        hashes.insert(name, digest);
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

    fn file_rec() -> FileRec {
        FileRec {
            kind: "file".into(),
            size: 42,
            mtime_ns: 123456789,
            hashes: [
                ("md5".to_string(), vec![1u8; 16]),
                ("sha256".to_string(), vec![2u8; 32]),
            ]
            .into_iter()
            .collect(),
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

    #[test]
    fn codec_preserves_unknown_algorithms() {
        // Forward compat: a future `--hash blake3` row must survive a
        // roundtrip through this binary, which never drops opaque entries.
        let mut rec = FileRec {
            kind: "file".into(),
            size: 7,
            mtime_ns: 9,
            hashes: [("blake3".to_string(), vec![7u8; 32])]
                .into_iter()
                .collect(),
        };
        rec.hashes.insert("md5".to_string(), vec![3u8; 16]);
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