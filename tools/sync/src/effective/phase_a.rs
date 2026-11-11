//! **Phase A** — walk the tree, consult the cache, read no bytes.

use anyhow::Result;
use glob::Pattern;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tracing::{info, trace};

use crate::cache::{CacheDb, FileRec};
use crate::config::{CommonOpts, ScanMode};
use crate::filter::is_excluded;
use crate::hash::{DigestSet, HashAlg};
use crate::scan::{LiveEnt, check_mixed_case, walk_live};
use crate::util::{elapsed_s, path_key};

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
/// data while leaving unrequested digests intact. This is `merge_row`'s rule,
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
    let algos: &[HashAlg] = &common.algos;
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
    // Only the keys decide whether a path has an alternate; the row is read
    // when a stale alternate is actually adopted, which is normally rare. Read
    // once up front, because the scan's batch handle starts from the same
    // committed state.
    let alt_keys = if case_sensitive {
        Vec::new()
    } else {
        cache.load_keys()?
    };
    // A folded key here can hold *several* stale spellings, and the lookup asks
    // how many there are, so this index is not the key-shaped `util::folded_keys`
    // one.
    let mut alt_index: HashMap<String, Vec<String>> = HashMap::new();
    for key in alt_keys {
        alt_index.entry(path_key(&key)).or_default().push(key);
    }
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
    for e in &live {
        // The filter is asked once per entry, here. An entry the filter hides
        // is treated as not existing at all: it is not in the map, and its
        // cache row is a prune.
        if is_excluded(&e.rel, includes, excludes, case_sensitive) {
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
                    match alt_index.get(&path_key(&e.rel)) {
                        Some(alts) => {
                            let others: Vec<&String> =
                                alts.iter().filter(|k| *k != &e.rel).collect();
                            if others.len() == 1 {
                                let old = others[0];
                                let rec = match batch.as_mut() {
                                    Some(w) => w.get(old)?,
                                    None => cache.get(old)?,
                                };
                                if let Some(w) = batch.as_mut() {
                                    w.remove(old)?; // old casing dropped; new one upserted below
                                }
                                adopted_keys.insert(old.clone());
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
        // An included entry pulls its ancestor directories back in: the
        // directories above it are part of the path the filter kept, and the
        // directory entry is what carries `is_link`.
        //
        // The pull-in is *recorded* rather than done here. The ancestor may be
        // an entry the walk has already passed — it was hidden by the filter on
        // its own account — and the walk's order says nothing about which of
        // the two comes first. The fix-up below inserts it, and inserts it
        // once.
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
    let cached_keys = match batch.as_mut() {
        Some(w) => w.load_keys()?,
        None => cache.load_keys()?,
    };
    // An adopted key was renamed, not pruned. A real run's batch has already
    // dropped it, but a dry run only decided to, so it is still in this
    // pre-adoption snapshot; without the exclusion it would be counted once as
    // a prune and once as the rename it actually is.
    let stale: Vec<String> = cached_keys
        .iter()
        .filter(|rel| !live_set.contains(rel.as_str()) && !adopted_keys.contains(rel.as_str()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::TestCacheBackend;
    use crate::filter::compile_patterns;
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
            includes: compile_patterns(&owned(includes)).unwrap(),
            excludes: compile_patterns(&owned(excludes)).unwrap(),
            case_sensitive: true,
            max_depth: 10,
            ignore_cache: false,
            dry_run: false,
            rw: RwLimits::default(),
            test_cache: TestCacheBackend::default(),
        }
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
        let kept = |rel: &str| {
            !is_excluded(
                rel,
                &common.includes,
                &common.excludes,
                common.case_sensitive,
            )
        };
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
}
