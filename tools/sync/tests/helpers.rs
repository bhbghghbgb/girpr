//! Unit-level tests for the primitives the commands are built on.

mod common;

use common::{opts, scan};
use girsync::CommonOpts;
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

    let db = open_db(&db_path, true, false, false).unwrap();
    let eff = build_effective_folder(
        &dir,
        &db,
        &sensitive,
        ScanMode {
            fast: true,
            force_hash: true,
            dry_run: false,
        },
    )
    .unwrap();
    assert!(eff.contains_key("a.txt"));
    drop(db);

    // Case-only rename via intermediate (works on case-insensitive FS too).
    let tmp = dir.join("girsync_rename_tmp");
    std::fs::rename(dir.join("a.txt"), &tmp).unwrap();
    std::fs::rename(&tmp, dir.join("A.txt")).unwrap();

    // Previously this errored in open_db (meta mismatch). Must succeed now.
    let db = open_db(&db_path, false, false, false).unwrap();
    let eff = build_effective_folder(&dir, &db, &insensitive, scan(true, false, false)).unwrap();
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
    let db = open_db(&db_path, true, false, false).unwrap();
    let eff = build_effective_folder(&dir, &db, &sensitive, scan(true, false, false)).unwrap();
    assert!(eff.contains_key("A.txt"));
    assert_eq!(eff["A.txt"].hashes.get("md5").unwrap(), &want);
    drop(db);

    std::fs::remove_dir_all(&dir).ok();
}
