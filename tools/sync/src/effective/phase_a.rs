//! **Phase A** — walk the tree, consult the cache, read no bytes.

use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tracing::{info, trace};

use crate::cache::{CacheDb, FileRec};
use crate::config::{CommonOpts, ScanMode};
use crate::filter::ExcludeMemo;
use crate::hash::{DigestSet, HashAlg};
use crate::scan::{LiveEnt, check_mixed_case, walk_live};
use crate::util::{elapsed_s, path_key};

use super::entry::{ScanStats, SideEntry, SideScan, merge_row, put_dir_row, stat_matches};

/// The cached spellings under one folded key.
///
/// The lookup asks two things of a group: how many spellings it holds, and
/// the spelling it may adopt when it holds exactly one. A group of two needs
/// both of its spellings — the disk's own spelling may be one of them, and
/// the other is the stale one — and a group of three or more is never
/// adopted, so it needs nothing but its count.
///
/// **The second spelling is what keeps the rule free of an invariant.** A
/// lookup that reaches a group has already missed the disk's own spelling
/// exactly, so the cache holds no row under it and the group cannot hold it
/// either — that is the usual shape, but it is not a rule this leans on: a
/// group of two answers for whichever of its two spellings is not the
/// disk's own, so the answer is the same whether or not the disk's own is
/// among them. Were that invariant ever stated as load-bearing, the second
/// spelling could go and a group of two would be two stale spellings,
/// adopted by nothing.
///
/// A group holds **where** its spellings are rather than copies of them, so a
/// folded key costs no allocation of its own: the index holds the keys it was
/// built from, and a group names up to two places in them.
struct Group {
    /// How many cached spellings fold to this key.
    count: usize,
    /// Where the group's first spelling is in the key set.
    first: usize,
    /// Where the group's second spelling is, when the group holds two.
    ///
    /// **Kept so the rule does not lean on the disk's own spelling never
    /// being in a group.** A lookup does not normally reach such a group —
    /// the exact lookup missed, so the cache holds no row under it — but the
    /// rule answers that case anyway rather than resting on it.
    second: Option<usize>,
}

/// The alternate-case index over the cached keys: a folded spelling → the
/// cached spellings under it.
///
/// Every cached key contributes one folded spelling, so building it costs a
/// copy of every key in the cache. The scan builds it only when a lookup
/// needs it — see the index's own note in [`scan_stat_only`].
///
/// The key set is **kept**, and not only folded: the prune asks the cache the
/// same question — "which rows exist?" — so the scan reads the table's keys
/// once and both passes answer from it.
///
/// A folded key can hold *several* stale spellings, and the lookup asks how
/// many there are, so this is not the key-shaped [`crate::util::folded_keys`] one.
struct AltIndex {
    /// Every cached key, as read.
    keys: Vec<String>,
    /// Folded spelling → the cached spellings under it.
    groups: HashMap<String, Group>,
}

/// The alternate-case index over the given keys.
///
/// The groups hold **where** their spellings are rather than copies of them:
/// a group is a count and up to two places in the key set. A folded key
/// therefore costs a folded spelling and nothing else — where the index used
/// to allocate a vector of spellings for every folded key it met, and a
/// group with several stale spellings was the shape it met most.
fn fold_alternates(keys: Vec<String>) -> AltIndex {
    let mut groups: HashMap<String, Group> = HashMap::new();
    for (at, key) in keys.iter().enumerate() {
        let folded = path_key(key);
        match groups.get_mut(&folded) {
            None => {
                groups.insert(
                    folded,
                    Group {
                        count: 1,
                        first: at,
                        second: None,
                    },
                );
            }
            Some(group) => {
                group.count += 1;
                if group.count == 2 {
                    group.second = Some(at);
                }
            }
        }
    }
    AltIndex { keys, groups }
}

impl AltIndex {
    /// The stale spelling under `folded`, if the group holds exactly one
    /// spelling that is not the disk's own.
    ///
    /// The disk's own spelling is the one the walk found, so a group that
    /// holds it holds no stale spelling at all, and a group that holds it
    /// beside another spelling offers that other one.
    ///
    /// A lookup does not normally reach a group that holds the disk's own
    /// spelling — the exact lookup missed, so the cache holds no row under
    /// it — but the rule does not lean on that: a group of two answers for
    /// whichever of its two spellings is not the disk's own, and a group of
    /// one answers for its only spelling unless that one is the disk's own.
    /// The shape is pinned by a case of its own.
    fn adoptable(&self, folded: &str, disk: &str) -> Option<&str> {
        let group = self.groups.get(folded)?;
        let first = self.keys[group.first].as_str();
        match group.count {
            1 => (first != disk).then_some(first),
            2 => {
                let second = self.keys[group.second?].as_str();
                if first == disk {
                    Some(second)
                } else if second == disk {
                    Some(first)
                } else {
                    None
                }
            }
            // Three or more: at least two of them are stale, and a group
            // with two stale spellings is not adopted.
            _ => None,
        }
    }
}

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
/// says it changed. `--hash-all-of none` and `--no-trust-cached-hashes` both write stat
/// data while leaving unrequested digests intact. This is `merge_row`'s rule,
/// stated here because this is where the rows are written.
///
/// Phase A opens, uses and commits its own batched write handle, so the row set
/// is durable before the expensive phase begins.
///
/// In insensitive mode the on-disk name governs: cached keys are indexed by
/// the folded key — the path key's ASCII-case fold — so a disk/cached casing
/// difference is fixed to the disk name first, carrying the cached hashes when
/// stat matches, instead of erroring. Multiple stale alternates are pruned,
/// not treated as a conflict.
pub fn scan_stat_only(
    root: &Path,
    cache: &CacheDb,
    common: &CommonOpts,
    mode: ScanMode,
) -> Result<SideScan<SideEntry>> {
    let algos: &[HashAlg] = &common.algos;
    let case_sensitive = common.case_sensitive;
    let max_depth = common.max_depth;
    let ScanMode {
        no_trust_cached_hashes,
        dry_run,
    } = mode;

    let live = walk_live(root, max_depth)?;
    check_mixed_case(&live, &root.display().to_string(), case_sensitive)?;
    let mut map: HashMap<String, SideEntry> = HashMap::new();
    // Borrowed from `live`, which is why the scan below walks `&live` rather
    // than consuming it. The set exists to answer "is this row still live?"
    // in O(1) without re-running every glob, and holding the path strings here
    // would be a second full copy of the tree's path set alive for the whole
    // phase — one allocation per file, for a lookup the answer to.
    let mut live_set: HashSet<&str> = HashSet::new();
    // Every live directory, so an included descendant can pull its ancestors
    // back in. `--include '*.txt'` matches files and not the directories above
    // them; without this, a `dst` link on that path would be missing from the
    // map and the planner could schedule a path *through* it. The directory
    // entry is what carries `is_link`, so keeping it is what makes the guard
    // possible.
    //
    // The entry is borrowed beside its key, because the pull-in needs the
    // directory's `is_link` and one map of borrowed keys answers both questions
    // without copying a path string.
    let dirs: HashMap<&str, &LiveEnt> = live
        .iter()
        .filter(|e| e.is_dir)
        .map(|e| (e.rel.as_str(), e))
        .collect();
    // The ancestors an included entry pulled back in, collected during the walk
    // and inserted after it. The fix-up below says why they wait.
    let mut pending_ancestors: HashSet<&str> = HashSet::new();
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
    // The alternate-case index, built when a lookup first needs it.
    //
    // Only the keys decide whether a path has an alternate; the row is read
    // when a stale alternate is actually adopted, which is normally rare. So
    // the index is built on first use rather than up front: a scan in which
    // every path is found in the cache under the spelling the disk uses
    // never asks for one, and building the index for it costs a folded
    // spelling per cached row — a price paid for nothing on that path.
    //
    // What triggers the build is a path that misses exactly, not a spelling
    // that differs: a path the cache has never held misses exactly too. So a
    // run that only adds a file pays for the index as well, even though no
    // spelling differs anywhere — the miss was about the path being new.
    //
    // The keys are the committed ones, read through the cache handle: the
    // scan's batch handle starts from the same committed state, and the rows
    // this run writes are keyed by live paths, which the mixed-case guard
    // keeps out of every other path's folded group — so the two views agree
    // about the group a lookup asks about.
    //
    // A folded key here can hold *several* stale spellings, and the lookup asks
    // how many there are, so this index is not the key-shaped `util::folded_keys`
    // one.
    let mut alt_index: Option<AltIndex> = None;
    // An adopted key is renamed to the disk casing and removed, so it must not
    // also be counted as stale when the prune set is read below.
    let mut adopted_keys: HashSet<String> = HashSet::new();
    // Non-dry runs funnel every cache mutation through one batched handle,
    // which auto-commits periodically so progress survives interruption.
    let mut batch = if dry_run {
        None
    } else {
        Some(cache.begin_write()?)
    };
    let mut last_prog = std::time::Instant::now();
    // The filter's decision, remembered per directory: an exclude pattern
    // that covers a directory covers everything under it, so a child of a
    // covered directory is covered without a test. The walk yields a
    // directory before its contents, so a child's parent has usually been
    // decided already — and where it has not, the memo falls back to the
    // whole chain, so the order can cost time but cannot change the answer.
    let mut excl = ExcludeMemo::new(&common.filter);
    for e in &live {
        // The filter decides once per entry, here. An entry the filter hides
        // is treated as not existing at all: it is not in the map, and its
        // cache row is a prune.
        if excl.is_excluded(e.rel.as_str(), e.is_dir) {
            continue; // treated as not existing at all
        }
        live_set.insert(e.rel.as_str());
        if e.is_dir {
            map.insert(e.rel.clone(), SideEntry::dir(e.is_link));
            if let Some(w) = batch.as_mut() {
                put_dir_row(w, &e.rel)?;
            }
        } else {
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
                    // The index is built here, on the first miss: a scan that
                    // never misses never pays for it.
                    if alt_index.is_none() {
                        alt_index = Some(fold_alternates(cache.load_keys()?));
                    }
                    // The one stale spelling this group offers, if it offers
                    // exactly one: the group is adopted only when a single
                    // spelling other than the disk's own is left, so nothing
                    // is collected to make the count.
                    let stale = alt_index
                        .as_ref()
                        .and_then(|index| index.adoptable(&path_key(&e.rel), &e.rel));
                    match stale {
                        Some(old) => {
                            let rec = match batch.as_mut() {
                                Some(w) => w.get(old)?,
                                None => cache.get(old)?,
                            };
                            if let Some(w) = batch.as_mut() {
                                w.remove(old)?; // old casing dropped; new one upserted below
                            }
                            adopted_keys.insert(old.to_string());
                            (rec, true)
                        }
                        // zero (truly new) or several stale: prune handles leftovers
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
                        _ => DigestSet::default(),
                    },
                    fresh,
                    is_link: e.is_link,
                },
            );
            // The two cases where phase A owes a write regardless of any plan: the
            // stat moved, so the old row is now wrong; or the key was adopted from a
            // stale casing, so the disk-cased key does not exist yet. The row is built
            // unconditionally — only the put is gated, so "what would be stored" is
            // never something that depends on whether the run may write.
            if !fresh || adopted {
                let row = merge_row(e.size, e.mtime_ns, DigestSet::default(), cached.as_ref());
                trace!(rel = %e.rel, fresh, adopted, "stat-only row");
                if let Some(w) = batch.as_mut() {
                    w.put(&e.rel, &row)?;
                }
            }
            n_done += 1;
            // The message names the phase. Two phases report progress under
            // one name, and the fields do not tell them apart: this one reads
            // no bytes, so it reports `hashed = 0` for the whole run, and a
            // reader (or a parser) that cannot say which phase an event came
            // from cannot say anything about it. The name says.
            if n_done.is_multiple_of(100) || last_prog.elapsed().as_secs() >= 5 {
                info!(
                    root = %root.display(),
                    done = n_done,
                    total = total_live,
                    hashed = 0,
                    cache_hit = 0,
                    elapsed_s = t_eff.elapsed().as_secs_f64(),
                    "scan progress: phase a"
                );
                last_prog = std::time::Instant::now();
            }
        }
        // An included entry pulls its ancestor directories back in: the map
        // holds every path the filter keeps *and* every directory above one
        // of them, so a directory above a kept path is in the map whether the
        // filter kept it or hid it. The directory entry is what carries
        // `is_link`.
        //
        // The pull-in is *recorded* rather than done here. The ancestor may be
        // an entry the walk has already passed — it was hidden by the filter on
        // its own account — and the walk's order says nothing about which of
        // the two comes first, so the pull-in cannot depend on it. The fix-up
        // below inserts it, and inserts it once.
        let mut end = 0;
        while let Some(i) = e.rel[end..].find('/') {
            let prefix = &e.rel[..end + i];
            if dirs.contains_key(prefix) && !live_set.contains(prefix) {
                pending_ancestors.insert(prefix);
            }
            end += i + 1;
        }
    }
    // The ancestor fix-up: the directories an included entry pulled back in.
    //
    // `--include '*.txt'` matches files and not the directories above them, so
    // those directories have to be in the map even though the filter hid them:
    // the directory entry is what carries `is_link`, and without it a planner
    // looking for a link ancestor sees nothing.
    //
    // It runs after the walk rather than inside it, so the map does not depend
    // on the order the walk produced: an ancestor is pulled in by the
    // descendant that needs it, whichever of the two the walk reached first.
    // Every ancestor of an included entry is in the set — the walk collects the
    // whole prefix chain at once — so one pass over it is enough.
    for prefix in pending_ancestors.drain() {
        let Some(ent) = dirs.get(prefix) else {
            continue;
        };
        // Already in the map: the directory was included in its own right, and
        // its row was written there. A dir row is written once per directory.
        if !live_set.insert(prefix) {
            continue;
        }
        map.insert(ent.rel.clone(), SideEntry::dir(ent.is_link));
        if let Some(w) = batch.as_mut() {
            put_dir_row(w, prefix)?;
        }
    }
    // Prune DB rows for files no longer on disk (or now excluded).
    //
    // The prune *decision* is an answer, not a side effect: how many rows this run
    // would drop is part of what the run reports, so it is computed in both modes
    // and only the `remove` calls are suppressed. Gating the whole loop on the
    // write handle would make a dry run report `pruned = 0` over a tree a real run
    // prunes, i.e. it would answer a different question.
    //
    // The key set the prune works from: the index's, when the index was built,
    // and the cache's otherwise.
    //
    // The two answer the same question — "which rows exist?" — so the scan
    // reads the table's keys once: a scan that missed a path exactly would
    // otherwise walk them twice, once for the index and once here. The views
    // differ by this run's own writes, which are live paths and are never
    // pruned, and by this run's adoptions, which are excluded below, so the
    // rows this drops are the same either way.
    //
    // Reading through the batch handle when there is one is not required for
    // correctness: every key this run has written is a live key, and a live key is
    // never pruned, so the batch's pending writes cannot change the set. It does
    // change nothing about the *count* either way.
    //
    // Keys, not records. Nothing below looks at a `FileRec` — the question is
    // only "does this row have a path on disk?" — and `load_all` would decode
    // the entire `entries` table, hash map by hash map, to answer it. The
    // alternate-case index above reads this table's keys for the same reason;
    // neither pass decodes a row it does not need.
    //
    // **The keys are streamed, not collected.** The question is "which rows
    // exist?", and the answer is usually "none of them": a `Vec<String>` with
    // one string per cached row is an allocation per row for a question that
    // rarely has a row to say. So the cache hands the keys over one at a time,
    // and the only ones this keeps are the stale ones — the rows to remove,
    // of which there are few.
    //
    // The index's keys are the same set read a moment earlier, and the index
    // holds them already, so the prune takes them from there rather than
    // reading the table a second time. They are moved rather than copied, so
    // the stale ones cost nothing to keep.
    let mut stale: Vec<String> = Vec::new();
    {
        // The rule, in one place: a row is stale when its path is not live and
        // was not adopted.
        //
        // An adopted key was renamed, not pruned. A real run's batch has already
        // dropped it, but a dry run only decided to, so it is still in this
        // pre-adoption snapshot; without the exclusion it would be counted once
        // as a prune and once as the rename it actually is.
        let is_stale = |rel: &str| !live_set.contains(rel) && !adopted_keys.contains(rel);
        match alt_index.take() {
            Some(index) => {
                for rel in index.keys {
                    if is_stale(&rel) {
                        stale.push(rel);
                    }
                }
            }
            None => match batch.as_mut() {
                Some(w) => w.visit_keys(|rel| {
                    if is_stale(rel) {
                        stale.push(rel.to_string());
                    }
                    Ok(())
                })?,
                None => cache.visit_keys(|rel| {
                    if is_stale(rel) {
                        stale.push(rel.to_string());
                    }
                    Ok(())
                })?,
            },
        }
    }
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
    // The message names the phase. The map is finished in two phases and each
    // of them says so, because the fields do not: only the phase that hashes
    // reports the stat-only count, so an event without that field is this
    // phase's — a thing a reader has to know to read the log at all.
    info!(
        root = %root.display(),
        files = stats.files,
        dirs = stats.dirs,
        hashed = 0,
        cache_hit = 0,
        pruned = stats.pruned,
        elapsed_s = elapsed_s(t_eff),
        "effective done: phase a"
    );
    Ok(SideScan { map, stats })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{FileRec, TestCacheBackend};
    use crate::filter::{Filter, compile_patterns};
    use crate::planner::{HashMode, StatTrust};
    use crate::rw::RwLimits;

    /// A fixture's files, with the directories they imply.
    fn tree(root: &Path, files: &[(&str, &[u8])]) {
        for (rel, bytes) in files {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        }
    }

    /// Scan options: one algorithm, and whatever filters the case asks for.
    fn opts(includes: &[&str], excludes: &[&str]) -> CommonOpts {
        let owned = |pats: &[&str]| -> Vec<String> { pats.iter().map(|s| s.to_string()).collect() };
        CommonOpts {
            algos: vec![HashAlg::Md5],
            stat: StatTrust::default(),
            hash_mode: HashMode::default(),
            why: false,
            show_identical: false,
            filter: Filter::new(
                compile_patterns(&owned(includes)).unwrap(),
                compile_patterns(&owned(excludes)).unwrap(),
                true,
            ),
            case_sensitive: true,
            max_depth: 10,
            ignore_cache: false,
            dry_run: false,
            rw: RwLimits::default(),
            test_cache: TestCacheBackend::default(),
            backup: crate::config::BackupPolicy::default(),
        }
    }

    /// The same options, in insensitive mode.
    ///
    /// The case mode is the one thing the alternate-case index turns on, so a
    /// case about it needs the other mode.
    fn insensitive(mut o: CommonOpts) -> CommonOpts {
        o.case_sensitive = false;
        o
    }

    /// The map's keys, sorted.
    ///
    /// A map is a set of paths, and the order it was built in is not a property
    /// anything depends on — so what a case asserts is the set.
    fn keys(scan: &SideScan<SideEntry>) -> Vec<String> {
        let mut k: Vec<String> = scan.map.keys().cloned().collect();
        k.sort();
        k
    }

    /// The paths the map holds for a given filter, stated as the rule rather
    /// than as a list: every path the filter keeps, plus every **directory**
    /// above one of them.
    ///
    /// Computed from the walk and the filter alone, so it says what the map is
    /// instead of saying how it is built.
    fn expected_keys(root: &Path, common: &CommonOpts) -> Vec<String> {
        let live = walk_live(root, common.max_depth).unwrap();
        let kept = |rel: &str| !common.filter.is_excluded(rel);
        let is_dir = |rel: &str| live.iter().any(|e| e.is_dir && e.rel == rel);
        let mut out: Vec<String> = Vec::new();
        for e in &live {
            if !kept(&e.rel) {
                continue;
            }
            out.push(e.rel.clone());
            // Every directory above a kept path, up to the root.
            let mut end = 0;
            while let Some(i) = e.rel[end..].find('/') {
                let prefix = &e.rel[..end + i];
                if is_dir(prefix) {
                    out.push(prefix.to_string());
                }
                end += i + 1;
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// The map holds the kept paths and the directories above them, for every
    /// filter — not for the one a case happened to pick.
    ///
    /// The rule is stated once, in [`expected_keys`], and the filters below are
    /// the shapes that make it interesting: a pattern that matches a file and
    /// not its directory, one that matches a directory and not the directory
    /// above it, and an exclude that hides a whole subtree.
    #[test]
    fn the_map_holds_the_kept_paths_and_the_directories_above_them() {
        let root = girpr_tempdir::TempDir::new("girsync_phase_a", "map_rule");
        tree(
            &root,
            &[
                ("a/keep/x.txt", b"x"),
                ("a/keep/deep/y.txt", b"y"),
                ("a/other/z.txt", b"z"),
                ("a/other/z.bin", b"b"),
                ("top.txt", b"t"),
            ],
        );
        let db = CacheDb::open_temp(true).unwrap();
        for (includes, excludes) in [
            (vec![], vec![]),
            (vec!["*.txt"], vec![]),
            (vec!["a/keep/*.txt"], vec![]),
            (vec!["a/keep"], vec![]),
            (vec![], vec!["a/other"]),
            (vec![], vec!["a"]),
            (vec![], vec!["*.bin"]),
            (vec!["*.txt"], vec!["a/keep/deep"]),
        ] {
            let common = opts(&includes, &excludes);
            let scan = scan_stat_only(&root, &db, &common, ScanMode::new(false, false)).unwrap();
            assert_eq!(
                keys(&scan),
                expected_keys(&root, &common),
                "includes={includes:?} excludes={excludes:?}"
            );
        }
    }

    /// `--include 'sub/*.txt'` matches the file and not the directory above it.
    /// The directory is in the map anyway: it is the entry that carries
    /// `is_link`, and a planner looking for a link ancestor sees nothing
    /// without it.
    #[test]
    fn a_narrowed_include_carries_the_directory_above_it() {
        let root = girpr_tempdir::TempDir::new("girsync_phase_a", "narrowed_include");
        tree(
            &root,
            &[("sub/a.txt", b"a"), ("sub/b.bin", b"b"), ("top.txt", b"t")],
        );
        let db = CacheDb::open_temp(true).unwrap();
        let scan = scan_stat_only(
            &root,
            &db,
            &opts(&["sub/*.txt"], &[]),
            ScanMode::new(false, false),
        )
        .unwrap();

        assert_eq!(keys(&scan), vec!["sub", "sub/a.txt"]);
        assert!(
            !scan.map["sub"].is_file(),
            "the ancestor is a directory entry, not a file"
        );
        assert!(
            db.get("sub").unwrap().unwrap().is_dir(),
            "the pulled-in directory has its own row: {:?}",
            db.get("sub").unwrap()
        );
    }

    /// The pull-in is transitive: a file three levels down brings every
    /// directory above it, not only the one it sits in.
    #[test]
    fn a_nested_include_carries_every_ancestor_up_to_the_root() {
        let root = girpr_tempdir::TempDir::new("girsync_phase_a", "nested_include");
        tree(
            &root,
            &[
                ("a/b/c/d.txt", b"d"),
                ("a/b/c/e.bin", b"e"),
                ("a/b/other.txt", b"o"),
            ],
        );
        let db = CacheDb::open_temp(true).unwrap();
        let scan = scan_stat_only(
            &root,
            &db,
            &opts(&["a/b/c/*.txt"], &[]),
            ScanMode::new(false, false),
        )
        .unwrap();

        assert_eq!(keys(&scan), vec!["a", "a/b", "a/b/c", "a/b/c/d.txt"]);
    }

    /// An excluded directory is a subtree: neither the directory nor anything
    /// under it is in the map.
    #[test]
    fn an_excluded_directory_hides_its_whole_subtree() {
        let root = girpr_tempdir::TempDir::new("girsync_phase_a", "excluded_subtree");
        tree(
            &root,
            &[
                ("keep/x.txt", b"x"),
                ("drop/y.txt", b"y"),
                ("drop/deep/z.txt", b"z"),
            ],
        );
        let db = CacheDb::open_temp(true).unwrap();
        let scan = scan_stat_only(
            &root,
            &db,
            &opts(&[], &["drop"]),
            ScanMode::new(false, false),
        )
        .unwrap();

        assert_eq!(keys(&scan), vec!["keep", "keep/x.txt"]);
    }

    /// A pattern that hides one file hides that file and nothing else: the
    /// directories around it are kept, and so is the sibling beside it.
    ///
    /// The point is the boundary of the hiding. A directory is kept because
    /// the filter kept it, not because anything below it was kept, and one
    /// hidden file in a subtree says nothing about the rest of that subtree.
    #[test]
    fn a_pattern_that_hides_one_file_hides_only_that_file() {
        let root = girpr_tempdir::TempDir::new("girsync_phase_a", "one_hidden_file");
        tree(
            &root,
            &[
                ("a/b/keep.txt", b"k"),
                ("a/b/hide.bin", b"h"),
                ("a/c/other.txt", b"o"),
            ],
        );
        let db = CacheDb::open_temp(true).unwrap();
        let scan = scan_stat_only(
            &root,
            &db,
            &opts(&[], &["*.bin"]),
            ScanMode::new(false, false),
        )
        .unwrap();

        assert_eq!(
            keys(&scan),
            vec!["a", "a/b", "a/b/keep.txt", "a/c", "a/c/other.txt"]
        );
    }

    /// The counters a narrowed include produces, on a tree whose cache was
    /// filled by a first, unfiltered scan.
    ///
    /// The numbers are the point. The map is a set of paths whichever way it
    /// is built, and `pruned` counts the rows the filter took out of it — the
    /// directory rows above an included file included.
    #[test]
    fn the_counters_of_a_narrowed_include_describe_the_same_tree() {
        let root = girpr_tempdir::TempDir::new("girsync_phase_a", "narrowed_counters");
        tree(
            &root,
            &[
                ("top.txt", b"t"),
                ("sub/a.txt", b"a"),
                ("sub/b.txt", b"b"),
                ("sub/deep/c.txt", b"c"),
                ("other/d.txt", b"d"),
            ],
        );
        let db = CacheDb::open_temp(true).unwrap();

        // A first, unfiltered scan: every path in the tree gets a row.
        let full =
            scan_stat_only(&root, &db, &opts(&[], &[]), ScanMode::new(false, false)).unwrap();
        assert_eq!(
            (
                full.stats.live,
                full.stats.files,
                full.stats.dirs,
                full.stats.pruned
            ),
            (8, 5, 3, 0)
        );

        // The narrowed scan: the `sub` tree's text files, with the directories
        // above them pulled in. `top.txt` and the whole of `other` are out of
        // the map, so their rows are pruned.
        let narrow = scan_stat_only(
            &root,
            &db,
            &opts(&["sub/*.txt"], &[]),
            ScanMode::new(false, false),
        )
        .unwrap();
        assert_eq!(
            keys(&narrow),
            vec![
                "sub",
                "sub/a.txt",
                "sub/b.txt",
                "sub/deep",
                "sub/deep/c.txt"
            ]
        );
        assert_eq!(
            (
                narrow.stats.live,
                narrow.stats.files,
                narrow.stats.dirs,
                narrow.stats.pruned
            ),
            (8, 3, 2, 3)
        );
    }

    /// **The alternate-case index is built when a lookup needs it.**
    ///
    /// The index costs a folded spelling per cached row, and a scan in which
    /// every path is found in the cache under the spelling the disk uses
    /// never asks for one. The count of entries-table reads says whether the
    /// scan paid for it: the index is one read of the key set, and a scan
    /// that never misses must make none.
    #[test]
    fn the_alternate_index_is_built_only_when_a_lookup_needs_it() {
        let root = girpr_tempdir::TempDir::new("girsync_phase_a", "alt_index");
        tree(&root, &[("a.txt", b"a")]);
        let db = CacheDb::open_temp(false).unwrap();
        let common = insensitive(opts(&[], &[]));

        // Nothing is cached, so the path misses exactly and the index is
        // built.
        let before = db.entries_read_count();
        scan_stat_only(&root, &db, &common, ScanMode::new(false, false)).unwrap();
        assert_eq!(
            db.entries_read_count() - before,
            1,
            "the miss cost one read of the key set: the index"
        );

        // The cache now holds the key under the spelling the disk uses, so
        // the second scan finds it exactly and never asks for an alternate.
        let before = db.entries_read_count();
        scan_stat_only(&root, &db, &common, ScanMode::new(false, false)).unwrap();
        assert_eq!(
            db.entries_read_count() - before,
            0,
            "no miss, no index, and no read of the key set either"
        );
    }

    /// **A run that only adds a file still pays for the index.**
    ///
    /// What builds the index is a path that misses exactly, and a path the
    /// cache has never held misses exactly too. So the run that adds one
    /// file to a tree whose other files are all cached builds the index, even
    /// though no spelling differs anywhere: the miss was about the path
    /// being new, not about its case.
    ///
    /// This is the shape the index's own note has to leave out. A scan whose
    /// cached keys all match the spelling the disk uses still pays for the
    /// index when it adds a file, so "the cached keys match" is not the
    /// condition that saves the work.
    #[test]
    fn a_run_that_only_adds_a_file_still_builds_the_index() {
        let root = girpr_tempdir::TempDir::new("girsync_phase_a", "alt_index_new");
        tree(&root, &[("a.txt", b"a")]);
        let db = CacheDb::open_temp(false).unwrap();
        let common = insensitive(opts(&[], &[]));

        // The first scan caches the file; nothing was cached, so the path
        // missed and the index was built.
        scan_stat_only(&root, &db, &common, ScanMode::new(false, false)).unwrap();

        // The second scan finds it exactly and builds nothing.
        let before = db.entries_read_count();
        scan_stat_only(&root, &db, &common, ScanMode::new(false, false)).unwrap();
        assert_eq!(db.entries_read_count() - before, 0, "no miss, no index");

        // A new file misses exactly as well, and the miss builds the index.
        tree(&root, &[("b.txt", b"b")]);
        let before = db.entries_read_count();
        scan_stat_only(&root, &db, &common, ScanMode::new(false, false)).unwrap();
        assert_eq!(
            db.entries_read_count() - before,
            1,
            "the new path missed exactly, so the index was built: the miss was not about the case"
        );
    }

    /// **A stale alternate is still adopted when the index is built on
    /// demand.**
    ///
    /// The index is built on the first miss, so the adoption it decides has
    /// to answer the same as an index built up front would: the one stale
    /// spelling is taken over, and the row it holds is carried under the
    /// disk's spelling — digests and all.
    #[test]
    fn a_stale_alternate_is_adopted_when_the_index_is_built_on_demand() {
        let root = girpr_tempdir::TempDir::new("girsync_phase_a", "alt_adopt");
        tree(&root, &[("a.txt", b"a")]);
        let db = CacheDb::open_temp(false).unwrap();
        let common = insensitive(opts(&[], &[]));

        // A row the cache holds under the other spelling, with a digest in it
        // and the file's own stat, so it is carried whole.
        let meta = std::fs::metadata(root.join("a.txt")).unwrap();
        let mut hashes = DigestSet::default();
        hashes.insert_named("md5", &[7u8; 16]);
        db.put(
            "A.txt",
            &FileRec {
                kind: "file".into(),
                size: meta.len(),
                mtime_ns: crate::util::mtime_ns_of(&meta),
                hashes,
            },
        )
        .unwrap();

        let scan = scan_stat_only(&root, &db, &common, ScanMode::new(false, false)).unwrap();
        assert_eq!(
            scan.map["a.txt"].cached.get_named("md5"),
            Some(&[7u8; 16][..]),
            "the adopted row's digest is carried under the disk's spelling"
        );
        assert!(
            scan.map["a.txt"].fresh,
            "and the stat still matches, so the row was carried whole"
        );
    }

    /// **The group is adopted only when exactly one of its spellings is
    /// stale.**
    ///
    /// A group is the spellings the cache holds under one folded key, and
    /// the lookup wants the one it may adopt: exactly one spelling other
    /// than the disk's own. The shapes are the ones a group can have — one
    /// spelling, two, three — and the disk's own spelling may be one of
    /// them or none of them.
    ///
    /// **The disk's own spelling being in the group is a case a lookup does
    /// not normally reach**, and the rule answers it anyway: the group names
    /// its second spelling so that the answer does not depend on the disk's
    /// own never being among them. The two spellings are therefore named
    /// both ways round here, and the case where neither is the disk's own is
    /// named too.
    #[test]
    fn a_group_is_adopted_only_when_exactly_one_spelling_is_stale() {
        let index = fold_alternates(
            [
                "A.txt", "B.txt", "c.txt", "C.txt", "e.txt", "f.txt", "F.txt", "f.TXT",
            ]
            .map(str::to_string)
            .to_vec(),
        );

        // One spelling in the group, and it is not the disk's own: it is the
        // stale one, and it is offered.
        assert_eq!(index.adoptable("a.txt", "a.txt"), Some("A.txt"));
        assert_eq!(index.adoptable("b.txt", "b.txt"), Some("B.txt"));

        // The disk's own spelling is the only one in the group: nothing is
        // stale, so nothing is offered.
        assert_eq!(index.adoptable("e.txt", "e.txt"), None);

        // Two spellings in the group, and the disk's own is one of them: the
        // other is the stale one.
        assert_eq!(index.adoptable("c.txt", "c.txt"), Some("C.txt"));
        assert_eq!(index.adoptable("c.txt", "C.txt"), Some("c.txt"));

        // Two spellings in the group, and the disk's own is neither: both are
        // stale, and a group with two stale spellings is not adopted.
        assert_eq!(index.adoptable("c.txt", "z.txt"), None);

        // Three spellings in the group: at least two are stale, so none of
        // them is offered.
        assert_eq!(index.adoptable("f.txt", "f.txt"), None);
    }
}
