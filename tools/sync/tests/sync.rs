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

/// A case-only pair whose *content also differs* must still be copied.
///
/// This is the test that pins the asymmetry between `plan_pairs` and `diff_maps`
/// in `sync`. `plan_pairs` gets `common.case_sensitive` — false here — so it
/// pairs `Data.txt` with `data.txt` and hashes them, because equal size and mtime
/// make the pair genuinely undecided. Then the rename pass collapses them onto
/// exact keys and `diff_maps` runs with a hardcoded `true`.
///
/// If either half were flipped the plan would go wrong in opposite directions:
/// `plan_pairs(.., true)` would call this pair one-sided and skip both digests,
/// and `diff_maps(.., false)` would report a `CASE-MISMATCH` instead of a
/// `CHANGED` — which the applier does not act on.
#[test]
fn run_sync_case_only_difference_with_other_content_is_copied() {
    let t = TempRoot::new("casecopy");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "Data.txt", b"src-content-longer");
    wfile(&dst, "data.txt", b"dst");
    // Equal size, equal mtime: the pair is stat-equal, so only a digest can tell
    // them apart. That makes the rename alone insufficient.
    sync_mtime(&src.join("Data.txt"), &dst.join("data.txt"));

    let mut o = sync(src.clone(), dst.clone());
    o.common.case_sensitive = false;
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    assert_eq!(
        entry_names(&dst),
        vec!["Data.txt".to_string()],
        "dst took src's casing"
    );
    assert_eq!(
        rfile(&dst, "Data.txt"),
        b"src-content-longer".to_vec(),
        "and src's content, so the pair was compared by digest and copied"
    );
    assert_eq!(cmd_compare(compare(src, dst), &log()).unwrap(), 0);
}

/// Phase A's alternate-case adoption and the rename pass both move the same cache
/// row, in sequence, in one run.
///
/// The row is keyed `DATA.txt` in the cache, the file on disk is `data.txt`, and
/// src says `Data.txt`. Phase A finds the exact-key miss and adopts the single
/// stale alternate onto the disk name; the rename pass then moves the disk name
/// onto src's name. Two moves, one row.
///
/// A row that survives both with its digest intact is the proof that neither move
/// lost the digests — which is the failure mode that would be invisible, because
/// a lost digest degrades to size+mtime rather than erroring. `os::rename`
/// preserves mtime, so the stat still matches at the end.
#[test]
fn alt_adoption_and_rename_both_move_the_same_row() {
    let t = TempRoot::new("altmove");
    let dst = t.mkdirs("dst");
    let src = t.mkdirs("src");

    // Warm dst's cache under an all-caps key.
    wfile(&dst, "DATA.txt", b"payload");
    cmd_update(update(dst.clone()), &log()).unwrap();
    assert_eq!(
        digests(&dst, "DATA.txt"),
        ["md5"],
        "the row starts keyed DATA.txt, with a digest"
    );

    // Rename on disk only. mtime is preserved by the rename, so the stat still
    // matches the cached row and the digests are reusable.
    std::fs::rename(dst.join("DATA.txt"), dst.join("data.txt")).unwrap();

    // src supplies the third spelling.
    wfile(&src, "Data.txt", b"payload");
    sync_mtime(&src.join("Data.txt"), &dst.join("data.txt"));

    let mut o = sync(src, dst.clone());
    o.common.case_sensitive = false;
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    let rows = recs(&dst);
    let mut keys: Vec<&String> = rows.keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        vec!["Data.txt"],
        "exactly one row survives, keyed to src's casing"
    );
    assert_eq!(
        digests(&dst, "Data.txt"),
        ["md5"],
        "and it kept its digest across both moves"
    );
    assert_eq!(
        rfile(&dst, "Data.txt"),
        b"payload".to_vec(),
        "equal content, so no copy was needed"
    );
}

/// A stat-differing pair is never read, so `sync` leaves its dst row exactly as
/// it found it. Under `--missing-only` the pair is also not copied, so nothing
/// replaces the row either — the cache is simply not touched.
///
/// This is better than it first looks. A row keeps its digests because
/// `merge_row` only ever drops them when the *stat* changed, and here the stat
/// did not: the file was neither read nor written. So the digest left in the row
/// is still correct, and the next run can use it.
///
/// The state that actually needs care is a *changed* stat, where the digest must
/// go — a pre-change digest would be judged against a post-change file. That is
/// `merge_row`'s carry rule, asserted in `helpers.rs` and in
/// `a_stat_differing_pair_is_decided_by_stat_and_its_stale_digests_are_dropped`.
#[test]
fn missing_only_leaves_a_stat_differing_pair_untouched() {
    let t = TempRoot::new("misonly");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    // Same size, different bytes: only mtime differs, so nothing here needs a
    // digest to reach a verdict.
    wfile(&src, "f.txt", b"aaaa");
    wfile(&dst, "f.txt", b"bbbb");
    sync_mtime(&src.join("f.txt"), &dst.join("f.txt"));
    src_mtime_older(&src.join("f.txt"));
    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(dst.clone()), &log()).unwrap();
    let dst_before = recs(&dst);

    let mut o = sync(src.clone(), dst.clone());
    o.common.case_sensitive = false;
    o.missing_only = true;
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    assert_eq!(
        rfile(&dst, "f.txt"),
        b"bbbb".to_vec(),
        "missing-only left the existing file alone"
    );
    assert_eq!(
        recs(&dst)["f.txt"].hashes,
        dst_before["f.txt"].hashes,
        "the dst row was neither read nor rewritten, so its digest is untouched \
         and still valid"
    );
    assert_eq!(
        digests(&dst, "f.txt"),
        ["md5"],
        "and the digest it kept is the one update wrote"
    );
    assert_eq!(
        cmd_compare(compare(src, dst), &log()).unwrap(),
        4,
        "the pair is still reported as different"
    );
}

/// The algorithms a side's cache holds for one path.
fn digests(dir: &Path, rel: &str) -> Vec<String> {
    let mut v: Vec<String> = recs(dir)[rel].hashes.keys().cloned().collect();
    v.sort();
    v
}

fn src_mtime_older(p: &Path) {
    let m = std::fs::metadata(p).unwrap().modified().unwrap();
    let older = m - std::time::Duration::from_secs(60);
    std::fs::File::options()
        .write(true)
        .open(p)
        .unwrap()
        .set_modified(older)
        .unwrap();
}
