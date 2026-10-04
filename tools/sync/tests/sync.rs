//! End-to-end runs of `cmd_sync` (real FS + real cache).

mod common;

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

    let mut o = sync_dry(src.clone(), dst.clone());
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
/// The flag name is not a promise about files only, and README's step 4 says the
/// unknown-dir removal is skipped too. It was not: `remove_unknown_dirs` ran
/// unconditionally, so a `--keep-extra` run deleted exactly the empty trees the
/// user had asked it to leave. Worse, the deletion was invisible in the plan —
/// the dry run announced `RMDIR` for the very directories a real run then
/// removed, so the two agreed with each other and both contradicted the flag.
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

/// The dry run and a real run must report the same thing, field for field.
///
/// This is the observable half of the `--dry-run` contract, and it is only
/// checkable from the outside: `cmd_sync` returns an exit code, so the records it
/// reported *are* the result. Both runs go through the real binary so the
/// comparison covers the `rename` and `fix-dir` records, which the plan builder
/// does not produce.
///
/// `rmdir` is the field that had to change to make this possible. It used to be
/// absent from the dry run because it was computed by walking dst *during* the
/// apply phase — too late to report. It is now decided by `build_plan`, so the
/// same count is available before anything is written.
#[test]
fn run_sync_dry_run_summary_matches_a_real_run() {
    // The `TempRoot` is returned alongside the paths and bound to `_keep`: dropping
    // it deletes the tree, so a fixture built inside a closure would be gone
    // before the subprocess ever ran.
    //
    // `sync_mtime` is load-bearing here, not tidiness. The two trees are built at
    // different moments, and two files written microseconds apart can land on the
    // same filesystem timestamp in one tree and different ones in the other — which
    // makes a case-only pair read as `CHANGED` in one run and equal in the other,
    // for reasons that have nothing to do with the code under test. Pinning the
    // stamps makes the two trees stat-identical, which is the precondition for
    // comparing anything they report.
    let build = |tag: &str| -> (TempRoot, std::path::PathBuf, std::path::PathBuf) {
        let t = TempRoot::new(tag);
        let src = t.mkdirs("src");
        let dst = t.mkdirs("dst");
        // One of every record there is.
        wfile(&src, "same.txt", b"identical");
        wfile(&dst, "same.txt", b"identical");
        sync_mtime(&src.join("same.txt"), &dst.join("same.txt"));
        wfile(&src, "differs.txt", b"src-content-that-is-longer");
        wfile(&dst, "differs.txt", b"dst");
        wfile(&src, "Data.txt", b"payload");
        wfile(&dst, "data.txt", b"payload");
        sync_mtime(&src.join("Data.txt"), &dst.join("data.txt"));
        wfile(&src, "onlysrc.txt", b"s");
        wfile(&dst, "onlydst.txt", b"d");
        wfile(&dst, "extradir/nested/deep.bin", b"x");
        // A src-only directory: the apply phase has to report its mkdir, and the
        // dry run already did. Without this the parity is untested, because a
        // fixture that plans no mkdir cannot tell the two modes apart here.
        wfile(&src, "newdir/inner.txt", b"n");
        // A file on dst standing where src has a directory: the `fix-dir` case,
        // likewise the only thing that exercises that record.
        wfile(&src, "fixdir/inner.txt", b"f");
        wfile(&dst, "fixdir", b"a blocking file");
        (t, src, dst)
    };

    let (_keep_a, dry_src, dry_dst) = build("sum_dry");
    let dry = run_binary(&dry_src, &dry_dst, &["--dry-run"]);
    let (_keep_b, real_src, real_dst) = build("sum_real");
    let real = run_binary(&real_src, &real_dst, &[]);

    // The whole document, with only the `dry_run` marker normalised away. There
    // is no per-record fudging left: the two modes report the same events, the
    // same counts and the same order, so any future divergence in vocabulary,
    // sequencing or numbers fails right here instead of reaching a caller.
    let normalised = |recs: &[Value]| -> Vec<Value> {
        recs.iter()
            .map(|r| {
                let mut r = r.clone();
                if let Some(o) = r.as_object_mut() {
                    o.remove("dry_run");
                }
                r
            })
            .collect()
    };
    let show = |recs: &[Value]| {
        recs.iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        normalised(&dry),
        normalised(&real),
        "a dry run reports what a real run reports\n--- dry ---\n{}\n--- real ---\n{}",
        show(&dry),
        show(&real)
    );
    let dry_summary = common::summary_of(&dry);
    assert_eq!(dry_summary["dry_run"], true, "the dry run marks itself");
    // The three labels this stage had to unify. Asserted explicitly so that
    // dropping either change fails with a pointed message instead of passing
    // because the fixture stopped exercising it.
    for (event, path) in [
        ("mkdir", "newdir"),
        ("fix-dir", "fixdir"),
        ("rmdir", "extradir"),
    ] {
        for recs in [&dry, &real] {
            assert!(
                recs.iter()
                    .any(|r| r["event"] == event && r["path"] == path),
                "`{event}` on {path} must appear in both modes, got {}",
                show(recs)
            );
        }
    }
    // The old spelling was `RMDIR-FILE`, which reads as "remove a directory that
    // is a file" — backwards. The record for that case is `fix-dir`, asserted
    // above; this guards the old name from creeping back in as a *new* event.
    assert!(
        !dry.iter().any(|r| r["event"].as_str() == Some("rmdir-file")),
        "the old spelling is gone: {}",
        show(&dry)
    );
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
    assert_eq!(
        common::summary_of(&real)["rmdir"],
        0,
        "real run: {real:?}"
    );
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
/// This is the one gate in this stage that catches a wrong planner case-flag, and
/// it exists because the fixture above cannot. That fixture's two sides hold the
/// same bytes, so a run that never learns a digest for the pair still concludes
/// "equal" and reaches the right answer by luck. Same for the case-only pair in
/// `run_sync_dry_run_summary_matches_a_real_run` and in the golden.
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
