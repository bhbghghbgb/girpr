//! End-to-end runs of `cmd_update` covering pruning, filtering, and the two
//! properties that make it `update` and not `compare`.

mod common;

use common::{
    TempRoot, compare, compare_self_opts, log, poison_digest, recs_of, rw, sync, sync_mtime,
    update, wfile,
};
use girsync::cache::{CACHE_PREFIX, load_all_records, open_db};
use girsync::filter::compile_patterns;
use girsync::{CommonOpts, cmd_compare, cmd_compare_self, cmd_sync, cmd_update};

#[test]
fn run_update_prunes_and_excludes() {
    let t = TempRoot::new("prune");
    let dir = t.mkdirs("w");
    wfile(&dir, "keep.txt", b"keep");
    wfile(&dir, "gone.txt", b"to be deleted");
    wfile(&dir, "skip.me", b"excluded content v1");

    cmd_update(update(dir.clone()), &log()).unwrap();
    {
        let db = open_db(&dir.join(CACHE_PREFIX), true, rw()).unwrap();
        let recs = load_all_records(&db).unwrap();
        assert!(recs.contains_key("gone.txt"));
        assert!(recs.contains_key("skip.me"));
    }

    // Deleting a file + re-update prunes it from the record.
    std::fs::remove_file(dir.join("gone.txt")).unwrap();
    cmd_update(update(dir.clone()), &log()).unwrap();
    {
        let db = open_db(&dir.join(CACHE_PREFIX), true, rw()).unwrap();
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
        let db = open_db(&dir.join(CACHE_PREFIX), true, rw()).unwrap();
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

/// **`update` recomputes every digest on every run, and this is the only test that
/// would notice if it stopped.**
///
/// Everything else about `update` is idempotent, so idempotence cannot catch this:
/// a run that reused a cached digest produces the same rows either way, and so
/// does a run that recomputed them. `lock_cache_writes_merge_unless_stat_changed`
/// cannot either — it asserts *which* algorithms end up on the row, and a reused
/// digest and a recomputed one are the same bytes. Only a digest that is *wrong*
/// distinguishes them.
///
/// The poison is the minimal such digest: `poison_digest` keeps the row's size and
/// mtime, so the row is `fresh` and every reuse path would take it. This is the
/// shape that matters, because it is also what makes a record unable to answer —
/// see the sibling test below, which is the same insight from the other side.
#[test]
fn update_recomputes_every_digest_and_repairs_a_wrong_one() {
    let t = TempRoot::new("upd_rehash");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"stable content");

    cmd_update(update(dir.clone()), &log()).unwrap();
    let good = recs_of(&dir)["a.txt"].hashes["md5"].clone();

    // Same stat, wrong content: only a rehash can settle it.
    poison_digest(&dir, "a.txt", "md5");
    let poisoned = recs_of(&dir);
    assert_eq!(
        poisoned["a.txt"].hashes["md5"],
        vec![0xABu8; 16],
        "the fixture is a wrong digest, not a missing one"
    );
    assert_eq!(
        poisoned["a.txt"].size, 14,
        "and the row is still fresh, so nothing but a rehash can fix it"
    );

    cmd_update(update(dir.clone()), &log()).unwrap();
    assert_eq!(
        recs_of(&dir)["a.txt"].hashes["md5"],
        good,
        "update trusts nothing, so the poison is recomputed away"
    );
}

/// **The exemption question, answered end to end rather than argued.
///
/// A side that cannot supply a requested digest is fatal. `update` is not exempt from
/// that rule: it is a folder asked to compute everything it has, so it can always
/// comply, and the same fixture that makes `compare-self` exit `3` is one `update`
/// away from being answerable.
///
/// The fixture is built by a real `sync`, so the stat-only row is one a lazy scan
/// genuinely writes rather than one a test forged: `sync`'s phase A records a
/// stat-only row for a path the cache has never seen, because the pair was decided
/// by presence and needed no digest. That row is *fresh*, so nothing fills it in on
/// its own. `compare_self.rs`'s
/// `a_lazy_sync_can_leave_a_record_that_cannot_answer_and_that_is_now_an_error` is
/// the same fixture asserted from the failing side; the pair of tests is the point,
/// because the only difference between them is which side of the run can hash.
#[test]
fn update_fills_the_gap_that_makes_an_audit_of_the_same_folder_fail() {
    let t = TempRoot::new("upd_fills_gap");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    for i in 0..2 {
        wfile(&src, &format!("f{i}.txt"), format!("body {i}").as_bytes());
        wfile(&dst, &format!("f{i}.txt"), format!("body {i}").as_bytes());
    }
    sync_mtime(&src.join("f0.txt"), &dst.join("f0.txt"));
    sync_mtime(&src.join("f1.txt"), &dst.join("f1.txt"));
    cmd_update(update(src.clone()), &log()).unwrap();
    // Arrives after the update, so src's cache has no row for it at all.
    wfile(&src, "late.txt", b"late arrival");
    cmd_sync(sync(src.clone(), dst), &log()).unwrap();
    assert!(
        recs_of(&src)["late.txt"].hashes.is_empty(),
        "the lazy sync recorded the newcomer's stat and no digest"
    );

    // A record cannot go and read it, so the audit refuses rather than compare by
    // stat alone.
    let err = cmd_compare_self(compare_self_opts(src.clone()), &log())
        .expect_err("a record has no filesystem to read the missing digest from");
    assert!(
        format!("{err:#}").contains("late.txt"),
        "and names the path it cannot answer for: {err:#}"
    );

    // `update` can, so the same folder is one command from answerable.
    cmd_update(update(src.clone()), &log()).unwrap();
    assert!(
        !recs_of(&src)["late.txt"].hashes.is_empty(),
        "update computed the digest the record could not"
    );
    assert_eq!(
        cmd_compare_self(compare_self_opts(src), &log()).unwrap(),
        0,
        "and the audit now answers, which is what makes the failure actionable"
    );
}
