//! One-shot `sled` -> `redb` cache converter.
//!
//! Reads a legacy `<root>/girpr-cache` sled directory (JSON values, hex
//! digests under the `\0meta` reservation) and writes a new redb file with
//! the binary codec from [`girsync::cache`]. Hex digests are decoded to raw
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
//! The temporary is written into the folder the destination cache lives in,
//! so its name is inside the tool's own namespace — see
//! [`CONVERT_TEMP_PREFIX`]. A conversion that is killed before it can clean
//! up after itself therefore leaves tool state behind, not a file the next
//! run would treat as ordinary content. Nothing reclaims it, and it stays in
//! the folder until it is removed by hand: a temporary is the file a live
//! conversion is writing, so no sweep may take one. The process id its name
//! carries says which run left it, and is there for the reader — it cannot
//! answer "is a conversion still writing this?", because a process id is
//! reused and a folder can be shared between hosts.
//!
//! Used by the `sled2redb` binary and covered by `tests/convert.rs`. This is
//! the only module in this crate that touches `sled`; girsync never does.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::info;

use girsync::hash::DigestSet;

/// Reserved sled key holding the schema meta blob.
const OLD_META_KEY: &str = "\0meta";

/// The prefix of the temporary sibling a conversion writes into.
///
/// **The name is in the tool's own namespace**, and it has to be. The
/// temporary is written into the folder the destination cache lives in, so a
/// conversion that is killed before it can clean up after itself leaves a
/// file in the user's tree. Every `girpr-cache*` component is reserved by the
/// scan, so a name inside that namespace leaves the leftover looking like
/// what it is — the tool's own residue — rather than like ordinary content,
/// which a later run would copy out of a source folder or delete as an extra
/// in a destination one.
///
/// It is not the live cache's name: that is `girpr-cache` itself, and only
/// the rename puts the finished database there.
///
/// **The whole name is `<prefix><pid>-<ts>.redb`**, and a candidate whose
/// name is taken is stepped over rather than reused — so the run may write
/// `<prefix><pid>-<ts>_2.redb`, and its `_3`, `_4`, … spellings, instead.
/// Every one of those spellings is inside the namespace too, so a leftover of
/// any of them is tool state and not content.
///
/// **Nothing reclaims it.** The retention bound ranges over the two published
/// lists beside a cache — the backups and the `old-*` snapshots — and not over
/// this name, so a temporary that a killed conversion left behind stays in the
/// folder until it is removed by hand. That is the deliberate answer rather
/// than a gap: a temporary is the file a live conversion is writing, and a
/// sweep that could not tell a live one from a dead one would take the copy a
/// running conversion is making. The name carries the process id, so a
/// leftover says which run left it.
///
/// **The process id is for the reader, not for the tool.** It names the run
/// that made the file, and it is not an oracle for "is a conversion still
/// writing this?". Two things stop it from being one:
///
/// - **A process id is reused.** The number in the name is the one the
///   conversion had; a later process on the same host can be handed the same
///   number. So "that process is alive" does not mean "the conversion that
///   made this file is alive" — it would keep a stale temporary forever, and
///   a live conversion's temporary could be judged dead by a different
///   process wearing the same number.
/// - **The folder can be shared between hosts.** A network folder, a
///   container mount, a synced directory: the number was minted in another
///   host's process table, where this host cannot ask about it at all. A
///   container has the same problem inside one host, because its process ids
///   belong to its own namespace.
///
/// So the only question a sweep could ask safely is whether a conversion is
/// running *now*, and the id in the name cannot answer it. The tool does not
/// sweep this name at all: a leftover is the reader's to remove.
pub const CONVERT_TEMP_PREFIX: &str = "girpr-cache-convert-";

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
///
/// The actual redb write lives in [`write_records`]; this function owns only
/// the sled half — opening, decoding and the `drop(old)` that releases the
/// lock before the cache is opened beside the source.
fn convert_into(sled_dir: &Path, temp: &Path) -> Result<Converted> {
    let old =
        sled::open(sled_dir).with_context(|| format!("open sled db {}", sled_dir.display()))?;
    let mut case_sensitive = true;
    let mut recs: Vec<(String, girsync::cache::FileRec)> = Vec::new();
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
            girsync::cache::FileRec {
                kind: o.kind,
                size: o.size,
                mtime_ns: o.mtime_ns,
                hashes,
            },
        ));
    }
    // The iteration above is read-only, so sled has nothing to flush; `drop`
    // releases the lock before the destination is created next to it. It stays
    // here, in the sled half, rather than moving into `write_records`: the
    // lock must be gone before the redb cache is opened beside the source.
    drop(old);

    // `case_sensitive` came from the sled `meta` row above; the write half
    // takes it as a plain argument and owns only the redb side.
    write_records(temp, case_sensitive, &recs)
}

/// Write already-decoded records into the redb cache at `temp`.
///
/// This is the redb-write half of [`convert_into`]. It owns the cache handle,
/// the write transaction, the per-kind counters and the lock-sidecar cleanup,
/// but knows nothing about sled: callers that already hold decoded records —
/// the sled reader above or a test with synthetic rows — use this seam
/// directly. `case_sensitive` is the mode [`convert_into`] read from the sled
/// `meta` row; it decides how the cache is opened.
///
/// The temporary's *name* and its cleanup on failure are the caller's job and
/// stay in [`sled_to_redb`]: this function writes to whichever path it is
/// given and leaves the surrounding atomicity guarantees untouched.
pub fn write_records(
    temp: &Path,
    case_sensitive: bool,
    recs: &[(String, girsync::cache::FileRec)],
) -> Result<Converted> {
    let cache = girsync::cache::open_db(
        temp,
        case_sensitive,
        girsync::cache::CacheOpen::ReadWrite {
            ignore_cache: false,
            backup_first: false,
            retain: None,
        },
    )?;
    let mut out = Converted::default();
    {
        let mut w = cache.begin_write()?;
        for (rel, rec) in recs {
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
    // The handle holds redb's file lock and the cache's lock sidecar, and is
    // dropped here so that the move below is not racing the handle that wrote
    // the file. The sidecar is named after the temp path, so it is removed
    // here too: the finished database is about to move onto its final name,
    // and an orphaned sidecar would be residue in the user's folder.
    drop(cache);
    girsync::cache::remove_lock_sidecar(temp)
        .with_context(|| format!("remove lock sidecar of {}", temp.display()))?;
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
        girsync::cache::remove_cache_path(dst)?;
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
/// interrupted run is recognisable rather than anonymous, and it carries the
/// tool's own prefix so that a scan reads such a leftover as tool state
/// rather than as content.
///
/// **A name that is taken is stepped over, not reused**, so the file this
/// returns is `<prefix><pid>-<ts>.redb` or one of its `<prefix><pid>-<ts>_2.redb`
/// spellings — every one of them inside the tool's namespace, so a leftover of
/// any of them is tool state rather than content.
fn claim_temp_sibling(dst: &Path) -> Result<PathBuf> {
    let parent = dst.parent().unwrap_or_else(|| Path::new("."));
    let stamp = format!("{}-{}", std::process::id(), girpr_log::timestamp());
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
        format!("{CONVERT_TEMP_PREFIX}{stamp}.redb")
    } else {
        format!("{CONVERT_TEMP_PREFIX}{stamp}_{n}.redb")
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
    // The sidecar is the temporary's own residue, so it goes with the
    // temporary. A failure to remove it never hides the primary failure.
    let primary = match girsync::cache::remove_lock_sidecar(temp) {
        Ok(()) => primary,
        Err(cleanup) => {
            let summary = format!("{primary}");
            primary.context(format!(
                "{summary}; removing the lock sidecar of {} also failed: {cleanup}",
                temp.display()
            ))
        }
    };
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

#[cfg(test)]
mod tests {
    use super::{CONVERT_TEMP_PREFIX, temp_candidate_name};

    /// **The temporary's name, and the retry spellings it takes.**
    ///
    /// The claim steps over a name that is already there rather than reusing
    /// it, so the names a conversion can leave behind are the plain one and
    /// the `_2`, `_3`, … spellings of it. Every one of them is inside the
    /// tool's namespace, so a leftover of any of them is tool state — and not
    /// a file the next run would mirror out of a source folder or delete as an
    /// extra in a destination one.
    #[test]
    fn the_temporary_name_and_its_retry_spellings_are_tool_state() {
        let stamp = "1234-20240101T000000.000000Z";
        assert_eq!(
            temp_candidate_name(stamp, 1),
            format!("{CONVERT_TEMP_PREFIX}{stamp}.redb"),
            "the first candidate is the plain name"
        );
        assert_eq!(
            temp_candidate_name(stamp, 2),
            format!("{CONVERT_TEMP_PREFIX}{stamp}_2.redb"),
            "and the next one is the same name with an underscore-n"
        );
        for n in 1..=3 {
            let name = temp_candidate_name(stamp, n);
            assert!(
                girsync::util::is_cache_rel(&name),
                "`{name}` is the tool's own state, not content"
            );
        }
    }
}
