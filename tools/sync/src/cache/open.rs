//! Opening a cache: [`open_db`], the backups and snapshots, and the version gate.
//!
//! Everything here is a decision about **the file as a whole** — whether it
//! exists, what version it is, whether it is copied aside first, how many
//! copies of it are kept beside it — rather than about any one row. That is
//! why [`CacheOpen`] is its own enum here rather than a parameter threaded
//! through [`CacheDb`]'s methods.
//!
//! ## A copy is published, not written in place
//!
//! A copy is written under a **working name** and is **published** under a
//! name of the bounded lists only once it is complete. The publish is a
//! rename, so a name of a list never holds half a copy, and a copy that is
//! still being made is not a member of any list — which is what makes the
//! bound safe to apply at any moment: it can only ever select a copy that is
//! whole.
//!
//! There is no age, no lease and no marker to interpret anywhere in that
//! story. "Finished" and "in flight" are told apart by the name, and the
//! name is the only thing the bound reads.

use anyhow::{Context, Result, bail};
use redb::ReadableDatabase;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
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

/// The prefix of a copy that is still being made.
///
/// A copy is written under this prefix and published under one of the two
/// above only once it is complete, so the bound — which ranges over the
/// published copies — cannot see a copy that is still being written, let
/// alone select it.
///
/// The name is in the tool's own namespace (`girpr-cache*`), so a working
/// copy is tool state and is never mirrored as content.
const WORK_PREFIX: &str = "girpr-cache-partial-";

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
    /// that copies nothing aside are bounded just the same. It is applied
    /// once the cache is open, so it runs under the cache's own lock.
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
/// The bound runs **after** the cache is open, and the open holds the cache
/// file's exclusive lock until the run is done with it. That is what makes
/// the bound safe against another run: while one run holds that lock, no
/// other run is in its run phase for the folder, so the copies another run
/// made and is still using are not the ones the bound can take away. The
/// bound is applied under a lock the tool already holds, rather than under a
/// lock of its own that a filesystem might not support.
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
            let db = CacheDb::open_file(db_path)?;
            CacheDb::init_meta(&db, case_sensitive)?;
            // The bound, applied whether or not a copy was made: it is a
            // statement about the folder, not about this open.
            //
            // It runs here, after the open, and that is the point: the open
            // holds the cache file's exclusive lock, so no other run is in
            // its run phase for this folder while the bound runs. A copy
            // another run is still using is therefore not one the bound can
            // take away, and a copy this run made is spared by name (see
            // [`retain_copies`]).
            if let Some(keep) = retain {
                retain_copies(db_path, keep)?;
            }
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
///
/// A path that is already gone is not a failure: two runs bounding one
/// folder can pick the same sibling, and the second one to reach it finds
/// nothing there.
pub fn remove_cache_path(db_path: &Path) -> Result<()> {
    if db_path.is_dir() {
        match std::fs::remove_dir_all(db_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("remove {}", db_path.display())),
        }
    } else if db_path.is_file() {
        match std::fs::remove_file(db_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("remove {}", db_path.display())),
        }
    }
    Ok(())
}

/// Copy an existing cache to `<parent>/girpr-cache-backup-<ts>`.
/// Works for redb files and legacy sled directories alike.
/// Returns the backup path, or `None` when there was nothing to copy.
///
/// The copy is **published**: it is written under a working name and renamed
/// onto its published name once it is complete, so the name never holds half
/// a copy and no bound can select a copy that is still being made.
///
/// How many copies are kept beside the cache is [`retain_copies`]'s business:
/// the bound is about the folder, so it is applied by the caller whether or
/// not this call made a copy.
#[tracing::instrument(skip_all, fields(db = %db_path.display()))]
pub fn backup_db(db_path: &Path) -> Result<Option<PathBuf>> {
    copy_aside(db_path, BACKUP_PREFIX, "backup", ts_now)
}

/// Copy an existing cache to `<parent>/girpr-cache-old-<ts>` to record the
/// pre-sync dst state. Returns the snapshot path, or `None` when absent.
///
/// The snapshot is published like a backup is: whole, or not at all.
///
/// The bound is [`retain_copies`]'s business here too, for the same reason.
#[tracing::instrument(skip_all, fields(db = %db_path.display()))]
pub fn snapshot_old(db_path: &Path) -> Result<Option<PathBuf>> {
    copy_aside(db_path, OLD_PREFIX, "snapshot-old", ts_now)
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
///
/// **The copies this process published are not the bound's to remove.** The
/// run that made a copy is the run that reports it, and a bound that took it
/// back would leave that run reporting a backup that is not on disk. The
/// bound still holds the folder to its bound: the count is made up from the
/// next-oldest copy instead.
pub fn retain_copies(db_path: &Path, keep: NonZeroUsize) -> Result<()> {
    let parent = db_path.parent().unwrap_or_else(|| Path::new("."));
    let spared = take_published_under(parent);
    retain_siblings(parent, BACKUP_PREFIX, keep, &spared)?;
    retain_siblings(parent, OLD_PREFIX, keep, &spared)
}

/// Copy `db_path` beside itself as `<prefix><ts>`. A directory (a legacy sled
/// cache) is copied recursively; a redb file is copied in one call. The
/// destination never replaces an existing sibling.
///
/// **The copy is published, not written in place.** It is written under a
/// working name ([`WORK_PREFIX`]) and renamed onto its published name once
/// it is complete, so:
///
/// - a name of a bounded list never holds half a copy, and
/// - a copy that is still being made is not a member of any list, so no
///   bound — however often it is applied — can select it.
///
/// The published name is claimed before the rename, so two runs cannot
/// publish over each other.
///
/// **The copy is stamped twice, and the two stamps say different things.** The
/// working name says when the copy *started*; it is scratch, and nothing but
/// the run making it ever reads it. The published name says when the copy
/// became *whole*, because that is the moment it joins a bounded list, and
/// the bound orders the copies by that name. A published name that said when
/// the copy started would make the copy of a slow run look old the moment it
/// landed, and the bound would take it away from the run that had just made
/// it.
fn copy_aside(
    db_path: &Path,
    prefix: &str,
    label: &str,
    mut stamp: impl FnMut() -> String,
) -> Result<Option<PathBuf>> {
    if !db_path.exists() {
        trace!(path = %db_path.display(), "{label} skip: db missing");
        return Ok(None);
    }
    let parent = db_path.parent().unwrap_or_else(|| Path::new("."));
    let is_dir = db_path.is_dir();
    let work = claim_sibling(parent, WORK_PREFIX, &stamp(), is_dir)?;
    info!(src = %db_path.display(), dst = %work.display(), "{label} start");
    let copied: Result<()> = if is_dir {
        copy_dir_all(db_path, &work)
    } else {
        std::fs::copy(db_path, &work)
            .with_context(|| format!("copy {} -> {}", db_path.display(), work.display()))
            .map(|_| ())
    };
    if let Err(e) = copied {
        // The working copy is ours and the copy did not land. A half-written
        // file is not a backup, and leaving it would be a file a later run
        // reads as one — so it goes, and the failure is reported either way.
        let _ = remove_cache_path(&work);
        return Err(e);
    }
    // The published name is only ever a whole copy, so the bytes behind it
    // have to be on disk before the name appears: a rename can reach the disk
    // before the data it renames does.
    if let Err(e) = flush_copy(&work, is_dir) {
        let _ = remove_cache_path(&work);
        return Err(e);
    }
    match publish_copy(&work, parent, prefix, &stamp(), is_dir) {
        Ok(dest) => {
            remember_published(&dest);
            info!(src = %db_path.display(), dst = %dest.display(), "{label} done");
            Ok(Some(dest))
        }
        Err(e) => {
            // Nothing was published, so there is no backup to report: the
            // run fails rather than say it made one.
            let _ = remove_cache_path(&work);
            Err(e)
        }
    }
}

/// Flush a finished copy before its name is published.
///
/// The publish is a rename, and a rename can be durable before the data it
/// renames is, so the bytes are flushed first. A legacy sled directory is
/// copied file by file and is left as it is: the live cache is a file, and
/// the directory case is a migration path rather than the one that runs.
fn flush_copy(work: &Path, is_dir: bool) -> Result<()> {
    if is_dir {
        return Ok(());
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(work)
        .with_context(|| format!("open {}", work.display()))?;
    file.sync_all()
        .with_context(|| format!("flush {}", work.display()))
}

/// Publish a finished copy under a name of the bounded lists.
///
/// The name is **claimed** first — `create_new` for a file — so two runs
/// cannot publish onto one name, and the finished copy is then renamed onto
/// the claim. The rename is the publish: the name appears whole or not at
/// all.
///
/// A directory takes its name by being renamed onto it, because a directory
/// cannot be renamed over an existing name on every platform; the rename
/// itself then says whether the name was taken.
fn publish_copy(
    work: &Path,
    parent: &Path,
    prefix: &str,
    stamp: &str,
    is_dir: bool,
) -> Result<PathBuf> {
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
        if is_dir {
            match std::fs::rename(work, &cand) {
                Ok(()) => return Ok(cand),
                Err(e) if name_is_taken(&e) => continue,
                Err(e) => {
                    return Err(e).with_context(|| format!("publish {}", cand.display()));
                }
            }
        }
        match claim_name(&cand) {
            Ok(()) => {}
            Err(e) if name_is_taken(&e) => continue,
            Err(e) => {
                return Err(e).with_context(|| format!("claim {}", cand.display()));
            }
        }
        match std::fs::rename(work, &cand) {
            Ok(()) => return Ok(cand),
            Err(e) => {
                // The claim is ours and the copy did not land on it.
                let _ = std::fs::remove_file(&cand);
                return Err(e).with_context(|| format!("publish {}", cand.display()));
            }
        }
    }
    bail!(
        "no free backup name in {} after {MAX_CLAIM_ATTEMPTS} tries",
        parent.display()
    )
}

/// Take a name for a published copy, so that no two runs publish onto one.
fn claim_name(path: &Path) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(drop)
}

/// Whether an error says the name was taken rather than that the copy failed.
fn name_is_taken(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::AlreadyExists
            | std::io::ErrorKind::DirectoryNotEmpty
            | std::io::ErrorKind::NotADirectory
    )
}

/// The copies this process published and has not bounded yet.
///
/// The bound must not take back a copy the run applying it made: that run is
/// the one reporting the copy, and a bound that removed it would leave the
/// run reporting a backup that is not on disk.
///
/// The record is process-wide because the copies are made in more than one
/// place — the command layer copies a cache aside before it opens one, and an
/// open can copy one aside itself — and the bound has to know about both. An
/// entry is forgotten once the bound for its folder has run, so a long-lived
/// process does not go on sparing copies that are long finished with.
fn published_registry() -> &'static Mutex<Vec<PathBuf>> {
    static PUBLISHED: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
    PUBLISHED.get_or_init(|| Mutex::new(Vec::new()))
}

/// Record a published copy, so that this process's own bound spares it.
fn remember_published(path: &Path) {
    let mut published = published_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    published.retain(|p| p != path);
    published.push(path.to_path_buf());
}

/// The copies this process published under `parent`, taken out of the record.
///
/// Taking them is deliberate: the bound that spares them has run, and a later
/// bound must not go on sparing a copy its run has finished with.
fn take_published_under(parent: &Path) -> Vec<PathBuf> {
    let mut published = published_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut taken = Vec::new();
    published.retain(|p| {
        if p.parent() == Some(parent) {
            taken.push(p.clone());
            false
        } else {
            true
        }
    });
    taken
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
///
/// This claims the **working** name of a copy. The published name is claimed
/// separately, by [`publish_copy`], and only once the copy is complete.
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
///
/// Every sibling of the prefix is a **published** copy — a copy that is
/// still being written carries a working name and is not in this list at all
/// — so the bound can only ever take a whole copy away.
///
/// The copies in `spared` are the ones this process published, and the bound
/// does not take them: the run that made one is the run that reports it. The
/// count is made up from the next-oldest copy instead, so the folder still
/// holds `keep` copies.
fn retain_siblings(
    parent: &Path,
    prefix: &str,
    keep: NonZeroUsize,
    spared: &[PathBuf],
) -> Result<()> {
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
    let mut to_remove = siblings.len() - keep;
    for (_, path) in &siblings {
        if to_remove == 0 {
            break;
        }
        if spared.iter().any(|p| p == path) {
            continue;
        }
        remove_cache_path(path)?;
        to_remove -= 1;
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

    /// A copy that is still being written is not a member of a bounded list,
    /// so the bound cannot select it.
    ///
    /// The bound ranges over the published copies only, and a copy in flight
    /// carries a working name — so a run that merely opens a folder cannot
    /// take away the copy another run is making there, however often the
    /// bound is applied.
    #[test]
    fn a_copy_that_is_still_being_written_is_not_the_bounds_business() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "working");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");

        // The copy another run is making: the working name carries neither
        // of the two published prefixes.
        let working = dir.join(format!("{WORK_PREFIX}{STAMP}"));
        std::fs::write(&working, b"half a copy").unwrap();
        // Two published copies, so the bound has something to remove.
        let older = dir.join(format!("{BACKUP_PREFIX}20240101T000000.000000Z"));
        let newer = dir.join(format!("{BACKUP_PREFIX}20240102T000000.000000Z"));
        std::fs::write(&older, b"old").unwrap();
        std::fs::write(&newer, b"newer").unwrap();

        retain_copies(&db, NonZeroUsize::new(1).unwrap()).unwrap();

        assert!(
            working.exists(),
            "the copy that is being written is left alone"
        );
        assert!(!older.exists(), "the oldest published copy is gone");
        assert!(
            newer.exists(),
            "the newest published copy is kept: a working copy is not one of the N"
        );
    }

    /// A published name holds a whole copy: the working name is gone and the
    /// published one has the bytes.
    #[test]
    fn a_published_copy_is_the_whole_cache() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "publish");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        std::fs::write(&db, b"the cache bytes").unwrap();

        let published = backup_db(&db).unwrap().expect("the cache was copied aside");

        assert_eq!(
            std::fs::read(&published).unwrap(),
            b"the cache bytes",
            "the published copy holds the cache"
        );
        let working: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(WORK_PREFIX))
            .collect();
        assert!(
            working.is_empty(),
            "the working name is gone once the copy is published"
        );
    }

    /// The bound does not take back the copy the run applying it published.
    ///
    /// That run is the one reporting the copy, so a bound that removed it
    /// would leave the run reporting a backup that is not on disk. The
    /// folder still ends within its bound: the count is made up from the
    /// next-oldest copy instead.
    #[test]
    fn the_bound_keeps_the_copy_the_run_published() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "spared");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        std::fs::write(&db, b"the cache bytes").unwrap();

        let published = backup_db(&db).unwrap().expect("the cache was copied aside");
        // Another run published a newer copy after this one.
        let newer = dir.join(format!("{BACKUP_PREFIX}29991231T235959.999999Z"));
        std::fs::write(&newer, b"newer").unwrap();

        retain_copies(&db, NonZeroUsize::new(1).unwrap()).unwrap();

        assert!(
            published.exists(),
            "the copy this process published is not the bound's to remove"
        );
        assert!(
            !newer.exists(),
            "and the folder still holds exactly the bound"
        );
    }

    /// A published copy is named after the time it became whole.
    ///
    /// The bound orders the copies by that name, so a name that said when the
    /// copy *started* would make the copy of a slow run look old the moment
    /// it landed — and the bound would take it away from the run that had
    /// just made it. The two stamps are therefore read at two different
    /// moments, and the published one is the later.
    #[test]
    fn a_published_copy_is_named_after_it_became_whole() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "stamp");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        std::fs::write(&db, b"the cache bytes").unwrap();

        // The clock answers once per name: the working copy first, the
        // published copy once it is whole.
        let mut stamps = ["20240101T000000.000000Z", "20240102T000000.000000Z"]
            .into_iter()
            .map(String::from);
        let mut clock = move || stamps.next().expect("one stamp per name");

        let published = copy_aside(&db, BACKUP_PREFIX, "backup", &mut clock)
            .unwrap()
            .expect("the cache was copied aside");

        assert_eq!(
            published.file_name().unwrap().to_string_lossy(),
            format!("{BACKUP_PREFIX}20240102T000000.000000Z"),
            "the published name says when the copy became whole"
        );
    }

    /// A sibling another run removed a moment ago is not a failure.
    #[test]
    fn a_sibling_another_run_removed_is_not_a_failure() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "gone");
        let dir = t.subdir("w");
        let missing = dir.join(format!("{BACKUP_PREFIX}{STAMP}"));

        remove_cache_path(&missing).expect("a copy that is already gone is not a failure");
    }
}
