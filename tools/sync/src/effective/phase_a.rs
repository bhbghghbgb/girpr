//! **Phase A** — walk the tree, consult the cache, read no bytes.

use anyhow::Result;
use glob::Pattern;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tracing::{info, trace};

use crate::cache::{CacheDb, FileRec};
use crate::config::{CommonOpts, ScanMode};
use crate::filter::is_excluded;
use crate::scan::{check_mixed_case, walk_live};
use crate::util::elapsed_s;

use super::entry::{ScanStats, SideEntry, SideScan, merge_row, put_dir_row, stat_matches};

/// **Phase A.** Walk `root`, consult the cache, and report what it knows about
/// every path — without reading a single file's bytes.
///
/// The output is a [`SideEntry`] per path: its kind, its stat, and the digests
/// the cache holds *for that exact stat*. Nothing here decides what still needs
/// computing; that is [`crate::planner::HashPlan`]'s job, and keeping the two apart is what lets
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
