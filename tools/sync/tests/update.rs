//! End-to-end runs of `cmd_update` covering pruning and filtering.

mod common;

use common::{compare, log, update, wfile, TempRoot};
use girsync::cache::{load_all_records, CACHE_PREFIX};
use girsync::filter::compile_patterns;
use girsync::{cmd_compare, cmd_update, CommonOpts};

#[test]
fn run_update_prunes_and_excludes() {
    let t = TempRoot::new("prune");
    let dir = t.mkdirs("w");
    wfile(&dir, "keep.txt", b"keep");
    wfile(&dir, "gone.txt", b"to be deleted");
    wfile(&dir, "skip.me", b"excluded content v1");

    cmd_update(update(dir.clone()), &log()).unwrap();
    {
        let db = sled::open(dir.join(CACHE_PREFIX)).unwrap();
        let recs = load_all_records(&db).unwrap();
        assert!(recs.contains_key("gone.txt"));
        assert!(recs.contains_key("skip.me"));
    }

    // Deleting a file + re-update prunes it from the record.
    std::fs::remove_file(dir.join("gone.txt")).unwrap();
    cmd_update(update(dir.clone()), &log()).unwrap();
    {
        let db = sled::open(dir.join(CACHE_PREFIX)).unwrap();
        let recs = load_all_records(&db).unwrap();
        assert!(!recs.contains_key("gone.txt"), "deleted file is pruned");
        assert!(recs.contains_key("keep.txt"));
    }

    // Excluded paths are treated as nonexistent: re-update with an
    // exclude prunes skip.me even though it is still on disk.
    let excluding = CommonOpts {
        excludes: compile_patterns(&["*.me".to_string()]).unwrap(),
        ..common::opts()
    };
    cmd_update(
        girsync::UpdateOpts {
            dir: dir.clone(),
            common: excluding.clone(),
        },
        &log(),
    )
    .unwrap();
    {
        let db = sled::open(dir.join(CACHE_PREFIX)).unwrap();
        let recs = load_all_records(&db).unwrap();
        assert!(!recs.contains_key("skip.me"), "excluded path is pruned");
    }
    assert!(
        dir.join("skip.me").exists(),
        "exclude never deletes disk files"
    );

    // Filters also apply to runs: two dirs differing only in an
    // excluded file compare equal with the filter, different without.
    // NOTE: compare treats size/mtime/hash as equality, so normalize
    // keep.txt's mtime across both dirs (same bytes, fresh writes would
    // otherwise differ by mtime alone and report CHANGED).
    let other = t.mkdirs("other");
    wfile(&other, "keep.txt", b"keep");
    wfile(&other, "skip.me", b"excluded content v2 (different)");
    {
        let mtime = std::fs::metadata(dir.join("keep.txt"))
            .unwrap()
            .modified()
            .unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(other.join("keep.txt"))
            .unwrap()
            .set_modified(mtime)
            .unwrap();
    }
    let mut filtered = compare(dir.clone(), other.clone());
    filtered.common = excluding.clone();
    let code = cmd_compare(filtered, &log()).unwrap();
    assert_eq!(code, 0, "excluded difference is invisible");
    let code = cmd_compare(compare(dir.clone(), other.clone()), &log()).unwrap();
    assert_eq!(code, 4, "same files differ without the filter");
}
