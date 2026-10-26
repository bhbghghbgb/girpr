//! End-to-end runs of `cmd_sync` (real FS + real cache).

mod common;

use std::num::NonZeroUsize;

use girsync::rw::RwLimits;

use common::{
    compare, compare_mem, entry_names, has_backup_sibling, log, parse_ndjson, recs_of, rfile, sync,
    sync_dry, sync_mem, sync_mtime, temp_root, update, update_mem, wfile,
};
use girsync::cache::{CACHE_PREFIX, TestCacheBackend};
use girsync::{cmd_compare, cmd_sync, cmd_update};
use serde_json::Value;
use std::path::Path;

fn recs(dir: &Path) -> std::collections::HashMap<String, girsync::cache::FileRec> {
    recs_of(dir)
}

/// Run the real binary and return the records it reported on stdout.
///
/// `--output json` throughout, so what a test reads is what a machine reads. The
/// alternative — matching rendered text lines — makes every case a second,
/// quieter statement of the vocabulary, and a reworded message fails a test whose
/// point was the plan.
///
/// `envs` are added on top of the in-memory cache switch, for the test-only
/// `GIRSYNC_TEST_COMMIT_*` limits a fixture lowers to make a commit observable.
fn run_binary(src: &Path, dst: &Path, extra: &[&str], envs: &[(&str, &str)]) -> Vec<Value> {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .arg("sync")
        .arg("--src")
        .arg(src)
        .arg("--dst")
        .arg(dst)
        .arg("--output")
        .arg("json")
        .args(extra)
        .env("GIRSYNC_TEST_CACHE", "memory")
        .envs(envs.iter().copied())
        .output()
        .expect("spawn girsync");
    assert!(
        out.status.success(),
        "sync failed: {}\n{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    parse_ndjson(&out.stdout)
}

/// The `path` of every record with `event` `name`, in order.
fn paths<'a>(recs: &'a [Value], name: &str) -> Vec<&'a str> {
    recs.iter()
        .filter(|r| r["event"] == name)
        .map(|r| r["path"].as_str().expect("a path record has a path"))
        .collect()
}

#[test]
fn run_sync_dry_run_writes_nothing() {
    let t = temp_root("dryrun");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"new content here");
    wfile(&dst, "a.txt", b"old");
    wfile(&dst, "extra.txt", b"stay for now");

    // A dry run, at a limit other than 1, so the copy phase actually has more than
    // one permitted operation in it -- the case where a limiter bug could do real
    // work it was not supposed to.
    let mut o = sync_dry(src.clone(), dst.clone());
    o.common.rw = RwLimits::Shared(NonZeroUsize::new(2).unwrap());
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
    let code = cmd_compare(compare_mem(src.clone(), dst.clone()), &log()).unwrap();
    assert_eq!(code, 4);
}

/// A dry run writes nothing, which includes not *creating* the caches: a
/// read/write open would have created one per side and written its `meta`.
#[test]
fn run_sync_dry_run_creates_no_cache() {
    let t = temp_root("drynocache");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"new content here");
    wfile(&dst, "a.txt", b"old");

    let o = sync_dry(src.clone(), dst.clone());
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
    let t = temp_root("dryignore");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"new content here");
    wfile(&dst, "a.txt", b"old");
    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(dst.clone()), &log()).unwrap();
    let (src_before, dst_before) = (recs(&src), recs(&dst));

    let mut o = sync_dry(src.clone(), dst.clone());
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
    let t = temp_root("flags");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "common.txt", b"v2-changed-and-longer");
    wfile(&dst, "common.txt", b"v1");
    wfile(&src, "newfile.txt", b"brand new");
    wfile(&dst, "extra.txt", b"keep me");

    let mut o = sync(src.clone(), dst.clone());
    o.common.test_cache = TestCacheBackend::Memory;
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
    let code = cmd_sync(sync_mem(src.clone(), dst.clone()), &log()).unwrap();
    assert_eq!(code, 0);
    assert_eq!(rfile(&dst, "common.txt"), b"v2-changed-and-longer");
    assert!(!dst.join("extra.txt").exists());
}

/// `--keep-extra` spares directories as well as files.
///
/// The flag name is not a promise about files only, so a `--keep-extra` run must
/// also leave dst-only directories in place — `remove_unknown_dirs` is skipped
/// by the same flag that skips `DELETE`.
#[test]
fn run_sync_keep_extra_spares_extra_dirs_too() {
    let t = temp_root("keep_dirs");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "shared.txt", b"same");
    wfile(&dst, "shared.txt", b"same");
    wfile(&dst, "extra.txt", b"extra file");
    wfile(&dst, "extradir/top.bin", b"x");
    wfile(&dst, "extradir/nested/deep.bin", b"y");

    let mut o = sync(src.clone(), dst.clone());
    o.common.test_cache = TestCacheBackend::Memory;
    o.keep_extra = true;
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    assert!(dst.join("extra.txt").is_file(), "extra file kept");
    assert!(dst.join("extradir").is_dir(), "extra dir kept");
    assert!(
        dst.join("extradir/nested/deep.bin").is_file(),
        "and its contents, since nothing was emptied"
    );

    // Without the flag the same tree is fully collected, dirs included.
    let t2 = temp_root("no_keep_dirs");
    let src2 = t2.subdir("src");
    let dst2 = t2.subdir("dst");
    wfile(&src2, "shared.txt", b"same");
    wfile(&dst2, "shared.txt", b"same");
    wfile(&dst2, "extradir/nested/deep.bin", b"y");
    assert_eq!(cmd_sync(sync_mem(src2, dst2.clone()), &log()).unwrap(), 0);
    assert!(!dst2.join("extradir").exists(), "collected by default");
}

/// A planned copy that lands on a dst *directory* clears it out of the way
/// recursively, so that directory's whole subtree is gone before the rmdir pass
/// could look at it. The plan must not promise removals that cannot happen: a
/// dry run that counted `extradir` here would report an `rmdir` record the real
/// run can never reach.
///
/// The fixture is deliberately the pathological one — src has `clash` as a
/// *file*, dst has `clash/sub/deep/` as directories, so `clash/sub` and
/// `clash/sub/deep` are unknown dirs that the copy wipes.
#[test]
fn a_copy_that_wipes_a_blocking_dir_is_not_also_planned_for_removal() {
    let t = temp_root("wiped");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "clash", b"iamafile");
    std::fs::create_dir_all(dst.join("clash/sub/deep")).unwrap();

    let dry = run_binary(&src, &dst, &["--dry-run"], &[]);
    assert!(
        !paths(&dry, "rmdir").iter().any(|p| p.starts_with("clash")),
        "the subtree is wiped by the copy, not removed by the rmdir pass: {dry:?}"
    );
    assert_eq!(
        common::summary_of(&dry)["rmdir"],
        0,
        "so the dry run must report zero, which is what the real run reaches: {dry:?}"
    );

    // And the real run agrees.
    let real = run_binary(&src, &dst, &[], &[]);
    assert_eq!(common::summary_of(&real)["rmdir"], 0, "real run: {real:?}");
    assert!(dst.join("clash").is_file(), "and the copy happened");
}

#[test]
fn run_sync_rejects_bad_inputs() {
    let t = temp_root("reject");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"x");
    wfile(&dst, "a.txt", b"x");

    // src == dst
    let mut o = sync(src.clone(), src.clone());
    o.common.test_cache = TestCacheBackend::Memory;
    assert!(cmd_sync(o, &log()).is_err());

    // record inputs are compare-only
    cmd_update(update_mem(src.clone()), &log()).unwrap();
    let mut o = sync(src.join(CACHE_PREFIX), dst.clone());
    o.common.test_cache = TestCacheBackend::Memory;
    assert!(cmd_sync(o, &log()).is_err());

    // A zero thread count has no equivalent to assert at this layer: `SyncOpts`
    // carries no thread count, `RwLimits` cannot hold a zero (`NonZeroUsize`),
    // and the flag that could name one is refused before it reaches `cmd_sync`.
    // `rw_threads_rejects_a_zero_count` pins the user-visible half of that,
    // through the real binary.
}

#[test]
fn run_sync_resolves_type_conflicts() {
    let t = temp_root("typeconf");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    // src file vs dst dir at the same relpath...
    wfile(&src, "node", b"i am a file");
    wfile(&dst, "node/inner.txt", b"i am a dir");
    // ...and src dir vs dst file.
    wfile(&src, "node2/f.txt", b"in src dir");
    wfile(&dst, "node2", b"i am a file");

    let mut o = sync(src.clone(), dst.clone());
    o.common.test_cache = TestCacheBackend::Memory;
    let code = cmd_sync(o, &log()).unwrap();
    assert_eq!(code, 0);
    assert_eq!(rfile(&dst, "node"), b"i am a file");
    assert!(dst.join("node2").is_dir(), "dst resolves toward src kind");
    assert_eq!(rfile(&dst, "node2/f.txt"), b"in src dir");

    let code = cmd_compare(compare_mem(src.clone(), dst.clone()), &log()).unwrap();
    assert_eq!(code, 0);
}

/// A case-only difference must be settled by renaming dst, not by copying.
///
/// Both modes have to agree here: a dry run must not leave its in-memory dst map
/// on the old casing, or it would plan both a COPY and a DELETE for one file.
#[test]
fn run_sync_case_only_difference_renames_instead_of_copying() {
    let t = temp_root("caseren");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "Data.txt", b"payload");
    wfile(&dst, "data.txt", b"payload");
    // Align mtime so casing is the *only* difference; otherwise the diff
    // legitimately reports CHANGED and a copy is the right answer.
    sync_mtime(&src.join("Data.txt"), &dst.join("data.txt"));

    // Dry run: prints the rename, changes nothing.
    let mut dry = sync(src.clone(), dst.clone());
    dry.common.test_cache = TestCacheBackend::Memory;
    dry.common.case_sensitive = false;
    dry.common.dry_run = true;
    assert_eq!(cmd_sync(dry, &log()).unwrap(), 0);
    assert_eq!(
        entry_names(&dst),
        vec!["data.txt".to_string()],
        "a dry run renames nothing on disk"
    );

    // Real run: dst adopts src's casing.
    let mut real = sync(src.clone(), dst.clone());
    real.common.test_cache = TestCacheBackend::Memory;
    real.common.case_sensitive = false;
    assert_eq!(cmd_sync(real, &log()).unwrap(), 0);
    assert_eq!(
        entry_names(&dst),
        vec!["Data.txt".to_string()],
        "dst adopts src casing"
    );

    // Converged: no further differences in either mode.
    let mut same = compare_mem(src.clone(), dst.clone());
    same.common.case_sensitive = false;
    assert_eq!(cmd_compare(same, &log()).unwrap(), 0);
    assert_eq!(cmd_compare(compare_mem(src, dst), &log()).unwrap(), 0);
}

/// A case-only pair whose **content** also differs must be renamed *and* copied.
///
/// This is the one gate that catches a wrong planner case-flag, and it exists
/// because the fixture above cannot. That fixture's two sides hold the same
/// bytes, so a run that never learns a digest for the pair still concludes
/// "equal" and reaches the right answer by luck. Same for the case-only pair in
/// `sync_plan/`'s fixture table, whose row for this shape is `Case.txt`.
///
/// Here the bytes differ at the same length with the mtime pinned, so size+mtime
/// agree and only a digest can tell them apart. If `plan_pairs` is handed
/// `case_sensitive: true` it pairs by exact key *before* the rename pass
/// collapses the pair, so neither side gets a digest; the rename then makes the
/// keys match, `hashes_differ` stays silent on dst's absent digest and falls back
/// to size+mtime, and the run reports success having copied nothing — leaving dst
/// with the wrong bytes and the mirror quietly lying.
///
/// So this asserts the *outcome*, not the plan: a plan assertion would be
/// satisfied by a `RENAME` line that is printed either way.
#[test]
fn a_case_only_difference_in_content_is_copied_not_merely_renamed() {
    let t = temp_root("caseren_content");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    // Same length, different bytes. Different lengths would let size settle the
    // pair, and the test would pass without a digest ever being needed.
    wfile(&src, "Data.txt", b"payload");
    wfile(&dst, "data.txt", b"PAYLOAD");
    sync_mtime(&src.join("Data.txt"), &dst.join("data.txt"));

    // The dry run must plan the copy, so the failure is visible before it is
    // committed to disk.
    let mut dry = sync(src.clone(), dst.clone());
    dry.common.test_cache = TestCacheBackend::Memory;
    dry.common.case_sensitive = false;
    dry.common.dry_run = true;
    assert_eq!(cmd_sync(dry, &log()).unwrap(), 0);
    assert_eq!(
        rfile(&dst, "data.txt"),
        b"PAYLOAD",
        "a dry run copies nothing"
    );

    let mut real = sync(src.clone(), dst.clone());
    real.common.test_cache = TestCacheBackend::Memory;
    real.common.case_sensitive = false;
    assert_eq!(cmd_sync(real, &log()).unwrap(), 0);

    assert_eq!(
        entry_names(&dst),
        vec!["Data.txt".to_string()],
        "dst adopts src casing"
    );
    assert_eq!(
        rfile(&dst, "Data.txt"),
        b"payload",
        "and the bytes are src's, not the ones dst already had"
    );
    assert_eq!(
        cmd_compare(compare_mem(src, dst), &log()).unwrap(),
        0,
        "converged"
    );
}

/// **A copy into a directory this same run created must leave a cache row.**
///
/// `prune_dst_cache` drops dst cache rows for paths that are no longer on disk,
/// and it has to know what is on disk *after* this run's own writes. The files it
/// copies are not in the map phase A read, so a prune that answered "what did
/// dst hold before I started" would delete the rows for everything it just wrote.
///
/// The fixture is the smallest shape where that can go wrong: `mkdir` creates
/// `fresh/`, `copy_one` then creates `fresh/a.txt` and `fresh/b.txt` inside it.
/// Neither file existed when phase A read dst.
///
/// Asserted against **the cache**, not against a reported prune count: a wrong
/// derivation does not necessarily report a wrong count, it deletes live rows,
/// and the damage only surfaces later as phantom drift from a `compare-self`.
#[test]
fn files_copied_into_a_directory_this_run_created_keep_their_cache_rows() {
    let t = temp_root("prune_keeps_copied");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    // One file at the root, so the run has work beyond the new directory and the
    // prune is not trivially looking at an empty table.
    wfile(&src, "top.txt", b"top");
    wfile(&src, "fresh/a.txt", b"a");
    wfile(&src, "fresh/b.txt", b"b");

    assert_eq!(
        cmd_sync(sync_mem(src.clone(), dst.clone()), &log()).unwrap(),
        0
    );

    // On disk, everything this run wrote is there...
    assert!(dst.join("top.txt").is_file());
    assert!(dst.join("fresh/a.txt").is_file());
    assert!(dst.join("fresh/b.txt").is_file());
    // ...and so is every one of them in dst's cache, including the directory.
    let rows = recs(&dst);
    for rel in ["top.txt", "fresh", "fresh/a.txt", "fresh/b.txt"] {
        assert!(
            rows.contains_key(rel),
            "no cache row for {rel}: {:?}",
            rows.keys().collect::<Vec<_>>()
        );
    }
}

/// **A row for a path that is genuinely gone must be dropped.**
///
/// The other half of the same prune, and the half that makes it worth doing at
/// all: without it a deleted file's row would sit in the cache describing
/// content that no longer exists. The path is removed from disk *after* the run
/// that created it, so nothing this run writes can put it back.
///
/// Asserted against the cache, per the reasoning above.
#[test]
fn a_cache_row_for_a_path_deleted_after_the_run_is_dropped() {
    let t = temp_root("prune_drops_gone");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "kept.txt", b"kept");
    wfile(&src, "doomed.txt", b"doomed");

    assert_eq!(
        cmd_sync(sync_mem(src.clone(), dst.clone()), &log()).unwrap(),
        0
    );
    assert!(recs(&dst).contains_key("doomed.txt"), "the run recorded it");

    // A later run, after the file is gone from both sides.
    std::fs::remove_file(src.join("doomed.txt")).unwrap();
    std::fs::remove_file(dst.join("doomed.txt")).unwrap();
    assert_eq!(
        cmd_sync(sync_mem(src.clone(), dst.clone()), &log()).unwrap(),
        0
    );

    let rows = recs(&dst);
    assert!(
        !rows.contains_key("doomed.txt"),
        "the row for a deleted path must not survive: {:?}",
        rows.keys().collect::<Vec<_>>()
    );
    assert!(
        rows.contains_key("kept.txt"),
        "and the rows for live paths must not be collateral: {:?}",
        rows.keys().collect::<Vec<_>>()
    );
}

/// **A row for a path a copy wiped must be dropped, with the whole subtree.**
///
/// src has `x` as a file; dst has `x/` as a directory holding `x/sub/f.txt`.
/// The type conflict puts `x` on the copy list, and `copy_one` clears a
/// blocking dst directory *recursively* — so `x/sub` and `x/sub/f.txt` leave the
/// disk without anything scheduled to remove their rows.
///
/// `Plan::rmdir` deliberately skips that subtree (`plan.rs`): the directory is
/// gone by the time the removal pass runs, so listing it would promise an
/// `rmdir` the run cannot reach. That leaves `prune_dst_cache` as the only
/// thing that can drop those rows, which makes it the only place that can tell
/// "dst held this before the run" from "dst holds it now".
///
/// Asserted against the cache, like the two tests above: a wrong answer here is
/// invisible in the run's own output and shows up later as cache bloat.
#[test]
fn rows_for_a_subtree_a_copy_wiped_are_dropped() {
    let t = temp_root("prune_wiped_subtree");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    // src: x is a file.
    wfile(&src, "x", b"file contents");
    // dst: x is a directory, with a subtree under it. Two levels, so the
    // subtraction is pinned by prefix at depth rather than by one exact entry.
    wfile(
        &dst,
        "x/sub/f.txt",
        b"a much longer body under a blocking directory",
    );
    wfile(
        &dst,
        "x/sub/deeper/g.txt",
        b"and a second level under that too",
    );

    assert_eq!(
        cmd_sync(sync_mem(src.clone(), dst.clone()), &log()).unwrap(),
        0
    );

    // The wipe actually happened: x is now a file and the subtree is gone.
    assert!(
        dst.join("x").is_file(),
        "the blocking dir was replaced by a file"
    );
    assert!(
        !dst.join("x/sub").exists(),
        "and it went with its whole subtree"
    );

    let rows = recs(&dst);
    assert!(
        rows.contains_key("x"),
        "the copied file keeps its row: {:?}",
        rows.keys().collect::<Vec<_>>()
    );
    for stale in ["x/sub", "x/sub/f.txt", "x/sub/deeper", "x/sub/deeper/g.txt"] {
        assert!(
            !rows.contains_key(stale),
            "a row survived for {stale}, which no longer exists: {:?}",
            rows.keys().collect::<Vec<_>>()
        );
    }
}

/// **Trigger (b): a directory this run created and then removed in the same run.**
///
/// Reachable only with `--case-sensitive` on a case-insensitive filesystem: src
/// holds `Sub/`, dst holds `sub/`, and the two name the same directory. `mkdir`
/// creates `Sub` and `rmdir` then removes `sub` — which is `Sub` — taking the
/// freshly created tree with it. Nothing is left on disk, so nothing should be
/// left in the cache.
///
/// Recorded here as a **known limitation of the derived present set**, not as a
/// passing assertion. Deriving cannot see this without also case-folding the
/// prune against the plan — in the one mode that deliberately does not case-fold.
/// Asking the filesystem instead would drop these rows, at the cost of a full
/// traversal of dst in every real run; the rows are stale until the next run's
/// phase A prunes them, and `compare-self` builds a cold cache so it cannot be
/// misled.
#[test]
#[ignore = "known: the derived present set keeps rows for a dir created and rmdir'd in the same run"]
fn rows_for_a_directory_created_and_removed_in_one_run_are_dropped() {
    let t = temp_root("prune_create_then_rmdir");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "Sub/One.txt", b"one");
    wfile(&src, "Sub/Two.txt", b"two");
    // Only reachable on a case-insensitive filesystem; skip elsewhere rather
    // than assert something that cannot happen.
    if !std::fs::metadata(src.join("Sub"))
        .map(|_| src.join("sub").exists())
        .unwrap_or(false)
    {
        return;
    }

    let mut o = sync(src.clone(), dst.clone());
    o.common.case_sensitive = true;
    let _ = cmd_sync(o, &log());

    let rows = recs(&dst);
    for stale in ["Sub", "Sub/One.txt", "Sub/Two.txt"] {
        assert!(
            !rows.contains_key(stale),
            "a row survived for {stale}: {:?}",
            rows.keys().collect::<Vec<_>>()
        );
    }
}

/// **The copy phase commits its cache rows while it is still copying.**
///
/// `CacheWrite`'s `CommitLimits` are only a bound on the work an interrupted run
/// loses if they are checked as that work happens. `copy_files` once collected
/// every verified record and inserted them only after the pool joined, so during
/// the phase itself no threshold could fire: a run that died mid-copy lost every
/// row, however many files had already landed on disk.
///
/// The log file is the observable. A threshold tripping is logged as
/// `cache batch committed` (trace), and the phase logs `copy done` after the pool
/// joins, so the last batch commit must come *before* that line. Under the old
/// layout the only batch commit of the phase came after it, with every file
/// already durable.
///
/// The fixture is one-sided and small: `GIRSYNC_TEST_COMMIT_OPS` lowers the
/// op-count trigger, so `copy_files` crosses it in ten copies rather than in
/// `COMMIT_BATCH`. One-sided on purpose: with nothing to hash, the scan and the
/// pre-drop each auto-commit before the copy phase starts, and nothing after it
/// does — so the *last* commit in the log is the copy phase's.
#[test]
fn the_copy_phase_commits_its_cache_rows_while_it_copies() {
    let ops = 10;
    let files = 2 * ops;
    let t = temp_root("copy_commit");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    for i in 0..files {
        // Empty, so the fixture's cost is file metadata rather than content; the
        // op-count trigger counts rows, not bytes.
        wfile(&src, &format!("f{i:04}.bin"), b"");
    }

    let logfile = t.subdir("logs").join("run.json");
    let recs = run_binary(
        &src,
        &dst,
        &[
            "--log-file",
            logfile.to_str().expect("the temp path is utf-8"),
            // The file layer defaults to debug, and the commit event is trace.
            "--log-file-level",
            "trace",
            // Eight workers, so the fixture is not a queue of fsyncs.
            "--rw-threads",
            "8",
            // No digest to compute: this test is about when the row is written,
            // not about what it holds, and hashing would only add time.
            "--hash-all-of",
            "none",
        ],
        &[
            // A threshold this fixture crosses mid-copy.
            ("GIRSYNC_TEST_COMMIT_OPS", "10"),
            // Keep the interval trigger out of it, so the commit seen during
            // the copies can only be the op-count one.
            ("GIRSYNC_TEST_COMMIT_INTERVAL_MS", "60000"),
        ],
    );
    assert_eq!(
        paths(&recs, "copy").len(),
        files,
        "the fixture must make the copy phase cross the op-count threshold"
    );

    let logged = parse_ndjson(&std::fs::read(&logfile).expect("the run wrote its log"));
    fn message(e: &Value) -> &str {
        e["fields"]["message"].as_str().unwrap_or("")
    }
    let commits: Vec<usize> = logged
        .iter()
        .enumerate()
        .filter(|(_, e)| message(e) == "cache batch committed")
        .map(|(i, _)| i)
        .collect();
    let done = logged
        .iter()
        .position(|e| message(e) == "copy done")
        .expect("the copy phase logs its end");
    assert!(
        commits.last().is_some_and(|&i| i < done),
        "no batch commit before `copy done` (index {done}): commits were at {commits:?}, \
         so the copy phase still hoards its rows until every file is on disk"
    );
}

/// `--max-depth` (default 10) silently skips deeper files: `sync` never
/// mirrors them and exits 0, and a later `compare` reports the deep file as
/// MISSING. The depth cap is the walk's own bound (`src/scan.rs`), so this is
/// the end-to-end pin for the documented gotcha.
#[test]
fn run_sync_max_depth_skips_deeper_files() {
    let t = temp_root("max_depth");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    // Depth 11 under the root: beyond the default cap of 10 (`one` is depth 1).
    let deep = "one/two/three/four/five/six/seven/eight/nine/ten/eleven.txt";
    wfile(&src, deep, b"deep content");
    wfile(&src, "top.txt", b"top");

    let mut o = sync(src.clone(), dst.clone());
    o.common.test_cache = TestCacheBackend::Memory;
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0, "the run itself succeeds");
    assert!(!dst.join(deep).exists(), "the deep file was not mirrored");
    assert_eq!(
        rfile(&dst, "top.txt"),
        b"top",
        "a shallower file still was mirrored"
    );
}
