//! One-shot `sled` -> `redb` cache converter.
//!
//! Reads a legacy `<root>/girpr-cache` sled directory (JSON values, hex
//! digests under the `\0meta` reservation) and writes a new redb file with
//! the binary codec from [`crate::cache`]. Hex digests are decoded to raw
//! bytes; unknown algorithms pass through untouched.
//!
//! Used by the `sled2redb` binary and covered by `tests/convert.rs`. This is
//! the only module that still touches `sled`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::info;

/// Reserved sled key holding the schema meta blob.
const OLD_META_KEY: &str = "\0meta";

#[derive(Serialize, Deserialize, Clone, Debug)]
struct OldMeta {
    version: u32,
    case_sensitive: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct OldFileRec {
    kind: String,
    size: u64,
    mtime_ns: i64,
    hashes: HashMap<String, String>,
}

/// What the conversion moved over, for the summary line.
#[derive(Debug, Default)]
pub struct Converted {
    pub files: usize,
    pub dirs: usize,
    pub hashes: usize,
}

fn hex_to_bytes(hex: &str, rel: &str, algo: &str) -> Result<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        anyhow::bail!("bad hex for {algo} of {rel}");
    }
    hex.as_bytes()
        .chunks(2)
        .map(|c| {
            let s = std::str::from_utf8(c).map_err(|e| anyhow::anyhow!("{e:#}"))?;
            u8::from_str_radix(s, 16).map_err(|e| anyhow::anyhow!("{e:#}"))
        })
        .collect::<Result<Vec<u8>>>()
        .with_context(|| format!("decode {algo} hex of {rel}"))
}

/// Convert sled DB at `sled_dir` into a new redb file at `redb_path`.
///
/// Refuses to overwrite an existing `redb_path` unless `force` is set.
/// Returns per-kind counts. The source directory is left untouched.
pub fn sled_to_redb(sled_dir: &Path, redb_path: &Path, force: bool) -> Result<Converted> {
    if !sled_dir.is_dir() {
        anyhow::bail!("sled source {} is not a directory", sled_dir.display());
    }
    if redb_path.exists() && !force {
        anyhow::bail!(
            "refusing to overwrite {} (pass --force)",
            redb_path.display()
        );
    }
    if force && redb_path.exists() {
        crate::cache::remove_cache_path(redb_path)?;
    }

    let old =
        sled::open(sled_dir).with_context(|| format!("open sled db {}", sled_dir.display()))?;
    let mut case_sensitive = true;
    let mut recs: Vec<(String, crate::cache::FileRec)> = Vec::new();
    for kv in old.iter() {
        let (k, v) = kv.context("read sled entry")?;
        if k.as_ref() == OLD_META_KEY.as_bytes() {
            let m: OldMeta = serde_json::from_slice(&v).context("parse sled meta (corrupt?)")?;
            case_sensitive = m.case_sensitive;
            info!(version = m.version, "sled meta");
            continue;
        }
        let rel = String::from_utf8(k.to_vec()).context("non-UTF8 key in sled cache")?;
        let o: OldFileRec =
            serde_json::from_slice(&v).with_context(|| format!("parse sled entry {rel}"))?;
        if o.kind != "file" && o.kind != "dir" {
            anyhow::bail!("invalid kind {:?} for {rel}", o.kind);
        }
        let mut hashes = HashMap::with_capacity(o.hashes.len());
        for (algo, hex) in &o.hashes {
            hashes.insert(algo.clone(), hex_to_bytes(hex, &rel, algo)?);
        }
        recs.push((
            rel,
            crate::cache::FileRec {
                kind: o.kind,
                size: o.size,
                mtime_ns: o.mtime_ns,
                hashes,
            },
        ));
    }
    // Release the sled lock before creating the destination next to it.
    old.flush().ok();
    drop(old);

    let cache = crate::cache::open_db(redb_path, case_sensitive, false, false)?;
    let mut out = Converted::default();
    {
        let mut w = cache.begin_write()?;
        for (rel, rec) in &recs {
            if rec.is_file() {
                out.files += 1;
            } else {
                out.dirs += 1;
            }
            out.hashes += rec.hashes.len();
            w.put(rel, rec)
                .with_context(|| format!("write redb entry {rel}"))?;
        }
        w.commit()?;
    }
    info!(
        src = %sled_dir.display(),
        dst = %redb_path.display(),
        files = out.files,
        dirs = out.dirs,
        "converted"
    );
    Ok(out)
}
