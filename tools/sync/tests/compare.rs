//! End-to-end runs of `cmd_update` and `cmd_compare` (real FS + real cache).

mod common;

use common::{TempRoot, compare, log, rfile, sync, update, wfile};
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

    // Folder vs itself is equal.
    let code = cmd_compare(compare(dir.clone(), dir.clone()), &log()).unwrap();
    assert_eq!(code, 0);

    // Record vs folder is equal without touching anything else.
    let record = dir.join(girsync::cache::CACHE_PREFIX);
    let code = cmd_compare(compare(record, dir.clone()), &log()).unwrap();
    assert_eq!(code, 0);
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
