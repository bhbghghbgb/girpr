//! One-shot `sled` -> `redb` cache converter.
//!
//! Reads a legacy `<root>/girpr-cache` sled directory (JSON values, hex
//! digests under the `\0meta` reservation) and writes a new redb file with
//! the binary codec from [`girsync_cache`]. Hex digests are decoded to raw
//! bytes; unknown algorithms pass through untouched.
//!
//! **The finished cache is moved into place once it is whole.** The new cache
//! is written into a temporary sibling and moved onto the destination in a
//! single rename, so the move cannot leave a half-written file and a failure
//! before it leaves the previous destination intact. Replacing an existing
//! file (under `--force`) is an atomic replace; a fresh destination is claimed
//! with a no-clobber rename, so a destination that appears after the initial
//! check is never overwritten. A legacy sled directory has to be removed
//! before a file can take its name, and that removal is the one step that is
//! not atomic.
//!
//! The temporary is claimed next to the destination with [`tempfile`], in the
//! tool's own namespace — see [`CONVERT_TEMP_PREFIX`]. It is owned by a
//! [`tempfile::NamedTempFile`], so a conversion that fails removes it on the
//! way out and leaves nothing of the tool's own in the folder. A kill that
//! skips destructors can leave the temporary behind, but its name is inside
//! the tool's namespace, so a leftover reads as tool state rather than as
//! ordinary content. Nothing reclaims it, and it stays in the folder until it
//! is removed by hand: a temporary is the file a live conversion is writing,
//! so no sweep may take one.
//!
//! Used by the `sled2redb` binary and covered by `tests/convert.rs`. This is
//! the only module in this crate that touches `sled`; girsync never does.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tempfile::NamedTempFile;
use tracing::info;

use girsync_cache::hash::DigestSet;

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
/// **The whole name is `<prefix><random>.redb`.** [`tempfile`] creates the
/// temporary with this prefix, a `.redb` suffix and a random middle, retrying
/// with a fresh random name on a collision — so two conversions into the same
/// folder cannot claim the same file, and no bounded family of numbered
/// candidate names has to be searched. Every name it can make is inside the
/// tool's namespace.
///
/// **Nothing reclaims it.** The retention bound ranges over the two published
/// lists beside a cache — the backups and the `old-*` snapshots — and not over
/// this name, so a temporary that a killed conversion left behind stays in the
/// folder until it is removed by hand. That is the deliberate answer rather
/// than a gap: a temporary is the file a live conversion is writing, and a
/// sweep that could not tell a live one from a dead one would take the copy a
/// running conversion is making. The random suffix names no run, so there is
/// nothing for a reader to interpret; a leftover is simply the tool's to
/// remove.
pub const CONVERT_TEMP_PREFIX: &str = "girpr-cache-convert-";

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

fn hex_to_bytes(text: &str, rel: &str, algo: &str) -> Result<Vec<u8>> {
    girpr_digest::decode_hex(text).with_context(|| format!("decode {algo} hex of {rel}"))
}

/// Convert sled DB at `sled_dir` into a new redb file at `redb_path`.
///
/// Refuses to overwrite an existing `redb_path` unless `force` is set.
/// Returns per-kind counts. The source directory is left untouched.
///
/// **The finished cache is moved into place once it is whole.** The new cache
/// is written into a temporary sibling and moved onto the destination in a
/// single rename, so a conversion that fails anywhere before that move leaves
/// the previous destination untouched — under `force` — and leaves no
/// destination at all without it. Replacing an existing file under `force` is
/// an atomic replace; a fresh destination is claimed with a no-clobber rename.
/// A *legacy sled directory* has to be removed before a file can take its
/// name, and that removal is the one step that is not atomic.
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
    // until the last step. The NamedTempFile owns the file, so dropping it on
    // any error path removes it; only its lock sidecar still needs taking with
    // it.
    let temp = claim_temp_sibling(redb_path)?;
    let temp_path = temp.path().to_path_buf();

    let out = match convert_into(sled_dir, &temp_path) {
        Ok(out) => out,
        Err(e) => return Err(cleanup_temp(e, &temp_path)),
    };

    if let Err(e) = replace_destination(temp, redb_path) {
        return Err(cleanup_temp(e, &temp_path));
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
    let mut recs: Vec<(String, girsync_cache::FileRec)> = Vec::new();
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
            girsync_cache::FileRec {
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
/// This is the redb-write half of `convert_into`. It owns the cache handle,
/// the write transaction, the per-kind counters and the lock-sidecar cleanup,
/// but knows nothing about sled: callers that already hold decoded records —
/// the sled reader above or a test with synthetic rows — use this seam
/// directly. `case_sensitive` is the mode `convert_into` read from the sled
/// `meta` row; it decides how the cache is opened.
///
/// The temporary's *name* and its cleanup on failure are the caller's job and
/// stay in [`sled_to_redb`]: this function writes to whichever path it is
/// given and leaves the surrounding atomicity guarantees untouched.
pub fn write_records(
    temp: &Path,
    case_sensitive: bool,
    recs: &[(String, girsync_cache::FileRec)],
) -> Result<Converted> {
    let cache = girsync_cache::open_db(
        temp,
        case_sensitive,
        girsync_cache::CacheOpen::ReadWrite {
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
    girsync_cache::remove_lock_sidecar(temp)
        .with_context(|| format!("remove lock sidecar of {}", temp.display()))?;
    Ok(out)
}

/// Move the finished temporary onto the destination.
///
/// An existing **file** destination is replaced by a single rename, so the
/// destination is either the old cache or the new one — never a half-written
/// mixture — and a failed rename leaves the old cache in place. A fresh
/// destination is created with a no-clobber rename, which refuses to overwrite
/// a file that appeared after the initial check.
///
/// A **directory** destination is a legacy sled cache. The directory has to
/// be removed before the new file can take its name, and that step is *not*
/// atomic: the window between the removal and the rename is the one case
/// where a failure leaves no cache at all.
fn replace_destination(temp: NamedTempFile, dst: &Path) -> Result<()> {
    let legacy_dir = dst.is_dir();
    if legacy_dir {
        // A legacy sled directory has to go before a file can take its name.
        // A conversion that failed before this point never got here, so the
        // previous cache is still intact on that path.
        girsync_cache::remove_cache_path(dst)?;
    }
    // Capture the name before the persist consumes the handle. On error the
    // returned `PersistError` still owns the NamedTempFile, so dropping the
    // error in `cleanup_temp` removes the temporary.
    let temp_display = temp.path().display().to_string();
    let result = if !legacy_dir && dst.is_file() {
        // `--force` over an existing file: replace it atomically so a failure
        // here still leaves the previous cache in place.
        temp.persist(dst).map(|_| ())
    } else {
        // Fresh destination, or a removed legacy directory: an atomic
        // no-clobber create that refuses a destination that raced in after
        // the initial check.
        temp.persist_noclobber(dst).map(|_| ())
    };
    result.with_context(|| {
        if legacy_dir {
            format!(
                "move the converted cache {temp_display} onto {} (the previous destination was already removed)",
                dst.display()
            )
        } else {
            format!(
                "move the converted cache {temp_display} onto {}",
                dst.display()
            )
        }
    })?;
    Ok(())
}

/// Claim a free temporary sibling next to `dst`.
///
/// The name is **claimed** by [`tempfile`]: `tempfile_in` creates the file
/// with `create_new`-style atomicity and retries a collision with a fresh
/// random suffix, so two conversions into the same folder cannot share a
/// temporary and no bounded family of candidate names has to be searched.
/// The returned handle owns the file — dropping it removes the temporary —
/// and its name is inside the tool's namespace ([`CONVERT_TEMP_PREFIX`]), so a
/// leftover reads as tool state rather than as content.
fn claim_temp_sibling(dst: &Path) -> Result<NamedTempFile> {
    let parent = dst.parent().unwrap_or_else(|| Path::new("."));
    let mut builder = tempfile::Builder::new();
    builder.prefix(CONVERT_TEMP_PREFIX).suffix(".redb");
    builder
        .tempfile_in(parent)
        .with_context(|| format!("claim conversion temporary in {}", parent.display()))
}

/// Remove the temporary's lock sidecar after a failure, and report the cleanup
/// failure beside the primary one rather than instead of it.
///
/// The temporary file itself is owned by the [`NamedTempFile`], so dropping it
/// on the error path removes the file; only the sidecar, which `tempfile` knows
/// nothing about, still needs taking with it. A sidecar that is already gone
/// is not a failure, so a failure before the cache was opened passes through.
fn cleanup_temp(primary: anyhow::Error, temp: &Path) -> anyhow::Error {
    match girsync_cache::remove_lock_sidecar(temp) {
        Ok(()) => primary,
        Err(cleanup) => {
            let summary = format!("{primary}");
            primary.context(format!(
                "{summary}; removing the lock sidecar of {} also failed: {cleanup}",
                temp.display()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CONVERT_TEMP_PREFIX, claim_temp_sibling, replace_destination};

    /// **The claimed temporary is tool state, and dropping it leaves nothing.**
    ///
    /// `tempfile` gives the claim a random name with the tool's prefix and a
    /// `.redb` suffix, so it is inside the tool's namespace and a leftover is
    /// not a file the next run would mirror out of a source folder or delete
    /// as an extra in a destination one. The handle owns the file, so dropping
    /// it removes the temporary without any help from the caller.
    #[test]
    fn the_claimed_temporary_is_tool_state_and_drops_cleanly() {
        let parent = tempfile::tempdir().unwrap();
        let dst = parent.path().join("girpr-cache");

        let temp = claim_temp_sibling(&dst).unwrap();
        let name = temp.path().file_name().unwrap().to_string_lossy();
        assert!(
            name.starts_with(CONVERT_TEMP_PREFIX),
            "`{name}` carries the tool's own prefix"
        );
        assert!(name.ends_with(".redb"), "`{name}` is a redb temporary");
        assert!(
            girsync::util::is_cache_rel(&name),
            "`{name}` is the tool's own state, not content"
        );

        let claimed = temp.path().to_path_buf();
        assert!(claimed.is_file(), "the claim created the file");
        drop(temp);
        assert!(
            !claimed.exists(),
            "dropping the claim removes the temporary"
        );
    }

    /// **A failed final persist under `--force` leaves the previous cache intact.**
    ///
    /// [`replace_destination`] is the seam `sled_to_redb` uses to overwrite an
    /// existing cache under `--force`: the old cache is a file at `dst`, and the
    /// finished conversion is a sibling temporary that has to be moved onto it.
    /// That move is the one step that can fail after the source has been read and
    /// the new cache is whole, so the old cache — often the only copy of hashes
    /// the legacy build had — must still be there if the move fails.
    ///
    /// The failure is induced deterministically at that final step by taking the
    /// temporary's file away just before the move: `persist` then fails because
    /// its source is gone. That is the same shape a temporary-file cleaner or a
    /// stale handle produces, and it lands exactly on the persist rather than on
    /// the source read or the cache write. The old cache holds distinctive bytes
    /// so a removal, a truncation or a half-written replacement is visible.
    #[test]
    fn a_failed_forced_persist_leaves_the_previous_cache_intact() {
        let parent = tempfile::tempdir().unwrap();
        let dst = parent.path().join("girpr-cache");
        std::fs::write(&dst, b"the previous cache, byte for byte").unwrap();
        let before = std::fs::read(&dst).unwrap();

        // The "converted" temporary, claimed in the destination's folder just
        // as a real run claims it. The handle stays alive so the move is handed
        // a genuine NamedTempFile; only the file it names is taken away.
        let temp = tempfile::Builder::new()
            .prefix(CONVERT_TEMP_PREFIX)
            .suffix(".redb")
            .tempfile_in(parent.path())
            .unwrap();
        let temp_path = temp.path().to_path_buf();
        std::fs::remove_file(&temp_path).unwrap();

        // This is the `--force` overwrite path: an existing file destination.
        // The persist must fail, and it must not have cost us the old cache.
        let err = replace_destination(temp, &dst).unwrap_err();
        assert!(
            format!("{err:#}").contains("move the converted cache"),
            "the failure comes from the final move: {err:#}"
        );
        assert!(
            dst.is_file(),
            "the previous destination still exists after the failed move"
        );
        assert_eq!(
            std::fs::read(&dst).unwrap(),
            before,
            "the previous cache is byte-identical after the failed --force move"
        );
        assert!(
            !temp_path.exists(),
            "the failed move left no temporary behind"
        );
    }
}
