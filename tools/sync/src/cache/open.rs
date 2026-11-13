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
//! name of the bounded lists only once it is complete. The publish puts the
//! whole copy under its name — a file copy is linked into place — so a name
//! of a list never holds half a copy, and a copy that is still being made is
//! not a member of any list — which is what makes the bound safe to apply at
//! any moment: it can only ever select a copy that is whole.
//!
//! A copy that cannot be linked into place — a directory copy, and a file
//! copy on a filesystem that cannot link one — is renamed onto a name it
//! claims first, and the name is briefly empty until the rename lands: an
//! empty file, or an empty directory for a directory copy. The claim is what
//! keeps two runs off one name, and a bound cannot tell a claim from a
//! published copy, so the publish checks that its claim is still in place
//! before the copy lands on it and fails loudly when it is not.
//!
//! The working name is scratch: it goes once the copy is published. A
//! removal that fails leaves it behind and the run warns about it — the copy
//! is published and whole either way, so the run does not fail over the
//! leftover.
//!
//! "Finished" and "in flight" are told apart by the name, and the name is
//! the only thing the bound reads to make that distinction.
//!
//! ## A published copy carries a record of its own
//!
//! A copy is published **before** the run opens the folder's cache, and that
//! gap is where a copy can be lost. The run that published it has not taken
//! the cache's own exclusive lock yet, so another run can be in its run phase
//! for the folder — and its bound can take the copy away from the run that is
//! about to report it.
//!
//! So a copy is published together with a **record**: a file of its own,
//! named after the copy, which the publishing run holds an exclusive OS lock
//! on until it opens the folder's cache. The lock is what says the run is
//! still in the gap, and the OS releases it when the process ends and when
//! the process is *killed*, so a record left behind by a dead run is stale
//! rather than a claim that never expires. There is no age and no lease in
//! it: either a run holds the record or it does not. The record is an empty
//! file, because the lock on it is the whole of what it says — there is
//! nothing in it to read, and nothing in it that could go stale.
//!
//! Once the run holds the cache's own lock, that lock is the protection: no
//! other run is in its run phase for the folder, so no other run's bound can
//! take the copy. The record has done its work, and it is released.
//!
//! A filesystem that cannot lock a file has no records to speak: the copy is
//! published without one, as it was published before a record existed, and the
//! run does not fail over a mark it cannot make.

use anyhow::{Context, Result, bail};
use redb::ReadableDatabase;
use std::collections::HashMap;
use std::fs::{File, OpenOptions, TryLockError};
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

/// The prefix every name of the tool's own state carries.
///
/// The live cache, its copies, its snapshots and its records all start with
/// `girpr-cache`, which is what keeps them out of a mirror.
const TOOL_PREFIX: &str = "girpr-cache-";

/// The prefix of the record that says a published copy is still in use.
///
/// A record is a file of its own beside the copy it speaks for, and the run
/// that published the copy holds an exclusive OS lock on it until the run
/// opens the folder's cache. The lock is the whole of the record's meaning:
/// the OS releases it when the process ends and when the process is killed,
/// so a record left behind by a dead run is stale rather than a claim that
/// never expires.
///
/// The record is empty. There is no body to it — no pid, no clock — because
/// the lock is what a bound reads, and a body would say nothing that could
/// be acted on: a run that holds the record is alive whatever it wrote down,
/// and a run that is gone cannot be asked about it.
///
/// The name is in the tool's own namespace, so a record is tool state and is
/// never mirrored as content.
const HOLD_PREFIX: &str = "girpr-cache-hold-";

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
/// A cache that does **not** open is bounded as well, under the same lock
/// taken for the bound: a corrupt cache, or a legacy sled directory the open
/// refuses, must not leave the copies beside it to accumulate until
/// `--ignore-cache` rebuilds the cache. The lock is taken on the cache file
/// whatever the file can be opened for, so a cache that is not writable is
/// locked too; only a cache that cannot be opened at all has no lock to take.
///
/// A copy is published **before** that lock is taken, so there is a gap in
/// which the lock does not yet speak for it: another run can be in its run
/// phase for the folder then, and its bound can take the copy away from the
/// run that is about to report it. A published copy therefore carries a
/// **record** of its own — a file the publishing run holds an OS lock on
/// until it opens this cache — and the open releases that record here, where
/// the cache's own lock takes over the protection.
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
            let db = match CacheDb::open_file(db_path) {
                Ok(db) => db,
                Err(e) => {
                    // The cache did not open, so the bound below does not
                    // run — and a corrupt cache, or a legacy sled directory
                    // the open refuses, would leave the copies beside it to
                    // accumulate until --ignore-cache rebuilt the cache. The
                    // folder is bounded here instead, under the cache's own
                    // lock where one can be taken.
                    //
                    // The run's copies are the ones published since the last
                    // open of this folder's cache, and they are the ones the
                    // bound spares.
                    begin_run(folder_of(db_path));
                    if let Some(keep) = retain
                        && let Err(bound) = bound_after_failed_open(db_path, keep)
                    {
                        warn!(
                            path = %db_path.display(),
                            error = %bound,
                            "the copies beside a cache that did not open could not be bounded"
                        );
                    }
                    return Err(e);
                }
            };
            // The cache's own exclusive lock is now held, so no other run is
            // in its run phase for this folder: from here on, another run's
            // bound cannot take a copy this run published. The records that
            // covered the gap between publishing a copy and taking this lock
            // have done their work.
            let parent = folder_of(db_path);
            release_holds_under(parent);
            // The copies published since the last open of this folder's cache
            // are the ones this run published, and the ones the bound below
            // spares: a copy an earlier run in this process published is an
            // ordinary copy again.
            begin_run(parent);
            CacheDb::init_meta(&db, case_sensitive)?;
            // The bound, applied whether or not a copy was made: it is a
            // statement about the folder, not about this open.
            //
            // It runs here, after the open, and that is the point: the open
            // holds the cache file's exclusive lock, so no other run is in
            // its run phase for this folder while the bound runs. A copy
            // another run is still using is therefore not one the bound can
            // take away, and a copy this run made is spared (see
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
/// The copy is **published**: it is written under a working name and
/// published under its own name once it is complete, so the name never holds
/// half a copy and no bound can select a copy that is still being made.
///
/// How many copies are kept beside the cache is the bound's business: the
/// bound is about the folder, so it is applied by the caller whether or not
/// this call made a copy.
#[tracing::instrument(skip_all, fields(db = %db_path.display()))]
pub fn backup_db(db_path: &Path) -> Result<Option<PathBuf>> {
    copy_aside(db_path, BACKUP_PREFIX, "backup", ts_now)
}

/// Copy an existing cache to `<parent>/girpr-cache-old-<ts>` to record the
/// pre-sync dst state. Returns the snapshot path, or `None` when absent.
///
/// The snapshot is published like a backup is: whole, or not at all.
///
/// The bound is applied here too, for the same reason: it is about the
/// folder, so the caller applies it whether or not this call made a copy.
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
/// **The bound runs under the folder's cache lock.** A bound that runs while
/// another run is in its run phase for the folder can take that run's
/// copies: the cache's own exclusive lock is what says no other run is
/// there. The open holds that lock while it bounds, and a folder whose cache
/// did not open takes it for the bound; a folder with no lock to take — a
/// legacy sled directory, a cache that cannot be locked, or a cache that
/// cannot be opened at all — is bounded without one, and the copies a live
/// run published there are protected by the records beside them instead.
///
/// **A copy a live run is using is not the bound's to remove.** A published
/// copy carries a record of its own, and the run that published it holds a
/// lock on that record until it opens the folder's cache. A bound that meets
/// a locked record leaves the copy alone: the run that published it is the
/// run that reports it, and a bound that took it back would leave that run
/// reporting a backup that is not on disk. The bound still holds the folder
/// to its bound: the count is made up from the next-oldest copy instead.
///
/// **The copy the run published is spared as well**, and for the same reason,
/// whether or not the copy could be given a record at all: a folder whose
/// filesystem cannot lock a file still must not lose the copy the run is
/// about to report. It is spared by every bound that run makes for that list,
/// and not only by the first one: a bound that took the copy's entry would
/// leave the next bound free to take the copy the run is still about to
/// report. A copy an **earlier** run in this process published is an
/// ordinary copy again — sparing it would keep an older copy in place of a
/// newer one — so only the copies the run that opened this folder's cache
/// published are spared.
fn retain_copies(db_path: &Path, keep: NonZeroUsize) -> Result<()> {
    let parent = folder_of(db_path);
    let published = published_under(parent);
    // One reading of the folder: the bound is a statement about what the
    // folder holds now, and a copy published after this is not one of them.
    let folder = read_folder(parent)?;
    retain_siblings(&folder, parent, BACKUP_PREFIX, keep, &published)?;
    retain_siblings(&folder, parent, OLD_PREFIX, keep, &published)?;
    reclaim_records(&folder)
}

/// Bound the folder of a cache that did not open.
///
/// The bound is applied under the cache's own exclusive lock, which the open
/// holds. A cache that did not open holds no such lock, so the folder is
/// bounded only when this run can take that lock itself: a lock another run
/// holds says that run is in its run phase for the folder, and the copies it
/// published are protected by it — so that run bounds the folder, and this
/// one leaves it alone.
///
/// A cache that is a directory — a legacy sled cache the open refuses — has
/// no file to lock, and nothing opens one, so every copy published beside it
/// is still marked by the record of the run that published it, and the bound
/// leaves those alone.
fn bound_after_failed_open(db_path: &Path, keep: NonZeroUsize) -> Result<()> {
    match take_the_cache_lock(db_path) {
        CacheLock::Ours(file) => {
            // The lock is held for as long as the bound runs, so no other
            // run is in its run phase for the folder while it does.
            let bounded = retain_copies(db_path, keep);
            drop(file);
            bounded
        }
        // Another run holds the cache: it is in its run phase for the
        // folder, and it bounds the folder itself.
        CacheLock::Theirs => Ok(()),
        // There is no lock to take: the bound runs without one, as it does
        // on a folder whose filesystem cannot lock a file.
        CacheLock::Unheld => retain_copies(db_path, keep),
    }
}

/// The cache's own exclusive lock, for a folder whose cache did not open.
enum CacheLock {
    /// The lock is ours, and is held for as long as the bound runs.
    Ours(File),
    /// Another run holds it: that run is in its run phase for the folder,
    /// and the copies it published are protected by that lock.
    Theirs,
    /// There is no lock to take: the bound runs without one.
    Unheld,
}

/// Take the cache's own exclusive lock, for a folder whose cache did not
/// open.
fn take_the_cache_lock(db_path: &Path) -> CacheLock {
    if !db_path.is_file() {
        // A cache that is not a file is a legacy sled directory, and
        // nothing opens one.
        return CacheLock::Unheld;
    }
    let file = match open_for_the_lock(db_path) {
        Ok(file) => file,
        Err(e) => {
            // The cache cannot be opened at all, so no run has it open
            // either, and there is no lock to take.
            warn!(
                path = %db_path.display(),
                error = %e,
                "the cache cannot be opened; the folder is bounded without its lock"
            );
            return CacheLock::Unheld;
        }
    };
    match file.try_lock() {
        Ok(()) => CacheLock::Ours(file),
        Err(TryLockError::WouldBlock) => CacheLock::Theirs,
        Err(TryLockError::Error(e)) => {
            // The filesystem cannot lock a file, so the records cannot
            // speak either: the bound is applied as it is on a folder whose
            // cache cannot be locked at all.
            warn!(
                path = %db_path.display(),
                error = %e,
                "the cache's lock cannot be taken; the folder is bounded without it"
            );
            CacheLock::Unheld
        }
    }
}

/// Open the cache file for the lock the bound takes.
///
/// The lock is taken on the cache file itself, and it is what says that no
/// other run is in its run phase for the folder. **A cache file that cannot
/// be opened for writing is still a file that can be locked.** A run in its
/// run phase holds the cache open for writing, so a cache nobody can open
/// for writing is a cache no run has open — but a run that opened it before
/// the file stopped being writable is still holding it, and the lock is
/// exactly what says so.
fn open_for_the_lock(db_path: &Path) -> std::io::Result<File> {
    match OpenOptions::new().read(true).write(true).open(db_path) {
        Ok(file) => Ok(file),
        Err(e) => match OpenOptions::new().read(true).open(db_path) {
            Ok(file) => Ok(file),
            Err(_) => Err(e),
        },
    }
}

/// The folder's entries, read once and ordered by name.
///
/// One reading serves both lists and the records: the bound is a statement
/// about the folder as it is now. The order is by name, which is very nearly
/// the order by age: the bound orders the copies it ranges over by age
/// proper (see [`published_age`]).
fn read_folder(parent: &Path) -> Result<Vec<(std::ffi::OsString, PathBuf)>> {
    let mut entries: Vec<(std::ffi::OsString, PathBuf)> = std::fs::read_dir(parent)
        .with_context(|| format!("read dir {}", parent.display()))?
        .filter_map(|e| e.ok())
        .map(|e| (e.file_name(), e.path()))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}

/// When a published copy became whole: the stamp its name carries and, when
/// the name carries the fallback suffix, the number after it.
///
/// The stamp sorts lexicographically, so comparing the stamps compares the
/// times. The fallback suffix does not: `_10` sorts before `_9`, so a folder
/// holding ten copies published inside one microsecond would order them
/// wrongly by name alone. The number is therefore read as a number and
/// compared as one, and the two together are the age of a copy.
///
/// The plain name has no suffix and is the first of its microsecond, so it
/// counts as zero.
fn published_age(path: &Path, prefix: &str) -> (String, u32) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tail = name.strip_prefix(prefix).unwrap_or(&name);
    match tail.rsplit_once('_') {
        Some((stamp, n)) => match n.parse::<u32>() {
            Ok(n) => (stamp.to_string(), n),
            Err(_) => (tail.to_string(), 0),
        },
        None => (tail.to_string(), 0),
    }
}

/// Copy `db_path` beside itself as `<prefix><ts>`. A directory (a legacy sled
/// cache) is copied recursively; a redb file is copied in one call. The
/// destination never replaces an existing sibling.
///
/// **The copy is published, not written in place.** It is written under a
/// working name ([`WORK_PREFIX`]) and published under its own name once it
/// is complete, so:
///
/// - a name of a bounded list never holds half a copy, and
/// - a copy that is still being made is not a member of any list, so no
///   bound — however often it is applied — can select it.
///
/// The publish is what claims the published name, so two runs cannot publish
/// over each other.
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
    // have to be on disk before the name appears: the name can reach the disk
    // before the data it names does.
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
/// The publish puts the name on the copy, and the name can be durable before
/// the data it names is, so the bytes are flushed first. A legacy sled
/// directory is copied file by file and is left as it is: the live cache is a
/// file, and the directory case is a migration path rather than the one that
/// runs.
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
/// A file copy is published by **linking** it into place, so the name is
/// created with the copy's bytes in it: the name appears whole, and it
/// appears *with* the copy rather than a moment before it. The link is also
/// the claim — it fails when the name is taken — so two runs cannot publish
/// onto one name.
///
/// A directory cannot be linked, so its name is claimed with an exclusive
/// create and the copy is renamed onto the claim: a POSIX rename onto an
/// existing empty directory succeeds, so the claim is what keeps two runs
/// from publishing onto one name.
///
/// **The record of the copy is taken before the name is claimed.** The record
/// says the copy is in use by the run that is about to publish it, and a
/// bound that met the name without the record would have nothing to read and
/// would take the copy away from the run publishing it. Taking the record
/// first also settles the name: a run whose record for a name is already held
/// is looking at a name another run is publishing under, so it moves on to the
/// next one rather than racing for it.
///
/// A filesystem that cannot lock a file has no records to speak, so the copy
/// is published without one — as it was published before a record existed —
/// rather than not published at all. The record the attempt made goes with it:
/// a record nobody holds and nobody can lock reads as "a live run holds it"
/// to a bound, which would spare the copy for good.
///
/// **A claim that is taken away fails the publish.** A bound cannot tell a
/// claim from a published copy, and on a filesystem that cannot lock a file
/// there is no record beside the name to say the name is being claimed, so
/// the claim can go. The publish checks that its claim is still in place
/// before the copy lands on it, and fails loudly when it is not: landing the
/// copy on a name this run no longer holds would publish it over whatever
/// holds it now.
fn publish_copy(
    work: &Path,
    parent: &Path,
    prefix: &str,
    stamp: &str,
    is_dir: bool,
) -> Result<PathBuf> {
    for n in 1..=MAX_CLAIM_ATTEMPTS {
        let name = candidate_name(prefix, stamp, n);
        let cand = parent.join(&name);
        // Ask first whether the name is taken: `create_new` on an existing
        // *directory* reports `Access is denied` rather than `AlreadyExists`,
        // so the error kind alone cannot distinguish "occupied" from "cannot
        // write here".
        match std::fs::symlink_metadata(&cand) {
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("stat {}", cand.display())),
        }
        // The record comes before the claim, so a published name is never
        // without one.
        let hold = match take_hold(parent, &name)? {
            Claim::Ours(hold) => Some(hold),
            // A live run holds this name's record: the name is that run's,
            // and the next one is ours to try.
            Claim::Taken => continue,
            // The filesystem cannot lock a file, so there is no record to
            // keep: the copy is published as it was before a record existed,
            // and the run's own bound still spares it through the published
            // registry.
            Claim::Unusable => None,
        };
        let published = if is_dir {
            publish_dir(work, &cand)
        } else {
            publish_file(work, &cand)
        };
        match published {
            Ok(()) => {
                keep_hold(hold);
                return Ok(cand);
            }
            // The name is another run's: the next one is ours to try.
            Err(PublishError::Taken) => {
                discard_hold(hold);
                continue;
            }
            Err(PublishError::Io(e)) => {
                discard_hold(hold);
                return Err(e).with_context(|| format!("publish {}", cand.display()));
            }
        }
    }
    bail!(
        "no free backup name in {} after {MAX_CLAIM_ATTEMPTS} tries",
        parent.display()
    )
}

/// Why a copy did not land on the name it was published under.
enum PublishError {
    /// The name is taken: another run has it, and the next name is ours.
    Taken,
    /// The publish itself failed.
    Io(std::io::Error),
}

/// Publish a finished file copy under its published name.
///
/// The publish is a **link**: the name is created with the copy's bytes in
/// it, so there is no moment at which the name is there and empty. A
/// rename cannot do that on its own — the rename has to be preceded by a
/// claim of the name, and the claim leaves the name empty for as long as
/// the rename takes.
///
/// The link is also the claim: it fails when the name is taken, so two runs
/// cannot publish onto one name.
///
/// A filesystem that cannot link a file has no such publish. The copy is
/// then renamed onto a name it claims first, and the name is briefly empty
/// there — which is how a copy was published before links were used. The
/// claim is checked before the copy lands on it, so a claim that was taken
/// away fails the publish rather than landing the copy on a name this run no
/// longer holds.
fn publish_file(work: &Path, cand: &Path) -> Result<(), PublishError> {
    match link_file(work, cand) {
        Ok(()) => {
            // The name exists with the copy's bytes in it. The working name
            // is scratch, so it goes: the copy now lives under its
            // published name alone.
            #[cfg(test)]
            pause_publish();
            if let Err(e) = remove_working_copy(work) {
                warn!(
                    path = %work.display(),
                    error = %e,
                    "the working copy of a published copy could not be removed; the working name is left behind"
                );
            }
            Ok(())
        }
        // The name is taken: it is another run's.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(PublishError::Taken),
        Err(e) => {
            // The filesystem cannot link a file, so the copy is published by
            // renaming it onto a name it claims first.
            trace!(
                path = %cand.display(),
                error = %e,
                "the copy cannot be linked into place; it is renamed onto a claimed name"
            );
            match claim_name(cand) {
                Ok(()) => {}
                Err(e) if name_is_taken(&e) => return Err(PublishError::Taken),
                Err(e) => return Err(PublishError::Io(e)),
            }
            // The name is claimed, and the claim is what the rename replaces.
            #[cfg(test)]
            pause_publish();
            // The claim has to still be ours when the copy lands on it.
            if !claim_is_in_place(cand) {
                return Err(PublishError::Io(claim_taken_away()));
            }
            match std::fs::rename(work, cand) {
                Ok(()) => Ok(()),
                Err(e) => {
                    // The claim is ours and the copy did not land on it.
                    let _ = std::fs::remove_file(cand);
                    Err(PublishError::Io(e))
                }
            }
        }
    }
}

/// Link a file under a second name, if the filesystem can.
///
/// A filesystem that cannot link a file is not something a test can make, so
/// the failure is injected here: with the switch on, the link fails the way
/// it fails on such a filesystem.
fn link_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if LINKING_UNAVAILABLE.with(|f| f.get()) {
        return Err(std::io::Error::other("this filesystem cannot link a file"));
    }
    std::fs::hard_link(src, dst)
}

/// Publish a finished directory copy under its published name.
///
/// A directory cannot be linked, so the name is **claimed** with an
/// exclusive create and the copy is renamed onto the claim. The claim is
/// what decides: a POSIX rename onto an existing **empty** directory
/// succeeds, so a rename on its own would let a second run publish onto a
/// name the first run has, and replace the copy that is there.
///
/// The rename replaces the claim where the platform allows it, so the name
/// is briefly an empty directory. Where a rename refuses to replace a
/// directory, the claim is dropped and the rename tried again: a name taken
/// in the meantime makes that rename fail, which is the answer we want.
///
/// The claim is checked before the copy lands on it, so a claim that was
/// taken away — a bound cannot tell a claim from a published copy — fails
/// the publish rather than landing the copy on a name this run no longer
/// holds.
fn publish_dir(work: &Path, cand: &Path) -> Result<(), PublishError> {
    match std::fs::create_dir(cand) {
        Ok(()) => {}
        // The name is taken: it is another run's.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(PublishError::Taken),
        Err(e) => return Err(PublishError::Io(e)),
    }
    // The publish is held still for a moment, so a test can look at the
    // folder in the middle of one: the claim is there, empty, under a
    // published name.
    #[cfg(test)]
    pause_publish();
    // The claim has to still be ours when the copy lands on it.
    if !claim_is_in_place(cand) {
        return Err(PublishError::Io(claim_taken_away()));
    }
    match std::fs::rename(work, cand) {
        Ok(()) => Ok(()),
        Err(_) => {
            // The claim is in the way of the rename. Only an empty directory
            // goes, so a copy another run has published is never dropped
            // here.
            let _ = std::fs::remove_dir(cand);
            match std::fs::rename(work, cand) {
                Ok(()) => Ok(()),
                Err(e) => {
                    // The name is there, so another run has it.
                    if cand.exists() {
                        Err(PublishError::Taken)
                    } else {
                        Err(PublishError::Io(e))
                    }
                }
            }
        }
    }
}

// The test-only switch that makes linking a file fail.
#[cfg(test)]
std::thread_local! {
    static LINKING_UNAVAILABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// The test-only switch that pauses a publish once the name is there.
//
// A publish is over in microseconds, so a test that wants to look at the
// folder in the middle of one needs the publish to wait. The pause is a
// plain sleep: it is only ever on for the length of one test.
#[cfg(test)]
std::thread_local! {
    static PUBLISH_PAUSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Hold a publish still for a moment, so a test can look at the folder.
#[cfg(test)]
fn pause_publish() {
    if PUBLISH_PAUSED.with(|f| f.get()) {
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

/// Take a name for a published copy, so that no two runs publish onto one.
fn claim_name(path: &Path) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(drop)
}

/// Remove the working copy of a published copy.
///
/// The working name is scratch: the copy now lives under its published name
/// alone, so the working name goes. A removal that fails is not a failure of
/// the publish — the copy is published and whole — so the caller warns about
/// the leftover rather than failing the run over it.
fn remove_working_copy(work: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if REMOVAL_UNAVAILABLE.with(|f| f.get()) {
        return Err(std::io::Error::other("this file cannot be removed"));
    }
    std::fs::remove_file(work)
}

/// Whether the claim on a published name is still in place.
///
/// The claim is what keeps two runs off one name, and it has to be there when
/// the copy lands on it. A bound that meets a claim cannot tell it from a
/// published copy — it is an empty directory, or an empty file, under a
/// published name — and on a filesystem that cannot lock a file there is no
/// record beside the name to say that the name is being claimed. So the claim
/// can be taken away, and a publish that finds it gone must not land the copy
/// on the name: it would be publishing over whatever holds it now.
fn claim_is_in_place(cand: &Path) -> bool {
    cand.symlink_metadata().is_ok()
}

/// The error a publish fails with when its claim was taken away.
fn claim_taken_away() -> std::io::Error {
    std::io::Error::other("the claimed name was taken away before the copy landed on it")
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

/// The copies this process published in one folder.
///
/// The bound must not take back a copy the run made: that run is the one
/// reporting the copy, and a bound that removed it would leave the run
/// reporting a backup that is not on disk.
///
/// The two lists are the two halves of that sentence. A copy is **pending**
/// from the moment it is published until the folder's cache is opened: the
/// run that opens the cache is the run that published it, and the copies
/// waiting when the cache opens are that run's copies. They are **current**
/// from then on, and the bounds that run makes spare them.
///
/// A copy an earlier run in this process published is neither: it is an
/// ordinary copy again, and sparing it would keep an older copy in place of
/// a newer one.
#[derive(Default)]
struct FolderCopies {
    /// Published since the folder's cache was last opened.
    pending: Vec<PathBuf>,
    /// Published by the run that last opened the folder's cache.
    current: Vec<PathBuf>,
}

/// The copies this process published, by the folder they sit in.
///
/// The registry is keyed by the folder, because the copies are made in more
/// than one place — the command layer copies a cache aside before it opens
/// one, and an open can copy one aside itself — and a bound for one folder
/// has to know about both of them without disturbing another folder's.
///
/// **A bound does not consume an entry.** The copy a run published is the copy
/// that run reports, so every bound that run makes for that list spares it,
/// and not only the first one: a bound that took the entry would leave the
/// next bound free to take the copy the run is still about to report. An
/// entry is forgotten when the copy is gone, because there is nothing left to
/// spare.
///
/// The bound spares the newest of a list's entries: a run publishes one copy
/// per list, so the newest is the copy the current run published, and the
/// ones before it belong to earlier runs in the same process.
fn published_registry() -> &'static Mutex<HashMap<PathBuf, FolderCopies>> {
    static PUBLISHED: OnceLock<Mutex<HashMap<PathBuf, FolderCopies>>> = OnceLock::new();
    PUBLISHED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record a published copy, so that the run that published it spares it.
///
/// The copy is **pending**: it belongs to the run that opens the folder's
/// cache next, and it becomes that run's copy when the cache is opened.
fn remember_published(path: &Path) {
    let mut published = published_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let copies = published.entry(folder_of(path).to_path_buf()).or_default();
    copies.pending.retain(|p| p != path);
    copies.pending.push(path.to_path_buf());
}

/// The copies the run that opened this folder's cache published.
///
/// The entries are read and not taken: a bound spares the copy the run
/// published, and a later bound for the same folder spares it too. An entry
/// whose copy is gone is forgotten — there is nothing left to spare.
fn published_under(folder: &Path) -> Vec<PathBuf> {
    let mut published = published_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let Some(copies) = published.get_mut(folder) else {
        return Vec::new();
    };
    copies.current.retain(|p| p.exists());
    copies.pending.retain(|p| p.exists());
    let spared = copies.current.clone();
    if spared.is_empty() && copies.pending.is_empty() {
        published.remove(folder);
    }
    spared
}

/// The run that opens this folder's cache is the run that published the
/// copies waiting for it.
///
/// The copies published since the folder's cache was last opened are the
/// ones this run published, and the ones its bound spares. The copies an
/// earlier run in this process published are ordinary copies again: sparing
/// one of them would keep an older copy in place of a newer one.
///
/// **A bound does not consume the entries.** The copy a run published is the
/// copy that run reports, so every bound that run makes for that list spares
/// it, and not only the first one: a bound that took the entry would leave
/// the next bound free to take the copy the run is still about to report.
fn begin_run(folder: &Path) {
    let mut published = published_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let Some(copies) = published.get_mut(folder) else {
        return;
    };
    copies.current = std::mem::take(&mut copies.pending);
}

/// The folder a path of the tool's own state sits in.
///
/// A path with no directory part sits in the current directory, and that is
/// the folder the copies beside it are counted in.
fn folder_of(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new("."))
}

/// The record that says a published copy is in use, and the lock that says the
/// run holding it is still in the gap between publishing the copy and
/// opening the folder's cache.
///
/// The **handle** is the record's life: the OS lock on it is what a bound
/// tests, and the handle has to stay open for the lock to be held. Dropping
/// it releases the lock — which is what happens when the process ends, and
/// what happens when the process is killed.
struct Hold {
    path: PathBuf,
    file: File,
}

impl Drop for Hold {
    fn drop(&mut self) {
        // The OS handle is the exclusion; releasing it lets the next run in.
        // The file is not removed here: a caller that gives the record up
        // removes it while the lock is still held, and a record that outlives
        // its holder says nothing and is reclaimed by the next bound.
        let _ = self.file.unlock();
    }
}

/// The records this process is holding, one per published copy.
///
/// A record is held from the moment the copy is published until the run
/// opens the folder's cache: that is the gap in which another run's bound
/// could take the copy, and it is over the moment the run holds the cache's
/// own exclusive lock.
///
/// The records are process-wide because the copies are made in more than one
/// place — the command layer copies a cache aside before it opens one, and an
/// open can copy one aside itself — and the open has to know about both.
fn hold_registry() -> &'static Mutex<Vec<Hold>> {
    static HOLDS: OnceLock<Mutex<Vec<Hold>>> = OnceLock::new();
    HOLDS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Keep a record until the folder's cache is open.
///
/// A copy published without a record has nothing to keep.
fn keep_hold(hold: Option<Hold>) {
    let Some(hold) = hold else {
        return;
    };
    hold_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(hold);
}

/// Release the records of the copies published beside `parent`.
///
/// The run has opened the folder's cache, so it holds the cache's own
/// exclusive lock, and no other run is in its run phase for the folder: from
/// here on the lock is what keeps another run's bound off the copy, and the
/// record has nothing left to say.
///
/// The record's file goes with it. The removal happens while the lock is
/// still held, so no other run can have taken the record over in the
/// meantime, and the copy is left with the protection that now covers it.
fn release_holds_under(parent: &Path) {
    let mut holds = hold_registry().lock().unwrap_or_else(|e| e.into_inner());
    let mut released = Vec::new();
    let mut kept = Vec::with_capacity(holds.len());
    for hold in holds.drain(..) {
        if hold.path.parent() == Some(parent) {
            released.push(hold);
        } else {
            kept.push(hold);
        }
    }
    *holds = kept;
    drop(holds);
    // The file goes while the lock is still held: a run that took the record
    // over in the meantime would be holding the lock, and the record would
    // not be ours to remove.
    for hold in &released {
        let _ = std::fs::remove_file(&hold.path);
    }
    drop(released);
}

/// Give a record up: the copy was not published, so nothing is using it.
///
/// The file goes before the handle does. While the lock is held no other run
/// can take the record over, so the removal is unambiguously ours.
///
/// A copy published without a record has nothing to give up.
fn discard_hold(hold: Option<Hold>) {
    let Some(hold) = hold else {
        return;
    };
    let _ = std::fs::remove_file(&hold.path);
    drop(hold);
}

/// The name of the record that speaks for a published copy.
///
/// The record is named after the copy: the copy's own name with the tool's
/// prefix (`girpr-cache-`) replaced by the record's (`girpr-cache-hold-`). A
/// `girpr-cache-backup-<ts>` copy is therefore spoken for by a
/// `girpr-cache-hold-backup-<ts>` record, and the two lists keep their own
/// records: a `girpr-cache-old-<ts>` snapshot by a `girpr-cache-hold-old-<ts>`
/// one. The name never carries a prefix of its own, so a record is not one of
/// the copies a bound ranges over.
fn hold_name(copy_name: &str) -> Option<String> {
    copy_name
        .strip_prefix(TOOL_PREFIX)
        .map(|tail| format!("{HOLD_PREFIX}{tail}"))
}

/// The record that speaks for a published copy, if it has one.
fn record_path(parent: &Path, copy_name: &str) -> Option<PathBuf> {
    hold_name(copy_name).map(|name| parent.join(name))
}

/// How many times a record is taken before the copy is published without one.
///
/// The retry is for the one race there is: a bound that meets a record between
/// its creation and its lock may reclaim it, and the record is then created
/// again. The number is a floor on a pathological folder, not a number
/// anything is expected to reach.
const HOLD_ATTEMPTS: u32 = 3;

/// What taking the record that speaks for a copy about to be published says.
///
/// The three answers are three different things, and the publisher reads each
/// of them differently: one is a mark it holds, one is a name that is not
/// its own, and one is a mark this filesystem cannot make at all.
enum Claim {
    /// The record is ours and locked: the copy is marked in use, and the lock
    /// is held until the run opens the folder's cache.
    Ours(Hold),
    /// Another live run holds the record: the name is that run's, and the
    /// next name is ours to try.
    Taken,
    /// The filesystem cannot lock a file: there is no record to keep, and the
    /// copy is published without one.
    Unusable,
}

/// Take the record that speaks for a copy about to be published.
///
/// The record is created and locked **before** the copy's published name
/// appears, so a published name does not appear without one beside it: the
/// two are published together, in that order.
///
/// The lock is what makes the record live. The OS releases it when the
/// holding process ends — and it releases it when the process is killed, so a
/// record left behind by a dead run is stale rather than a claim that never
/// expires. There is no age and no lease in it: either a run holds the
/// record or it does not. The record is an empty file, because the lock is
/// what a bound reads and there is nothing else for it to say.
///
/// A filesystem that cannot lock a file has no records to speak: the copy is
/// published without one, which is what it was published as before a record
/// existed, and the run does not fail over a mark it cannot make.
fn take_hold(parent: &Path, copy_name: &str) -> Result<Claim> {
    let Some(path) = record_path(parent, copy_name) else {
        return Ok(Claim::Unusable);
    };
    for _ in 1..=HOLD_ATTEMPTS {
        // Create the record, or open the one that is already there.
        let (file, created) = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => (file, true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                match OpenOptions::new().read(true).write(true).open(&path) {
                    Ok(file) => (file, false),
                    // The record was reclaimed between the two opens: it is
                    // made again from scratch.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => {
                        return Err(e).with_context(|| format!("open record {}", path.display()));
                    }
                }
            }
            Err(e) => {
                return Err(e).with_context(|| format!("create record {}", path.display()));
            }
        };
        // The lock is the record: it is taken for as long as the copy is in
        // use, and the OS takes it back when the holder is gone. It is taken
        // once, here, on the handle that will keep it.
        match lock_record(&file) {
            Ok(()) => {
                // The record must still be at its path: a bound that met it
                // between the create and the lock may have reclaimed it, and a
                // record that is not at its path says nothing to a bound.
                if path.symlink_metadata().is_err() {
                    continue;
                }
                return Ok(Claim::Ours(Hold { path, file }));
            }
            Err(TryLockError::WouldBlock) => {
                // A live run holds this record: the copy is that run's.
                return Ok(Claim::Taken);
            }
            Err(TryLockError::Error(e)) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "cannot lock the record of a copy; the copy is published without one"
                );
                // A record this run made goes with the decision: nobody holds
                // it, and a record nobody holds *and nobody can lock* reads as
                // "a live run holds it" to a bound, which would spare the copy
                // for good. One this run did not make is left alone: it is
                // not ours to remove, and the name is not ours to publish
                // under.
                if created {
                    let _ = std::fs::remove_file(&path);
                    return Ok(Claim::Unusable);
                }
                return Ok(Claim::Taken);
            }
        }
    }
    // The record would not stay put: it was taken away between the create and
    // the lock, again and again. A mark that will not stay put is not a mark,
    // so the copy is published without one rather than not published at all.
    Ok(Claim::Unusable)
}

/// Take an exclusive lock on a record, if the filesystem can lock a file.
///
/// A filesystem that cannot lock one is a filesystem whose records cannot say
/// anything: the publisher publishes the copy without a record rather than
/// failing the run, and a bound reads such a record as saying nothing at
/// all.
fn lock_record(file: &File) -> std::result::Result<(), TryLockError> {
    #[cfg(test)]
    if LOCKING_UNAVAILABLE.with(|f| f.get()) {
        return Err(TryLockError::Error(std::io::Error::other(
            "this filesystem cannot lock a file",
        )));
    }
    file.try_lock()
}

// The test-only switch that makes taking a record's lock fail.
//
// A filesystem that cannot lock a file is not something a test can make, so
// the failure is injected here: with the switch on, a record's lock fails the
// way it fails on such a filesystem — for the publisher and for the bound
// alike, because both read the same lock.
#[cfg(test)]
std::thread_local! {
    static LOCKING_UNAVAILABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// The test-only switch that makes removing the working copy fail.
//
// A removal that fails is not something a test can make the same way on
// every platform, so the failure is injected here: with the switch on, the
// removal fails the way it fails when the file will not go.
#[cfg(test)]
std::thread_local! {
    static REMOVAL_UNAVAILABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// What a published copy's record says about it.
enum Record {
    /// Nothing says the copy is in use: there is no record, or the record
    /// cannot be read or cannot be locked — and either of those says
    /// nothing at all.
    Unmarked,
    /// A live run holds the record: the copy is that run's to keep.
    InUse,
    /// The record is stale — the run that made it is gone. The lock is held
    /// here, so the record can be reclaimed with the copy it spoke for.
    Stale { path: PathBuf, _file: File },
}

/// What a copy's record says, and whether the bound may take the copy.
///
/// A record a live run holds spares the copy. A stale record says nothing, so
/// the copy is the bound's to take like any other, and the record is
/// reclaimed with it.
///
/// A record that cannot be read or cannot be locked says nothing as well, so
/// it does not spare the copy either: nobody can be holding a lock on a file
/// the filesystem cannot lock, and a record that cannot be read cannot be the
/// record of a live run. Such a record is left where it is — a bound that
/// cannot judge a record cannot claim it either — but it does not stop the
/// bound, so the copy it spoke for is an ordinary copy again.
fn record_state(parent: &Path, copy_name: &str) -> Result<Record> {
    let Some(path) = record_path(parent, copy_name) else {
        return Ok(Record::Unmarked);
    };
    match take_the_lock(&path) {
        RecordLock::Stale(file) => Ok(Record::Stale { path, _file: file }),
        RecordLock::Held => {
            trace!(path = %path.display(), "a copy's record is held; the copy is spared");
            Ok(Record::InUse)
        }
        RecordLock::Unreadable | RecordLock::Untestable => Ok(Record::Unmarked),
    }
}

/// What a record's lock says.
enum RecordLock {
    /// The lock was taken: nobody holds the record, so it is stale.
    Stale(File),
    /// A live run holds the record.
    Held,
    /// The record cannot be read.
    Unreadable,
    /// The lock cannot be tested: the filesystem cannot lock a file.
    Untestable,
}

/// Take the lock on a record, if it can be taken.
///
/// The answers are told apart because the bound reads them differently: a
/// record nobody holds is stale and the copy is the bound's to take, a record
/// a live run holds spares the copy, and a record that cannot be read or
/// whose lock cannot be tested says nothing at all — so it spares nothing
/// either.
///
/// The record is opened the way the run that holds it opens it, so a record
/// that cannot be opened that way is one no run could be holding.
fn take_the_lock(path: &Path) -> RecordLock {
    let file = match open_record(path) {
        Ok(file) => file,
        Err(e) => {
            trace!(path = %path.display(), error = %e, "a record cannot be read");
            return RecordLock::Unreadable;
        }
    };
    match lock_record(&file) {
        Ok(()) => RecordLock::Stale(file),
        Err(TryLockError::WouldBlock) => RecordLock::Held,
        Err(TryLockError::Error(e)) => {
            trace!(path = %path.display(), error = %e, "cannot test a record's lock");
            RecordLock::Untestable
        }
    }
}

/// Open a record, the way the run that holds it opens it.
///
/// The run that publishes a copy opens its record for reading and writing,
/// so a record that cannot be opened that way is one no run is holding.
fn open_record(path: &Path) -> std::io::Result<File> {
    match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => Ok(file),
        Err(e) => match File::open(path) {
            Ok(file) => Ok(file),
            Err(_) => Err(e),
        },
    }
}

/// Reclaim the records that no live run is holding.
///
/// A record whose run has ended says nothing: the copy it spoke for is an
/// ordinary copy again, and the record is tool state with no business sitting
/// in the folder. The lock is what tells the two apart, so a record a live run
/// holds is left alone.
///
/// The removal happens while the lock is held, so a run that took the record
/// over in the meantime — which it can only do once the lock is free — is not
/// left holding a record that is no longer there.
fn reclaim_records(folder: &[(std::ffi::OsString, PathBuf)]) -> Result<()> {
    for (name, path) in folder {
        if !name.to_string_lossy().starts_with(HOLD_PREFIX) {
            continue;
        }
        // Only a record whose lock can be taken is reclaimed: that is the
        // one that says its run is gone. A record that cannot be read or
        // whose lock cannot be tested says nothing, and a bound that cannot
        // judge it does not own it.
        if let RecordLock::Stale(file) = take_the_lock(path) {
            let _ = std::fs::remove_file(path);
            drop(file);
        }
    }
    Ok(())
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
/// The sibling names carry a timestamp that sorts lexicographically, so the
/// age of a copy is read from its name (see [`published_age`]), and the
/// newest `keep` are the ones that are left. Everything older is removed — a
/// directory recursively, a file outright — because the user asked for a
/// bound and a bound that does not remove anything is not a bound.
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
/// The copies in `published` that this process published under `prefix`.
///
/// A run publishes one copy per list, and the copy it publishes is the one
/// it reports, so the bound spares the **newest** copy this process published
/// in that list. The copies it published before that belong to earlier runs
/// in the same process, and those are ordinary copies again: the bound takes
/// them like any other, which is what keeps the folder within its bound.
fn newest_published<'a>(published: &'a [PathBuf], prefix: &str) -> Option<&'a Path> {
    published
        .iter()
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(prefix))
        })
        // The stamp says when the copy became whole, and the fallback suffix
        // says which of the copies made in that microsecond it is.
        .max_by(|a, b| published_age(a, prefix).cmp(&published_age(b, prefix)))
        .map(PathBuf::as_path)
}

/// A copy whose record a live run holds is spared for the same reason, and by
/// the same rule: the run that published it is still using it.
fn retain_siblings(
    folder: &[(std::ffi::OsString, PathBuf)],
    parent: &Path,
    prefix: &str,
    keep: NonZeroUsize,
    published: &[PathBuf],
) -> Result<()> {
    let mut siblings: Vec<&(std::ffi::OsString, PathBuf)> = folder
        .iter()
        .filter(|(name, _)| name.to_string_lossy().starts_with(prefix))
        .collect();
    // The oldest first: the bound takes the oldest copies away, so the order
    // it walks them in is the order of their age and not the order of their
    // names — the fallback suffix does not sort as a number.
    siblings.sort_by_key(|s| published_age(&s.1, prefix));
    let keep = keep.get();
    if siblings.len() <= keep {
        return Ok(());
    }
    let spared = newest_published(published, prefix);
    let mut to_remove = siblings.len() - keep;
    for (name, path) in siblings {
        if to_remove == 0 {
            break;
        }
        if spared == Some(path.as_path()) {
            continue;
        }
        match record_state(parent, &name.to_string_lossy())? {
            // A live run is using this copy: it is not the bound's to take.
            Record::InUse => continue,
            Record::Unmarked => {}
            Record::Stale {
                path: record,
                _file,
            } => {
                // The record is stale, so it goes with the copy it spoke for.
                // The removal happens while the lock is held, so no other run
                // can have taken the record over in the meantime.
                let _ = std::fs::remove_file(&record);
            }
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

    /// The bound orders the copies by age, not by their names.
    ///
    /// The fallback suffix does not sort as a number: `_10` sorts before
    /// `_9`, so ten copies published inside one microsecond would be ordered
    /// wrongly by name alone. The bound takes the oldest away, so the order
    /// it walks the copies in is the order they were published in.
    #[test]
    fn the_bound_orders_the_copies_by_age_and_not_by_name() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "age_order");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");

        // Ten copies published inside one microsecond: the plain name and
        // the `_2` … `_10` fallbacks.
        let names: Vec<String> = (1..=10)
            .map(|n| candidate_name(BACKUP_PREFIX, STAMP, n))
            .collect();
        for name in &names {
            std::fs::write(dir.join(name), b"a copy").unwrap();
        }

        retain_copies(&db, NonZeroUsize::new(1).unwrap()).unwrap();

        assert!(
            dir.join(&names[9]).exists(),
            "the newest copy is kept: {}",
            names[9]
        );
        assert!(
            !dir.join(&names[8]).exists(),
            "and the one published before it is gone: {}",
            names[8]
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

    /// A published name appears with the copy's bytes in it.
    ///
    /// The copy is published by linking it into place, so the name is
    /// created with the copy in it: there is no moment at which the name is
    /// there and empty. The publish is held still for a moment so the folder
    /// can be looked at in the middle of one.
    #[test]
    fn a_published_name_appears_with_the_copy_it_carries() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "publish_whole");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        std::fs::write(&db, b"the cache bytes").unwrap();

        let handle = std::thread::spawn(move || {
            PUBLISH_PAUSED.with(|f| f.set(true));
            backup_db(&db)
        });

        let published = wait_for_published(&dir);
        assert_eq!(
            std::fs::read(&published).unwrap(),
            b"the cache bytes",
            "the published name holds the copy from the moment it exists"
        );

        let published = handle
            .join()
            .unwrap()
            .unwrap()
            .expect("the cache was copied aside");
        assert_eq!(
            std::fs::read(&published).unwrap(),
            b"the cache bytes",
            "and the copy is whole once the publish is done"
        );
    }

    /// A filesystem that cannot link a file still gets the copy published.
    ///
    /// There is no link to publish with, so the copy is renamed onto a name
    /// it claims first — and the copy is whole either way.
    #[test]
    fn a_copy_is_published_on_a_filesystem_that_cannot_link() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "nolink");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        std::fs::write(&db, b"the cache bytes").unwrap();

        LINKING_UNAVAILABLE.with(|f| f.set(true));
        let published = backup_db(&db).unwrap().expect("the cache was copied aside");
        LINKING_UNAVAILABLE.with(|f| f.set(false));

        assert_eq!(
            std::fs::read(&published).unwrap(),
            b"the cache bytes",
            "the copy is published even though it cannot be linked into place"
        );
        let working: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(WORK_PREFIX))
            .collect();
        assert!(
            working.is_empty(),
            "and the working name is gone once the copy is published"
        );
    }

    /// The published copy in `dir`, once it is there.
    ///
    /// A publish is over in microseconds, so a test that looks at the folder
    /// in the middle of one relies on the publish being held still: the name
    /// is then there long enough to be found.
    fn wait_for_published(dir: &Path) -> PathBuf {
        for _ in 0..100 {
            if let Some(found) = std::fs::read_dir(dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(BACKUP_PREFIX)
                })
            {
                return found;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the publish did not create a name");
    }

    /// A published directory name is claimed, not renamed onto.
    ///
    /// A POSIX rename onto an existing empty directory succeeds, so a rename
    /// on its own would let a second run replace what the first one
    /// published. The name is claimed with an exclusive create instead, so
    /// the second publish is told the name is taken and the copy that is
    /// there is left alone.
    #[test]
    fn a_published_directory_name_is_claimed_and_the_copy_left_alone() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "publish_dir_claim");
        let dir = t.subdir("w");

        // A name that is already there.
        let cand = dir.join(format!("{BACKUP_PREFIX}{STAMP}"));
        std::fs::create_dir(&cand).unwrap();

        // The copy that wants to be published under it.
        let work = dir.join("work");
        std::fs::create_dir(&work).unwrap();
        std::fs::write(work.join("a"), b"the copy").unwrap();

        assert!(
            matches!(publish_dir(&work, &cand), Err(PublishError::Taken)),
            "the name that is there is not published onto"
        );
        assert!(
            std::fs::read_dir(&cand).unwrap().next().is_none(),
            "and what is under that name is left as it is"
        );
        assert!(
            work.join("a").exists(),
            "the copy that wanted the name still has its own"
        );
    }

    /// A legacy sled cache is copied aside as a directory, and published
    /// whole.
    ///
    /// A sled cache is a directory, so its copy is a directory too: it is
    /// published by being renamed onto a name it claims first.
    #[test]
    fn a_directory_copy_is_published_whole() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "publish_dir");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        std::fs::create_dir(&db).unwrap();
        std::fs::write(db.join("a"), b"the cache bytes").unwrap();

        let published = backup_db(&db).unwrap().expect("the cache was copied aside");

        assert!(
            published.is_dir(),
            "the copy of a legacy sled cache is a directory"
        );
        assert_eq!(
            std::fs::read(published.join("a")).unwrap(),
            b"the cache bytes",
            "and it holds the cache"
        );
    }

    /// A publish whose claim was taken away fails loudly.
    ///
    /// The claim is what keeps two runs off one name. A bound that meets a
    /// claim cannot tell it from a published copy — it is an empty directory
    /// under a published name — and on a filesystem that cannot lock a file
    /// there is no record beside the name to say it is being claimed, so the
    /// bound can take the claim away. The publish has to notice: landing the
    /// copy on a name this run no longer holds would publish it over whatever
    /// holds it now.
    #[test]
    fn a_publish_whose_claim_was_taken_away_fails_loudly() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "claim_gone");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        // A legacy sled cache is a directory, and a directory copy is
        // published by being renamed onto a name it claims first.
        std::fs::create_dir(&db).unwrap();
        std::fs::write(db.join("a"), b"the cache bytes").unwrap();

        let handle = std::thread::spawn(move || {
            // A filesystem that cannot lock a file: there is no record to
            // say the name is being claimed.
            LOCKING_UNAVAILABLE.with(|f| f.set(true));
            PUBLISH_PAUSED.with(|f| f.set(true));
            backup_db(&db)
        });

        // The claim: an empty directory under a published name.
        let claim = wait_for_published(&dir);
        assert!(claim.is_dir(), "the claim is an empty directory");
        std::fs::remove_dir(&claim).unwrap();

        let err = handle
            .join()
            .unwrap()
            .expect_err("a publish whose claim was taken away fails");
        assert!(
            format!("{err:#}").contains("claimed name"),
            "the failure says the claim was taken away: {err:#}"
        );
        assert!(
            published_under(&dir).is_empty(),
            "and nothing was published under a name the run no longer held"
        );
        assert!(
            working_under(&dir).is_empty(),
            "and the working copy does not sit there looking like a backup"
        );
    }

    /// A working copy that cannot be removed is left behind, and the run
    /// carries on.
    ///
    /// The copy is published and whole — it is under its published name — so
    /// the leftover scratch is not worth failing a run over. The run warns
    /// about it, and the working name is left in the folder for a human to
    /// remove.
    #[test]
    fn a_working_copy_that_cannot_be_removed_is_left_behind() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "work_left");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        std::fs::write(&db, b"the cache bytes").unwrap();

        REMOVAL_UNAVAILABLE.with(|f| f.set(true));
        let published = backup_db(&db).unwrap().expect("the cache was copied aside");
        REMOVAL_UNAVAILABLE.with(|f| f.set(false));

        assert_eq!(
            std::fs::read(&published).unwrap(),
            b"the cache bytes",
            "the copy is published and whole"
        );
        let working = working_under(&dir);
        assert_eq!(
            working.len(),
            1,
            "the working name is left behind, and it is the only one: {working:?}"
        );
    }

    /// The names in `dir` that carry the working prefix.
    fn working_under(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(WORK_PREFIX))
            .collect()
    }

    /// A publish whose claim was taken away fails loudly on a filesystem that
    /// cannot link a file either.
    ///
    /// That filesystem has no link to publish with, so the copy is renamed
    /// onto a name it claims first, and the claim is an empty file under a
    /// published name — the same shape a bound cannot tell from a published
    /// copy.
    #[test]
    fn a_publish_whose_claimed_name_was_taken_away_fails_loudly() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "claim_gone_file");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        std::fs::write(&db, b"the cache bytes").unwrap();

        let handle = std::thread::spawn(move || {
            LINKING_UNAVAILABLE.with(|f| f.set(true));
            LOCKING_UNAVAILABLE.with(|f| f.set(true));
            PUBLISH_PAUSED.with(|f| f.set(true));
            backup_db(&db)
        });

        // The claim: an empty file under a published name.
        let claim = wait_for_published(&dir);
        assert!(claim.is_file(), "the claim is an empty file");
        std::fs::remove_file(&claim).unwrap();

        let err = handle
            .join()
            .unwrap()
            .expect_err("a publish whose claim was taken away fails");
        assert!(
            format!("{err:#}").contains("claimed name"),
            "the failure says the claim was taken away: {err:#}"
        );
        assert!(
            published_under(&dir).is_empty(),
            "and nothing was published under a name the run no longer held"
        );
    }

    /// The folder of a cache that did not open is bounded just the same.
    ///
    /// A legacy sled directory the open refuses must not leave the copies
    /// beside it to accumulate until `--ignore-cache` rebuilds the cache.
    #[test]
    fn the_folder_of_a_legacy_sled_cache_is_bounded() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "bound_sled");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        // A legacy sled cache is a directory, and the open refuses it.
        std::fs::create_dir(&db).unwrap();

        let older = dir.join(format!("{BACKUP_PREFIX}20240101T000000.000000Z"));
        let newer = dir.join(format!("{BACKUP_PREFIX}20240102T000000.000000Z"));
        std::fs::write(&older, b"old").unwrap();
        std::fs::write(&newer, b"newer").unwrap();

        let err = match open_db(
            &db,
            true,
            CacheOpen::ReadWrite {
                ignore_cache: false,
                backup_first: false,
                retain: NonZeroUsize::new(1),
            },
        ) {
            Ok(_) => panic!("a legacy sled directory is refused"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("sled"),
            "the open refuses a legacy sled directory: {err}"
        );

        assert!(
            !older.exists(),
            "the oldest copy is gone: the folder is bounded even though the cache did not open"
        );
        assert!(newer.exists(), "the newest copy is kept");
    }

    /// The folder of a corrupt cache is bounded just the same.
    ///
    /// A cache that cannot be decoded must not leave the copies beside it to
    /// accumulate until `--ignore-cache` rebuilds the cache.
    #[test]
    fn the_folder_of_a_corrupt_cache_is_bounded() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "bound_corrupt");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        std::fs::write(&db, b"this is not a cache").unwrap();

        let older = dir.join(format!("{BACKUP_PREFIX}20240101T000000.000000Z"));
        let newer = dir.join(format!("{BACKUP_PREFIX}20240102T000000.000000Z"));
        std::fs::write(&older, b"old").unwrap();
        std::fs::write(&newer, b"newer").unwrap();

        assert!(
            open_db(
                &db,
                true,
                CacheOpen::ReadWrite {
                    ignore_cache: false,
                    backup_first: false,
                    retain: NonZeroUsize::new(1),
                },
            )
            .is_err(),
            "a corrupt cache is refused"
        );

        assert!(
            !older.exists(),
            "the oldest copy is gone: the folder is bounded even though the cache did not open"
        );
        assert!(newer.exists(), "the newest copy is kept");
    }

    /// The folder of a cache another run holds is left to that run.
    ///
    /// The bound runs under the cache's own lock. A cache another run holds
    /// cannot be opened, so this run must leave the folder alone: the run
    /// that is in its run phase for the folder bounds it, and the copies it
    /// published are protected by the lock it holds.
    #[test]
    fn the_folder_of_a_cache_another_run_holds_is_left_to_it() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "bound_held");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        std::fs::write(&db, b"the cache bytes").unwrap();

        // Another run holds the cache.
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&db)
            .expect("the cache can be opened");
        held.try_lock().expect("the other run holds the cache");

        let older = dir.join(format!("{BACKUP_PREFIX}20240101T000000.000000Z"));
        let newer = dir.join(format!("{BACKUP_PREFIX}20240102T000000.000000Z"));
        std::fs::write(&older, b"old").unwrap();
        std::fs::write(&newer, b"newer").unwrap();

        assert!(
            open_db(
                &db,
                true,
                CacheOpen::ReadWrite {
                    ignore_cache: false,
                    backup_first: false,
                    retain: NonZeroUsize::new(1),
                },
            )
            .is_err(),
            "the cache another run holds cannot be opened"
        );

        assert!(
            older.exists(),
            "the folder is left to the run that holds the cache"
        );
        assert!(newer.exists(), "and nothing in it is touched");
    }

    /// A cache file that cannot be opened for writing is still locked.
    ///
    /// The lock is what says that no other run is in its run phase for the
    /// folder, and it is taken on the cache file whatever the file can be
    /// opened for. A cache that is not writable is a cache no run has open —
    /// a run in its run phase holds the cache open for writing — but a run
    /// that opened it before the file stopped being writable is still
    /// holding it, and the lock is what says so.
    #[test]
    fn a_cache_that_cannot_be_opened_for_writing_is_still_locked() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "bound_readonly");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        std::fs::write(&db, b"the cache bytes").unwrap();

        // Another run holds the cache.
        let held = OpenOptions::new()
            .read(true)
            .open(&db)
            .expect("the cache can be read");
        held.try_lock().expect("the other run holds the cache");

        // The cache is not writable, so it cannot be opened for writing.
        let mut perms = std::fs::metadata(&db).unwrap().permissions();
        set_read_only(&mut perms);
        std::fs::set_permissions(&db, perms).unwrap();

        let older = dir.join(format!("{BACKUP_PREFIX}20240101T000000.000000Z"));
        let newer = dir.join(format!("{BACKUP_PREFIX}20240102T000000.000000Z"));
        std::fs::write(&older, b"old").unwrap();
        std::fs::write(&newer, b"newer").unwrap();

        assert!(
            open_db(
                &db,
                true,
                CacheOpen::ReadWrite {
                    ignore_cache: false,
                    backup_first: false,
                    retain: NonZeroUsize::new(1),
                },
            )
            .is_err(),
            "a cache that is not writable does not open"
        );

        assert!(
            older.exists(),
            "the folder is left to the run that holds the cache: the lock was taken"
        );
        assert!(newer.exists(), "and nothing in it is touched");

        // The cache is writable again, so the scratch tree can be removed.
        let mut perms = std::fs::metadata(&db).unwrap().permissions();
        set_writable(&mut perms);
        std::fs::set_permissions(&db, perms).unwrap();
    }

    /// Make a file read-only, whichever way this platform spells it.
    fn set_read_only(perms: &mut std::fs::Permissions) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o444);
        }
        #[cfg(windows)]
        perms.set_readonly(true);
    }

    /// Make a file writable again, whichever way this platform spells it.
    fn set_writable(perms: &mut std::fs::Permissions) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o644);
        }
        #[cfg(windows)]
        perms.set_readonly(false);
    }

    /// A copy whose record has been released is safe while the cache is open.
    ///
    /// The record speaks for a copy while its run is between publishing it
    /// and opening the folder's cache. Once the cache is open the record is
    /// released — it goes with the run's hold on it — and the cache's own
    /// exclusive lock is what speaks for the copy: a run that cannot open the
    /// cache is not in its run phase for the folder, so it does not bound the
    /// folder at all.
    ///
    /// The copy here is an ordinary file with no record beside it, because
    /// that is what a released record leaves behind, and nothing in this
    /// process published it: the bound that would take it is the only thing
    /// that can save it.
    ///
    /// The refusal the second run gets is the cache's **exclusive lock**,
    /// and the case says so: the lock is the whole of the protection here,
    /// so a refusal for any other reason would not be this guarantee.
    #[test]
    fn a_copy_whose_record_is_released_is_kept_while_the_cache_is_open() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "released");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");

        // The copy whose record was released, and a newer one: a bound that
        // ran would take the older of the two.
        let copy = dir.join(format!("{BACKUP_PREFIX}20240101T000000.000000Z"));
        std::fs::write(&copy, b"a copy").unwrap();
        let newer = dir.join(format!("{BACKUP_PREFIX}20240102T000000.000000Z"));
        std::fs::write(&newer, b"newer").unwrap();

        // The cache is open, so its own exclusive lock is held.
        let _cache = open_db(
            &db,
            true,
            CacheOpen::ReadWrite {
                ignore_cache: false,
                backup_first: false,
                retain: None,
            },
        )
        .expect("the cache opens");

        // A run that comes along cannot open the cache, so it is not in its
        // run phase for the folder and it does not bound the folder.
        //
        // The failure is the cache's **exclusive lock**, and saying so is the
        // point of the case: the lock is what protects a copy whose record
        // has been released, so the refusal has to be the lock's and not
        // something else that happened to go wrong.
        let err = match open_db(
            &db,
            true,
            CacheOpen::ReadWrite {
                ignore_cache: false,
                backup_first: false,
                retain: NonZeroUsize::new(1),
            },
        ) {
            Ok(_) => panic!("the cache is open, so it cannot be opened again"),
            Err(e) => e,
        };
        assert!(
            err.chain().any(|cause| {
                cause
                    .downcast_ref::<redb::DatabaseError>()
                    .is_some_and(|e| matches!(e, redb::DatabaseError::DatabaseAlreadyOpen))
            }),
            "the open failed on the cache's exclusive lock: {err:#}"
        );

        assert!(
            copy.exists(),
            "the copy whose record was released is not taken while the cache is open"
        );
        assert!(newer.exists(), "and the folder is left alone");
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
        {
            // A first open, so that the cache is a cache and not a copy of
            // one.
            let _first = open_db(
                &db,
                true,
                CacheOpen::ReadWrite {
                    ignore_cache: false,
                    backup_first: false,
                    retain: None,
                },
            )
            .expect("the cache opens");
        }

        // The run publishes a copy of the cache.
        let published = backup_db(&db).unwrap().expect("the cache was copied aside");
        // The run opens the folder's cache: the copy is the run's, and the
        // run's bound is the one that must not take it.
        {
            let _cache = open_db(
                &db,
                true,
                CacheOpen::ReadWrite {
                    ignore_cache: false,
                    backup_first: false,
                    retain: None,
                },
            )
            .expect("the cache opens again");
        }
        // Another run published a newer copy after this one.
        let newer = dir.join(format!("{BACKUP_PREFIX}29991231T235959.999999Z"));
        std::fs::write(&newer, b"newer").unwrap();

        retain_copies(&db, NonZeroUsize::new(1).unwrap()).unwrap();

        assert!(
            published.exists(),
            "the copy this run published is not the bound's to remove"
        );
        assert!(
            !newer.exists(),
            "and the folder still holds exactly the bound"
        );
    }

    /// A copy an earlier run published is not spared by a later run that
    /// published nothing.
    ///
    /// The bound keeps the newest copies, so sparing an older one would keep
    /// an older copy in place of a newer one. The run that applies the bound
    /// published nothing into this list, so it has nothing of its own to
    /// spare — and the copy the earlier run published is an ordinary copy
    /// again.
    #[test]
    fn an_earlier_runs_copy_is_not_spared_by_a_run_that_published_nothing() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "earlier_not_spared");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        {
            // A first open, so that the cache is a cache and not a copy of
            // one.
            let _first = open_db(
                &db,
                true,
                CacheOpen::ReadWrite {
                    ignore_cache: false,
                    backup_first: false,
                    retain: None,
                },
            )
            .expect("the cache opens");
        }

        // The run publishes a copy and opens the cache: the copy is that
        // run's, and its own bound spares it.
        let published = backup_db(&db).unwrap().expect("the cache was copied aside");
        {
            let _run = open_db(
                &db,
                true,
                CacheOpen::ReadWrite {
                    ignore_cache: false,
                    backup_first: false,
                    retain: None,
                },
            )
            .expect("the cache opens again");
        }

        // Another run published a newer copy.
        let newer = dir.join(format!("{BACKUP_PREFIX}29991231T235959.999999Z"));
        std::fs::write(&newer, b"newer").unwrap();

        // A run that publishes nothing into this list opens the cache: the
        // earlier run's copy belongs to that run, not to this one.
        {
            let _second = open_db(
                &db,
                true,
                CacheOpen::ReadWrite {
                    ignore_cache: false,
                    backup_first: false,
                    retain: None,
                },
            )
            .expect("the cache opens again");
        }

        retain_copies(&db, NonZeroUsize::new(1).unwrap()).unwrap();

        assert!(
            !published.exists(),
            "the earlier run's copy is an ordinary copy again"
        );
        assert!(
            newer.exists(),
            "and the newer copy is the one the bound keeps"
        );
    }

    /// A second bound for a folder still spares the copy the run published.
    ///
    /// The copy a run published is the copy that run reports, so every bound
    /// the process makes for that folder spares it, and not only the first
    /// one. A bound that took the copy's entry would leave the next bound
    /// free to take the copy the run is still about to report.
    ///
    /// The copy has no record beside it here: the run opened the cache, so
    /// the record was released, and what is left to speak for the copy is the
    /// bound's own knowledge of what this process published.
    #[test]
    fn a_second_bound_still_spares_the_copy_the_process_published() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "spared_twice");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        {
            // A first open, so that the cache is a cache and not a copy of
            // one.
            let _first = open_db(
                &db,
                true,
                CacheOpen::ReadWrite {
                    ignore_cache: false,
                    backup_first: false,
                    retain: None,
                },
            )
            .expect("the cache opens");
        }

        // The run publishes a copy of the cache: it carries a record.
        let published = backup_db(&db).unwrap().expect("the cache was copied aside");
        // A copy another run published after this one, so that the bound has
        // something to remove.
        let newer = dir.join(format!("{BACKUP_PREFIX}29991231T235959.999999Z"));
        std::fs::write(&newer, b"newer").unwrap();

        // The run opens the cache: the record is released, and the cache's
        // own lock is what speaks for the copy from here on.
        let _cache = open_db(
            &db,
            true,
            CacheOpen::ReadWrite {
                ignore_cache: false,
                backup_first: false,
                retain: None,
            },
        )
        .expect("the cache opens again");

        // The first bound.
        retain_copies(&db, NonZeroUsize::new(1).unwrap()).unwrap();
        assert!(
            published.exists(),
            "the first bound spares the copy this process published"
        );
        assert!(
            !newer.exists(),
            "and the folder still holds exactly the bound"
        );

        // Another copy lands, and a second bound runs.
        let newest = dir.join(format!("{BACKUP_PREFIX}29991232T235959.999999Z"));
        std::fs::write(&newest, b"newest").unwrap();
        retain_copies(&db, NonZeroUsize::new(1).unwrap()).unwrap();

        assert!(
            published.exists(),
            "the second bound spares the copy this process published too"
        );
        assert!(
            !newest.exists(),
            "and the folder still holds exactly the bound"
        );
    }

    /// The copies this process published for earlier runs are ordinary copies.
    ///
    /// A process that runs more than once against a folder publishes a copy
    /// each time, and only the newest one belongs to the run that is running
    /// now. The earlier ones are ordinary copies again — the run that
    /// published them is done with them — so the bound takes them like any
    /// other and the folder still ends within its bound.
    #[test]
    fn an_earlier_runs_copy_is_an_ordinary_copy_again() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "earlier");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        {
            // A first open, so that the cache is a cache and not a copy of
            // one.
            let _first = open_db(
                &db,
                true,
                CacheOpen::ReadWrite {
                    ignore_cache: false,
                    backup_first: false,
                    retain: None,
                },
            )
            .expect("the cache opens");
        }

        // Three runs, each publishing a copy and each opening the cache: the
        // open releases the copy's record, so what is left to speak for the
        // two older copies is the bound's own knowledge of what this process
        // published.
        let mut published = Vec::new();
        for _ in 0..3 {
            published.push(backup_db(&db).unwrap().expect("the cache was copied aside"));
            let _cache = open_db(
                &db,
                true,
                CacheOpen::ReadWrite {
                    ignore_cache: false,
                    backup_first: false,
                    retain: None,
                },
            )
            .expect("the cache opens again");
        }

        retain_copies(&db, NonZeroUsize::new(1).unwrap()).unwrap();

        assert!(
            published[2].exists(),
            "the copy the current run published is spared"
        );
        assert!(
            !published[0].exists(),
            "an earlier run's copy is an ordinary copy again"
        );
        assert!(!published[1].exists(), "and so is the one before it");
    }

    /// A published copy carries a record of its own, and the record is held.
    ///
    /// The record says the copy is in use by the run that published it, and
    /// the lock on it is what says that run is still alive: while the run
    /// holds it, no other handle can take it.
    #[test]
    fn a_published_copy_carries_a_record_of_its_own() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "record");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        std::fs::write(&db, b"the cache bytes").unwrap();

        let published = backup_db(&db).unwrap().expect("the cache was copied aside");

        let record = dir.join(
            hold_name(published.file_name().unwrap().to_str().unwrap())
                .expect("the record is named after the copy"),
        );
        assert!(
            record.exists(),
            "a published copy is published with a record of its own"
        );
        let other = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&record)
            .expect("the record can be opened");
        assert!(
            other.try_lock().is_err(),
            "the record is held by the run that published the copy"
        );
    }

    /// A record is a lock and nothing else.
    ///
    /// The record carries no body: the lock on it is the whole of what it
    /// says, and a pid or a clock would say nothing a bound could act on. A
    /// run that holds the record is alive whatever it wrote down, and a run
    /// that is gone cannot be asked about it.
    #[test]
    fn a_record_carries_nothing_but_its_lock() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "record_body");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        std::fs::write(&db, b"the cache bytes").unwrap();

        let published = backup_db(&db).unwrap().expect("the cache was copied aside");

        let record = dir.join(
            hold_name(published.file_name().unwrap().to_str().unwrap())
                .expect("the record is named after the copy"),
        );
        assert_eq!(
            std::fs::metadata(&record).unwrap().len(),
            0,
            "the record says nothing but its lock"
        );
    }

    /// A copy a live run is using is not the bound's to take.
    ///
    /// The run that published the copy is the run that reports it, so a bound
    /// that took it back would leave that run reporting a backup that is not
    /// on disk. The record is what says the copy is in use, and the lock on
    /// it is what says the run is still alive.
    #[test]
    fn a_copy_a_live_run_is_using_is_not_the_bounds_to_take() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "in_use");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");

        // A copy another run published, and the record saying that run is
        // still using it.
        let name = format!("{BACKUP_PREFIX}{STAMP}");
        let copy = dir.join(&name);
        std::fs::write(&copy, b"a copy").unwrap();
        let record = dir.join(hold_name(&name).expect("the record is named after the copy"));
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&record)
            .expect("the record is created");
        held.try_lock().expect("the other run holds the record");

        // A newer copy, so the bound has something to remove.
        let newer = dir.join(format!("{BACKUP_PREFIX}29991231T235959.999999Z"));
        std::fs::write(&newer, b"newer").unwrap();

        retain_copies(&db, NonZeroUsize::new(1).unwrap()).unwrap();

        assert!(
            copy.exists(),
            "a copy a live run is using is not the bound's to take"
        );
        assert!(
            !newer.exists(),
            "and the folder still ends within its bound"
        );
        assert!(record.exists(), "the record of a copy in use is left alone");
    }

    /// A record whose run is gone does not stop the bound.
    ///
    /// The OS releases the lock when the holding process ends — and when it
    /// is killed — so a record left behind says nothing: the copy it spoke
    /// for is an ordinary copy again, and the bound takes it like any other.
    #[test]
    fn a_record_whose_run_is_gone_does_not_stop_the_bound() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "stale");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");

        let name = format!("{BACKUP_PREFIX}{STAMP}");
        let copy = dir.join(&name);
        std::fs::write(&copy, b"a copy").unwrap();
        // A record nobody holds: the run that made it is gone.
        let record = dir.join(hold_name(&name).expect("the record is named after the copy"));
        std::fs::write(&record, b"").unwrap();

        let newer = dir.join(format!("{BACKUP_PREFIX}29991231T235959.999999Z"));
        std::fs::write(&newer, b"newer").unwrap();

        retain_copies(&db, NonZeroUsize::new(1).unwrap()).unwrap();

        assert!(
            !copy.exists(),
            "a copy whose run is gone is the bound's to take again"
        );
        assert!(!record.exists(), "and the stale record goes with it");
        assert!(newer.exists(), "the newest copy is kept");
    }

    /// A record that cannot be locked does not spare its copy for good.
    ///
    /// A filesystem that cannot lock a file has no records to speak: a
    /// record left behind by an older build says nothing, so the copy it
    /// spoke for is an ordinary copy again and the bound takes it like any
    /// other. A record that spared the copy for good would leave the copy
    /// beside the cache for ever.
    #[test]
    fn a_record_that_cannot_be_locked_does_not_spare_its_copy() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "nolock_record");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");

        let name = format!("{BACKUP_PREFIX}{STAMP}");
        let copy = dir.join(&name);
        std::fs::write(&copy, b"a copy").unwrap();
        // A record nobody holds, on a filesystem that cannot lock a file:
        // the lock cannot be tested, so the record says nothing.
        let record = dir.join(hold_name(&name).expect("the record is named after the copy"));
        std::fs::write(&record, b"").unwrap();

        let newer = dir.join(format!("{BACKUP_PREFIX}29991231T235959.999999Z"));
        std::fs::write(&newer, b"newer").unwrap();

        LOCKING_UNAVAILABLE.with(|f| f.set(true));
        retain_copies(&db, NonZeroUsize::new(1).unwrap()).unwrap();
        LOCKING_UNAVAILABLE.with(|f| f.set(false));

        assert!(
            !copy.exists(),
            "a record that cannot be locked does not spare the copy for good"
        );
        assert!(newer.exists(), "the newest copy is kept");
    }

    /// A record nothing speaks for any more is reclaimed.
    ///
    /// The copy it spoke for is gone and the run that made it is gone with
    /// it, so the record is tool state with nothing left to say.
    #[test]
    fn a_record_of_a_copy_that_is_gone_is_reclaimed() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "orphan");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");

        let record = dir.join(format!("{HOLD_PREFIX}backup-{STAMP}"));
        std::fs::write(&record, b"").unwrap();

        retain_copies(&db, NonZeroUsize::new(1).unwrap()).unwrap();

        assert!(!record.exists(), "a record no live run holds is reclaimed");
    }

    /// A record is not one of the copies the bound ranges over.
    ///
    /// The record is named after the copy it speaks for, but it is not a
    /// copy: the bound counts copies, and a folder holding two copies and a
    /// record is a folder holding two copies.
    #[test]
    fn a_record_is_not_one_of_the_copies_the_bound_counts() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "count");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");

        let older = dir.join(format!("{BACKUP_PREFIX}20240101T000000.000000Z"));
        let newer = dir.join(format!("{BACKUP_PREFIX}20240102T000000.000000Z"));
        std::fs::write(&older, b"old").unwrap();
        std::fs::write(&newer, b"newer").unwrap();
        let record = dir.join(hold_name(older.file_name().unwrap().to_str().unwrap()).unwrap());
        std::fs::write(&record, b"").unwrap();

        retain_copies(&db, NonZeroUsize::new(2).unwrap()).unwrap();

        assert!(
            older.exists(),
            "two copies and a record are two copies, and the bound keeps them"
        );
        assert!(newer.exists(), "the newest copy is kept");
    }

    /// Two runs publishing at once each end up with a copy and a record.
    ///
    /// The record is taken before the name is claimed, so the two runs cannot
    /// land on one name: the second run finds the first run's record held and
    /// moves on to the next name.
    #[test]
    fn concurrent_publishes_get_copies_and_records_of_their_own() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "publish_race");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        std::fs::write(&db, b"the cache bytes").unwrap();

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let db = db.clone();
                std::thread::spawn(move || backup_db(&db).unwrap())
            })
            .collect();
        let published: Vec<PathBuf> = handles
            .into_iter()
            .map(|h| h.join().unwrap().expect("the cache was copied aside"))
            .collect();

        let names: std::collections::HashSet<&PathBuf> = published.iter().collect();
        assert_eq!(
            names.len(),
            published.len(),
            "every run published under a name of its own"
        );
        for copy in &published {
            let record = dir.join(hold_name(copy.file_name().unwrap().to_str().unwrap()).unwrap());
            assert!(
                record.exists(),
                "every published copy carries a record of its own"
            );
        }
    }

    /// The record is released once the run holds the cache's own lock.
    ///
    /// From then on the cache's lock is what keeps another run's bound off
    /// the copy: no other run is in its run phase for a folder whose cache is
    /// open. The record has nothing left to say, and it goes.
    #[test]
    fn the_record_is_released_once_the_cache_is_open() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "release");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        {
            // A first open, so that the cache is a cache and not a copy of
            // one.
            let _first = open_db(
                &db,
                true,
                CacheOpen::ReadWrite {
                    ignore_cache: false,
                    backup_first: false,
                    retain: None,
                },
            )
            .expect("the cache opens");
        }

        let published = backup_db(&db).unwrap().expect("the cache was copied aside");
        let record = dir.join(
            hold_name(published.file_name().unwrap().to_str().unwrap())
                .expect("the record is named after the copy"),
        );
        assert!(
            record.exists(),
            "the copy is published with a record of its own"
        );

        {
            let _second = open_db(
                &db,
                true,
                CacheOpen::ReadWrite {
                    ignore_cache: false,
                    backup_first: false,
                    retain: None,
                },
            )
            .expect("the cache opens again");
        }

        assert!(
            !record.exists(),
            "the record is released once the cache's lock is held"
        );
        assert!(
            published.exists(),
            "and the copy is left with the lock's protection"
        );
    }

    /// A filesystem that cannot lock a file still gets the copy published.
    ///
    /// The record is what says a published copy is in use, and a filesystem
    /// that cannot lock one cannot say it. The copy is published without a
    /// record rather than not published at all, and the record the attempt
    /// made goes with it: a record nobody holds and nobody can lock reads as
    /// "a live run holds it" to a bound, which would spare the copy for good.
    #[test]
    fn a_copy_is_published_on_a_filesystem_that_cannot_lock_a_record() {
        let t = girpr_tempdir::TempDir::new("girsync_cache", "nolock");
        let dir = t.subdir("w");
        let db = dir.join("girpr-cache");
        std::fs::write(&db, b"the cache bytes").unwrap();

        LOCKING_UNAVAILABLE.with(|f| f.set(true));
        let published = backup_db(&db).unwrap().expect("the cache was copied aside");
        LOCKING_UNAVAILABLE.with(|f| f.set(false));

        assert!(
            published.exists(),
            "the copy is published even though it cannot be marked"
        );
        let name = published.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            !name.ends_with("_2"),
            "and the publish did not walk the names looking for one it could lock"
        );
        let records: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(HOLD_PREFIX))
            .collect();
        assert!(
            records.is_empty(),
            "and no record is left behind to spare the copy for good"
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
