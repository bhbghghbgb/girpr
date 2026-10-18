//! End-to-end runs of `cmd_sync` (real FS + real cache).

mod common;

use std::num::NonZeroUsize;

use girsync::rw::RwLimits;

use common::{
    TempRoot, compare, entry_names, has_backup_sibling, log, parse_ndjson, rfile, rw, sync,
    sync_dry, sync_mtime, update, wfile,
};
use girsync::cache::{CACHE_PREFIX, FileRec, load_all_records, open_db};
use girsync::{cmd_compare, cmd_sync, cmd_update};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

fn recs(dir: &Path) -> HashMap<String, FileRec> {
    load_all_records(&open_db(&dir.join(CACHE_PREFIX), true, rw()).unwrap()).unwrap()
}

/// Run the real binary and return the records it reported on stdout.
///
/// `--output json` throughout, so what a test reads is what a machine reads. The
/// alternative — matching rendered text lines — makes every case a second,
/// quieter statement of the vocabulary, and a reworded message fails a test whose
/// point was the plan.
fn run_binary(src: &Path, dst: &Path, extra: &[&str]) -> Vec<Value> {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .arg("sync")
        .arg("--src")
        .arg(src)
        .arg("--dst")
        .arg(dst)
        .arg("--output")
        .arg("json")
        .args(extra)
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
    let t = TempRoot::new("dryrun");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
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
    let t = TempRoot::new("dryignore");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
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

/// `--keep-extra` spares directories as well as files.
///
/// The flag name is not a promise about files only, so a `--keep-extra` run must
/// also leave dst-only directories in place — `remove_unknown_dirs` is skipped
/// by the same flag that skips `DELETE`.
#[test]
fn run_sync_keep_extra_spares_extra_dirs_too() {
    let t = TempRoot::new("keep_dirs");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "shared.txt", b"same");
    wfile(&dst, "shared.txt", b"same");
    wfile(&dst, "extra.txt", b"extra file");
    wfile(&dst, "extradir/top.bin", b"x");
    wfile(&dst, "extradir/nested/deep.bin", b"y");

    let mut o = sync(src.clone(), dst.clone());
    o.keep_extra = true;
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    assert!(dst.join("extra.txt").is_file(), "extra file kept");
    assert!(dst.join("extradir").is_dir(), "extra dir kept");
    assert!(
        dst.join("extradir/nested/deep.bin").is_file(),
        "and its contents, since nothing was emptied"
    );

    // Without the flag the same tree is fully collected, dirs included.
    let t2 = TempRoot::new("no_keep_dirs");
    let src2 = t2.mkdirs("src");
    let dst2 = t2.mkdirs("dst");
    wfile(&src2, "shared.txt", b"same");
    wfile(&dst2, "shared.txt", b"same");
    wfile(&dst2, "extradir/nested/deep.bin", b"y");
    assert_eq!(cmd_sync(sync(src2, dst2.clone()), &log()).unwrap(), 0);
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
    let t = TempRoot::new("wiped");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "clash", b"iamafile");
    std::fs::create_dir_all(dst.join("clash/sub/deep")).unwrap();

    let dry = run_binary(&src, &dst, &["--dry-run"]);
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
    let real = run_binary(&src, &dst, &[]);
    assert_eq!(common::summary_of(&real)["rmdir"], 0, "real run: {real:?}");
    assert!(dst.join("clash").is_file(), "and the copy happened");
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

    // A zero thread count has no equivalent to assert at this layer: `SyncOpts`
    // carries no thread count, `RwLimits` cannot hold a zero (`NonZeroUsize`),
    // and the flag that could name one is refused before it reaches `cmd_sync`.
    // `rw_threads_rejects_a_zero_count` pins the user-visible half of that,
    // through the real binary.
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
///
/// Both modes have to agree here: a dry run must not leave its in-memory dst map
/// on the old casing, or it would plan both a COPY and a DELETE for one file.
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
    dry.common.dry_run = true;
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
    let t = TempRoot::new("caseren_content");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    // Same length, different bytes. Different lengths would let size settle the
    // pair, and the test would pass without a digest ever being needed.
    wfile(&src, "Data.txt", b"payload");
    wfile(&dst, "data.txt", b"PAYLOAD");
    sync_mtime(&src.join("Data.txt"), &dst.join("data.txt"));

    // The dry run must plan the copy, so the failure is visible before it is
    // committed to disk.
    let mut dry = sync(src.clone(), dst.clone());
    dry.common.case_sensitive = false;
    dry.common.dry_run = true;
    assert_eq!(cmd_sync(dry, &log()).unwrap(), 0);
    assert_eq!(
        rfile(&dst, "data.txt"),
        b"PAYLOAD",
        "a dry run copies nothing"
    );

    let mut real = sync(src.clone(), dst.clone());
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
        cmd_compare(compare(src, dst), &log()).unwrap(),
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
    let t = TempRoot::new("prune_keeps_copied");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    // One file at the root, so the run has work beyond the new directory and the
    // prune is not trivially looking at an empty table.
    wfile(&src, "top.txt", b"top");
    wfile(&src, "fresh/a.txt", b"a");
    wfile(&src, "fresh/b.txt", b"b");

    assert_eq!(cmd_sync(sync(src.clone(), dst.clone()), &log()).unwrap(), 0);

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
    let t = TempRoot::new("prune_drops_gone");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "kept.txt", b"kept");
    wfile(&src, "doomed.txt", b"doomed");

    assert_eq!(cmd_sync(sync(src.clone(), dst.clone()), &log()).unwrap(), 0);
    assert!(recs(&dst).contains_key("doomed.txt"), "the run recorded it");

    // A later run, after the file is gone from both sides.
    std::fs::remove_file(src.join("doomed.txt")).unwrap();
    std::fs::remove_file(dst.join("doomed.txt")).unwrap();
    assert_eq!(cmd_sync(sync(src.clone(), dst.clone()), &log()).unwrap(), 0);

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
    let t = TempRoot::new("prune_wiped_subtree");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
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

    assert_eq!(cmd_sync(sync(src.clone(), dst.clone()), &log()).unwrap(), 0);

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
    let t = TempRoot::new("prune_create_then_rmdir");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
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
