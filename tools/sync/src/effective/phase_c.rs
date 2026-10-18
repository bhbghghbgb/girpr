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

use crate::cache::CacheDb;
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
/// ## Two stages per window, and why they are not one
///
/// [`crate::cache::CacheWrite`] owns a live `redb::WriteTransaction`, so it is not `Sync` and
/// must never be touched from a pool thread. So a window is **read** in parallel
/// and then **written** in sequence:
///
/// 1. `pool.install`: every path in the window is hashed, each holding one permit
///    on `side`'s counter. No cache handle is in scope.
/// 2. the sequential loop: results are folded into the map and written through
///    the batch handle, in the window's original order.
///
/// The `collect()` is over a slice, so it preserves index order. That is what
/// makes stage 2's writes land in sorted order, and what makes the reported
/// error the lowest-index one — a `collect::<Result<_, _>>()` would instead let
/// rayon short-circuit and surface whichever failure happened to finish first,
/// which is not reproducible.
///
/// Two promises hold, and they differ in shape:
///
/// - **Write order is sorted; read order is not.** The rows must land in the same
///   order every run, so the cache bytes are reproducible and so is the progress
///   heartbeat. Which files are *read* in which order is not a promise at all: with
///   more than one worker the reads interleave.
/// - **Durability is per-window rather than per-digest.**
///   [`crate::cache::COMMIT_INTERVAL`] bounds how much hashing work an
///   interruption can lose, and that only holds if a digest becomes durable
///   shortly after it is computed. A window keeps it true and bounded; batching
///   the whole phase until the last hash would keep all of it in memory and none
///   of it on disk.
///
/// A window rather than one `collect()` over the whole pending set for a third
/// reason: peak memory. Every pending file holds a live `HashMap<String,
/// Vec<u8>>` until the batch writes it, which on a 200k-file tree is tens of MB
/// of map structure the sequential form never allocated.
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
    let mut batch = if dry_run {
        None
    } else {
        Some(cache.begin_write()?)
    };

    // Sorted, so a window's **writes** land in the same order every run and the
    // progress heartbeat is reproducible. The reads inside a window do not keep
    // this order — see the doc comment.
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
    let window = rw.window_size();
    let total = pending.len();
    for (w, chunk) in pending.chunks(window).enumerate() {
        // Stage 1 — parallel, one permit per worker, no cache handle in scope.
        //
        // The permit is taken *around* the read rather than inside `hash_file`, so
        // the gate stays a property of a phase and `hash_file` keeps the signature
        // every other caller already has.
        let computed: Vec<Result<HashMap<String, Vec<u8>>>> = rw.pool().install(|| {
            use rayon::prelude::*;
            chunk
                .par_iter()
                .map(|rel| {
                    let _permit = rw.acquire(side);
                    let path = root.join(rel);
                    let algos = plan.get(rel);
                    debug!(rel = %rel, path = %path.display(), algos = ?algos, "hashing");
                    let r = hash_file(&path, &algos)
                        .with_context(|| format!("hash {}", path.display()));
                    trace!(
                        rel = %rel,
                        algos = ?r.as_ref().map(|h| h.keys().collect::<Vec<_>>()),
                        "hashed"
                    );
                    r
                })
                .collect()
        });

        // Stage 2 — sequential, in the window's original order. The first `Err` in
        // *index* order is the one that propagates, which is what keeps the
        // reported path reproducible under any number of workers.
        for (rel, computed) in chunk.iter().zip(computed) {
            let computed = computed?;
            let e = &phase_a.map[*rel];
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
        }

        // Heartbeat per window, so it reports whole windows and `window_size` says
        // how much of the run the counter spans.
        if n_hashed.is_multiple_of(100) || n_hashed == total || last_prog.elapsed().as_secs() >= 5 {
            info!(
                root = %root.display(),
                done = n_hashed,
                total,
                hashed = n_hashed,
                cache_hit = 0,
                window = w + 1,
                window_size = window,
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
