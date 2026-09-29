//! End-to-end runs of `cmd_update` and `cmd_compare` (real FS + real cache).

mod common;

use common::{TempRoot, compare, log, rfile, sync, sync_mtime, update, wfile};
use girsync::{cmd_compare, cmd_sync, cmd_update};

#[test]
fn run_update_then_compare_equal() {
    let t = TempRoot::new("upd_eq");
    let dir = t.mkdirs("a");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.txt", b"world");

    let code = cmd_update(update(dir.clone()), &log()).unwrap();
    assert_eq!(code, 0);
    assert!(
        dir.join(girsync::cache::CACHE_PREFIX).is_file(),
        "update creates the cache file"
    );

    // A second folder with the same content. mtime is part of identity, so pin
    // the copy's stamps to the original's; a same-root copy is not an option,
    // since `compare` refuses to name one cache twice.
    let mirror = t.mkdirs("a-mirror");
    wfile(&mirror, "a.txt", b"hello");
    wfile(&mirror, "sub/b.txt", b"world");
    for rel in ["a.txt", "sub/b.txt"] {
        sync_mtime(&dir.join(rel), &mirror.join(rel));
    }

    let code = cmd_compare(compare(dir.clone(), mirror.clone()), &log()).unwrap();
    assert_eq!(code, 0);

    // Record vs folder is equal without touching anything else.
    let record = dir.join(girsync::cache::CACHE_PREFIX);
    let code = cmd_compare(compare(record, mirror), &log()).unwrap();
    assert_eq!(code, 0);
}

/// A run must never name one cache twice: redb locks the file, and a folder
/// side rewrites its cache as it scans.
///
/// The self-collision this forbids is not merely wasteful. A folder side
/// populates its cache while building the effective map, so a record compared
/// against its own folder is diffed against a view the run is still mutating —
/// paths reported as drifted are written into the record before the report is
/// even printed, and a follow-up run over the same pair comes back clean. The
/// audit repairs the drift it was supposed to surface.
#[test]
fn run_compare_rejects_same_cache() {
    let t = TempRoot::new("cmp_self");
    let dir = t.mkdirs("a");
    wfile(&dir, "a.txt", b"hello");
    cmd_update(update(dir.clone()), &log()).unwrap();
    let record = dir.join(girsync::cache::CACHE_PREFIX);

    // The same folder on both sides.
    assert!(cmd_compare(compare(dir.clone(), dir.clone()), &log()).is_err());
    // The same record on both sides.
    assert!(
        cmd_compare(compare(record.clone(), record.clone()), &log()).is_err(),
        "one record cannot be compared against itself"
    );
    // A folder and the record inside it, in either order.
    assert!(
        cmd_compare(compare(dir.clone(), record.clone()), &log()).is_err(),
        "the record is the dst folder's own cache"
    );
    assert!(
        cmd_compare(compare(record.clone(), dir.clone()), &log()).is_err(),
        "the record is the src folder's own cache"
    );
    // Two spellings of one folder are one target, not two.
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let spelled = dir.join("sub").join("..");
    assert!(cmd_compare(compare(dir.clone(), spelled), &log()).is_err());

    // A genuinely different folder on the other side still works.
    let other = t.mkdirs("b");
    wfile(&other, "a.txt", b"hello");
    assert!(cmd_compare(compare(dir.clone(), other), &log()).is_ok());
}

#[test]
fn run_compare_detects_diff_then_sync_converges() {
    let t = TempRoot::new("diff_sync");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "keep.txt", b"same");
    wfile(&dst, "keep.txt", b"same");
    wfile(&src, "changed.txt", b"src-new-content-much-longer");
    wfile(&dst, "changed.txt", b"dst-old");
    wfile(&src, "src_only.txt", b"only in src");
    wfile(&dst, "dst_only.txt", b"only in dst");
    wfile(&src, "sub/nested.txt", b"nested");

    let code = cmd_compare(compare(src.clone(), dst.clone()), &log()).unwrap();
    assert_eq!(code, 4, "differences must exit 4");

    let mut o = sync(src.clone(), dst.clone());
    o.jobs = 2;
    let code = cmd_sync(o, &log()).unwrap();
    assert_eq!(code, 0);

    assert_eq!(rfile(&dst, "keep.txt"), b"same");
    assert_eq!(rfile(&dst, "changed.txt"), b"src-new-content-much-longer");
    assert_eq!(rfile(&dst, "src_only.txt"), b"only in src");
    assert_eq!(rfile(&dst, "sub/nested.txt"), b"nested");
    assert!(
        !dst.join("dst_only.txt").exists(),
        "extra deleted by default"
    );

    let code = cmd_compare(compare(src.clone(), dst.clone()), &log()).unwrap();
    assert_eq!(code, 0, "dst must equal src after sync");
}
