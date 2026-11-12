//! One-shot `sled` -> `redb` cache converter.
//!
//! Reads a legacy `<root>/girpr-cache` sled directory (JSON values, hex
//! digests under the `\0meta` reservation) and writes a new redb file with
//! the binary codec from [`crate::cache`]. Hex digests are decoded to raw
//! bytes; unknown algorithms pass through untouched.
//!
//! **The conversion is atomic for a file destination.** The new cache is
//! written into a temporary sibling and moved onto the destination only once
//! it is whole, so a conversion that fails leaves the previous destination
//! exactly as it was — under `--force` — and leaves no destination at all
//! without it. Replacing a *legacy sled directory* is the one step that is
//! not atomic: the directory has to be removed before the new file can take
//! its name.
//!
//! Used by the `sled2redb` binary and covered by `tests/convert.rs`. This is
//! the only module that still touches `sled`.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::info;

use crate::hash::DigestSet;

/// Reserved sled key holding the schema meta blob.
const OLD_META_KEY: &str = "\0meta";

/// The prefix of the temporary sibling a conversion writes into.
///
/// Deliberately **not** `girpr-cache`: that prefix is the cache's own
/// namespace — the live cache, its `backup-*` copies and its `old-*`
/// snapshots — and a scan prunes anything under it. A temporary with that
/// name would be mistaken for a live cache.
const TEMP_PREFIX: &str = ".girsync-convert-";

/// How many candidate names a claim tries before it gives up.
///
/// The `_2`, `_3`, … spellings mean a candidate is only ever taken by a run
/// that collided on the same process id and timestamp, so the bound is
/// generous on purpose: it is there so a pathological directory cannot spin
/// the loop forever, not because the number is expected to be reached.
const MAX_CLAIM_ATTEMPTS: u32 = 1000;

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
///
/// **The destination is replaced by a single rename.** The new cache is
/// written into a temporary sibling and moved onto the destination only once
/// it is whole, so a conversion that fails anywhere before that move leaves
/// the previous destination untouched — under `force` — and leaves no
/// destination at all without it. The one exception is a destination that is
/// a *legacy sled directory*: it has to be removed before the new file can
/// take its name, and that step is not atomic.
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

    // The temporary is claimed before the source is read, so that a source
    // which turns out to be unreadable leaves the same thing behind as any
    // other failure: nothing. The destination itself is not touched at all
    // until the last step.
    let temp = claim_temp_sibling(redb_path)?;

    let out = match convert_into(sled_dir, &temp) {
        Ok(out) => out,
        Err(e) => return Err(cleanup_temp(e, &temp)),
    };

    if let Err(e) = replace_destination(&temp, redb_path) {
        return Err(cleanup_temp(e, &temp));
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

/// Convert the sled DB at `sled_dir` into the redb cache at `temp`.
///
/// The rows are collected before the cache is opened, and that is not a
/// leftover to undo but a requirement: the cache's case mode comes from the
/// sled `meta` row, so the mode is only known once the source has been read,
/// and the sled handle has to be closed before the destination is created
/// beside it. The `Vec` is therefore the shape this conversion has; what the
/// module is for is that the *result* is whole or absent.
fn convert_into(sled_dir: &Path, temp: &Path) -> Result<Converted> {
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
        // `insert_named` is the routing rule, so a sled record carrying an
        // algorithm this build has never heard of lands in the same foreign arm a
        // redb row would -- one place decides what "an unknown algorithm" means.
        let mut hashes = DigestSet::default();
        for (algo, hex) in &o.hashes {
            hashes.insert_named(algo, &hex_to_bytes(hex, &rel, algo)?);
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

    let cache = crate::cache::open_db(
        temp,
        case_sensitive,
        crate::cache::CacheOpen::ReadWrite {
            ignore_cache: false,
            backup_first: false,
            retain: None,
        },
    )?;
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
    // The handle holds redb's file lock, and it is dropped here so that the
    // move below is not racing the handle that wrote the file.
    drop(cache);
    Ok(out)
}

/// Move the finished temporary onto the destination.
///
/// A **file** destination is replaced by a single rename, so the destination
/// is either the old cache or the new one — never a half-written mixture —
/// and a failed rename leaves the old cache in place.
///
/// A **directory** destination is a legacy sled cache. The directory has to
/// be removed before the new file can take its name, and that step is *not*
/// atomic: the window between the removal and the rename is the one case
/// where a failure leaves no cache at all.
fn replace_destination(temp: &Path, dst: &Path) -> Result<()> {
    let legacy_dir = dst.is_dir();
    if legacy_dir {
        crate::cache::remove_cache_path(dst)?;
    }
    std::fs::rename(temp, dst).with_context(|| {
        if legacy_dir {
            format!(
                "move the converted cache {} onto {} (the legacy cache directory was already removed)",
                temp.display(),
                dst.display()
            )
        } else {
            format!(
                "move the converted cache {} onto {}",
                temp.display(),
                dst.display()
            )
        }
    })
}

/// Claim a free temporary sibling next to `dst` and return its path.
///
/// The name is **claimed**, not chosen: `create_new` is what takes it, so two
/// conversions into the same directory cannot pick the same file. Probing for
/// a free name and then creating it has a window between the two, and that
/// window is exactly where two concurrent runs would collide.
///
/// The name carries the process id and a timestamp, so a leftover from an
/// interrupted run is recognisable rather than anonymous, and it never
/// starts with `girpr-cache` so that a scan cannot mistake it for a live
/// cache.
fn claim_temp_sibling(dst: &Path) -> Result<PathBuf> {
    let parent = dst.parent().unwrap_or_else(|| Path::new("."));
    let stamp = format!("{}-{}", std::process::id(), crate::util::ts_now());
    for n in 1..=MAX_CLAIM_ATTEMPTS {
        let cand = parent.join(temp_candidate_name(&stamp, n));
        // Ask first whether the name is taken: `create_new` on an existing
        // *directory* reports `Access is denied` rather than `AlreadyExists`,
        // so the error kind alone cannot distinguish "occupied" from "cannot
        // write here".
        match std::fs::symlink_metadata(&cand) {
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("stat {}", cand.display())),
        }
        let claimed = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&cand)
            .map(drop);
        match claimed {
            Ok(()) => return Ok(cand),
            Err(e) => {
                // A racing creator, or a candidate that appeared between the
                // stat and the claim: the name is ours only if we created it.
                match std::fs::symlink_metadata(&cand) {
                    Ok(_) => continue,
                    Err(_) => {
                        return Err(e).with_context(|| format!("claim {}", cand.display()));
                    }
                }
            }
        }
    }
    bail!(
        "no free conversion temporary beside {} after {MAX_CLAIM_ATTEMPTS} tries",
        parent.display()
    )
}

/// The `n`th candidate name for a stamp: the plain one, then `_2`, `_3`, and
/// so on.
fn temp_candidate_name(stamp: &str, n: u32) -> String {
    if n == 1 {
        format!("{TEMP_PREFIX}{stamp}.redb")
    } else {
        format!("{TEMP_PREFIX}{stamp}_{n}.redb")
    }
}

/// Remove the temporary sibling after a failure, and report the cleanup
/// failure beside the primary one rather than instead of it.
///
/// The temporary is the tool's own residue: it is not a cache and it is not
/// the user's file, so it must not outlive the failure — and a cleanup that
/// fails is part of the failure the user needs to see, not a result to drop.
/// The same rule the case-fix rename pass follows for its own placeholder.
fn cleanup_temp(primary: anyhow::Error, temp: &Path) -> anyhow::Error {
    match std::fs::remove_file(temp) {
        Ok(()) => primary,
        Err(cleanup) => {
            // The primary stays the source of the chain and the cleanup is
            // reported beside it, so neither failure hides the other.
            let summary = format!("{}", primary);
            primary.context(format!(
                "{summary}; removing the temporary {} also failed: {cleanup}",
                temp.display()
            ))
        }
    }
}
