//! Unit-level tests for the primitives the commands are built on.

mod common;

use std::path::Path;

use common::{TempRoot, md5arg, opts, rw, scan, wfile};
use girsync::CommonOpts;
use girsync::ScanStats;
use girsync::cache::{CACHE_PREFIX, CacheDb, FileRec, load_all_records, open_db};
use girsync::config::ScanMode;
use girsync::effective::{SideScan, open_folder_cache};
use girsync::effective::{resolve_folder, scan_stat_only};
use girsync::filter::{compile_patterns, is_excluded};
use girsync::hash::parse_hash_list;
use girsync::planner::HashPlan;

/// A folder-side run's counters, kept **per phase** rather than merged.
///
/// `build_effective_folder` handed back one merged `ScanStats` and these tests
/// asserted it. Nothing merges any more: `cmd_update` reports phase A's
/// `files`/`dirs` and phase C's `hashed`, because those are the only two it needs
/// and they come from different phases — so a merged number here would assert a
/// view nothing ships.
///
/// The split is also the better assertion. "Phase A hashed 0" is the property the
/// whole planner rests on, and a merged total cannot show it: `update`'s eager
/// `files = 3, hashed = 3` looks identical whether phase A read the bytes or phase
/// C did, which is precisely the thing stage 2 moved and stage 3 depends on.
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
/// `cmd_update` does the same three steps inline, and `build_effective_folder` used
/// to hide them. Duplicating them here rather than calling the command is what lets
/// a unit test assert on any one phase, which is the only reason these tests exist
/// as units at all.
fn phased(root: &Path, cache: &CacheDb, common: &CommonOpts, mode: ScanMode) -> Run {
    let phase_a: SideScan<girsync::SideEntry> = scan_stat_only(root, cache, common, mode).unwrap();
    let plan = HashPlan::plan_one_side(&phase_a.map, &common.algos, mode.no_trust_cached_hashes);
    let resolved = resolve_folder(root, cache, mode, &phase_a, &plan).unwrap();
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
    assert!(parse_hash_list(&["md5".to_string()]).unwrap() == vec!["md5".to_string()]);
}

#[test]
fn exclude_wins_over_include() {
    let inc = compile_patterns(&["*.dat".to_string()]).unwrap();
    let exc = compile_patterns(&["secret*".to_string()]).unwrap();
    assert!(is_excluded("other.txt", &inc, &exc, true));
    assert!(!is_excluded("a.dat", &inc, &exc, true));
    assert!(is_excluded("secret.dat", &inc, &exc, true));
}

#[test]
fn insensitive_mode_adopts_disk_casing_and_survives_mode_switch() {
    // Cache created sensitive with "a.txt"; disk renames to "A.txt".
    // Insensitive run must NOT error: disk governs, cache key becomes
    // "A.txt" (hashes reused), and a later sensitive run still works.
    let dir = std::env::temp_dir().join(format!("girsync_test_casefix_{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), b"hello").unwrap();
    let db_path = dir.join(CACHE_PREFIX);

    let sensitive = opts();
    let insensitive = CommonOpts {
        case_sensitive: false,
        ..opts()
    };

    let db = open_db(&db_path, true, rw()).unwrap();
    let eff = resolved_map(
        &dir,
        &db,
        &sensitive,
        ScanMode {
            no_trust_cached_hashes: true,
            dry_run: false,
        },
    );
    assert!(eff.contains_key("a.txt"));
    drop(db);

    // Case-only rename via intermediate (works on case-insensitive FS too).
    let tmp = dir.join("girsync_rename_tmp");
    std::fs::rename(dir.join("a.txt"), &tmp).unwrap();
    std::fs::rename(&tmp, dir.join("A.txt")).unwrap();

    // Previously this errored in open_db (meta mismatch). Must succeed now.
    let db = open_db(&db_path, false, rw()).unwrap();
    let eff = resolved_map(&dir, &db, &insensitive, scan(false, false));
    assert!(eff.contains_key("A.txt"), "disk casing governs");
    let want = md5::compute(b"hello").0.to_vec();
    assert_eq!(eff["A.txt"].hashes.get("md5").unwrap(), &want);
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
    assert_eq!(eff["A.txt"].hashes.get("md5").unwrap(), &want);
    drop(db);

    std::fs::remove_dir_all(&dir).ok();
}

/// A three-file folder scanned on a cold cache, and the same folder scanned again
/// once warm. Returns `(cold, warm)`, each carrying both phases' counters.
fn cold_then_warm(tag: &str, algos: Vec<String>) -> (Run, Run) {
    let t = TempRoot::new(tag);
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.txt", b"world!");
    wfile(&dir, "sub/deep/c.txt", b"third");
    let common = CommonOpts { algos, ..opts() };
    let db_path = dir.join(CACHE_PREFIX);

    let one = || {
        let db = open_db(&db_path, true, rw()).unwrap();
        phased(&dir, &db, &common, scan(false, false))
    };
    (one(), one())
}

/// A cold cache reads every file, a warm one reads nothing — and **phase A reads
/// nothing either way**, which is the property the planner depends on and the one a
/// merged counter cannot show.
///
/// This is the number stage 3 moves. Nothing about the *map* changes when it does,
/// which is why the counters had to become part of the result in the first place —
/// the diff output alone cannot tell a lazy run from an eager one.
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
    for algos in [
        md5arg(),
        vec!["md5".to_string(), "sha256".to_string()],
        vec![],
    ] {
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

/// `--hash none` used to misreport. `hash_file` with no algorithms returns empty
/// *without opening the file*, yet the old single-pass scan still counted the
/// file as hashed, because a cold cache took the rehash branch and that branch
/// incremented the counter: a cold tree reported 3 files hashed and read none.
///
/// The phase split fixes it for free — an empty algorithm set makes the plan
/// empty, so phase C never runs. The files land in `stat_only` rather than
/// `cache_hit`, because nothing was served from a cache; nothing needed serving.
///
/// Recorded here because it is the first case where `hashed` stopped being
/// honest, and the lazy-counts work depends on it being honest.
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
        "and the cold run read nothing either, which is the bug this fixed"
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
    let t = TempRoot::new("phase_a");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.txt", b"world!");
    wfile(&dir, "sub/deep/c.txt", b"third");
    let db_path = dir.join(CACHE_PREFIX);

    // Cold cache, md5 requested, trust denied: the most eager configuration there
    // is. Phase A must still read zero bytes.
    let db = open_db(&db_path, true, rw()).unwrap();
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
    let db = open_db(&db_path, true, rw()).unwrap();
    let _ = phased(&dir, &db, &opts(), scan(false, false));
    let pa = scan_stat_only(&dir, &db, &opts(), scan(false, false)).unwrap();
    assert_eq!(pa.stats.hashed, 0);
    assert!(pa.map["a.txt"].fresh);
    assert!(
        pa.map["a.txt"].cached.contains_key("md5"),
        "the cached digest is carried for the planner"
    );
    drop(db);
}

/// **Phase A never hashes, and never withholds an answer because it may not
/// write.** Asserted at the phase where the bug lived, so `update` and `sync`
/// are covered as well as `compare`: they reach phase A through the same
/// function.
///
/// The fixture leaves one orphan row and one stale row, so `pruned` is non-zero
/// and the stat-only rewrite has something to do. A dry run must reach the same
/// decisions and report the same counts while writing nothing — the counters are
/// answers, not side effects.
#[test]
fn phase_a_reports_the_same_decisions_when_it_cannot_write() {
    let t = TempRoot::new("phase_a_dry");
    let dir = t.mkdirs("w");
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
    let t = TempRoot::new("corrupt_dry");
    let dir = t.mkdirs("w");
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
/// digest — the exact bug this carry rule exists to prevent.
#[test]
fn phase_a_does_not_carry_a_stat_changed_rows_digests() {
    let t = TempRoot::new("phase_a_stale");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    let db_path = dir.join(CACHE_PREFIX);
    let db = open_db(&db_path, true, rw()).unwrap();
    let _ = phased(&dir, &db, &opts(), scan(false, false));
    let real = load_all_records(&db).unwrap()["a.txt"].hashes["md5"].clone();
    let mut poisoned = load_all_records(&db).unwrap()["a.txt"].clone();
    poisoned.hashes.insert("md5".to_string(), vec![0u8; 16]);
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
    assert_ne!(after["a.txt"].hashes.get("md5"), Some(&real));
    drop(db);
}
