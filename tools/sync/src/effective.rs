//! Building the *effective* map for one side of a comparison.
//!
//! An effective map is the authoritative view of a side once the cache, the
//! glob filters, and the case mode have been applied. Disk always governs:
//! stale cache entries are dropped wholesale and rehashed, and rows for paths
//! that vanished are pruned.

use anyhow::{Context, Result, bail};
use glob::Pattern;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{debug, info, trace, warn};

use crate::cache::{CACHE_PREFIX, CacheDb, CacheOpen, CacheWrite, FileRec, open_db};
use crate::config::{CommonOpts, ScanMode};
use crate::filter::is_excluded;
use crate::hash::hash_file;
use crate::planner::HashPlan;
use crate::scan::{check_mixed_case, walk_live};
use crate::util::{elapsed_s, is_cache_rel, is_record_path};

/// What one side's scan actually did.
///
/// Counters, not a verdict: they describe how much work the run performed. They
/// are the only honest measure of laziness — the diff output is identical either
/// way, so a run that hashes nothing and a run that hashes everything print the
/// same thing. They are returned from the scan rather than emitted as log fields
/// alone, because a distinction nothing can assert is a distinction nothing
/// distinguishes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanStats {
    /// Entries the walk found, before glob filters.
    pub live: usize,
    /// Files in the resulting map.
    pub files: usize,
    /// Dirs in the resulting map.
    pub dirs: usize,
    /// Files whose *bytes* were read to compute a digest. A dir never counts, and
    /// neither does a file served entirely from a stat-matching cached row.
    pub hashed: usize,
    /// Files answered wholly from the cache: stat matched and every requested
    /// algo was already stored, so the file was never opened.
    ///
    /// "Wholly from the cache" is literal — a path the plan left alone *and* the
    /// cache had nothing for is [`stat_only`](Self::stat_only), not a hit. An
    /// empty algo set over a cold tree produces no hits at all, which is the
    /// honest answer: nothing was served from a cache, because nothing was
    /// needed and nothing was read.
    pub cache_hit: usize,
    /// Files that needed no digest at all: either the request set was empty
    /// (`--hash none`), or the pair was settled by stat before a digest was ever
    /// asked for.
    ///
    /// Distinct from `cache_hit` because a run that read nothing and served nothing
    /// is a different claim from one that read nothing and served everything, and
    /// merging the two would let a run look efficient for the wrong reason.
    pub stat_only: usize,
    /// Cache rows dropped for paths no longer live (or now filtered out).
    pub pruned: usize,
}

/// A side's map at one phase, plus the counters describing how it was built.
///
/// The two travel together because the counters are only meaningful next to the
/// map they were produced for, and a caller that drops the map has no use for
/// them either. Generic over the entry type because both phases produce a map:
/// phase A an [`SideEntry`] per path, phase C an [`EffRec`] per path.
#[derive(Debug, Default)]
pub struct SideScan<T = EffRec> {
    /// The map, keyed by relative path.
    pub map: HashMap<String, T>,
    pub stats: ScanStats,
}

impl SideScan<EffRec> {
    /// Count a resolved map's files and dirs, leaving the work counters at zero.
    ///
    /// For a record side, which is read with no filesystem access: there is no
    /// hashing to report and nothing to prune, so zero is the truth rather than
    /// a stand-in for "unknown".
    fn of_map(map: HashMap<String, EffRec>) -> Self {
        let stats = ScanStats {
            files: map.values().filter(|r| r.kind == "file").count(),
            dirs: map.values().filter(|r| r.kind == "dir").count(),
            ..ScanStats::default()
        };
        Self { map, stats }
    }
}

impl SideScan<SideEntry> {
    /// Count an entry set's files and dirs, leaving the work counters at zero.
    fn of_entries(map: HashMap<String, SideEntry>) -> Self {
        let stats = ScanStats {
            files: map.values().filter(|e| e.is_file()).count(),
            dirs: map.values().filter(|e| !e.is_file()).count(),
            ..ScanStats::default()
        };
        Self { map, stats }
    }
}

/// A resolved path: kind, stat data, and whatever hashes were needed.
/// Hash values are raw digest bytes (see [`crate::hash::hash_file`]).
///
/// `PartialEq` exists so a test can assert that two runs produced *the same
/// effective map*, which is the strongest available statement of the dry-run
/// contract. Comparing printed verdicts is weaker: two runs can agree on every
/// `CHANGED` line while one of them hashed a different set of files and reached
/// the same conclusion by luck. Equality of the map cannot be faked that way,
/// because the map holds the digests themselves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffRec {
    /// "file" or "dir"
    pub kind: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub hashes: HashMap<String, Vec<u8>>,
}

impl EffRec {
    fn dir() -> Self {
        Self {
            kind: "dir".into(),
            size: 0,
            mtime_ns: 0,
            hashes: HashMap::new(),
        }
    }
}

/// True when `prior` still describes a file at exactly this stat.
///
/// This is the *only* definition of row freshness in the crate, and it is
/// deliberately also the only thing [`merge_row`] consults. A run that decides
/// "is this row reusable?" by one rule and decides "what do I store?" by another
/// can write back a digest it just decided was stale.
fn stat_matches(prior: Option<&FileRec>, size: u64, mtime_ns: i64) -> bool {
    prior
        .map(|c| c.kind == "file" && c.size == size && c.mtime_ns == mtime_ns)
        .unwrap_or(false)
}

/// The row a scan stores for one file: the merge-write rule, as a pure function.
///
/// `computed` is what this run hashed (empty for `--hash none` and for a
/// stat-only pass); `prior` is whatever the cache held, or `None`.
///
/// **The write shape.** See [`scan_stat_only`] for the table; the property worth
/// naming here is that trust is absent from it. Distrusting the cache means
/// "re-read the file", not "forget what we already know about it"; keying the
/// write on the trust flag would make every `update` run (which always distrusts)
/// discard digests it never recomputed.
///
/// The unchanged branch is what keeps a digest-free scan from destroying good
/// digests: `--hash none` and stat-only recording both land here with an empty
/// `computed`, so they record the new stat and leave the rest of the row alone.
///
/// A named function rather than an expression inside the scan loop because it is a
/// cache-durability rule, not a scan detail, and the rule belongs in one place
/// whatever calls it.
fn merge_row(
    size: u64,
    mtime_ns: i64,
    computed: HashMap<String, Vec<u8>>,
    prior: Option<&FileRec>,
) -> FileRec {
    let hashes = match prior.filter(|c| stat_matches(Some(c), size, mtime_ns)) {
        Some(c) => {
            let mut merged = c.hashes.clone();
            merged.extend(computed);
            merged
        }
        None => computed,
    };
    FileRec {
        kind: "file".into(),
        size,
        mtime_ns,
        hashes,
    }
}

/// One side's contribution to a pair decision: what phase A learned about a
/// path without reading its bytes.
///
/// This is phase A's whole output, and it is deliberately *not* an [`EffRec`].
/// An `EffRec` claims to answer a diff question, and answering it is phase C's
/// job; an entry here only says what the path is and what the cache already
/// holds for it. Keeping them apart is what lets the planner see a side's full
/// digest availability before committing to a read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SideEntry {
    /// "file" or "dir"
    pub kind: String,
    pub size: u64,
    pub mtime_ns: i64,
    /// Digests carried **only** when the stat still matches the row, and
    /// **every** algorithm on that row — requested or not, because the planner
    /// needs the whole availability picture to choose one per path.
    ///
    /// Empty for a dir, for a new path, and for a path whose stat moved. That
    /// last case is the load-bearing one: a stale row's digests must never reach
    /// the planner, or a changed file gets judged against its pre-change digest.
    pub cached: HashMap<String, Vec<u8>>,
    /// Whether the cache row's stat still matches disk.
    ///
    /// Not derivable from `cached` being empty, because a fresh row can carry no
    /// digests at all — that is what a `--hash none` history looks like, and it
    /// is a real state. Phase C needs the distinction to reconstruct the prior
    /// row and hand it to [`merge_row`], so it is carried explicitly rather than
    /// inferred.
    pub fresh: bool,
}

impl SideEntry {
    fn dir() -> Self {
        Self {
            kind: "dir".into(),
            size: 0,
            mtime_ns: 0,
            cached: HashMap::new(),
            fresh: true,
        }
    }

    /// True when this entry is a file rather than a directory. Dirs are compared
    /// by presence alone and never need a digest.
    pub fn is_file(&self) -> bool {
        self.kind == "file"
    }

    /// The cache row as phase C needs it back, or `None` when the stat moved —
    /// which is what makes [`merge_row`] able to distinguish a merge over the
    /// stored digests from a replacement of them.
    fn prior(&self) -> Option<FileRec> {
        self.fresh.then(|| FileRec {
            kind: "file".into(),
            size: self.size,
            mtime_ns: self.mtime_ns,
            hashes: self.cached.clone(),
        })
    }
}

/// What a side is *able* to do — two independent bits, not one property of the
/// cache handle.
///
/// `CacheDb::is_read_only` answers "can this handle write?". It does not answer
/// "can this side produce a digest?", and the two come apart in both directions
/// within a single command:
///
/// | side                          | handle     | can_hash_from_disk | can_write_cache |
/// | ----------------------------- | ---------- | ------------------ | --------------- |
/// | folder, normal run            | ReadWrite  | yes                | yes             |
/// | folder, `sync --dry-run`, `compare-self` | ReadOnly | yes      | no              |
/// | record                        | ReadOnly  | no                 | no              |
///
/// Reading a read-only handle as "cannot hash" would make the planner refuse to
/// hash a dry run's folder — the one case where hashing is the only thing left to
/// do. Reading it as "can hash and write" would try to hash a record, which has no
/// filesystem to hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SideCapability {
    /// Can produce a digest that is not already cached, by reading the file.
    pub can_hash_from_disk: bool,
    /// Can persist what it produces.
    pub can_write_cache: bool,
}

impl SideCapability {
    /// A folder side. Full filesystem access regardless of how the cache was
    /// opened, so it can always hash; persisting needs both a writable handle
    /// and a run that is permitted to write.
    pub fn for_folder(cache: &CacheDb, mode: ScanMode) -> Self {
        Self {
            can_hash_from_disk: true,
            can_write_cache: !mode.dry_run && !cache.is_read_only(),
        }
    }

    /// A record side: no filesystem access, so it can neither produce a digest
    /// it does not already hold nor store one.
    pub fn record() -> Self {
        Self {
            can_hash_from_disk: false,
            can_write_cache: false,
        }
    }
}

/// Record a directory as presence-only, leaving an existing dir row alone.
///
/// A dir row is fully determined by its key, so rewriting an identical one per
/// scan would be pure write amplification on a tree with many directories. The
/// write is still made when the row is absent or is a *file* row: a file that
/// became a directory must lose its digests, or a later scan would read a
/// directory's stat against a digest taken from the file that preceded it.
fn put_dir_row(w: &mut CacheWrite<'_>, rel: &str) -> Result<()> {
    if w.get(rel)?.map(|c| c.kind != "dir").unwrap_or(true) {
        w.put(rel, &FileRec::dir())?;
    }
    Ok(())
}

/// **Phase A.** Walk `root`, consult the cache, and report what it knows about
/// every path — without reading a single file's bytes.
///
/// The output is a [`SideEntry`] per path: its kind, its stat, and the digests
/// the cache holds *for that exact stat*. Nothing here decides what still needs
/// computing; that is [`HashPlan`]'s job, and keeping the two apart is what lets
/// the plan choose an algorithm per path instead of inheriting whatever this pass
/// decided to open.
///
/// Writes back to the cache unless `mode.dry_run`, and prunes rows for paths that
/// are no longer on disk (or are now filtered out). Writes are batched and
/// committed periodically, so an interrupted scan keeps most of its progress (the
/// commit is the durability point). The writes are the ones that do not depend on
/// any plan:
///
/// - dir rows (presence-only);
/// - an alternate-cased key being retired in favour of the disk name;
/// - a file whose stat moved, which gets a stat-only row. A stat with no digest
///   is a valid state, not a half-done one: the next run sees a fresh row with
///   nothing cached, and the plan asks for a digest again. Writing it here rather
///   than waiting for a digest is what stops a changed file keeping a row that
///   still carries its pre-change hashes.
///
/// **Write shape.** What a rewritten row keeps depends only on whether the stat
/// still matches — never on what this run computed, and never on whether the run
/// trusted the cache:
///
/// | stat      | digests stored                                           |
/// | --------- | -------------------------------------------------------- |
/// | changed   | only the algos computed this run (all others are suspect) |
/// | unchanged | those, merged over the algos already on the row          |
///
/// So a run never destroys a digest it did not recompute unless the file's stat
/// says it changed. `--hash none` and `--no-trust-cached-hashes` both write stat
/// data while leaving unrequested digests intact. This is [`merge_row`]'s rule,
/// stated here because this is where the rows are written.
///
/// Phase A opens, uses and commits its own batched write handle, so the row set
/// is durable before the expensive phase begins.
///
/// In insensitive mode the on-disk name governs: cached keys are indexed by
/// lowercase so a disk/cached casing difference is fixed to the disk name first,
/// carrying the cached hashes when stat matches, instead of erroring. Multiple
/// stale alternates are pruned, not treated as a conflict.
pub fn scan_stat_only(
    root: &Path,
    cache: &CacheDb,
    common: &CommonOpts,
    mode: ScanMode,
) -> Result<SideScan<SideEntry>> {
    let algos: &[String] = &common.algos;
    let includes: &[Pattern] = &common.includes;
    let excludes: &[Pattern] = &common.excludes;
    let case_sensitive = common.case_sensitive;
    let max_depth = common.max_depth;
    let ScanMode {
        no_trust_cached_hashes,
        dry_run,
    } = mode;

    let live = walk_live(root, max_depth)?;
    check_mixed_case(&live, &root.display().to_string(), case_sensitive)?;
    let mut map: HashMap<String, SideEntry> = HashMap::new();
    let mut live_set: HashSet<String> = HashSet::new();
    for e in &live {
        if is_excluded(&e.rel, includes, excludes, case_sensitive) {
            continue; // treated as not existing at all
        }
        live_set.insert(e.rel.clone());
    }
    info!(
        root = %root.display(),
        live = live.len(),
        no_trust_cached_hashes,
        dry_run,
        algos = ?algos,
        "effective start"
    );
    let t_eff = std::time::Instant::now();
    let mut n_done: usize = 0;
    let total_live = live.len();
    let alt_recs: HashMap<String, FileRec> = if !case_sensitive {
        // Read once up front; the scan's batch handle starts from the same
        // committed state.
        cache.load_all()?
    } else {
        HashMap::new()
    };
    let mut alt_index: HashMap<String, Vec<String>> = HashMap::new();
    if !case_sensitive {
        for key in alt_recs.keys() {
            alt_index
                .entry(key.to_lowercase())
                .or_default()
                .push(key.clone());
        }
    }
    // Non-dry runs funnel every cache mutation through one batched handle,
    // which auto-commits periodically so progress survives interruption.
    let mut batch = if dry_run {
        None
    } else {
        Some(cache.begin_write()?)
    };
    let mut last_prog = std::time::Instant::now();
    for e in live {
        // `live_set` was built from this same predicate over this same vec, so
        // membership here is the exclusion test — and a hash lookup rather than
        // re-running every glob against the path.
        if !live_set.contains(&e.rel) {
            continue;
        }
        if e.is_dir {
            map.insert(e.rel.clone(), SideEntry::dir());
            if let Some(w) = batch.as_mut() {
                put_dir_row(w, &e.rel)?;
            }
            continue;
        }
        // file
        // `adopted` = cache entry came from an alternate-cased key, so the
        // disk-cased key must be (re)written even when no digest is computed.
        let (cached, adopted): (Option<FileRec>, bool) = match match batch.as_mut() {
            Some(w) => w.get(&e.rel)?,
            None => cache.get(&e.rel)?,
        } {
            Some(c) => (Some(c), false),
            // Exact-case miss in insensitive mode: adopt the single stale
            // alternate-cased entry as the cache candidate (disk name wins).
            None if !case_sensitive => {
                match alt_index.get(&e.rel.to_lowercase()) {
                    Some(alts) => {
                        let others: Vec<&String> = alts.iter().filter(|k| *k != &e.rel).collect();
                        if others.len() == 1 {
                            let old = others[0];
                            let rec = alt_recs.get(old).cloned();
                            if let Some(w) = batch.as_mut() {
                                w.remove(old)?; // old casing dropped; new one upserted below
                            }
                            (rec, true)
                        } else {
                            (None, false) // zero (truly new) or several stale: prune handles leftovers
                        }
                    }
                    None => (None, false),
                }
            }
            None => (None, false),
        };
        let fresh = stat_matches(cached.as_ref(), e.size, e.mtime_ns);
        // A stale row's digests are dropped on the floor here. This is the carry
        // rule, and it is the same rule `merge_row` writes by: a row is valid iff
        // its stat matches. Carrying them would let a changed file be judged
        // against its pre-change digest.
        map.insert(
            e.rel.clone(),
            SideEntry {
                kind: "file".into(),
                size: e.size,
                mtime_ns: e.mtime_ns,
                cached: match (fresh, cached.as_ref()) {
                    (true, Some(c)) => c.hashes.clone(),
                    _ => HashMap::new(),
                },
                fresh,
            },
        );
        // The two cases where phase A owes a write regardless of any plan: the
        // stat moved, so the old row is now wrong; or the key was adopted from a
        // stale casing, so the disk-cased key does not exist yet. The row is built
        // unconditionally — only the put is gated, so "what would be stored" is
        // never something that depends on whether the run may write.
        if !fresh || adopted {
            let row = merge_row(e.size, e.mtime_ns, HashMap::new(), cached.as_ref());
            trace!(rel = %e.rel, fresh, adopted, "stat-only row");
            if let Some(w) = batch.as_mut() {
                w.put(&e.rel, &row)?;
            }
        }
        n_done += 1;
        if n_done.is_multiple_of(100) || last_prog.elapsed().as_secs() >= 5 {
            info!(
                root = %root.display(),
                done = n_done,
                total = total_live,
                hashed = 0,
                cache_hit = 0,
                elapsed_s = t_eff.elapsed().as_secs_f64(),
                "scan progress"
            );
            last_prog = std::time::Instant::now();
        }
    }
    // Prune DB rows for files no longer on disk (or now excluded).
    //
    // The prune *decision* is an answer, not a side effect: how many rows this run
    // would drop is part of what the run reports, so it is computed in both modes
    // and only the `remove` calls are suppressed. Gating the whole loop on the
    // write handle — which is what this did — makes a dry run report `pruned = 0`
    // over a tree a real run prunes, i.e. it answers a different question.
    //
    // Reading through the batch handle when there is one is not required for
    // correctness: every key this run has written is a live key, and a live key is
    // never pruned, so the batch's pending writes cannot change the set. It does
    // change nothing about the *count* either way.
    let existing = match batch.as_mut() {
        Some(w) => w.load_all()?,
        None => cache.load_all()?,
    };
    let stale: Vec<String> = existing
        .keys()
        .filter(|rel| !live_set.contains(*rel))
        .cloned()
        .collect();
    let pruned = stale.len();
    for rel in &stale {
        trace!(rel = %rel, "prune cache");
        if let Some(w) = batch.as_mut() {
            w.remove(rel)?;
        }
    }
    if let Some(w) = batch.take() {
        // Durability point for the row set. Phase C opens its own handle, so this
        // is not the end of the scan — it is the point at which the cache describes
        // the tree even if every hash after it is lost.
        w.commit()?;
    }
    let stats = ScanStats {
        live: total_live,
        files: map.values().filter(|e| e.is_file()).count(),
        dirs: map.values().filter(|e| !e.is_file()).count(),
        hashed: 0,
        cache_hit: 0,
        stat_only: 0,
        pruned,
    };
    info!(
        root = %root.display(),
        files = stats.files,
        dirs = stats.dirs,
        hashed = 0,
        cache_hit = 0,
        pruned = stats.pruned,
        elapsed_s = elapsed_s(t_eff),
        "effective done"
    );
    Ok(SideScan { map, stats })
}

/// **Phase C.** Hash what the plan asked for, and finalize each path's map entry.
///
/// This is the only phase that opens a file, and it opens exactly the ones
/// [`HashPlan`] names — no more, so a stat-mismatch short circuit in the planner
/// is worth real I/O; no fewer, so a digest the diff will compare is always
/// present on both sides.
///
/// A path the plan leaves alone is finalized from what phase A carried, and
/// costs no write: its row is already correct. A path that is hashed is written
/// through [`merge_row`], which is the same write rule phase A used, so the two
/// phases cannot spell it differently.
pub fn resolve_folder(
    root: &Path,
    cache: &CacheDb,
    mode: ScanMode,
    phase_a: &SideScan<SideEntry>,
    plan: &HashPlan,
) -> Result<SideScan> {
    let ScanMode { dry_run, .. } = mode;
    let t_eff = std::time::Instant::now();
    info!(
        root = %root.display(),
        pending = plan.pending().len(),
        digests = plan.digest_count(),
        "resolve start"
    );
    let mut batch = if dry_run {
        None
    } else {
        Some(cache.begin_write()?)
    };

    // Sorted, so a run reads the same paths in the same order every time and the
    // progress heartbeat is reproducible.
    let mut pending: Vec<&String> = phase_a
        .map
        .iter()
        .filter(|(rel, e)| e.is_file() && !plan.is_settled(rel))
        .map(|(rel, _)| rel)
        .collect();
    pending.sort();

    let mut map: HashMap<String, EffRec> = HashMap::new();
    let mut n_hashed = 0usize;
    let mut last_prog = std::time::Instant::now();
    // Hashing and writing stay interleaved on purpose: `COMMIT_INTERVAL` bounds
    // how much hashing work an interruption can lose, and that only holds if a
    // digest becomes durable shortly after it is computed. Batching all the
    // writes until after the last hash would keep the whole hash phase in
    // memory and none of it on disk.
    for (i, rel) in pending.iter().enumerate() {
        let path = root.join(rel);
        let algos = plan.get(rel);
        let e = &phase_a.map[*rel];
        debug!(rel = %rel, path = %path.display(), algos = ?algos, "hashing");
        let computed =
            hash_file(&path, algos).with_context(|| format!("hash {}", path.display()))?;
        trace!(rel = %rel, algos = ?computed.keys().collect::<Vec<_>>(), "hashed");
        let row = merge_row(e.size, e.mtime_ns, computed, e.prior().as_ref());
        if let Some(w) = batch.as_mut() {
            w.put(rel, &row)?;
        }
        map.insert(
            (*rel).clone(),
            EffRec {
                kind: "file".into(),
                size: row.size,
                mtime_ns: row.mtime_ns,
                hashes: row.hashes,
            },
        );
        n_hashed += 1;
        if (i + 1).is_multiple_of(100) || last_prog.elapsed().as_secs() >= 5 {
            info!(
                root = %root.display(),
                done = i + 1,
                total = pending.len(),
                hashed = n_hashed,
                cache_hit = 0,
                elapsed_s = t_eff.elapsed().as_secs_f64(),
                "scan progress"
            );
            last_prog = std::time::Instant::now();
        }
    }

    // Everything the plan settled: a file, a dir, and no read.
    let mut n_cache_hit = 0usize;
    let mut n_stat_only = 0usize;
    for (rel, e) in &phase_a.map {
        if map.contains_key(rel) {
            continue;
        }
        if !e.is_file() {
            map.insert(rel.clone(), EffRec::dir());
            continue;
        }
        if e.cached.is_empty() {
            n_stat_only += 1;
            trace!(rel = %rel, "stat-only");
        } else {
            n_cache_hit += 1;
            trace!(rel = %rel, "cache-hit");
        }
        map.insert(
            rel.clone(),
            EffRec {
                kind: "file".into(),
                size: e.size,
                mtime_ns: e.mtime_ns,
                hashes: e.cached.clone(),
            },
        );
    }

    if let Some(w) = batch.take() {
        w.commit()?;
    }
    let stats = ScanStats {
        hashed: n_hashed,
        cache_hit: n_cache_hit,
        stat_only: n_stat_only,
        // Phase A owns `files`, `dirs` and `pruned` — it is the only phase that
        // walks the tree, so it is the only one that can count them. Carried
        // across explicitly: a `..ScanStats::default()` here would silently
        // reset `pruned` to 0 on every run, and the log line (which reads
        // phase A directly) would then disagree with the returned stats.
        files: phase_a.stats.files,
        dirs: phase_a.stats.dirs,
        pruned: phase_a.stats.pruned,
        live: phase_a.stats.live,
    };
    info!(
        root = %root.display(),
        files = phase_a.stats.files,
        dirs = phase_a.stats.dirs,
        hashed = stats.hashed,
        cache_hit = stats.cache_hit,
        stat_only = stats.stat_only,
        pruned = phase_a.stats.pruned,
        elapsed_s = elapsed_s(t_eff),
        "effective done"
    );
    Ok(SideScan { map, stats })
}

/// **Phase C** for a record side: no filesystem access, so it can only hand back
/// what it already had.
///
/// It does not check that the plan was answerable — the planner never asks it to
/// be, because [`crate::planner::plan_pairs`] only plans work for a side that can
/// hash from disk. So this function copies phase A's rows verbatim and there is
/// nothing here for it to fail on.
///
/// The *coverage* check does exist, and it is in the planner rather than here for
/// two reasons: the planner is the only place that holds both sides, so it can
/// report every uncovered path in one message rather than stopping at the first;
/// and a shortfall is a property of a (path, side) pair inside the undecided set,
/// never of the record as a whole — which a preflight over the record itself could
/// not tell. See `plan_pairs`' module docs.
pub fn resolve_record(phase_a: &SideScan<SideEntry>) -> SideScan<EffRec> {
    let map = phase_a
        .map
        .iter()
        .map(|(rel, e)| {
            (
                rel.clone(),
                EffRec {
                    kind: e.kind.clone(),
                    size: e.size,
                    mtime_ns: e.mtime_ns,
                    hashes: e.cached.clone(),
                },
            )
        })
        .collect();
    SideScan::of_map(map)
}

/// **Phase A** for a record side: the DB read as-is, with no FS access and no
/// writes. Cache rows and filtered paths are dropped.
///
/// Takes an already-open cache rather than a path so a caller needing the record
/// view *and* a disk scan of the same folder can do both from one handle. That is
/// not a lock requirement — two read-only handles share the file — it is a cost
/// one: each `CacheDb` builds its own copy of the file's tables, so opening twice
/// doubles the memory a large cache occupies. The same reasoning applies harder to
/// a writable handle, where the second open would be refused outright.
///
/// `label` names the record in error messages; the caller knows the path.
pub fn load_record_side_from(
    cache: &CacheDb,
    common: &CommonOpts,
    label: &str,
) -> Result<SideScan<SideEntry>> {
    let all = cache.load_all()?;
    let mut out: HashMap<String, SideEntry> = HashMap::new();
    for (rel, r) in all {
        if is_cache_rel(&rel) {
            continue;
        }
        if is_excluded(
            &rel,
            &common.includes,
            &common.excludes,
            common.case_sensitive,
        ) {
            continue;
        }
        out.insert(
            rel,
            SideEntry {
                kind: r.kind,
                size: r.size,
                mtime_ns: r.mtime_ns,
                cached: r.hashes,
                fresh: true,
            },
        );
    }
    if !common.case_sensitive {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for k in out.keys() {
            map.entry(k.to_lowercase()).or_default().push(k.clone());
        }
        for (_, v) in map {
            let uniq: HashSet<&String> = v.iter().collect();
            if uniq.len() > 1 {
                bail!(
                    "mixed-case collision in record {}: {}",
                    label,
                    v.join(" vs ")
                );
            }
        }
    }
    // A record's row *is* its stat — there is no disk to disagree with — so
    // every row is fresh and its digests are carried whole.
    Ok(SideScan::of_entries(out))
}

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
    side: Side,
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

/// **Phase C**: produce what `plan` asked for, and finalize the map.
///
/// Dispatches on what the side *is*, not on whether its handle can write: a
/// read-only folder still hashes freely, and a record cannot hash at all.
pub fn resolve_side(opened: &mut OpenSide, mode: ScanMode, plan: &HashPlan) -> Result<SideScan> {
    match &opened.side {
        Side::Folder(root) => {
            let root = root.clone();
            resolve_folder(&root, &opened.cache, mode, &opened.phase_a, plan)
        }
        Side::Record(_) => Ok(resolve_record(&opened.phase_a)),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A file row carrying `md5` and `sha256` digests, distinguishable by a
    /// single byte so an assertion cannot pass on length alone.
    fn row(size: u64, mtime_ns: i64, md5: u8, sha256: u8) -> FileRec {
        FileRec {
            kind: "file".into(),
            size,
            mtime_ns,
            hashes: [
                ("md5".to_string(), vec![md5; 16]),
                ("sha256".to_string(), vec![sha256; 32]),
            ]
            .into_iter()
            .collect(),
        }
    }

    fn digests(algos: &[(&str, u8)]) -> HashMap<String, Vec<u8>> {
        algos
            .iter()
            .map(|(a, b)| (a.to_string(), vec![*b; 16]))
            .collect()
    }

    fn algos_of(rec: &FileRec) -> Vec<String> {
        let mut v: Vec<String> = rec.hashes.keys().cloned().collect();
        v.sort();
        v
    }

    // The whole write rule is one question: does the stat still match?

    #[test]
    fn stat_matches_only_an_identical_file_row() {
        let r = row(10, 99, 1, 2);
        assert!(stat_matches(Some(&r), 10, 99), "same size and mtime");
        assert!(!stat_matches(Some(&r), 11, 99), "size moved");
        assert!(!stat_matches(Some(&r), 10, 100), "mtime moved");
        assert!(!stat_matches(None, 10, 99), "no prior row");
    }

    /// A dir row is presence-only, so its stat fields are all zero. Without the
    /// kind check, a file that replaced a directory of the same name could
    /// inherit the directory's (empty) digest set as a "fresh" match, and a
    /// 0-byte file at mtime 0 would be judged fresh against it forever.
    #[test]
    fn stat_matches_rejects_a_dir_row() {
        assert!(!stat_matches(Some(&FileRec::dir()), 0, 0));
    }

    #[test]
    fn merge_row_without_a_prior_keeps_only_what_was_computed() {
        let out = merge_row(10, 99, digests(&[("md5", 7)]), None);
        assert_eq!((out.size, out.mtime_ns), (10, 99));
        assert_eq!(algos_of(&out), vec!["md5"]);
        assert_eq!(out.hashes["md5"], vec![7u8; 16]);
    }

    /// Stat changed: the content is assumed to have changed with it, so every
    /// stored digest is suspect — including `sha256`, which this run never asked
    /// for and therefore never had reason to distrust. This is the drop that
    /// keeps a changed file from being judged against its pre-change digest.
    #[test]
    fn merge_row_drops_unrequested_digests_when_the_stat_changed() {
        let prior = row(10, 99, 1, 2);
        let out = merge_row(10, 100, digests(&[("md5", 7)]), Some(&prior));
        assert_eq!(algos_of(&out), vec!["md5"], "sha256 was never recomputed");
        assert_eq!(
            out.hashes["md5"],
            vec![7u8; 16],
            "and md5 took the new value"
        );
    }

    /// Stat unchanged: digests this run did not ask for are still valid and ride
    /// along, while the ones it did ask for are replaced.
    #[test]
    fn merge_row_merges_over_the_prior_row_when_the_stat_holds() {
        let prior = row(10, 99, 1, 2);
        let out = merge_row(10, 99, digests(&[("md5", 7)]), Some(&prior));
        assert_eq!(algos_of(&out), vec!["md5", "sha256"], "sha256 rides along");
        assert_eq!(out.hashes["md5"], vec![7u8; 16], "md5 was recomputed");
        assert_eq!(out.hashes["sha256"], vec![2u8; 32], "sha256 was not");
    }

    /// `--hash none` with an unchanged stat: stat is recorded, digests survive.
    /// This is the case that a digest-free scan must get right, since it is the
    /// only thing standing between `girsync update --hash none` and destroying
    /// every digest in the cache.
    #[test]
    fn merge_row_with_nothing_computed_preserves_digests_when_the_stat_holds() {
        let prior = row(10, 99, 1, 2);
        let out = merge_row(10, 99, HashMap::new(), Some(&prior));
        assert_eq!(algos_of(&out), vec!["md5", "sha256"]);
        assert_eq!(out.hashes["md5"], vec![1u8; 16]);
    }

    /// `--hash none` with a *changed* stat: the row becomes stat-only. A row with
    /// a stat and no digest is a normal, valid state, and it is the expected one for
    /// every file a lazy scan decided by stat.
    #[test]
    fn merge_row_with_nothing_computed_drops_digests_when_the_stat_changed() {
        let prior = row(10, 99, 1, 2);
        let out = merge_row(10, 100, HashMap::new(), Some(&prior));
        assert!(out.hashes.is_empty(), "stat-only row");
        assert_eq!((out.size, out.mtime_ns), (10, 100));
    }

    #[test]
    fn put_dir_row_writes_when_absent_or_wrong_kind() {
        let db = CacheDb::open_temp(true).unwrap();
        let mut w = db.begin_write().unwrap();
        // Absent -> written.
        put_dir_row(&mut w, "a").unwrap();
        assert!(w.get("a").unwrap().unwrap().is_dir());
        // Already a dir -> left alone, so a dir-heavy tree is not rewritten per
        // scan for no change.
        w.put("b", &FileRec::dir()).unwrap();
        put_dir_row(&mut w, "b").unwrap();
        assert!(w.get("b").unwrap().unwrap().is_dir());
        // A *file* row must lose its digests: a directory replaced it, and a
        // later scan must not read the directory's stat against them.
        w.put("c", &row(10, 99, 1, 2)).unwrap();
        put_dir_row(&mut w, "c").unwrap();
        let c = w.get("c").unwrap().unwrap();
        assert!(c.is_dir());
        assert!(c.hashes.is_empty());
    }
}
