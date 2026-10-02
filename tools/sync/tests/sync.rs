//! End-to-end runs of `cmd_sync` (real FS + real cache).

mod common;

use common::{
    TempRoot, compare, entry_names, has_backup_sibling, log, rfile, rw, sync, sync_mtime, update,
    wfile,
};
use girsync::cache::{CACHE_PREFIX, FileRec, load_all_records, open_db};
use girsync::{cmd_compare, cmd_sync, cmd_update};
use std::collections::HashMap;
use std::path::Path;

fn recs(dir: &Path) -> HashMap<String, FileRec> {
    load_all_records(&open_db(&dir.join(CACHE_PREFIX), true, rw()).unwrap()).unwrap()
}

#[test]
fn run_sync_dry_run_writes_nothing() {
    let t = TempRoot::new("dryrun");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"new content here");
    wfile(&dst, "a.txt", b"old");
    wfile(&dst, "extra.txt", b"stay for now");

    let mut o = sync(src.clone(), dst.clone());
    o.dry_run = true;
    o.jobs = 2;
    let code = cmd_sync(o, &log()).unwrap();
    assert_eq!(code, 0);
    // Nothing changed on disk.
    assert_eq!(rfile(&dst, "a.txt"), b"old");
    assert_eq!(rfile(&dst, "extra.txt"), b"stay for now");
    assert!(
        !has_backup_sibling(&dst.join(CACHE_PREFIX)),
        "dry-run makes no backups"
    );

    // Still different afterwards.
    let code = cmd_compare(compare(src.clone(), dst.clone()), &log()).unwrap();
    assert_eq!(code, 4);
}

/// A dry run writes nothing, which includes not *creating* the caches: a
/// read/write open would have created one per side and written its `meta`.
#[test]
fn run_sync_dry_run_creates_no_cache() {
    let t = TempRoot::new("drynocache");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"new content here");
    wfile(&dst, "a.txt", b"old");

    let mut o = sync(src.clone(), dst.clone());
    o.dry_run = true;
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    for d in [&src, &dst] {
        assert!(
            !d.join(CACHE_PREFIX).exists(),
            "dry-run leaves {} without a cache",
            d.display()
        );
        assert!(
            !has_backup_sibling(&d.join(CACHE_PREFIX)),
            "dry-run makes no backups"
        );
    }
    // The plan was still the real one, and nothing was applied.
    assert_eq!(rfile(&dst, "a.txt"), b"old");
    assert_eq!(cmd_compare(compare(src, dst), &log()).unwrap(), 4);
}

/// `--ignore-cache` asks for a rebuild, which a dry run may not perform. The
/// real cache is treated as absent rather than deleted: left exactly as it was.
#[test]
fn run_sync_dry_run_with_ignore_cache_leaves_caches_intact() {
    let t = TempRoot::new("dryignore");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"new content here");
    wfile(&dst, "a.txt", b"old");
    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(dst.clone()), &log()).unwrap();
    let (src_before, dst_before) = (recs(&src), recs(&dst));

    let mut o = sync(src.clone(), dst.clone());
    o.dry_run = true;
    o.common.ignore_cache = true;
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    for (dir, before) in [(&src, &src_before), (&dst, &dst_before)] {
        assert!(
            !has_backup_sibling(&dir.join(CACHE_PREFIX)),
            "ignore-cache under dry-run backs nothing up"
        );
        assert_eq!(&recs(dir), before, "cache left exactly as it was");
    }
    assert_eq!(rfile(&dst, "a.txt"), b"old");
}

#[test]
fn run_sync_keep_extra_and_missing_only() {
    let t = TempRoot::new("flags");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "common.txt", b"v2-changed-and-longer");
    wfile(&dst, "common.txt", b"v1");
    wfile(&src, "newfile.txt", b"brand new");
    wfile(&dst, "extra.txt", b"keep me");

    let mut o = sync(src.clone(), dst.clone());
    o.missing_only = true; // copy newfile, skip content update
    o.keep_extra = true; // leave extra.txt alone
    let code = cmd_sync(o, &log()).unwrap();
    assert_eq!(code, 0);
    assert_eq!(rfile(&dst, "newfile.txt"), b"brand new");
    assert_eq!(
        rfile(&dst, "common.txt"),
        b"v1",
        "missing-only skips updates"
    );
    assert_eq!(
        rfile(&dst, "extra.txt"),
        b"keep me",
        "keep-extra spares dst-only"
    );

    // Default flags converge fully.
    let code = cmd_sync(sync(src.clone(), dst.clone()), &log()).unwrap();
    assert_eq!(code, 0);
    assert_eq!(rfile(&dst, "common.txt"), b"v2-changed-and-longer");
    assert!(!dst.join("extra.txt").exists());
}

#[test]
fn run_sync_rejects_bad_inputs() {
    let t = TempRoot::new("reject");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"x");
    wfile(&dst, "a.txt", b"x");

    // src == dst
    assert!(cmd_sync(sync(src.clone(), src.clone()), &log()).is_err());

    // record inputs are compare-only
    cmd_update(update(src.clone()), &log()).unwrap();
    assert!(cmd_sync(sync(src.join(CACHE_PREFIX), dst.clone()), &log()).is_err());

    // jobs == 0 is a runtime error
    let mut o = sync(src.clone(), dst.clone());
    o.jobs = 0;
    assert!(cmd_sync(o, &log()).is_err());
}

#[test]
fn run_sync_resolves_type_conflicts() {
    let t = TempRoot::new("typeconf");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    // src file vs dst dir at the same relpath...
    wfile(&src, "node", b"i am a file");
    wfile(&dst, "node/inner.txt", b"i am a dir");
    // ...and src dir vs dst file.
    wfile(&src, "node2/f.txt", b"in src dir");
    wfile(&dst, "node2", b"i am a file");

    let code = cmd_sync(sync(src.clone(), dst.clone()), &log()).unwrap();
    assert_eq!(code, 0);
    assert_eq!(rfile(&dst, "node"), b"i am a file");
    assert!(dst.join("node2").is_dir(), "dst resolves toward src kind");
    assert_eq!(rfile(&dst, "node2/f.txt"), b"in src dir");

    let code = cmd_compare(compare(src.clone(), dst.clone()), &log()).unwrap();
    assert_eq!(code, 0);
}

/// A case-only difference must be settled by renaming dst, not by copying.
/// Regression: the dry run used to leave its in-memory dst map on the old
/// casing, so it planned both a COPY and a DELETE for the same file.
#[test]
fn run_sync_case_only_difference_renames_instead_of_copying() {
    let t = TempRoot::new("caseren");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "Data.txt", b"payload");
    wfile(&dst, "data.txt", b"payload");
    // Align mtime so casing is the *only* difference; otherwise the diff
    // legitimately reports CHANGED and a copy is the right answer.
    sync_mtime(&src.join("Data.txt"), &dst.join("data.txt"));

    // Dry run: prints the rename, changes nothing.
    let mut dry = sync(src.clone(), dst.clone());
    dry.common.case_sensitive = false;
    dry.dry_run = true;
    assert_eq!(cmd_sync(dry, &log()).unwrap(), 0);
    assert_eq!(
        entry_names(&dst),
        vec!["data.txt".to_string()],
        "a dry run renames nothing on disk"
    );

    // Real run: dst adopts src's casing.
    let mut real = sync(src.clone(), dst.clone());
    real.common.case_sensitive = false;
    assert_eq!(cmd_sync(real, &log()).unwrap(), 0);
    assert_eq!(
        entry_names(&dst),
        vec!["Data.txt".to_string()],
        "dst adopts src casing"
    );

    // Converged: no further differences in either mode.
    let mut same = compare(src.clone(), dst.clone());
    same.common.case_sensitive = false;
    assert_eq!(cmd_compare(same, &log()).unwrap(), 0);
    assert_eq!(cmd_compare(compare(src, dst), &log()).unwrap(), 0);
}
