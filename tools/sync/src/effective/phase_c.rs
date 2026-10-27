//! **Phase C** — hash what the planner asked for, and finalize the map.
//!
//! The only phase that opens a file, and it opens exactly the ones
//! [`HashPlan`] names. Three entry points, because a side is not one thing:
//! [`resolve_folder`] reads, [`resolve_record`] does not have anything to read,
//! and [`resolve_side`] picks between them.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info, trace};

use crate::cache::{CacheDb, CacheWrite};
use crate::config::ScanMode;
use crate::hash::hash_file;
use crate::planner::HashPlan;
use crate::rw::{RwRuntime, RwSide};
use crate::util::elapsed_s;

use super::entry::{EffRec, ScanStats, SideEntry, SideScan, merge_row};
use super::side::{OpenSide, Side};

/// **Phase C.** Hash what the plan asked for, and finalize each path's map entry.
///
/// This is the only phase that opens a file, and it opens exactly the ones
/// [`HashPlan`] names — no more, so a stat-mismatch short circuit in the planner
/// is worth real I/O; no fewer, so a digest the diff will compare is always
/// present on both sides.
///
/// A path the plan leaves alone is finalized from what phase A carried, and
/// costs no write: its row is already correct. A path that is hashed is written
/// through `merge_row`, which is the same write rule phase A used, so the two
/// phases cannot spell it differently.
///
/// ## Reads in parallel, each row written as its hash lands
///
/// [`crate::cache::CacheWrite`] owns a live `redb::WriteTransaction`, so it is not
/// `Sync`. The phase wraps it in a mutex — the same shape the copy phase uses — and
/// each worker writes its own row as soon as its hash verifies:
///
/// 1. the worker hashes one path, holding one permit on `side`'s counter;
/// 2. it releases the permit, takes the mutex, writes the row through the batch
///    handle, and inserts the `EffRec` into the map.
///
/// A `put` is one table insert, so the hand-off is per file rather than a queue, and
/// [`crate::cache::CommitLimits`] is checked on every completed digest instead of
/// once per window of them. At most one digest per pool thread is in flight, so the
/// hand-off buffer is bounded by the pool; the map the phase returns is tree-sized,
/// as it has to be.
///
/// The permit is released before the mutex is taken. A worker waiting on the cache
/// lock therefore holds no permit, and the cache lock is never nested with another;
/// the rw wait-for graph is unchanged.
///
/// **Write order is not a promise.** Rows land in completion order. That cannot
/// change a decision: the effective map is a `HashMap` and the planner looks paths
/// up by key, and the cache is a keyed table, so a later read sees the same rows
/// whatever order they were inserted in. What it does drop is byte-for-byte
/// reproducibility of `girpr-cache`: redb's physical layout can depend on insertion
/// order. Nothing consumes those bytes as a contract — the byte-identity promises
/// are about dry runs doing no writes at all — and the old windowed form bought the
/// property by holding a whole window's digests in memory until its last hash
/// finished.
///
/// A failure is the first one to finish rather than the lowest path: the run aborts,
/// and the rows in the uncommitted batch are dropped with it.
pub fn resolve_folder(
    root: &Path,
    cache: &CacheDb,
    mode: ScanMode,
    phase_a: &SideScan<SideEntry>,
    plan: &HashPlan,
    side: RwSide,
    rw: &RwRuntime,
) -> Result<SideScan> {
    let ScanMode { dry_run, .. } = mode;
    let t_eff = std::time::Instant::now();
    info!(
        root = %root.display(),
        pending = plan.pending().len(),
        digests = plan.digest_count(),
        "resolve start"
    );
    // Sorted even though writes are completion-ordered: it gives the pool the same
    // starting order every run, so which paths are in flight when a failure aborts
    // the run does not depend on `HashMap` iteration order.
    let mut pending: Vec<&String> = phase_a
        .map
        .iter()
        .filter(|(rel, e)| e.is_file() && !plan.is_settled(rel))
        .map(|(rel, _)| rel)
        .collect();
    pending.sort();

    // What the workers share: the batch handle, the map they are building, and the
    // counters that drive the heartbeat. One mutex covers all three because a row
    // being on disk and the map holding its `EffRec` is one commit, not two.
    struct Sink<'a> {
        batch: Option<CacheWrite<'a>>,
        map: HashMap<String, EffRec>,
        hashed: usize,
        last_prog: std::time::Instant,
    }

    let sink = std::sync::Mutex::new(Sink {
        batch: if dry_run {
            None
        } else {
            Some(cache.begin_write()?)
        },
        map: HashMap::new(),
        hashed: 0,
        last_prog: std::time::Instant::now(),
    });
    let total = pending.len();
    let outcome = rw.pool().install(|| {
        use rayon::prelude::*;
        pending.par_iter().try_for_each(|rel| -> Result<()> {
            // The permit covers the read. The row write is cache work, which the
            // gate deliberately does not cover, and taking the mutex while holding
            // a permit would put an fsync inside the rw wait-for graph.
            let computed = {
                let _permit = rw.acquire(side);
                let path = root.join(rel);
                let algos = plan.get(rel);
                debug!(rel = %rel, path = %path.display(), algos = ?algos, "hashing");
                let r =
                    hash_file(&path, &algos).with_context(|| format!("hash {}", path.display()));
                trace!(
                    rel = %rel,
                    algos = ?r.as_ref().map(|h| h.iter().map(|(a, _)| a).collect::<Vec<_>>()),
                    "hashed"
                );
                r?
            };
            let e = &phase_a.map[*rel];
            let row = merge_row(e.size, e.mtime_ns, computed, e.prior().as_ref());
            let mut sink = sink.lock().expect("resolve lock is never poisoned");
            if let Some(w) = sink.batch.as_mut() {
                w.put(rel, &row)?;
            }
            sink.map.insert(
                (*rel).clone(),
                EffRec {
                    kind: "file".into(),
                    size: row.size,
                    mtime_ns: row.mtime_ns,
                    hashes: row.hashes,
                    is_link: e.is_link,
                },
            );
            sink.hashed += 1;
            // Logged under the lock so the `done` counts appear in the order they
            // were reached rather than the order workers got scheduled to log.
            if sink.hashed.is_multiple_of(100)
                || sink.hashed == total
                || sink.last_prog.elapsed().as_secs() >= 5
            {
                info!(
                    root = %root.display(),
                    done = sink.hashed,
                    total,
                    hashed = sink.hashed,
                    cache_hit = 0,
                    elapsed_s = t_eff.elapsed().as_secs_f64(),
                    "scan progress"
                );
                sink.last_prog = std::time::Instant::now();
            }
            Ok(())
        })
    });
    outcome?;

    let Sink {
        batch,
        mut map,
        hashed: n_hashed,
        ..
    } = sink.into_inner().expect("resolve lock is never poisoned");
    if let Some(w) = batch {
        w.commit()?;
    }

    // Everything the plan settled: a file, a dir, and no read.
    let mut n_cache_hit = 0usize;
    let mut n_stat_only = 0usize;
    for (rel, e) in &phase_a.map {
        if map.contains_key(rel) {
            continue;
        }
        if !e.is_file() {
            map.insert(rel.clone(), EffRec::dir(e.is_link));
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
                is_link: e.is_link,
            },
        );
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
                    is_link: e.is_link,
                },
            )
        })
        .collect();
    SideScan::of_map(map)
}

/// **Phase C**: produce what `plan` asked for, and finalize the map.
///
/// Dispatches on what the side *is*, not on whether its handle can write: a
/// read-only folder still hashes freely, and a record cannot hash at all.
///
/// `side` is which side of the pair this is, and is only consulted under a split
/// limit — under one shared counter both sides draw on the same gate. It is
/// threaded in rather than inferred, because a [`Side::Record`] never reaches the
/// hashing path that would need it.
pub fn resolve_side(
    opened: &mut OpenSide,
    mode: ScanMode,
    plan: &HashPlan,
    side: RwSide,
    rw: &RwRuntime,
) -> Result<SideScan> {
    match &opened.side {
        Side::Folder(root) => {
            let root = root.clone();
            resolve_folder(&root, opened.cache(), mode, &opened.phase_a, plan, side, rw)
        }
        Side::Record(_) => Ok(resolve_record(&opened.phase_a)),
    }
}
