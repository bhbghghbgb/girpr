//! What a *side* is, how its cache is opened, and the rules that hold across two.
//!
//! Everything a run needs to know before either phase starts: whether an input is
//! a folder or a record ([`classify`]), how its cache handle is opened
//! ([`open_side`], [`open_folder_cache`]), and the two rules that are properties of
//! a *pair* rather than of a side — one cache per run
//! ([`ensure_distinct_sides`]), and resolving both at once
//! ([`resolve_sides_concurrently`]).

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::cache::{CACHE_PREFIX, CacheDb, CacheOpen, open_db};
use crate::config::{CommonOpts, ScanMode};
use crate::util::is_record_path;

use super::entry::{SideCapability, SideEntry, SideScan};
use super::phase_a::scan_stat_only;
use super::record::load_record_side_from;

/// One side of a `compare`: either a folder root or a record file.
#[derive(Debug, Clone)]
pub enum Side {
    Folder(PathBuf),
    Record(PathBuf),
}

impl Side {
    /// The cache file this side reads from or writes to: a record side *is* its
    /// cache, a folder side is backed by the `<root>/girpr-cache` inside it.
    pub fn cache_path(&self) -> PathBuf {
        match self {
            Side::Record(p) => p.clone(),
            Side::Folder(root) => root.join(CACHE_PREFIX),
        }
    }
}

/// A side opened and statted, ready for a plan and then a resolve.
///
/// The handle stays live across both, which is the point: phase C writes rows
/// for this same side, and a caller planning across two sides has to hold both
/// sets of phase-A output at once. redb allows one writable handle per file, so
/// this is only sound because [`ensure_distinct_sides`] runs before any open.
pub struct OpenSide {
    /// What it is, which decides how phase C runs.
    pub(super) side: Side,
    cache: CacheDb,
    /// Phase A's output.
    pub phase_a: SideScan<SideEntry>,
    /// What this side is able to do, independent of how the handle was opened.
    pub cap: SideCapability,
}

impl OpenSide {
    /// The open cache, for a caller that has to write it itself (`sync`'s
    /// applier, which pre-drops rows and records verified copies).
    pub fn cache(&self) -> &CacheDb {
        &self.cache
    }
}

/// **Phase A** for a record or folder side: open the cache and stat the tree.
///
/// Both handles stay open, because a caller that plans across two sides needs this
/// side's phase-A output while the other side's handle is still open — and both
/// still open when phase C writes.
///
/// How a folder's cache is opened — and therefore whether anything touches disk
/// — is [`open_folder_cache`]'s decision, not this function's. A record side is
/// opened read-only and never written, with or without a dry run.
pub fn open_side(side: &Side, common: &CommonOpts, mode: ScanMode) -> Result<OpenSide> {
    let (cache, phase_a, cap) = match side {
        Side::Record(dbp) => {
            info!(record = %dbp.display(), "load record side");
            let cache = CacheDb::open_record(dbp)?;
            let phase_a = load_record_side_from(&cache, common, &dbp.display().to_string())?;
            info!(
                record = %dbp.display(),
                entries = phase_a.map.len(),
                "record side loaded"
            );
            (cache, phase_a, SideCapability::record())
        }
        Side::Folder(root) => {
            if !root.is_dir() {
                bail!("folder {} not found", root.display());
            }
            let cache = open_folder_cache(root, common, mode, true).map_err(|e| {
                warn!(root = %root.display(), error = format!("{:#}", e), "cache open failed");
                e.context(format!(
                    "cache for {} (use --ignore-cache to rebuild)",
                    root.display()
                ))
            })?;
            let phase_a = scan_stat_only(root, &cache, common, mode)?;
            let cap = SideCapability::for_folder(&cache, mode);
            (cache, phase_a, cap)
        }
    };
    Ok(OpenSide {
        side: side.clone(),
        cache,
        phase_a,
        cap,
    })
}

/// Open a folder side's cache, honouring [`ScanMode::dry_run`].
///
/// The one place that decides *how* a folder's cache is opened, so every command
/// gets the same answer and none can forget. `backup_first` is a separate
/// argument because `sync` backs up both sides before it opens either — it has
/// to, to snapshot dst's old state before anything moves — while `compare` has no
/// such ordering requirement and lets the open do it.
///
/// A dry run must not write to disk, and the cache is on disk, so under
/// `dry_run`:
///
/// - an existing cache is opened **read-only**: no `meta` rewrite, no backup, no
///   write handle at all, so the file comes out byte-identical;
/// - a **missing** cache is served from an in-memory DB, because creating one
///   would leave `girpr-cache` behind in a folder that had none;
/// - `--ignore-cache` is treated as *absent* rather than honoured, since
///   rebuilding is a write. The folder is then scanned from disk alone — which is
///   what a real `--ignore-cache` run does too, so the two agree.
///
/// Each fallback still yields the **same** effective map a real run would, which
/// is the point: a dry run answers the question, it does not answer a different
/// one. The read paths are identical; only the write handle is absent.
///
/// A **corrupt** cache is deliberately *not* on that list. A real run refuses it,
/// and a dry run must refuse it too: falling back to memory would have the dry
/// run report a confident verdict over a cache neither mode could read, which is
/// the worst outcome available — it looks like an answer and is not one. The two
/// branches that *are* safe are the ones where the cache contributes nothing a
/// real run would have used (it does not exist, or it was going to be discarded
/// anyway); a corrupt cache is neither.
///
/// This is also why this function exists rather than a `dry_run` check at each
/// call site — a per-command check is a per-command chance to get it wrong.
pub fn open_folder_cache(
    root: &Path,
    common: &CommonOpts,
    mode: ScanMode,
    backup_first: bool,
) -> Result<CacheDb> {
    let db_path = root.join(CACHE_PREFIX);
    if !mode.dry_run {
        return open_db(
            &db_path,
            common.case_sensitive,
            CacheOpen::ReadWrite {
                ignore_cache: common.ignore_cache,
                backup_first,
            },
        );
    }
    if common.ignore_cache || !db_path.exists() {
        info!(path = %db_path.display(), "dry-run: using an in-memory cache");
        return CacheDb::open_temp(common.case_sensitive);
    }
    info!(path = %db_path.display(), "dry-run: opening read-only");
    open_db(&db_path, common.case_sensitive, CacheOpen::ReadOnly)
}

/// Classify an input path by basename: `girpr-cache*` means record, else folder.
pub fn classify(p: &Path) -> Side {
    if is_record_path(p) {
        Side::Record(p.to_path_buf())
    } else {
        Side::Folder(p.to_path_buf())
    }
}

/// Reject two sides that would resolve to the same cache file.
///
/// redb permits one live handle per file, so a run must never name the same
/// cache twice. This covers every shape of the collision in one predicate: the
/// same folder on both sides, the same record on both sides, and a folder
/// paired with the record that lives inside it. The identity compared is the
/// *canonical* cache path, so two spellings of one target (`F/sub/..` vs `F`)
/// collide too. Canonicalization falls back to the raw path when it fails, so a
/// missing folder still reaches `load_side` and reports "not found" rather than
/// a canonicalize error.
///
/// Beyond the handle clash, a self-collision makes the run meaningless. A
/// folder side populates its cache as a side effect of scanning, so a record
/// compared against its own folder is diffed against a view the very same run
/// is still mutating: a path reported as drifted on one line is written into the
/// record before the next, and a follow-up run over the same pair comes back
/// clean. The audit silently repairs the drift it was asked to report, which is
/// exactly the property a comparison is supposed to lack.
pub fn ensure_distinct_sides(src: &Side, dst: &Side) -> Result<()> {
    let a = cache_identity(src);
    let b = cache_identity(dst);
    if a == b {
        bail!("src and dst resolve to the same cache {}", display_path(&a));
    }
    Ok(())
}

/// Canonical cache path of a side. A path that does not exist yet cannot
/// collide with anything but an identically-spelled one, so the raw path is a
/// good enough identity.
fn cache_identity(side: &Side) -> PathBuf {
    let raw = side.cache_path();
    raw.canonicalize().unwrap_or(raw)
}

/// Render a path for humans, dropping the `\\?\` verbatim prefix that
/// `canonicalize` adds on Windows — a user never types it, so echoing it back
/// only makes the message harder to match against a flag.
fn display_path(p: &Path) -> String {
    let s = p.display().to_string();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

/// Resolve both sides of a pair at once, and report src's failure in preference
/// to dst's.
///
/// Nothing here decides *whether* the two overlap — the limiter does. Under
/// [`crate::rw::RwLimits::Shared`] there is one counter, so the two hash phases take
/// turns and the run is sequential. Under a split limit each side has its own
/// counter, so the two phases genuinely proceed at once, which is the entire point
/// of the flag: on two separate physical drives, src being read does not have to
/// wait for dst to finish being read.
///
/// **Why this is sound.** [`ensure_distinct_sides`] already ran and guarantees the
/// two sides are different cache files, and redb permits one writable handle per
/// file — so the two resolves never touch the same database. Each takes its own
/// batch handle from its own `CacheDb` before spawning, so the transaction
/// lifetime never crosses a thread boundary.
///
/// **Why the error is src's.** Both sides can fail at once, and which one finished
/// first is a function of I/O timing. src runs on the spawned thread and dst on the
/// calling one, so the ordering is not fixed — what is fixed is that the caller
/// unwraps src's result first, so src's failure is the one reported whatever the
/// timing was, and two runs over the same broken tree print the same message. The
/// cost is that a dst-only failure waits for src's to be seen, which is invisible —
/// the run stops either way.
///
/// `std::thread::scope` rather than a detached spawn: both sides' borrows are live
/// here, and a scoped thread cannot outlive them.
pub fn resolve_sides_concurrently<S, D>(
    src: impl FnOnce() -> Result<S> + Send,
    dst: impl FnOnce() -> Result<D> + Send,
) -> (Result<S>, Result<D>)
where
    S: Send,
    D: Send,
{
    std::thread::scope(|scope| {
        let src = scope.spawn(src);
        let dst = dst();
        // A panic on either side is a bug, not a run outcome: re-raise it with its
        // original payload rather than folding it into the `Result` the caller is
        // about to match on, so the backtrace that names the bug survives.
        let src = src
            .join()
            .unwrap_or_else(|payload| std::panic::resume_unwind(payload));
        (src, dst)
    })
}
