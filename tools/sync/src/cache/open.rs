//! Opening a cache: [`open_db`], the backups and snapshots, and the version gate.
//!
//! Everything here is a decision about **the file as a whole** — whether it
//! exists, what version it is, whether it is copied aside first, how many
//! copies of it are kept beside it — rather than about any one row. That is
//! why [`CacheOpen`] is its own enum here rather than a parameter threaded
//! through [`CacheDb`]'s methods.

use anyhow::{Context, Result, bail};
use redb::ReadableDatabase;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use tracing::{info, trace, warn};

use crate::util::{copy_dir_all, ts_now};

use super::codec::decode_meta;
use super::db::{CacheDb, redb_builder};
use super::schema::{CACHE_VERSION, FileRec, Meta};
use super::{META_KEY, META_TBL};

/// The prefix of the copies the tool keeps beside a cache.
///
/// Every sibling carrying this prefix is one of the tool's own, so a bound
/// over them is the tool's business and never the user's files'.
const BACKUP_PREFIX: &str = "girpr-cache-backup-";

/// The prefix of the pre-sync snapshots the tool keeps beside a cache.
const OLD_PREFIX: &str = "girpr-cache-old-";

/// How a cache file should be opened.
///
/// A read/write open is a *participant* in the run: it may create the file,
/// back it up, and rewrite `meta`. A read-only open is an *observer*: it
/// requires the file to already be a valid current-version cache and never
/// opens it for writing, so the file comes out byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheOpen {
    /// Create if missing, and optionally back up and/or rebuild first.
    ///
    /// `ignore_cache` backs up (when `backup_first`) and deletes the file so it
    /// is rebuilt from scratch. `backup_first` copies it aside to
    /// `girpr-cache-backup-<ts>` before opening, and `retain` says how many
    /// siblings of each prefix are kept beside it — `None` keeps them all.
    ///
    /// The bound is about the **folder**, not about this open: it is applied
    /// whether or not a copy was made, so a folder with no cache and an open
    /// that copies nothing aside are bounded just the same.
    ReadWrite {
        ignore_cache: bool,
        backup_first: bool,
        /// How many siblings of each prefix to keep beside the cache.
        retain: Option<NonZeroUsize>,
    },
    /// Open the existing file for reading only: no create-if-missing, no
    /// backup, no `meta` rewrite.
    ///
    /// A read/write open silently rewrites `meta` when the recorded
    /// `case_sensitive` flag disagrees with the mode the run is in. That flag is
    /// documented as informational, so a read-only open cannot act on the
    /// disagreement — it warns and moves on rather than refusing a cache that
    /// is perfectly usable.
    ReadOnly,
}

/// Open (creating if needed) the cache at `db_path`.
///
/// `ignore_cache` backs up (when `backup_first`) and deletes the cache to
/// force a rebuild. A corrupt cache is a hard error; the message points at
/// `--ignore-cache`.
///
/// A read/write open applies the **bound** it was given to the folder the
/// cache lives in: only the newest `retain` siblings of each prefix are kept
/// beside it. The bound is about the folder rather than about the copy this
/// open made, so it holds for a folder whose cache is absent and for an open
/// that copies nothing aside — the copies already sitting beside a folder are
/// the ones the bound is about.
///
/// [`CacheOpen::ReadOnly`] inverts every side effect: the file must exist, is
/// never opened for writing, and is left byte-identical. It still validates the
/// schema version, because a cache this build cannot decode is useless
/// whatever the mode. A `case_sensitive` flag disagreeing with the run's mode
/// only warns — a read/write open would silently rewrite `meta` to match, and
/// that flag is informational (see [`Meta::case_sensitive`]).
#[tracing::instrument(skip_all, fields(db = %db_path.display(), mode = ?mode))]
pub fn open_db(db_path: &Path, case_sensitive: bool, mode: CacheOpen) -> Result<CacheDb> {
    match mode {
        CacheOpen::ReadWrite {
            ignore_cache,
            backup_first,
            retain,
        } => {
            if ignore_cache && db_path.exists() {
                if backup_first {
                    backup_db(db_path)?;
                }
                remove_cache_path(db_path)?;
                info!(path = %db_path.display(), "ignore-cache removed");
            } else if backup_first && db_path.exists() {
                backup_db(db_path)?;
            }
            // The bound, applied whether or not a copy was made: it is a
            // statement about the folder, not about this open. After the copy,
            // so the copy just made is one of the newest the folder keeps.
            if let Some(keep) = retain {
                retain_copies(db_path, keep)?;
            }
            let db = CacheDb::open_file(db_path)?;
            CacheDb::init_meta(&db, case_sensitive)?;
            // NOTE: case_sensitive is informational only. The cache stays usable
            // across modes: in insensitive mode a disk/cached casing difference is
            // fixed to the on-disk name (disk always governs), so the same record
            // remains valid for a later sensitive run. Only genuine conflicts
            // (two live/record paths differing only by case in insensitive mode)
            // abort, detected by the callers — never here.
            Ok(CacheDb::open_writable(db))
        }
        CacheOpen::ReadOnly => {
            if !db_path.exists() {
                bail!("cache {} not found", db_path.display());
            }
            let db = open_read_only(db_path)?;
            let Some(meta) = read_meta(&db)? else {
                bail!(
                    "cache {} has no meta (corrupt? use --ignore-cache)",
                    db_path.display()
                );
            };
            require_current_version(meta.version, db_path, "cache")?;
            if meta.case_sensitive != case_sensitive {
                // A read/write open would quietly rewrite meta to match. It is
                // documented as informational, so the disagreement is not a
                // reason to refuse a cache that reads fine.
                warn!(
                    path = %db_path.display(),
                    recorded = meta.case_sensitive,
                    requested = case_sensitive,
                    "cache case mode differs; read-only open leaves meta as recorded"
                );
            }
            Ok(CacheDb::open_read_only(db))
        }
    }
}

/// Remove a cache path regardless of whether it is a redb file or a legacy
/// sled directory.
pub fn remove_cache_path(db_path: &Path) -> Result<()> {
    if db_path.is_dir() {
        std::fs::remove_dir_all(db_path)
            .with_context(|| format!("remove {}", db_path.display()))?;
    } else if db_path.is_file() {
        std::fs::remove_file(db_path).with_context(|| format!("remove {}", db_path.display()))?;
    }
    Ok(())
}

/// Copy an existing cache to `<parent>/girpr-cache-backup-<ts>`.
/// Works for redb files and legacy sled directories alike.
/// Returns the backup path, or `None` when there was nothing to copy.
///
/// How many copies are kept beside the cache is [`retain_copies`]'s business:
/// the bound is about the folder, so it is applied by the caller whether or
/// not this call made a copy.
#[tracing::instrument(skip_all, fields(db = %db_path.display()))]
pub fn backup_db(db_path: &Path) -> Result<Option<PathBuf>> {
    copy_aside(db_path, BACKUP_PREFIX, "backup")
}

/// Copy an existing cache to `<parent>/girpr-cache-old-<ts>` to record the
/// pre-sync dst state. Returns the snapshot path, or `None` when absent.
///
/// The bound is [`retain_copies`]'s business here too, for the same reason.
#[tracing::instrument(skip_all, fields(db = %db_path.display()))]
pub fn snapshot_old(db_path: &Path) -> Result<Option<PathBuf>> {
    copy_aside(db_path, OLD_PREFIX, "snapshot-old")
}

/// Keep only the newest `keep` copies the tool keeps beside a cache.
///
/// **The bound is about the folder, not about a copy.** The copies a folder
/// accumulates are bounded by what the user asked for whether or not this run
/// added one, so the bound is a step of its own rather than a side effect of
/// a successful copy: a folder whose cache is absent, and a run that copies
/// nothing aside, are bounded just the same.
///
/// The two prefixes are separate lists with separate bounds — the
/// `girpr-cache-backup-*` copies and the `girpr-cache-old-*` snapshots are
/// different things, and a bound on one is not a bound on the other. Only
/// siblings carrying a prefix are considered, so a file of the user's that
/// happens to sit in the same directory is never touched.
pub fn retain_copies(db_path: &Path, keep: NonZeroUsize) -> Result<()> {
    let parent = db_path.parent().unwrap_or_else(|| Path::new("."));
    retain_siblings(parent, BACKUP_PREFIX, keep)?;
    retain_siblings(parent, OLD_PREFIX, keep)
}

/// Copy `db_path` beside itself as `<prefix><ts>`. A directory (a legacy sled
/// cache) is copied recursively; a redb file is copied in one call. The
/// destination never replaces an existing sibling.
fn copy_aside(db_path: &Path, prefix: &str, label: &str) -> Result<Option<PathBuf>> {
    if !db_path.exists() {
        trace!(path = %db_path.display(), "{label} skip: db missing");
        return Ok(None);
    }
    let parent = db_path.parent().unwrap_or_else(|| Path::new("."));
    let is_dir = db_path.is_dir();
    let dest = claim_sibling(parent, prefix, &ts_now(), is_dir)?;
    info!(src = %db_path.display(), dst = %dest.display(), "{label} start");
    let copied: Result<()> = if is_dir {
        copy_dir_all(db_path, &dest)
    } else {
        std::fs::copy(db_path, &dest)
            .with_context(|| format!("copy {} -> {}", db_path.display(), dest.display()))
            .map(|_| ())
    };
    if let Err(e) = copied {
        // The claim is ours and the copy did not land. A half-written sibling
        // is not a backup, and leaving it would be a file a later run reads
        // as one — so it goes, and the failure is reported either way.
        let _ = remove_cache_path(&dest);
        return Err(e);
    }
    info!(src = %db_path.display(), dst = %dest.display(), "{label} done");
    Ok(Some(dest))
}

/// How many candidate names a claim tries before it gives up.
///
/// The `_2`, `_3` … spellings mean a candidate is only ever taken by a run
/// that collided on the same microsecond, so the bound is generous on
/// purpose: it is there so a pathological directory cannot spin the loop
/// forever, not because the number is expected to be reached.
const MAX_CLAIM_ATTEMPTS: u32 = 1000;

/// Claim a free `<parent>/<prefix><ts>` sibling and return it.
///
/// The name is **claimed**, not chosen: `create_new` for a file and
/// `create_dir` for a legacy sled directory are what take it, so two runs
/// against one root cannot pick the same sibling. Probing for a free name
/// and then creating it has a window between the two, and that window is
/// exactly where two concurrent runs would collide.
///
/// The fallback names are the `_2`, `_3` … spellings the tool has always
/// used, so a run that collides on the timestamp still leaves a
/// recognisable sibling.
fn claim_sibling(parent: &Path, prefix: &str, stamp: &str, is_dir: bool) -> Result<PathBuf> {
    for n in 1..=MAX_CLAIM_ATTEMPTS {
        let cand = parent.join(candidate_name(prefix, stamp, n));
        // Ask first whether the name is taken: `create_new` on an existing
        // *directory* reports `Access is denied` rather than `AlreadyExists`,
        // so the error kind alone cannot distinguish "occupied" from "cannot
        // write here".
        match std::fs::symlink_metadata(&cand) {
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("stat {}", cand.display())),
        }
        let claimed = if is_dir {
            std::fs::create_dir(&cand).map(drop)
        } else {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&cand)
                .map(drop)
        };
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
        "no free backup name in {} after {MAX_CLAIM_ATTEMPTS} tries",
        parent.display()
    )
}

/// The `n`th candidate name for a prefix and a timestamp: the plain one, then
/// `_2`, `_3`, and so on.
fn candidate_name(prefix: &str, stamp: &str, n: u32) -> String {
    if n == 1 {
        format!("{prefix}{stamp}")
    } else {
        format!("{prefix}{stamp}_{n}")
    }
}

/// Keep only the newest `keep` siblings of `prefix` in `parent`.
///
/// The sibling names carry a timestamp that sorts lexicographically, so
/// sorting by name is sorting by age, and the newest `keep` are the ones at
/// the end. Everything older is removed — a directory recursively, a file
/// outright — because the user asked for a bound and a bound that does not
/// remove anything is not a bound.
///
/// Only siblings carrying **this** prefix are considered: the tool's own
/// namespace. A file of the user's that happens to sit in the same directory
/// is never touched, and neither is a sibling of the other prefix — the
/// `girpr-cache-backup-*` copies and the `girpr-cache-old-*` snapshots are two
/// separate lists, each with its own bound.
fn retain_siblings(parent: &Path, prefix: &str, keep: NonZeroUsize) -> Result<()> {
    let mut siblings: Vec<(std::ffi::OsString, PathBuf)> = std::fs::read_dir(parent)
        .with_context(|| format!("read dir {}", parent.display()))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            name.to_string_lossy()
                .starts_with(prefix)
                .then(|| (name, e.path()))
        })
        .collect();
    siblings.sort_by(|a, b| a.0.cmp(&b.0));
    let keep = keep.get();
    if siblings.len() <= keep {
        return Ok(());
    }
    for (_, path) in &siblings[..siblings.len() - keep] {
        remove_cache_path(path)?;
    }
    Ok(())
}

/// Open an existing cache file for reading, without ever opening it for
/// writing.
///
/// `ReadOnlyDatabase` is the reason a read-only run can promise the file is
/// untouched: the backend has no write path at all, so there is nothing to
/// accidentally commit through, and open-time recovery cannot rewrite pages.
pub(super) fn open_read_only(db_path: &Path) -> Result<redb::ReadOnlyDatabase> {
    redb_builder().open_read_only(db_path).with_context(|| {
        format!(
            "open redb cache {} (corrupt? use --ignore-cache)",
            db_path.display()
        )
    })
}

/// The `meta` row's bytes, or `None` when the table or the key is absent.
///
/// Absence is reported rather than invented: a read-only open cannot create
/// the row a read/write open would, so the caller has to decide what a
/// meta-less cache means.
///
/// The bytes rather than a decoded row, because the two callers word a
/// corrupt row differently: an observer of a cache can only say it is
/// corrupt, while a read/write open can also say that `--ignore-cache`
/// rebuilds it. The decoding is therefore the caller's, and so is the
/// wording.
///
/// The row is read from a transaction the caller already holds, so a caller
/// that needs more than the row (the read/write open needs to know whether
/// the cache is whole as well) reads it from the same snapshot as the rest
/// of what it is checking.
pub(super) fn read_meta_bytes(txn: &redb::ReadTransaction) -> Result<Option<Vec<u8>>> {
    let tbl = match txn.open_table(META_TBL) {
        Ok(t) => t,
        Err(e) if is_table_missing(&e) => return Ok(None),
        Err(e) => bail!("open meta table: {e:#}"),
    };
    let g = tbl.get(META_KEY).context("read meta")?;
    Ok(g.map(|g| g.value().to_vec()))
}

/// The `meta` row, or `None` when the table or the key is absent.
pub(super) fn read_meta(db: &impl ReadableDatabase) -> Result<Option<Meta>> {
    let txn = db.begin_read().context("begin read (meta)")?;
    match read_meta_bytes(&txn)? {
        None => Ok(None),
        Some(raw) => Ok(Some(
            decode_meta(&raw).context("parse cache meta (corrupt?)")?,
        )),
    }
}

/// Refuse a cache this build cannot interpret, whatever the open mode: a wrong
/// version means every row would be decoded under the wrong assumptions.
pub(super) fn require_current_version(version: u32, db_path: &Path, label: &str) -> Result<()> {
    if version != CACHE_VERSION {
        bail!(
            "unsupported {label} version {} in {} (want {CACHE_VERSION}; use --ignore-cache to rebuild or sled2redb to convert)",
            version,
            db_path.display()
        );
    }
    Ok(())
}

/// A missing table is a *fresh* cache, not a corrupt one — so this is a
/// predicate rather than an error every caller has to write out.
pub(super) fn is_table_missing(e: &redb::TableError) -> bool {
    matches!(e, redb::TableError::TableDoesNotExist(_))
}

/// Every cached path.
pub fn load_all_records(cache: &CacheDb) -> Result<HashMap<String, FileRec>> {
    cache.load_all()
}

/// Upsert one cached path (immediately committed; bulk callers prefer
/// [`CacheWrite`](super::CacheWrite)).
pub fn put_rec(cache: &CacheDb, rel: &str, rec: &FileRec) -> Result<()> {
    cache.put(rel, rec)
}

/// Fetch one cached path.
pub fn get_rec(cache: &CacheDb, rel: &str) -> Result<Option<FileRec>> {
    cache.get(rel)
}

/// Remove one cached path (immediately committed).
pub fn remove_rec(cache: &CacheDb, rel: &str) -> Result<()> {
    cache.remove(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stamp every claim in these cases asks for: a claim is asked for a
    /// *name*, so the name is the thing under test and the timestamp is
    /// pinned to keep the expectation readable.
    const STAMP: &str = "20240101T000000.000000Z";

    /// A claim steps over a name that is already taken rather than taking it:
    /// the name is claimed, not chosen.
    #[test]
    fn a_claim_steps_over_a_taken_name() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "claim");
        let dir = t.subdir("w");

        let first = claim_sibling(&dir, "girpr-cache-backup-", STAMP, false).unwrap();
        assert_eq!(
            first.file_name().unwrap().to_string_lossy(),
            format!("girpr-cache-backup-{STAMP}")
        );

        let second = claim_sibling(&dir, "girpr-cache-backup-", STAMP, false).unwrap();
        assert_eq!(
            second.file_name().unwrap().to_string_lossy(),
            format!("girpr-cache-backup-{STAMP}_2"),
            "the second claim takes the next name"
        );
        assert_ne!(first, second, "the two claims are two names");
    }

    /// The same for a directory claim: a legacy sled cache is a directory, and
    /// so is the copy of one.
    #[test]
    fn a_directory_claim_steps_over_a_taken_name() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "claim_dir");
        let dir = t.subdir("w");

        let first = claim_sibling(&dir, "girpr-cache-backup-", STAMP, true).unwrap();
        assert!(first.is_dir(), "a directory claim leaves a directory");
        let second = claim_sibling(&dir, "girpr-cache-backup-", STAMP, true).unwrap();
        assert!(second.is_dir(), "and so does the next one");
        assert_ne!(first, second);
        assert!(
            second
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("_2"),
            "the second claim takes the next name"
        );
    }

    /// Concurrent claims of one name each end up with a name of their own.
    /// The timestamp is the same for all of them, so nothing but the claim
    /// itself keeps them apart — which is the whole point of claiming rather
    /// than choosing.
    #[test]
    fn concurrent_claims_get_names_of_their_own() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "claim_race");
        let dir = t.subdir("w");
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    claim_sibling(&dir, "girpr-cache-backup-", STAMP, false).unwrap()
                })
            })
            .collect();
        let names: Vec<PathBuf> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let unique: std::collections::HashSet<&PathBuf> = names.iter().collect();
        assert_eq!(
            unique.len(),
            names.len(),
            "every claim got a name of its own"
        );
    }
}
