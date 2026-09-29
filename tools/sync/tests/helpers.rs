//! Unit-level tests for the primitives the commands are built on.

mod common;

use common::{TempRoot, md5arg, opts, rw, scan, wfile};
use girsync::CommonOpts;
use girsync::ScanStats;
use girsync::cache::{CACHE_PREFIX, load_all_records, open_db};
use girsync::config::ScanMode;
use girsync::effective::build_effective_folder;
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

/// Characterization of the scan counters **as they stand today**, i.e. before W2.
///
/// The number pinned here is the one W2 exists to change: a scan hashes a file
/// whenever it is not a fresh, complete cache hit, so on a cold cache it reads
/// every file — including the ones whose verdict size+mtime already decided.
/// W2's first behavioural stage turns `cold.hashed` from 3 into 0 for a tree
/// whose pairs all differ in stat, and leaves it at 3 for one where they do not.
/// Nothing about the *map* changes either way, which is why the counters had to
/// become part of the result: the diff output alone cannot tell the two apart.
#[test]
fn scan_counters_characterize_the_eager_baseline() {
    let (cold, warm) = cold_then_warm("stats_md5", md5arg());

    assert_eq!(
        (
            cold.files,
            cold.dirs,
            cold.hashed,
            cold.cache_hit,
            cold.pruned
        ),
        (3, 2, 3, 0, 0),
        "a cold cache reads every file"
    );
    assert_eq!(
        (
            warm.files,
            warm.dirs,
            warm.hashed,
            warm.cache_hit,
            warm.pruned
        ),
        (3, 2, 0, 3, 0),
        "a warm cache with matching stat reads nothing"
    );
}

/// The counter misreports on `--hash none`, and this pins that rather than the
/// truth: `hash_file` with no algorithms returns empty *without opening the
/// file*, yet the scan still counts the file as hashed, because it took the
/// rehash branch on a cold cache and that branch increments the counter.
///
/// So today `--hash none` over a cold tree reports 3 files hashed and reads
/// none. W2 makes the counter honest — the plan calls this out as the reason
/// `hashed` is the only measure worth asserting on — and this test is what
/// records the starting number.
#[test]
fn hash_none_misreports_hashed_today() {
    let (cold, warm) = cold_then_warm("stats_none", vec![]);
    assert_eq!(
        (cold.hashed, cold.cache_hit),
        (3, 0),
        "PINNED BUG: nothing was read, but the counter says otherwise"
    );
    assert_eq!(
        (warm.hashed, warm.cache_hit),
        (0, 3),
        "warm, it reports honestly: an empty algo set is a cache hit"
    );
}
