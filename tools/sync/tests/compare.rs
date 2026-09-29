//! End-to-end runs of `cmd_update` and `cmd_compare` (real FS + real cache).

mod common;

use common::{TempRoot, compare, log, md5arg, rfile, sync, update, wfile};
use girsync::cache::{get_rec, open_db, put_rec};
use girsync::hash::hash_file;
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

/// A record side cannot backfill a digest, so asking it for an algorithm its
/// entries do not hold is refused rather than silently answered by size+mtime.
///
/// Without the guard, this run reported `total_diff=0` on genuinely different
/// content: the folder side paid to hash every file, and `hashes_differ` stayed
/// quiet because the record had nothing to compare against.
#[test]
fn run_compare_refuses_digest_the_record_lacks() {
    let t = TempRoot::new("rec_digest");
    let dir = t.mkdirs("a");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.bin", b"");
    std::fs::create_dir_all(dir.join("sub")).unwrap();

    // A --hash none record stores stat rows with no digests at all.
    let mut u = update(dir.clone());
    u.common.algos = vec![];
    assert_eq!(cmd_update(u, &log()).unwrap(), 0);

    let record = dir.join(girsync::cache::CACHE_PREFIX);
    let err = cmd_compare(compare(record.clone(), dir.clone()), &log()).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("hold none of the requested digests"),
        "explains the shortfall: {msg}"
    );
    assert!(
        msg.contains("--hash none"),
        "offers the size+mtime escape hatch: {msg}"
    );
    assert!(
        msg.contains("none at all"),
        "reports what the record actually provides: {msg}"
    );

    // --hash none is always answerable, so the same run succeeds.
    let mut c = compare(record, dir.clone());
    c.common.algos = vec![];
    assert_eq!(cmd_compare(c, &log()).unwrap(), 0);
}

/// Coverage is judged per file, not per record: an entry missing a digest is
/// fatal only when it holds *none* of the requested ones. The error names the
/// uncovered entries and the per-algorithm coverage so the remedy is obvious.
#[test]
fn run_compare_refuses_only_fully_uncovered_entries() {
    let t = TempRoot::new("rec_partial");
    let dir = t.mkdirs("a");
    wfile(&dir, "has.txt", b"one");
    wfile(&dir, "bare.txt", b"two");
    assert_eq!(cmd_update(update(dir.clone()), &log()).unwrap(), 0);

    // Strip the digest from exactly one entry, leaving a mixed-history record.
    let db = open_db(&dir.join(girsync::cache::CACHE_PREFIX), true, false, false).unwrap();
    let mut rec = get_rec(&db, "bare.txt").unwrap().unwrap();
    rec.hashes.clear();
    put_rec(&db, "bare.txt", &rec).unwrap();
    drop(db);

    let record = dir.join(girsync::cache::CACHE_PREFIX);
    let err = cmd_compare(compare(record.clone(), dir.clone()), &log()).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("1 of 2 file entries"),
        "counts only the uncovered entry: {msg}"
    );
    assert!(
        msg.contains("bare.txt") && !msg.contains("has.txt"),
        "names the uncovered path: {msg}"
    );
    assert!(
        msg.contains("md5 (1/2)"),
        "reports per-algorithm coverage: {msg}"
    );

    // Repairing the entry makes the same run succeed.
    let db = open_db(&dir.join(girsync::cache::CACHE_PREFIX), true, false, false).unwrap();
    put_rec(
        &db,
        "bare.txt",
        &girsync::cache::FileRec {
            kind: "file".into(),
            size: rec.size,
            mtime_ns: rec.mtime_ns,
            hashes: hash_file(&dir.join("bare.txt"), &md5arg()).unwrap(),
        },
    )
    .unwrap();
    drop(db);
    assert_eq!(
        cmd_compare(compare(record, dir.clone()), &log()).unwrap(),
        0
    );
}
