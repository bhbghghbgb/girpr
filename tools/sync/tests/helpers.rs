//! Unit-level tests for the primitives the commands are built on.

mod common;

use common::{TempRoot, md5arg, opts, rw, scan, wfile};
use girsync::CommonOpts;
use girsync::ScanStats;
use girsync::cache::{CACHE_PREFIX, FileRec, load_all_records, open_db};
use girsync::config::ScanMode;
use girsync::effective::{build_effective_folder, open_folder_cache, scan_stat_only};
use girsync::filter::{compile_patterns, is_excluded};
use girsync::hash::parse_hash_list;

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
    let eff = build_effective_folder(
        &dir,
        &db,
        &sensitive,
        ScanMode {
            no_trust_cached_hashes: true,
            dry_run: false,
        },
    )
    .unwrap()
    .map;
    assert!(eff.contains_key("a.txt"));
    drop(db);

    // Case-only rename via intermediate (works on case-insensitive FS too).
    let tmp = dir.join("girsync_rename_tmp");
    std::fs::rename(dir.join("a.txt"), &tmp).unwrap();
    std::fs::rename(&tmp, dir.join("A.txt")).unwrap();

    // Previously this errored in open_db (meta mismatch). Must succeed now.
    let db = open_db(&db_path, false, rw()).unwrap();
    let eff = build_effective_folder(&dir, &db, &insensitive, scan(false, false))
        .unwrap()
        .map;
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
    let eff = build_effective_folder(&dir, &db, &sensitive, scan(false, false))
        .unwrap()
        .map;
    assert!(eff.contains_key("A.txt"));
    assert_eq!(eff["A.txt"].hashes.get("md5").unwrap(), &want);
    drop(db);

    std::fs::remove_dir_all(&dir).ok();
}

/// A three-file folder scanned on a cold cache, and the same folder scanned again
/// once warm. Returns `(cold, warm)`.
fn cold_then_warm(tag: &str, algos: Vec<String>) -> (ScanStats, ScanStats) {
    let t = TempRoot::new(tag);
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.txt", b"world!");
    wfile(&dir, "sub/deep/c.txt", b"third");
    let common = CommonOpts { algos, ..opts() };
    let db_path = dir.join(CACHE_PREFIX);

    let cold = {
        let db = open_db(&db_path, true, rw()).unwrap();
        build_effective_folder(&dir, &db, &common, scan(false, false))
            .unwrap()
            .stats
    };
    let warm = {
        let db = open_db(&db_path, true, rw()).unwrap();
        build_effective_folder(&dir, &db, &common, scan(false, false))
            .unwrap()
            .stats
    };
    (cold, warm)
}

/// A resolved folder's counters, unchanged by the phase split: a cold cache reads
/// every file, a warm one reads nothing.
///
/// This is the number stage 3 moves. Nothing about the *map* changes when it
/// does, which is why the counters had to become part of the result in the first
/// place — the diff output alone cannot tell a lazy run from an eager one.
#[test]
fn scan_counters_characterize_the_eager_baseline() {
    let (cold, warm) = cold_then_warm("stats_md5", md5arg());

    assert_eq!(
        (
            cold.files,
            cold.dirs,
            cold.hashed,
            cold.cache_hit,
            cold.stat_only,
            cold.pruned
        ),
        (3, 2, 3, 0, 0, 0),
        "a cold cache reads every file"
    );
    assert_eq!(
        (
            warm.files,
            warm.dirs,
            warm.hashed,
            warm.cache_hit,
            warm.stat_only,
            warm.pruned
        ),
        (3, 2, 0, 3, 0, 0),
        "a warm cache with matching stat reads nothing"
    );
}

/// Every file a folder scan resolves falls in exactly one bucket: read, served
/// from the cache, or never needed a digest. Without this the three counters can
/// overlap or leave files unaccounted for, and a "0 hashed" result stops meaning
/// anything.
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
                s.hashed + s.cache_hit + s.stat_only,
                s.files,
                "files={} hashed={} hit={} stat_only={} algos={algos:?}",
                s.files,
                s.hashed,
                s.cache_hit,
                s.stat_only
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
        (cold.hashed, cold.cache_hit, cold.stat_only),
        (0, 0, 3),
        "nothing read, nothing from a cache, nothing needed a digest"
    );
    assert_eq!(
        (warm.hashed, warm.cache_hit, warm.stat_only),
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
    let _ = build_effective_folder(&dir, &db, &opts(), scan(false, false)).unwrap();
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
        let _ = build_effective_folder(&dir, &db, &opts(), scan(false, false)).unwrap();
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
    let _ = build_effective_folder(&dir, &db, &opts(), scan(false, false)).unwrap();
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
