//! Unit-level tests for the primitives the commands are built on.

mod common;

use std::path::Path;

use common::{md5arg, opts, rw, scan, serial_rw, temp_root, wfile};
use girsync::cache::{CACHE_PREFIX, CacheDb, FileRec, load_all_records, open_db};
use girsync::config::ScanMode;
use girsync::effective::{SideScan, open_folder_cache, resolve_folder, scan_stat_only};
use girsync::filter::{compile_patterns, is_excluded};
use girsync::hash::{HashAlg, parse_hash_list};
use girsync::planner::HashPlan;
use girsync::rw::RwSide;
use girsync::{CommonOpts, ScanStats};

/// A folder-side run's counters, kept **per phase** rather than merged.
///
/// Nothing merges the phases: `cmd_update` reports phase A's `files`/`dirs` and
/// phase C's `hashed`, because those are the only two it needs and they come from
/// different phases — so a merged number here would assert a view nothing ships.
///
/// The split is also the better assertion. "Phase A hashed 0" is the property the
/// whole planner rests on, and a merged total cannot show it: `update`'s eager
/// `files = 3, hashed = 3` looks identical whether phase A read the bytes or phase
/// C did.
#[derive(Debug)]
struct Run {
    /// Phase C's resolved map, keyed by relative path.
    map: std::collections::HashMap<String, girsync::EffRec>,
    /// Phase A's counters: the walk, and `hashed = 0` always.
    a: ScanStats,
    /// Phase C's counters: who got read, who was served, who needed no digest.
    c: ScanStats,
}

/// Phase A, then [`HashPlan::plan_one_side`], then phase C — the pipeline a folder
/// side runs, spelled out.
///
/// `cmd_update` does the same three steps inline. Spelling them out here rather than
/// calling the command is what lets a unit test assert on any one phase, which is the
/// only reason these tests exist as units at all.
fn phased(root: &Path, cache: &CacheDb, common: &CommonOpts, mode: ScanMode) -> Run {
    let phase_a: SideScan<girsync::SideEntry> = scan_stat_only(root, cache, common, mode).unwrap();
    let plan = HashPlan::plan_one_side(&phase_a.map, &common.algos, mode.no_trust_cached_hashes);
    let resolved = resolve_folder(
        root,
        cache,
        mode,
        &phase_a,
        &plan,
        RwSide::Src,
        &serial_rw(),
    )
    .unwrap();
    Run {
        map: resolved.map,
        a: phase_a.stats,
        c: resolved.stats,
    }
}

/// [`phased`] for the tests that want the resolved map and nothing else.
fn resolved_map(
    root: &Path,
    cache: &CacheDb,
    common: &CommonOpts,
    mode: ScanMode,
) -> std::collections::HashMap<String, girsync::EffRec> {
    phased(root, cache, common, mode).map
}

#[test]
fn hash_arg_none_exclusive() {
    assert!(parse_hash_list(&["none".to_string()]).unwrap().is_empty());
    assert!(parse_hash_list(&["NONE".to_string()]).unwrap().is_empty());
    assert!(parse_hash_list(&["md5".to_string(), "none".to_string()]).is_err());
    assert!(parse_hash_list(&["md5".to_string()]).unwrap() == vec![HashAlg::Md5]);
}

#[test]
fn exclude_wins_over_include() {
    let inc = compile_patterns(&["*.dat".to_string()]).unwrap();
    let exc = compile_patterns(&["secret*".to_string()]).unwrap();
    assert!(is_excluded("other.txt", &inc, &exc, true));
    assert!(!is_excluded("a.dat", &inc, &exc, true));
    assert!(is_excluded("secret.dat", &inc, &exc, true));
}

/// An excluded directory is a subtree, not one entry. Before this, `--exclude
/// link` dropped `link` but kept `link/x.txt`, so the planner scheduled a path
/// under the excluded link and the runtime guard deleted the link itself.
#[test]
fn excluding_a_directory_excludes_its_subtree() {
    let exc = compile_patterns(&["link".to_string()]).unwrap();
    assert!(is_excluded("link", &[], &exc, true));
    assert!(is_excluded("link/x.txt", &[], &exc, true));
    assert!(is_excluded("link/sub/y.txt", &[], &exc, true));
    assert!(
        !is_excluded("linkage/x.txt", &[], &exc, true),
        "a prefix is not a subtree: the separator is the boundary"
    );
    assert!(!is_excluded("other/x.txt", &[], &exc, true));
}

/// `--include '*.txt'` matches the file and not the directory above it. The
/// effective map still has to carry that directory: it is the entry that holds
/// `is_link`, and without it a planner looking for a link ancestor sees nothing.
#[test]
fn an_included_file_pulls_its_ancestor_directories_into_the_map() {
    let t = temp_root("include_ancestors");
    let dir = t.subdir("w");
    wfile(&dir, "top.txt", b"top");
    wfile(&dir, "sub/a.txt", b"a");
    wfile(&dir, "sub/skip.bin", b"skip");

    let common = CommonOpts {
        includes: compile_patterns(&["*.txt".to_string()]).unwrap(),
        ..opts()
    };
    let db = CacheDb::open_temp(common.case_sensitive).unwrap();
    let scan = scan_stat_only(&dir, &db, &common, scan(false, false)).unwrap();

    assert!(scan.map.contains_key("top.txt"));
    assert!(
        scan.map.contains_key("sub"),
        "the ancestor directory of an included file is in the map: {:?}",
        scan.map.keys().collect::<Vec<_>>()
    );
    assert!(scan.map.contains_key("sub/a.txt"));
    assert!(
        !scan.map.contains_key("sub/skip.bin"),
        "the include still narrows the file set"
    );
}

#[test]
fn insensitive_mode_adopts_disk_casing_and_survives_mode_switch() {
    // Cache created sensitive with "a.txt"; disk renames to "A.txt".
    // Insensitive run must NOT error: disk governs, cache key becomes
    // "A.txt" (hashes reused), and a later sensitive run still works.
    let t = temp_root("casefix");
    let dir = t.subdir("w");
    std::fs::write(dir.join("a.txt"), b"hello").unwrap();
    let db_path = dir.join(CACHE_PREFIX);

    let sensitive = opts();
    let insensitive = CommonOpts {
        case_sensitive: false,
        ..opts()
    };

    let db = open_db(&db_path, true, rw()).unwrap();
    let eff = resolved_map(&dir, &db, &sensitive, ScanMode::new(true, false));
    assert!(eff.contains_key("a.txt"));
    drop(db);

    // Case-only rename via intermediate (works on case-insensitive FS too).
    let tmp = dir.join("girsync_rename_tmp");
    std::fs::rename(dir.join("a.txt"), &tmp).unwrap();
    std::fs::rename(&tmp, dir.join("A.txt")).unwrap();

    // Disk casing and recorded case mode disagree, and that must not be fatal:
    // disk governs.
    let db = open_db(&db_path, false, rw()).unwrap();
    let eff = resolved_map(&dir, &db, &insensitive, scan(false, false));
    assert!(eff.contains_key("A.txt"), "disk casing governs");
    let want = md5::compute(b"hello").0.to_vec();
    assert_eq!(eff["A.txt"].hashes.get_named("md5").unwrap(), &want);
    let keys: Vec<String> = load_all_records(&db).unwrap().keys().cloned().collect();
    assert!(
        keys.contains(&"A.txt".to_string()),
        "cache key fixed to disk"
    );
    assert!(!keys.contains(&"a.txt".to_string()), "stale casing pruned");
    drop(db);

    // Same record must remain usable in a later sensitive run.
    let db = open_db(&db_path, true, rw()).unwrap();
    let eff = resolved_map(&dir, &db, &sensitive, scan(false, false));
    assert!(eff.contains_key("A.txt"));
    assert_eq!(eff["A.txt"].hashes.get_named("md5").unwrap(), &want);
    drop(db);
}

/// **An adopted key is renamed, not pruned.** The dry run still reads the
/// pre-adoption cache, so the old-cased row is in its key snapshot; counting it
/// as stale would make `pruned` one higher than the real run's, and a dry run
/// answers the same question. The fixture leaves one real orphan beside the
/// adopted row, so the count has a baseline the adoption cannot hide in.
#[test]
fn insensitive_dry_run_counts_an_adopted_key_as_renamed_not_pruned() {
    let t = temp_root("adopt_prune");
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"hello");
    let db_path = dir.join(CACHE_PREFIX);
    {
        let db = open_db(&db_path, true, rw()).unwrap();
        let _ = phased(&dir, &db, &opts(), scan(false, false));
        db.put("vanished.txt", &FileRec::dir()).unwrap();
    }

    // Case-only rename via an intermediate, so it works on either kind of
    // filesystem.
    let tmp = dir.join("girsync_rename_tmp");
    std::fs::rename(dir.join("a.txt"), &tmp).unwrap();
    std::fs::rename(&tmp, dir.join("A.txt")).unwrap();

    let insensitive = CommonOpts {
        case_sensitive: false,
        ..opts()
    };
    let (dry_map, dry_stats) = {
        let db = open_db(&db_path, false, rw()).unwrap();
        let pa = scan_stat_only(&dir, &db, &insensitive, scan(false, true)).unwrap();
        (pa.map, pa.stats)
    };
    assert!(dry_map.contains_key("A.txt"), "disk casing governs");
    assert!(
        dry_map["A.txt"].fresh && dry_map["A.txt"].cached.get_named("md5").is_some(),
        "the adopted row's digests are carried"
    );
    assert_eq!(
        dry_stats.pruned, 1,
        "the orphan only: the adopted old key is renamed, not pruned"
    );
    // The dry run decided to rename and wrote neither half of it.
    let rows = load_all_records(&open_db(&db_path, false, rw()).unwrap()).unwrap();
    assert!(
        rows.contains_key("a.txt"),
        "the old-cased row survives a dry run"
    );
    assert!(
        !rows.contains_key("A.txt"),
        "and the disk-cased row was not created"
    );

    let real = {
        let db = open_db(&db_path, false, rw()).unwrap();
        scan_stat_only(&dir, &db, &insensitive, scan(false, false)).unwrap()
    };
    assert_eq!(
        dry_stats, real.stats,
        "a dry run answers the same question; only the writes are gone"
    );
    assert_eq!(real.stats.pruned, 1, "the same one orphan");
    let rows = load_all_records(&open_db(&db_path, false, rw()).unwrap()).unwrap();
    assert!(rows.contains_key("A.txt"), "the real run renamed the key");
    assert!(!rows.contains_key("a.txt"), "and dropped the old casing");
}

/// **Every adopted key is excluded from the prune count, per key.**
///
/// The exclusion is a set of keys, so one adoption cannot stand in for another.
/// The fixture adopts `a.txt` and `b.txt`, and next to them leaves `c.txt` with
/// two stale alternates (`C.txt`, `c.TXT`) in its lowercase group: adoption only
/// takes a group with exactly one leftover, so those two are pruned like any
/// other stale row. Their count is what proves the exclusion is per key rather
/// than "the adopted path's group is skipped".
#[test]
fn insensitive_dry_run_counts_every_adopted_key_as_renamed_not_pruned() {
    let t = temp_root("adopt_prune_many");
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "b.txt", b"world");
    let db_path = dir.join(CACHE_PREFIX);
    {
        let db = open_db(&db_path, true, rw()).unwrap();
        let _ = phased(&dir, &db, &opts(), scan(false, false));
        // Two stale spellings of a third path, so its lowercase group has no
        // single candidate to adopt. Both rows must still be pruned.
        let stale = FileRec {
            kind: "file".into(),
            size: 4,
            mtime_ns: 0,
            hashes: Default::default(),
        };
        db.put("C.txt", &stale).unwrap();
        db.put("c.TXT", &stale).unwrap();
        db.put("vanished.txt", &FileRec::dir()).unwrap();
    }
    wfile(&dir, "c.txt", b"third");

    // Case-only renames via an intermediate, so they work on either kind of
    // filesystem.
    for (from, to) in [("a.txt", "A.txt"), ("b.txt", "B.txt")] {
        let tmp = dir.join(format!("girsync_rename_tmp_{to}"));
        std::fs::rename(dir.join(from), &tmp).unwrap();
        std::fs::rename(&tmp, dir.join(to)).unwrap();
    }

    let insensitive = CommonOpts {
        case_sensitive: false,
        ..opts()
    };
    let (dry_map, dry_stats) = {
        let db = open_db(&db_path, false, rw()).unwrap();
        let pa = scan_stat_only(&dir, &db, &insensitive, scan(false, true)).unwrap();
        (pa.map, pa.stats)
    };
    for adopted in ["A.txt", "B.txt"] {
        assert!(dry_map.contains_key(adopted), "disk casing governs");
        assert!(
            dry_map[adopted].fresh && dry_map[adopted].cached.get_named("md5").is_some(),
            "the adopted row's digests are carried for {adopted}"
        );
    }
    assert!(
        dry_map.contains_key("c.txt"),
        "the multi-alternate path is still on disk"
    );
    assert_eq!(
        dry_stats.pruned, 3,
        "both adopted rows are renamed, not pruned; only C.txt, c.TXT and the orphan are stale"
    );
    // The dry run decided to rename and wrote neither half of it.
    let rows = load_all_records(&open_db(&db_path, false, rw()).unwrap()).unwrap();
    for old in ["a.txt", "b.txt"] {
        assert!(
            rows.contains_key(old),
            "the old-cased row survives a dry run: {old}"
        );
    }
    for new in ["A.txt", "B.txt"] {
        assert!(
            !rows.contains_key(new),
            "and the disk-cased row was not created: {new}"
        );
    }
    assert!(
        rows.contains_key("C.txt") && rows.contains_key("c.TXT"),
        "a dry run prunes no row, adopted or not"
    );

    let real = {
        let db = open_db(&db_path, false, rw()).unwrap();
        scan_stat_only(&dir, &db, &insensitive, scan(false, false)).unwrap()
    };
    assert_eq!(
        dry_stats, real.stats,
        "a dry run answers the same question; only the writes are gone"
    );
    assert_eq!(
        real.stats.pruned, 3,
        "the same three stale rows, with neither adoption counted"
    );
    let rows = load_all_records(&open_db(&db_path, false, rw()).unwrap()).unwrap();
    for adopted in ["A.txt", "B.txt"] {
        assert!(rows.contains_key(adopted), "the real run renamed the key");
    }
    assert!(rows.contains_key("c.txt"), "the live path keeps its row");
    for gone in ["a.txt", "b.txt", "C.txt", "c.TXT", "vanished.txt"] {
        assert!(
            !rows.contains_key(gone),
            "a stale row survived: {gone}; left: {:?}",
            rows.keys().collect::<Vec<_>>()
        );
    }
}

/// A three-file folder scanned on a cold cache, and the same folder scanned again
/// once warm. Returns `(cold, warm)`, each carrying both phases' counters.
fn cold_then_warm(tag: &str, algos: Vec<HashAlg>) -> (Run, Run) {
    let t = temp_root(tag);
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.txt", b"world!");
    wfile(&dir, "sub/deep/c.txt", b"third");
    let common = CommonOpts { algos, ..opts() };
    let db = CacheDb::open_temp(common.case_sensitive).unwrap();

    let one = || phased(&dir, &db, &common, scan(false, false));
    (one(), one())
}

/// A cold cache reads every file, a warm one reads nothing — and **phase A reads
/// nothing either way**, which is the property the planner depends on and the one a
/// merged counter cannot show.
///
/// Nothing about the *map* changes when this number does, which is why the counters
/// are part of the result at all — the diff output alone cannot tell a lazy run from
/// an eager one.
#[test]
fn scan_counters_characterize_the_eager_baseline() {
    let (cold, warm) = cold_then_warm("stats_md5", md5arg());

    assert_eq!(
        (cold.a.files, cold.a.dirs, cold.a.live),
        (3, 2, 5),
        "phase A sees the tree and nothing else"
    );
    assert_eq!(
        (cold.a.hashed, cold.a.cache_hit, cold.a.stat_only),
        (0, 0, 0),
        "phase A never hashes, whatever the cache holds"
    );
    assert_eq!(
        (cold.c.hashed, cold.c.cache_hit, cold.c.stat_only),
        (3, 0, 0),
        "a cold cache leaves every read to phase C"
    );
    assert_eq!(
        (cold.a.pruned, cold.c.hashed - 3),
        (0, 0),
        "and prunes nothing"
    );

    assert_eq!(
        (warm.c.hashed, warm.c.cache_hit, warm.c.stat_only),
        (0, 3, 0),
        "a warm cache with matching stat reads nothing"
    );
    assert_eq!(
        (warm.a.files, warm.a.hashed, warm.a.pruned),
        (3, 0, 0),
        "phase A is the same walk either way"
    );
}

/// Every file a folder scan resolves falls in exactly one bucket: read, served
/// from the cache, or never needed a digest. Without this the three counters can
/// overlap or leave files unaccounted for, and a "0 hashed" result stops meaning
/// anything.
///
/// The buckets are phase C's and the total is phase A's, which is the honest pairing:
/// phase C buckets the files phase A found.
#[test]
fn every_file_lands_in_exactly_one_bucket() {
    for algos in [md5arg(), vec![HashAlg::Md5, HashAlg::Sha256], vec![]] {
        let (cold, warm) = cold_then_warm("stats_buckets", algos.clone());
        for s in [cold, warm] {
            assert_eq!(
                s.c.hashed + s.c.cache_hit + s.c.stat_only,
                s.a.files,
                "files={} hashed={} hit={} stat_only={} algos={algos:?}",
                s.a.files,
                s.c.hashed,
                s.c.cache_hit,
                s.c.stat_only
            );
        }
    }
}

/// `--hash none` is a case where a naive counter would lie. `hash_file` with no
/// algorithms returns empty *without opening the file*, so a counter incremented on
/// the rehash branch would report a cold tree as 3 files hashed having read none.
///
/// The phase split makes it correct for free — an empty algorithm set makes the plan
/// empty, so phase C never runs. The files land in `stat_only` rather than
/// `cache_hit`, because nothing was served from a cache; nothing needed serving.
#[test]
fn hash_none_reads_nothing_and_says_so() {
    let (cold, warm) = cold_then_warm("stats_none", vec![]);
    assert_eq!(
        (cold.c.hashed, cold.c.cache_hit, cold.c.stat_only),
        (0, 0, 3),
        "nothing read, nothing from a cache, nothing needed a digest"
    );
    assert_eq!(
        (cold.a.hashed, cold.c.hashed),
        (0, 0),
        "and the cold run read nothing either: a --hash none run answers from stat alone"
    );
    assert_eq!(
        (warm.c.hashed, warm.c.cache_hit, warm.c.stat_only),
        (0, 0, 3),
        "a row from a --hash none run is stat-only, not a cache hit"
    );
}

/// **Phase A never hashes.** Asserted in isolation, not inferred from a resolved
/// run's counters: a resolved run hides phase A behind phase C, and this is the
/// property that lets the planner see a side's whole digest availability before
/// committing to a read.
#[test]
fn phase_a_never_hashes() {
    let t = temp_root("phase_a");
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.txt", b"world!");
    wfile(&dir, "sub/deep/c.txt", b"third");

    // Cold cache, md5 requested, trust denied: the most eager configuration there
    // is. Phase A must still read zero bytes.
    let db = CacheDb::open_temp(opts().case_sensitive).unwrap();
    let pa = scan_stat_only(&dir, &db, &opts(), scan(true, false)).unwrap();
    assert_eq!(
        (pa.stats.hashed, pa.stats.cache_hit, pa.stats.files),
        (0, 0, 3)
    );
    assert_eq!((pa.stats.live, pa.stats.dirs), (5, 2));
    // Digests it cannot have: the cache was empty, so nothing is carried and no
    // row is fresh.
    assert!(pa.map.values().all(|e| e.cached.is_empty()));
    assert!(pa.map.values().filter(|e| e.is_file()).all(|e| !e.fresh));
    drop(db);

    // Warm cache: phase A carries the digests, and still reads nothing. This is
    // the availability picture the planner needs.
    let db = CacheDb::open_temp(opts().case_sensitive).unwrap();
    let _ = phased(&dir, &db, &opts(), scan(false, false));
    let pa = scan_stat_only(&dir, &db, &opts(), scan(false, false)).unwrap();
    assert_eq!(pa.stats.hashed, 0);
    assert!(pa.map["a.txt"].fresh);
    assert!(
        pa.map["a.txt"].cached.get_named("md5").is_some(),
        "the cached digest is carried for the planner"
    );
    drop(db);
}

/// **Phase A never hashes, and never withholds an answer because it may not
/// write.** Asserted at the phase that computes the answers, so `update` and
/// `sync` are covered as well as `compare`: they reach phase A through the same
/// function.
///
/// The fixture leaves one orphan row and one stale row, so `pruned` is non-zero
/// and the stat-only rewrite has something to do. A dry run must reach the same
/// decisions and report the same counts while writing nothing — the counters are
/// answers, not side effects.
#[test]
fn phase_a_reports_the_same_decisions_when_it_cannot_write() {
    let t = temp_root("phase_a_dry");
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "gone.txt", b"was-here");
    let db_path = dir.join(CACHE_PREFIX);
    {
        let db = open_db(&db_path, true, rw()).unwrap();
        let _ = phased(&dir, &db, &opts(), scan(false, false));
        // An orphan row and a row whose stat no longer matches its file.
        db.put("vanished.txt", &FileRec::dir()).unwrap();
        let mut stale = load_all_records(&db).unwrap()["gone.txt"].clone();
        stale.size = 999;
        db.put("gone.txt", &stale).unwrap();
    }

    let dry = {
        let db = open_db(&db_path, true, rw()).unwrap();
        scan_stat_only(&dir, &db, &opts(), scan(false, true)).unwrap()
    };
    let dry_stats = dry.stats;
    assert!(dry_stats.pruned >= 1, "the orphan row is a prune candidate");
    assert!(
        !dry.map["gone.txt"].fresh && dry.map["gone.txt"].cached.is_empty(),
        "the stale row's digests are dropped from the planner's view either way"
    );
    // The one thing that does differ: nothing was written. Checked *before* the
    // real run below, which will legitimately drop these rows.
    let rows = load_all_records(&open_db(&db_path, true, rw()).unwrap()).unwrap();
    assert!(rows.contains_key("vanished.txt"), "the orphan row survived");
    assert!(
        rows["gone.txt"].size == 999,
        "and the stale row was left as it was, not corrected"
    );
    drop(dry.map);

    let real = {
        let db = open_db(&db_path, true, rw()).unwrap();
        scan_stat_only(&dir, &db, &opts(), scan(false, false)).unwrap()
    };
    assert_eq!(
        dry_stats, real.stats,
        "a dry run answers the same question; only the writes are gone"
    );
}

/// A corrupt cache is refused by both modes. The tempting shortcut — fall back
/// to an in-memory cache so the dry run "still answers" — is exactly the
/// divergence this rules out: a real run exits `3` with no answer at all, so a
/// dry run that reports a verdict is answering a question nobody asked. It looks
/// like an answer, which is what makes it worse than a crash.
#[test]
fn a_corrupt_cache_is_refused_by_a_dry_run_too() {
    let t = temp_root("corrupt_dry");
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"hello");
    let db_path = dir.join(CACHE_PREFIX);
    std::fs::write(&db_path, b"not a redb database").unwrap();

    let err = open_folder_cache(&dir, &opts(), scan(false, true), false)
        .and_then(|db| scan_stat_only(&dir, &db, &opts(), scan(false, true)))
        .expect_err("a dry run must refuse a corrupt cache like a real run");
    assert!(
        format!("{:#}", err).contains("not a redb database")
            || format!("{:#}", err).contains("corrupt"),
        "a dry run refuses a corrupt cache like a real run: {:#}",
        err
    );
}

/// A stat-changed row's digests must not reach the planner. Phase A is where that
/// decision is made, so it is asserted here: poison a row with a digest the file
/// does not have, change the stat, and confirm the poison is neither carried nor
/// left on disk. Carrying it would judge a changed file against its pre-change
/// digest, which is the whole reason the carry rule keys on the stat alone.
#[test]
fn phase_a_does_not_carry_a_stat_changed_rows_digests() {
    let t = temp_root("phase_a_stale");
    let dir = t.subdir("w");
    wfile(&dir, "a.txt", b"hello");
    let db_path = dir.join(CACHE_PREFIX);
    let db = open_db(&db_path, true, rw()).unwrap();
    let _ = phased(&dir, &db, &opts(), scan(false, false));
    let real = load_all_records(&db).unwrap()["a.txt"]
        .hashes
        .get_named("md5")
        .unwrap()
        .to_vec();
    let mut poisoned = load_all_records(&db).unwrap()["a.txt"].clone();
    poisoned.hashes.insert_named("md5", &[0u8; 16]);
    db.put("a.txt", &poisoned).unwrap();
    drop(db);

    // Touch the file so the stat moves. Same length, so only mtime can.
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(120);
    std::fs::OpenOptions::new()
        .write(true)
        .open(dir.join("a.txt"))
        .unwrap()
        .set_modified(later)
        .unwrap();

    let db = open_db(&db_path, true, rw()).unwrap();
    let pa = scan_stat_only(&dir, &db, &opts(), scan(false, false)).unwrap();
    let e = &pa.map["a.txt"];
    assert!(!e.fresh, "the stat moved, so the row is stale");
    assert!(
        e.cached.is_empty(),
        "a stale row's digests must not reach the planner"
    );
    // And the row on disk became stat-only, so the poison is gone rather than
    // merely unread.
    let after = load_all_records(&db).unwrap();
    assert!(after["a.txt"].hashes.is_empty());
    assert_ne!(
        after["a.txt"].hashes.get_named("md5"),
        Some(real.as_slice())
    );
    drop(db);
}
