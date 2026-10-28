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

/// Run the real binary and return its records, without asserting success.
///
/// `run_binary` above is the happy-path form; a case whose point is the exit
/// code needs the raw output, because the assertion is on stderr and on what was
/// *not* written.
fn run_binary_raw(
    src: &Path,
    dst: &Path,
    extra: &[&str],
    envs: &[(&str, &str)],
) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .arg("sync")
        .arg("--src")
        .arg(src)
        .arg("--dst")
        .arg(dst)
        .arg("--output")
        .arg("json")
        .args(extra)
        .envs(envs.iter().copied())
        .output()
        .expect("spawn girsync")
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

/// `FIX-DIR` must run before `MKDIR`: a src directory nested under a path where
/// dst holds a blocking file cannot be created until the file is replaced. The
/// dry run used to print this valid plan while the real run aborted on
/// `AlreadyExists` (os error 183), so the two modes disagreed about a plan that
/// `fix_dirs` could satisfy.
#[test]
fn a_type_conflict_nested_directory_is_planned_and_applied_in_order() {
    let t = temp_root("nestedfix");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a/b/x.txt", b"x");
    wfile(&dst, "a", b"blocking file");

    let dry = run_binary(&src, &dst, &["--dry-run"], &[]);
    let order: Vec<&str> = dry
        .iter()
        .filter_map(|r| r["event"].as_str())
        .filter(|e| matches!(*e, "fix-dir" | "mkdir"))
        .collect();
    assert_eq!(
        order,
        ["fix-dir", "mkdir"],
        "the dry run reports the phases in apply order: {dry:?}"
    );
    assert_eq!(paths(&dry, "fix-dir"), ["a"], "{dry:?}");
    assert_eq!(paths(&dry, "mkdir"), ["a/b"], "{dry:?}");
    assert!(
        dst.join("a").is_file(),
        "the dry run changes nothing: {dry:?}"
    );

    let real = run_binary(&src, &dst, &[], &[]);
    assert!(
        dst.join("a").is_dir(),
        "the blocking file became a directory: {real:?}"
    );
    assert_eq!(rfile(&dst, "a/b/x.txt"), b"x");
    assert_eq!(common::summary_of(&real)["mkdir"], 1, "{real:?}");
    assert_eq!(cmd_compare(compare_mem(src, dst), &log()).unwrap(), 0);
}

/// **P6-1.** A dst file hidden by `--include` still blocks the `MKDIR` the plan
/// derives from src's retained ancestor directory. Both modes must refuse it
/// before the first filesystem change — including the case-only rename.
#[test]
fn a_filtered_dst_file_blocking_a_mkdir_is_refused_before_the_rename() {
    let t = temp_root("filtered_blocker");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "foo/bar.txt", b"new");
    wfile(&dst, "foo", b"blocker");
    // A case-only pair the insensitive rename pass would move first if the
    // refusal were still taken after it. Same bytes and mtime, so casing is the
    // only difference.
    wfile(&src, "Other.txt", b"same");
    wfile(&dst, "other.txt", b"same");
    sync_mtime(&src.join("Other.txt"), &dst.join("other.txt"));

    let envs = [("GIRSYNC_TEST_CACHE", "memory")];
    for extra in [
        &["--include", "*.txt", "--dry-run"][..],
        &["--include", "*.txt"][..],
    ] {
        let out = run_binary_raw(&src, &dst, extra, &envs);
        assert_eq!(
            out.status.code(),
            Some(3),
            "a mkdir blocked by a filtered dst file must be a plan-time refusal: {extra:?}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("`foo`"), "names the blocker: {err}");
        assert!(
            err.contains("cannot create `foo`"),
            "names the mkdir: {err}"
        );
        assert!(err.contains("as a file"), "says what is in the way: {err}");
        let recs = parse_ndjson(&out.stdout);
        for action in ["mkdir", "copy", "rename", "fix-dir"] {
            assert!(
                !recs.iter().any(|r| r["event"] == action),
                "no `{action}` runs before the refusal: {recs:?}"
            );
        }
    }
    assert_eq!(rfile(&dst, "foo"), b"blocker", "the blocker is untouched");
    assert_eq!(
        entry_names(&dst),
        vec!["foo".to_string(), "other.txt".to_string()],
        "the refusal precedes the case-only rename"
    );
}

/// **P6-2.** `dst/a.txt` is a hard link to `src/b.txt`, so truncating it writes
/// through to a source file. Both hash modes must refuse before `File::create`,
/// leaving the shared bytes and the link in place.
#[test]
fn a_dst_hard_link_is_refused_without_touching_the_shared_inode() {
    let t = temp_root("hardlink_guard");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"AAAA");
    wfile(&src, "b.txt", b"BBBBBBBBBB");
    std::fs::hard_link(src.join("b.txt"), dst.join("a.txt"))
        .expect("hard links need no privilege on this host");

    let envs = [("GIRSYNC_TEST_CACHE", "memory")];
    for extra in [
        &["--exclude", "b.txt"][..],
        &["--exclude", "b.txt", "--hash-all-of", "none"][..],
    ] {
        let out = run_binary_raw(&src, &dst, extra, &envs);
        assert_eq!(
            out.status.code(),
            Some(3),
            "a shared destination inode must be refused: {extra:?}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("hard links"), "names the hazard: {err}");
        assert!(err.contains("a.txt"), "names the destination: {err}");
        assert_eq!(
            rfile(&src, "b.txt"),
            b"BBBBBBBBBB",
            "the source the hard link shares must be unchanged"
        );
        assert_eq!(
            rfile(&dst, "a.txt"),
            b"BBBBBBBBBB",
            "the destination must not be truncated or replaced"
        );
    }
    assert_eq!(
        rfile(&src, "a.txt"),
        b"AAAA",
        "the copied source is untouched"
    );
}

/// **P7-4.** The hard-link refusal is a plan-time decision both modes take.
///
/// `copy_one` runs only in a real run, so a dry run used to report `COPY` and
/// exit 0 for this tree while the real run exited 3. With `--keep-extra` the
/// second name survives, so both modes refuse before any action record.
/// Without it, the delete phase removes the second name first and both modes
/// succeed — the preflight credits the planned delete instead of refusing every
/// count above one.
#[test]
fn the_hard_link_refusal_is_the_same_decision_in_both_modes() {
    let t = temp_root("hardlink_parity");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"NEW");
    wfile(&dst, "a.txt", b"OLD");
    std::fs::hard_link(dst.join("a.txt"), dst.join("shared.txt"))
        .expect("hard links need no privilege on this host");

    let envs = [("GIRSYNC_TEST_CACHE", "memory")];
    for extra in [&["--keep-extra", "--dry-run"][..], &["--keep-extra"][..]] {
        let out = run_binary_raw(&src, &dst, extra, &envs);
        assert_eq!(
            out.status.code(),
            Some(3),
            "the kept second name makes the copy unsafe: {extra:?}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("hard links"), "names the hazard: {err}");
        assert!(err.contains("a.txt"), "names the copy: {err}");
        let recs = parse_ndjson(&out.stdout);
        assert!(
            !recs.iter().any(|r| r["event"] == "copy"),
            "no COPY is reported for a copy that cannot run: {recs:?}"
        );
        assert_eq!(
            rfile(&dst, "a.txt"),
            b"OLD",
            "the shared inode is untouched"
        );
        assert_eq!(rfile(&dst, "shared.txt"), b"OLD");
    }

    // The default plan deletes the extra name before the copy, which reduces the
    // count to one. The dry run must reach that decision too.
    let dry = run_binary(&src, &dst, &["--dry-run"], &envs);
    assert_eq!(paths(&dry, "delete"), ["shared.txt"], "{dry:?}");
    assert_eq!(paths(&dry, "copy"), ["a.txt"], "{dry:?}");
    assert_eq!(rfile(&dst, "a.txt"), b"OLD", "the dry run changed nothing");

    let real = run_binary(&src, &dst, &[], &envs);
    assert_eq!(paths(&real, "delete"), ["shared.txt"], "{real:?}");
    assert_eq!(paths(&real, "copy"), ["a.txt"], "{real:?}");
    assert_eq!(rfile(&dst, "a.txt"), b"NEW");
    assert!(!dst.join("shared.txt").exists());
    assert_eq!(cmd_compare(compare_mem(src, dst), &log()).unwrap(), 0);
}

/// **P8-3.** A case-only dst hard link is judged at plan time in both modes.
///
/// The rename pass re-keys `dm` in a dry run but moves nothing on disk, so the
/// hard-link preflight used to stat the src-cased name the rename would create
/// and find nothing: the dry run planned a `COPY` and exited 0, while the real
/// run renamed first and then refused with exit 3. Both modes now stat the
/// spelling still on disk. `--keep-extra` makes the second name survive, which
/// is what leaves the inode with two names at copy time.
#[test]
fn a_case_only_hard_link_is_the_same_decision_in_both_modes() {
    let t = temp_root("hardlink_case_parity");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    skip_if_case_insensitive!(&dst);
    wfile(&src, "a.txt", b"NEW");
    wfile(&dst, "A.txt", b"OLD");
    std::fs::hard_link(dst.join("A.txt"), dst.join("shared.txt"))
        .expect("hard links need no privilege on this host");

    let envs = [("GIRSYNC_TEST_CACHE", "memory")];
    for extra in [&["--keep-extra", "--dry-run"][..], &["--keep-extra"][..]] {
        let out = run_binary_raw(&src, &dst, extra, &envs);
        assert_eq!(
            out.status.code(),
            Some(3),
            "a case-only hard link must be refused in both modes: {extra:?}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("hard links"), "names the hazard: {err}");
        assert!(err.contains("a.txt"), "names the copy: {err}");
        let recs = parse_ndjson(&out.stdout);
        assert!(
            !recs.iter().any(|r| r["event"] == "copy"),
            "no COPY is reported for a copy that cannot run: {recs:?}"
        );
    }

    assert_eq!(
        entry_names(&dst),
        ["a.txt", "shared.txt"],
        "the real run renamed A.txt to src's casing before the refusal"
    );
    assert_eq!(
        rfile(&dst, "a.txt"),
        b"OLD",
        "the shared inode is untouched"
    );
    assert_eq!(rfile(&dst, "shared.txt"), b"OLD");
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

/// **P7-1, P8-4.** The two-step case rename must not reuse a predictable
/// sibling path.
///
/// The temporary used to be `from.with_extension("girsync_case_tmp")`, a name in
/// the user's namespace: a file there was replaced by step1 (silent loss, exit
/// 0) and a directory made the pass abort. It is now a **fixed short** name,
/// `.girsync_case_tmp.<n>`, claimed before use, so an occupied candidate is
/// stepped over and left exactly as it was. The fixed name is also what keeps a
/// source whose own name is near `NAME_MAX` renameable (P8-4): the earlier
/// full-name suffix pushed the temporary past the limit.
#[test]
fn run_sync_case_rename_steps_over_occupied_temporaries() {
    let t = temp_root("case_tmp_collision");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "Data.txt", b"payload");
    wfile(&dst, "data.txt", b"payload");
    sync_mtime(&src.join("Data.txt"), &dst.join("data.txt"));
    // The directory form takes the other claim branch (create_dir, not
    // create_new), so the same candidates are exercised twice, in sequence.
    wfile(&src, "Dir/nested.txt", b"nested");
    wfile(&dst, "dir/nested.txt", b"nested");
    sync_mtime(&src.join("Dir/nested.txt"), &dst.join("dir/nested.txt"));
    // The name the first implementation used, then the first three candidates:
    // a file, a directory, and another file. None is the tool's to overwrite.
    wfile(&dst, "data.girsync_case_tmp", b"OLD-NAME");
    wfile(&dst, ".girsync_case_tmp.0", b"PRECIOUS");
    wfile(&dst, ".girsync_case_tmp.1/keep.txt", b"KEEP");
    wfile(&dst, ".girsync_case_tmp.2", b"PRECIOUS-DIR");

    let recs = run_binary(
        &src,
        &dst,
        &["--keep-extra"],
        &[("GIRSYNC_TEST_CACHE", "memory")],
    );

    let renames: Vec<&str> = recs
        .iter()
        .filter(|r| r["event"] == "rename")
        .map(|r| r["from"].as_str().expect("a rename names its source"))
        .collect();
    assert_eq!(renames, ["data.txt", "dir"], "{recs:?}");
    assert_eq!(rfile(&dst, "Data.txt"), b"payload", "the rename happened");
    assert_eq!(rfile(&dst, "Dir/nested.txt"), b"nested");
    // The filesystem is case-insensitive here, so `dst.join("dir").exists()`
    // cannot tell the two spellings apart; the directory entries can.
    let names = entry_names(&dst);
    assert!(
        names.iter().any(|n| n == "Dir") && !names.iter().any(|n| n == "dir"),
        "the directory moved to src's casing, leaving no old spelling: {names:?}"
    );
    assert_eq!(
        rfile(&dst, "data.girsync_case_tmp"),
        b"OLD-NAME",
        "the abandoned first temporary name is not a path this code writes"
    );
    assert_eq!(
        rfile(&dst, ".girsync_case_tmp.0"),
        b"PRECIOUS",
        "an occupied candidate is stepped over, not replaced"
    );
    assert_eq!(
        rfile(&dst, ".girsync_case_tmp.1/keep.txt"),
        b"KEEP",
        "a directory at a candidate is stepped over too"
    );
    assert_eq!(
        rfile(&dst, ".girsync_case_tmp.2"),
        b"PRECIOUS-DIR",
        "the third candidate is stepped over as well"
    );
    assert!(
        !dst.join(".girsync_case_tmp.3").exists(),
        "the claimed temporary is consumed by the rename: {:?}",
        entry_names(&dst)
    );
}

/// **P8-4.** A case-only rename of a name that is already near the platform's
/// component limit must not fail because of the temporary's name.
///
/// The temporary used to append `.girsync_case_tmp.<n>` to the full source name,
/// so a 244-character name produced a 262-character sibling and step1 failed
/// with `ENAMETOOLONG` (Windows: `os error 123`) after the case fix had already
/// claimed the record order. The fixed short name has no such dependency.
#[test]
fn a_case_rename_of_a_very_long_name_succeeds() {
    let t = temp_root("case_tmp_long_name");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    // 244 bytes: long enough that the old suffix would pass a 255-byte
    // NAME_MAX, short enough to be legal itself on every host we run on.
    let lower = format!("{}.txt", "a".repeat(240));
    let upper = format!("{}.txt", "A".repeat(240));
    wfile(&src, &upper, b"payload");
    wfile(&dst, &lower, b"payload");
    sync_mtime(&src.join(&upper), &dst.join(&lower));

    let recs = run_binary(&src, &dst, &[], &[("GIRSYNC_TEST_CACHE", "memory")]);
    let renames: Vec<&str> = recs
        .iter()
        .filter(|r| r["event"] == "rename")
        .map(|r| r["from"].as_str().expect("a rename names its source"))
        .collect();
    assert_eq!(renames, [lower.as_str()], "{recs:?}");
    assert_eq!(rfile(&dst, &upper), b"payload");
    assert!(
        !dst.join(".girsync_case_tmp.0").exists(),
        "the claimed temporary is consumed: {:?}",
        entry_names(&dst)
    );
}

/// **P7-3.** A failed rename must publish no `RENAME` record.
///
/// The record used to be emitted before the two `fs::rename` calls, so an
/// interrupted case fix left a line claiming an action that did not happen. The
/// injected failure names the step, because a real rename failure (a lock, a
/// volume that went away) cannot be arranged portably.
#[test]
fn a_failed_case_rename_leaves_no_record() {
    let t = temp_root("case_rename_fail");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "Data.txt", b"payload");
    wfile(&dst, "data.txt", b"payload");
    sync_mtime(&src.join("Data.txt"), &dst.join("data.txt"));

    let out = run_binary_raw(
        &src,
        &dst,
        &[],
        &[
            ("GIRSYNC_TEST_CACHE", "memory"),
            ("GIRSYNC_TEST_FAIL_RENAME", "data.txt"),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(3),
        "the injected rename failure is a run failure; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("injected rename failure"),
        "the error names the failure: {err}"
    );
    let recs = parse_ndjson(&out.stdout);
    assert!(
        !recs.iter().any(|r| r["event"] == "rename"),
        "no record for a rename that did not happen: {recs:?}"
    );
    assert_eq!(
        entry_names(&dst),
        vec!["data.txt".to_string()],
        "dst is left exactly as it was, with no tool temporary"
    );
    assert_eq!(rfile(&dst, "data.txt"), b"payload");
}

/// **P8-5, P8-6.** A step-1 failure happens after the temporary was claimed.
///
/// The file claim creates an empty placeholder that step 1 replaces only on
/// success, so this is the branch that must remove the tool's own residue. The
/// failure is injected between the claim and the rename because a real one (a
/// lock, a volume that went away) cannot be arranged portably; the cleanup is
/// the production code's.
#[test]
fn a_failed_case_rename_step1_removes_the_placeholder() {
    let t = temp_root("case_rename_fail_step1");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "Data.txt", b"payload");
    wfile(&dst, "data.txt", b"payload");
    sync_mtime(&src.join("Data.txt"), &dst.join("data.txt"));

    let out = run_binary_raw(
        &src,
        &dst,
        &[],
        &[
            ("GIRSYNC_TEST_CACHE", "memory"),
            ("GIRSYNC_TEST_FAIL_RENAME_STEP1", "data.txt"),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(3),
        "the injected step-1 failure is a run failure; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("injected"),
        "the error names the failure: {err}"
    );
    assert!(
        err.contains("case fix step1"),
        "and the step it failed at: {err}"
    );
    let recs = parse_ndjson(&out.stdout);
    assert!(
        !recs.iter().any(|r| r["event"] == "rename"),
        "no record for a rename that did not happen: {recs:?}"
    );
    assert_eq!(
        entry_names(&dst),
        vec!["data.txt".to_string()],
        "the claimed placeholder is removed again"
    );
    assert_eq!(rfile(&dst, "data.txt"), b"payload");
}

/// **P8-6.** A step-2 failure happens after step 1 moved the entry into the
/// temporary, so the pass has to put it back where it was.
///
/// The rename is injected between the two real moves, which is the only way to
/// reach the rollback portably. Without it a failed second rename would leave
/// dst holding `.girsync_case_tmp.<n>` under the wrong name.
#[test]
fn a_failed_case_rename_step2_rolls_back() {
    let t = temp_root("case_rename_fail_step2");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "Data.txt", b"payload");
    wfile(&dst, "data.txt", b"payload");
    sync_mtime(&src.join("Data.txt"), &dst.join("data.txt"));

    let out = run_binary_raw(
        &src,
        &dst,
        &[],
        &[
            ("GIRSYNC_TEST_CACHE", "memory"),
            ("GIRSYNC_TEST_FAIL_RENAME_STEP2", "data.txt"),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(3),
        "the injected step-2 failure is a run failure; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("injected"),
        "the error names the failure: {err}"
    );
    assert!(
        err.contains("case fix step2"),
        "and the step it failed at: {err}"
    );
    let recs = parse_ndjson(&out.stdout);
    assert!(
        !recs.iter().any(|r| r["event"] == "rename"),
        "no record for a rename that did not happen: {recs:?}"
    );
    assert_eq!(
        entry_names(&dst),
        vec!["data.txt".to_string()],
        "the entry is rolled back to its old name, leaving no temporary"
    );
    assert_eq!(rfile(&dst, "data.txt"), b"payload", "and its content");
}

/// **P7-2.** A dst **file link** standing where src has a directory is a type
/// conflict: `fix_dirs` unlinks it and creates the directory before the nested
/// `mkdir`/`copy`, so the link guard must not refuse the descendants the plan
/// already made applicable. This needs a file link, which Windows without
/// developer mode refuses; the case reports the skip rather than passing.
#[test]
fn a_dst_file_link_at_a_src_directory_is_replaced_not_refused() {
    let t = temp_root("file_link_fixdir");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    wfile(&outside, "target.txt", b"not ours");
    wfile(&src, "a/b/c.txt", b"under the fixed link");
    skip_if_no_link!(
        "file",
        common::file_link(&outside.join("target.txt"), &dst.join("a"))
    );

    let recs = run_binary(&src, &dst, &[], &[]);

    assert_eq!(paths(&recs, "fix-dir"), ["a"], "{recs:?}");
    assert_eq!(paths(&recs, "mkdir"), ["a/b"], "{recs:?}");
    assert_eq!(paths(&recs, "copy"), ["a/b/c.txt"], "{recs:?}");
    assert!(dst.join("a").is_dir(), "the link became a directory");
    assert_eq!(rfile(&dst, "a/b/c.txt"), b"under the fixed link");
    assert_eq!(
        rfile(&outside, "target.txt"),
        b"not ours",
        "unlinking the link left its target alone"
    );
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

/// **`--max-depth` silently skips deeper files, and `sync` calls that success.**
///
/// The walk is unit-tested at a small cap; this is the end-to-end half, and the
/// half that matters: a real game tree deeper than the default 10 gets a green
/// run and an incomplete mirror, with nothing on stdout saying a file was
/// skipped. The shallow file is asserted present in the same fixture so "the
/// deep file is absent" cannot pass because the run did nothing at all.
#[test]
fn run_sync_max_depth_skips_deeper_files() {
    let t = temp_root("maxdepth");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "top.txt", b"top");
    // Depth 11 (ten directories, then the file); the default cap is 10.
    let deep = "d1/d2/d3/d4/d5/d6/d7/d8/d9/d10/deep.txt";
    wfile(&src, deep, b"deep");

    assert_eq!(
        cmd_sync(sync_mem(src.clone(), dst.clone()), &log()).unwrap(),
        0,
        "a file beyond the cap is not a run failure"
    );
    assert_eq!(rfile(&dst, "top.txt"), b"top", "the shallow file is copied");
    assert!(
        !dst.join(deep).exists(),
        "the depth-11 file is beyond the default cap and is not mirrored"
    );
}

/// **A failed copy discards only the cache tail since the last commit.**
///
/// `copy_files` writes each verified copy's row as it completes, and
/// `CommitLimits` commits periodically. The final partial batch is committed
/// only after every copy succeeded, so an abort returns before that commit and
/// the in-flight batch is dropped with its transaction.
///
/// The fixture is exactly that shape: four copies, one injected failure, and an
/// op threshold of two. Three copies reach `put`; the threshold commits after
/// the second, the third stays in the dropped batch. The counts - two rows, not
/// zero and not three - are the assertion: zero means the phase hoarded its rows
/// until the end, three means every `put` was durable and there was no tail.
///
/// Which two rows survive depends on completion order, so the case asserts the
/// count and the membership, never the names. The rerun then converges, which is
/// the property a user actually cares about ("No resume" in the README).
#[test]
fn a_failed_copy_discards_only_the_uncommitted_cache_tail() {
    let t = temp_root("copy_fail_tail");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    for i in 0..4 {
        wfile(
            &src,
            &format!("f{i}.bin"),
            format!("content {i}").as_bytes(),
        );
    }

    // A file-cache run: the rows have to outlive the process that wrote them.
    let run = |fail: Option<&str>| {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"));
        cmd.arg("sync")
            .arg("--src")
            .arg(&src)
            .arg("--dst")
            .arg(&dst)
            .arg("--output")
            .arg("json")
            .env("GIRSYNC_TEST_COMMIT_OPS", "2")
            // Keep the time trigger out of it: the op count is the only commit
            // this fixture may observe during the copy phase.
            .env("GIRSYNC_TEST_COMMIT_INTERVAL_MS", "60000");
        if let Some(rel) = fail {
            cmd.env("GIRSYNC_TEST_FAIL_COPY", rel);
        }
        cmd.output().expect("spawn girsync")
    };

    let out = run(Some("f2.bin"));
    assert_eq!(
        out.status.code(),
        Some(3),
        "an injected copy failure is a run failure: stderr {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The three copies that did not fail landed on disk.
    let copied = ["f0.bin", "f1.bin", "f3.bin"];
    for rel in copied {
        assert!(
            dst.join(rel).is_file(),
            "{rel} should have been copied before the abort"
        );
    }
    assert!(
        !dst.join("f2.bin").exists(),
        "the failing copy wrote nothing"
    );

    let rows = recs(&dst);
    let cached: Vec<&str> = copied
        .iter()
        .copied()
        .filter(|rel| rows.contains_key(*rel))
        .collect();
    assert_eq!(
        cached.len(),
        2,
        "two rows committed, the third discarded with the batch; rows are {:?}",
        rows.keys().collect::<Vec<_>>()
    );
    assert!(
        !rows.contains_key("f2.bin"),
        "a cache row must never claim content the dst does not have"
    );

    // And the rerun converges: no resume, but no missing copy either.
    let out = run(None);
    assert!(
        out.status.success(),
        "the rerun failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rows = recs(&dst);
    for i in 0..4 {
        let rel = format!("f{i}.bin");
        assert!(rows.contains_key(&rel), "rerun left no row for {rel}");
    }
    assert_eq!(
        cmd_compare(compare_mem(src.clone(), dst.clone()), &log()).unwrap(),
        0,
        "the rerun produces a clean mirror"
    );
}

/// A mirror may not contain its own source, in either direction.
///
/// With `dst` inside `src`, the walk of src includes dst, so the copy plan
/// writes a parent's namesake over the child's file and creates a nested copy;
/// with `src` inside `dst`, the delete pass removes the source files before the
/// copy that needed them. Both are refused before the first mutation, in dry
/// run too. A sibling that merely shares a textual prefix is not nested.
#[test]
fn run_sync_refuses_nested_roots_before_touching_anything() {
    let t = temp_root("nested_roots");
    let root = t.subdir("root");
    let sub = t.subdir("root/sub");
    wfile(&root, "y.txt", b"SRC-Y");
    wfile(&sub, "y.txt", b"SRC-SUB-Y");
    wfile(&sub, "x.txt", b"SRC-SUB-X");

    for extra in [&[][..], &["--dry-run"][..]] {
        let out = run_binary_raw(&root, &sub, extra, &[]);
        assert_eq!(
            out.status.code(),
            Some(3),
            "dst inside src must be refused; stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("disjoint"),
            "the error names the overlap: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let out = run_binary_raw(&sub, &root, &[], &[]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "src inside dst must be refused; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("disjoint"),
        "the error names the overlap: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Neither direction changed a byte, made a nested copy, or opened a cache.
    // Neither direction changed a byte, made a nested copy, or opened a cache.
    assert_eq!(rfile(&root, "y.txt"), b"SRC-Y");
    assert_eq!(rfile(&sub, "y.txt"), b"SRC-SUB-Y");
    assert_eq!(rfile(&sub, "x.txt"), b"SRC-SUB-X");
    assert!(!sub.join("sub").exists(), "no nested copy was made");
    assert!(!root.join(CACHE_PREFIX).exists(), "no src cache was made");
    assert!(!sub.join(CACHE_PREFIX).exists(), "no dst cache was made");

    // Component-wise containment, not a string prefix: `root2` is a sibling.
    let sibling = t.subdir("root2");
    let out = run_binary_raw(&root, &sibling, &[], &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a shared prefix is not nesting; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(rfile(&sibling, "y.txt"), b"SRC-Y");
}

/// A failed flush is a failed copy: the run exits 3 and caches no row for the
/// file.
///
/// `sync_all` used to be `.ok()`, so the verify re-read the same page cache the
/// flush never wrote through, cached the digest, and the run exited 0 claiming
/// durability it did not obtain. The injector is the only portable way to make
/// the flush fail; the row is what a rerun would otherwise trust.
#[test]
fn a_failed_flush_aborts_before_the_copy_is_cached() {
    let t = temp_root("flush_fail");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.bin", b"payload");

    // `GIRSYNC_TEST_NO_FSYNC=0` because the workspace's nextest setup turns the
    // flush off for speed; this case is about the flush itself.
    let out = run_binary_raw(
        &src,
        &dst,
        &[],
        &[
            ("GIRSYNC_TEST_FAIL_FLUSH", "a.bin"),
            ("GIRSYNC_TEST_NO_FSYNC", "0"),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(3),
        "an injected flush failure is a run failure; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("injected flush failure"),
        "the error names the flush: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !recs(&dst).contains_key("a.bin"),
        "a cache row must not claim a copy whose flush failed"
    );

    // No resume: the rerun copies it and caches it.
    let out = run_binary_raw(&src, &dst, &[], &[("GIRSYNC_TEST_NO_FSYNC", "0")]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "the rerun converges; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(rfile(&dst, "a.bin"), b"payload");
    assert!(recs(&dst).contains_key("a.bin"));
}

/// A case-only rename of a directory must move the subtree once, not try to
/// rename its children at the old path.
///
/// The old pass renamed the parent and then called `fs::rename` on
/// `dir/file.txt`; on a case-sensitive host that path no longer exists, so the
/// run exited 3 after printing renames it did not perform. Here the child's
/// leaf casing is already src's, so the parent move is the whole job.
#[test]
fn run_sync_renames_a_case_only_directory_before_its_children() {
    let t = temp_root("casedir_rename");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "Dir/file.txt", b"payload");
    wfile(&dst, "dir/file.txt", b"payload");
    sync_mtime(&src.join("Dir/file.txt"), &dst.join("dir/file.txt"));

    let mut o = sync(src.clone(), dst.clone());
    o.common.test_cache = TestCacheBackend::Memory;
    o.common.case_sensitive = false;
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    assert_eq!(entry_names(&dst), vec!["Dir".to_string()]);
    assert_eq!(rfile(&dst, "Dir/file.txt"), b"payload");
    // The subtree was moved, not duplicated under the new parent.
    assert!(!dst.join("Dir/Dir").exists());

    let mut same = compare_mem(src, dst);
    same.common.case_sensitive = false;
    assert_eq!(cmd_compare(same, &log()).unwrap(), 0);
}

/// Two spellings of one root that differ only by case are the same cache.
///
/// The pre-open identity check used to canonicalize the cache *file*, which does
/// not exist on a fresh tree, and then fall back to the raw path: `root` and
/// `ROOT` compared unequal, the src side created the cache, and the dst open
/// failed with "Database already open … use --ignore-cache". The advice was
/// wrong and `--ignore-cache` would delete the cache the src side just made.
#[cfg(windows)]
#[test]
fn run_sync_refuses_case_only_spellings_of_one_root() {
    let t = temp_root("case_roots");
    let root = t.subdir("root");
    wfile(&root, "a.txt", b"payload");
    let upper = t.subdir("ROOT");

    let out = run_binary_raw(&root, &upper, &[], &[]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "case-only roots are one folder; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("resolve to the same cache"),
        "the error names the collision: {stderr}"
    );
    assert!(
        !stderr.contains("--ignore-cache"),
        "a lock is not corruption: {stderr}"
    );
    assert!(
        !root.join(CACHE_PREFIX).exists(),
        "the refusal happens before either cache is opened"
    );
}
