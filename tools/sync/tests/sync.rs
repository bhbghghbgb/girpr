//! End-to-end runs of `cmd_sync` (real FS + real cache).

mod common;

use std::num::NonZeroUsize;

use girsync::rw::RwLimits;

use common::{
    compare, compare_mem, compare_self_opts, entry_names, has_backup_sibling, is_case_sensitive,
    log, parse_ndjson, recs_of, rfile, skip_if_no_link, sync, sync_dry, sync_mem, sync_mtime,
    temp_root, update, update_mem, wfile,
};
use girsync::cache::{
    CACHE_PREFIX, CacheOpen, CommitLimits, TestCacheBackend, load_all_records, open_db,
};
use girsync::{BackupPolicy, TestHooks, cmd_compare, cmd_compare_self, cmd_sync, cmd_update};
use serde_json::Value;
use std::path::Path;

fn recs(dir: &Path) -> std::collections::HashMap<String, girsync::cache::FileRec> {
    recs_of(dir)
}

/// The names of the entries in `dir` that start with `prefix`, sorted.
///
/// The tool's own namespace, so the answer is "what the tool left behind"
/// rather than "what is in the directory".
fn siblings(dir: &Path, prefix: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(prefix))
        .collect();
    names.sort();
    names
}

/// The md5 digest a folder's cache holds for `rel`.
fn md5_in(dir: &Path, rel: &str) -> Vec<u8> {
    recs(dir)[rel]
        .hashes
        .get_named("md5")
        .expect("the row holds an md5")
        .to_vec()
}

/// The md5 digest a cache *file* holds for `rel` — a copy of one, read
/// without touching it.
fn md5_in_file(path: &Path, rel: &str) -> Vec<u8> {
    let db = open_db(path, true, CacheOpen::ReadOnly).expect("the copy is a readable cache");
    load_all_records(&db).unwrap()[rel]
        .hashes
        .get_named("md5")
        .expect("the row holds an md5")
        .to_vec()
}

/// Run `girsync sync` in-process and return the records it reported.
///
/// `--output json` throughout, so what a test reads is the records a machine
/// would parse off stdout. The alternative — matching rendered text lines —
/// makes every case a second, quieter statement of the vocabulary.
fn run_binary(src: &Path, dst: &Path, extra: &[&str], envs: &[(&str, &str)]) -> Vec<Value> {
    let out = run_binary_raw(src, dst, extra, envs);
    assert!(
        out.status.success(),
        "sync failed: {}\n{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    parse_ndjson(&out.stdout)
}

/// Run `girsync sync` in-process and return its exit status, records and error
/// string, without asserting success.
///
/// `run_binary` above is the happy-path form; a case whose point is the exit
/// code or the error text needs the raw output, because the assertion is on
/// what was (or was not) reported. `envs` is kept in the signature for the
/// process-based helpers below; the in-process path pins the file cache, which
/// is what the on-disk-cache cases want anyway.
fn run_binary_raw(src: &Path, dst: &Path, extra: &[&str], _envs: &[(&str, &str)]) -> RawRun {
    // The environment fallback keeps the runner's inherited hooks (NO_FSYNC,
    // DURABILITY) for every case that does not inject its own.
    run_binary_raw_with_hooks(src, dst, extra, TestHooks::from_env())
}

/// The in-process form of [`run_binary_raw`] with hooks injected as data. This
/// is the downshift of the failure-path child spawns: no `GIRSYNC_TEST_*`
/// variable is set on a child, so two runs in one test can ask for different
/// failures without a process-global `OnceLock` getting in the way.
fn run_binary_raw_with_hooks(src: &Path, dst: &Path, extra: &[&str], hooks: TestHooks) -> RawRun {
    let mut args: Vec<String> = vec![
        "sync".into(),
        "--src".into(),
        src.display().to_string(),
        "--dst".into(),
        dst.display().to_string(),
        "--output".into(),
        "json".into(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    let out = common::run_args_with_hooks(args, hooks);
    let stdout = common::values(&out)
        .iter()
        .map(|r| format!("{r}\n"))
        .collect::<String>()
        .into_bytes();
    let stderr = out.error.map(|e| e.into_bytes()).unwrap_or_default();
    RawRun {
        status: Status { code: out.code },
        stdout,
        stderr,
    }
}

/// The process-free stand-in for the parts of `std::process::Output` these
/// cases read: exit status, captured stdout records and the `fatal` error string
/// `main` would have logged to stderr. Keeping the shape lets the call sites read
/// unchanged.
struct RawRun {
    status: Status,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Status {
    code: i32,
}

impl Status {
    fn success(&self) -> bool {
        self.code == 0
    }
    fn code(&self) -> Option<i32> {
        Some(self.code)
    }
}

/// The `path` of every record with `event` `name`, in order.
fn paths<'a>(recs: &'a [Value], name: &str) -> Vec<&'a str> {
    recs.iter()
        .filter(|r| r["event"] == name)
        .map(|r| r["path"].as_str().expect("a path record has a path"))
        .collect()
}

/// A dry run writes nothing on disk *and* creates no caches.
///
/// Merged from the formerly separate `run_sync_dry_run_writes_nothing` and
/// `run_sync_dry_run_creates_no_cache`: they overlapped almost entirely, so a
/// single run now carries every one of their checks. A read/write open would
/// have created a cache per side and written its `meta`; the dry run must leave
/// both sides untouched. The `--ignore-cache` case is distinct and stays in its
/// own test below.
#[test]
fn run_sync_dry_run_writes_nothing_and_creates_no_cache() {
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

    // Nor did the run create a cache on either side, or back one up. These
    // assertions run before the compares below, which open their own caches.
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

    // The plan was still the real one, and nothing was applied -- in both cache
    // modes the two originals asserted.
    assert_eq!(
        cmd_compare(compare_mem(src.clone(), dst.clone()), &log()).unwrap(),
        4
    );
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

/// A backslash is a literal filename character on POSIX, not a separator. A
/// walk that rewrote it would plan `\lead.txt` at an absolute path — which
/// replaces the destination root when joined onto it — and `a\b.txt` under a
/// directory that does not exist on disk. The plan must keep both names as the
/// disk spells them, and the copies must land under dst with those names.
#[cfg(not(windows))]
#[test]
fn a_backslash_in_a_posix_filename_stays_literal_and_the_copy_stays_under_dst() {
    let t = temp_root("posix_backslash");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a\\b.txt", b"inside");
    wfile(&src, "\\lead.txt", b"leading");

    // Read the plan before the real run: an absolute `/lead.txt` would copy
    // outside dst, so the dry run is where that must be caught, before a byte is
    // written anywhere.
    let dry = run_binary(&src, &dst, &["--dry-run"], &[]);
    let copies = paths(&dry, "copy");
    assert!(copies.contains(&"\\lead.txt"), "{dry:?}");
    assert!(copies.contains(&"a\\b.txt"), "{dry:?}");
    assert!(!copies.contains(&"/lead.txt"), "{dry:?}");
    assert!(!copies.contains(&"a/b.txt"), "{dry:?}");
    assert!(entry_names(&dst).is_empty(), "the dry run writes nothing");

    assert_eq!(cmd_sync(sync_mem(src, dst.clone()), &log()).unwrap(), 0);

    assert_eq!(rfile(&dst, "\\lead.txt"), b"leading");
    assert_eq!(rfile(&dst, "a\\b.txt"), b"inside");
    assert_eq!(
        entry_names(&dst),
        vec!["\\lead.txt".to_string(), "a\\b.txt".to_string()]
    );
    assert!(
        !dst.join("a").exists(),
        "the backslash did not invent a directory"
    );
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
/// dry run must print the same plan as the real run, so `fix_dirs` can satisfy
/// it before `AlreadyExists` (os error 183) can abort anything.
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

/// A dst file hidden by `--include` still blocks the `MKDIR` the plan derives
/// from src's retained ancestor directory. Both modes must refuse it before the
/// first filesystem change — including the case-only rename.
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

    for extra in [
        &["--include", "*.txt", "--dry-run"][..],
        &["--include", "*.txt"][..],
    ] {
        let out = run_binary_raw(&src, &dst, extra, &[]);
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

/// Seed the "dst inode with two names" shape shared by the hard-link refusal
/// dimensions below: src holds the bytes a sync will want to copy, dst holds
/// the old bytes under `dst_name`, and `alias` is a second name for that same
/// inode. `dst_name` differs from `src_name` only for the case-only dimension.
fn seed_dst_hard_link_pair(src: &Path, dst: &Path, src_name: &str, dst_name: &str, alias: &str) {
    wfile(src, src_name, b"NEW");
    wfile(dst, dst_name, b"OLD");
    std::fs::hard_link(dst.join(dst_name), dst.join(alias))
        .expect("hard links need no privilege on this host");
}

/// `dst/a.txt` is a hard link to `src/b.txt`, so truncating it writes through
/// to a source file. Both hash modes must refuse before `File::create`, leaving
/// the shared bytes and the link in place.
#[test]
fn a_dst_hard_link_is_refused_without_touching_the_shared_inode() {
    let t = temp_root("hardlink_guard");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"AAAA");
    wfile(&src, "b.txt", b"BBBBBBBBBB");
    std::fs::hard_link(src.join("b.txt"), dst.join("a.txt"))
        .expect("hard links need no privilege on this host");

    for extra in [
        &["--exclude", "b.txt"][..],
        &["--exclude", "b.txt", "--hash-all-of", "none"][..],
    ] {
        let out = run_binary_raw(&src, &dst, extra, &[]);
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

/// Hard links are refused at plan time in every dimension: an ordinary dst
/// pair, its case-only spelling, and (in the sibling test above) a dst link
/// into src. The refusal is the same decision both modes take: with
/// `--keep-extra` the second name survives, so both modes refuse before any
/// action record; without it, the delete phase removes the second name first
/// and both modes succeed — the preflight credits the planned delete instead of
/// refusing every count above one.
///
/// The ordinary-pair and case-only dimensions share one fixture: it holds both
/// hazards at once, so a single dry/real refusal pair pins them together. This
/// keeps both dimensions asserted with fewer binary spawns than running each
/// against its own tree. They are not strictly subsumed — the case-only pass
/// re-keys `dm` in a dry run but moves nothing on disk, so both modes stat the
/// spelling still on disk and the real run renames before refusing — so each
/// dimension keeps its own assertions. The default-plan half uses a separate,
/// single-pair fixture so the case rename cannot alter the exact plan.
#[test]
fn the_hard_link_refusal_is_the_same_decision_in_all_dimensions() {
    // ----- dimensions 1 & 2: one fixture, two dst hard-link pairs -----------
    let t = temp_root("hardlink_parity");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    seed_dst_hard_link_pair(&src, &dst, "n.txt", "n.txt", "n_shared.txt");
    // The case-only pair is observable only where the two spellings differ.
    let case_sensitive = is_case_sensitive(&dst);
    if case_sensitive {
        seed_dst_hard_link_pair(&src, &dst, "c.txt", "C.txt", "c_shared.txt");
    } else {
        eprintln!("skip: this filesystem pairs `CaseProbe` with `caseprobe`");
    }

    for extra in [&["--keep-extra", "--dry-run"][..], &["--keep-extra"][..]] {
        let out = run_binary_raw(&src, &dst, extra, &[]);
        assert_eq!(
            out.status.code(),
            Some(3),
            "the kept second name makes the copy unsafe: {extra:?}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("hard links"), "names the hazard: {err}");
        assert!(
            err.contains("n.txt") || err.contains("c.txt") || err.contains("C.txt"),
            "names the copy: {err}"
        );
        let recs = parse_ndjson(&out.stdout);
        assert!(
            !recs.iter().any(|r| r["event"] == "copy"),
            "no COPY is reported for a copy that cannot run: {recs:?}"
        );
        // Ordinary pair: both names still hold dst's bytes.
        assert_eq!(
            rfile(&dst, "n.txt"),
            b"OLD",
            "the shared inode is untouched"
        );
        assert_eq!(rfile(&dst, "n_shared.txt"), b"OLD");
        // Case-only pair: same invariant, spelled as it is on disk at this
        // point. Only the real run's rename pass moves C.txt to c.txt.
        if case_sensitive {
            let on_disk = if extra.contains(&"--dry-run") {
                "C.txt"
            } else {
                "c.txt"
            };
            assert_eq!(rfile(&dst, on_disk), b"OLD", "case pair inode untouched");
            assert_eq!(rfile(&dst, "c_shared.txt"), b"OLD");
        }
    }
    // The real --keep-extra run renamed C.txt to src's casing before refusing;
    // the dry run (which moves nothing on disk) did not.
    if case_sensitive {
        let names = entry_names(&dst);
        assert!(
            names.iter().any(|n| n == "c.txt"),
            "renamed to src's casing: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n == "C.txt"),
            "no old spelling left: {names:?}"
        );
    }

    // ----- dimension 1's other half: the default plan deletes the extra -----
    // A fresh single-pair fixture, so the case pair cannot change the plan. The
    // default plan deletes the second name before the copy, reducing the count
    // to one; both modes must reach that decision.
    let t2 = temp_root("hardlink_default");
    let src2 = t2.subdir("src");
    let dst2 = t2.subdir("dst");
    seed_dst_hard_link_pair(&src2, &dst2, "a.txt", "a.txt", "shared.txt");

    let dry = run_binary(&src2, &dst2, &["--dry-run"], &[]);
    assert_eq!(paths(&dry, "delete"), ["shared.txt"], "{dry:?}");
    assert_eq!(paths(&dry, "copy"), ["a.txt"], "{dry:?}");
    assert_eq!(rfile(&dst2, "a.txt"), b"OLD", "the dry run changed nothing");

    let real = run_binary(&src2, &dst2, &[], &[]);
    assert_eq!(paths(&real, "delete"), ["shared.txt"], "{real:?}");
    assert_eq!(paths(&real, "copy"), ["a.txt"], "{real:?}");
    assert_eq!(rfile(&dst2, "a.txt"), b"NEW");
    assert!(!dst2.join("shared.txt").exists());
    assert_eq!(cmd_compare(compare_mem(src2, dst2), &log()).unwrap(), 0);
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
    dry.common.set_case_sensitive(false);
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
    real.common.set_case_sensitive(false);
    assert_eq!(cmd_sync(real, &log()).unwrap(), 0);
    assert_eq!(
        entry_names(&dst),
        vec!["Data.txt".to_string()],
        "dst adopts src casing"
    );

    // Converged: no further differences in either mode.
    let mut same = compare_mem(src.clone(), dst.clone());
    same.common.set_case_sensitive(false);
    assert_eq!(cmd_compare(same, &log()).unwrap(), 0);
    assert_eq!(cmd_compare(compare_mem(src, dst), &log()).unwrap(), 0);
}

/// The two-step case rename must not reuse a predictable sibling path.
///
/// The temporary is a **fixed short** name, `.girsync_case_tmp.<n>`, claimed
/// before use, so an occupied candidate is stepped over and left exactly as it
/// was. A name in the user's namespace would be replaced by step1 (silent
/// loss) or make the pass abort on a directory. The fixed name also keeps a
/// source near `NAME_MAX` renameable: a full-name suffix would push the
/// temporary past the limit.
#[test]
fn run_sync_case_rename_steps_over_occupied_temporaries() {
    let t = temp_root("case_tmp_collision");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    // **The two src names mix case on purpose.** The rename pass orders its
    // steps by the *folded* src key, and the folded keys are `data.txt` and
    // `dir` — so the file moves first. Sorted by the exact key the order would
    // be the other way round, because `Dir` sorts before `data.txt` byte-wise.
    // A fixture whose names all started with the same case would have one
    // order under both rules, and the case could not tell them apart.
    wfile(&src, "data.txt", b"payload");
    wfile(&dst, "Data.txt", b"payload");
    sync_mtime(&src.join("data.txt"), &dst.join("Data.txt"));
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

    let recs = run_binary(&src, &dst, &["--keep-extra"], &[]);

    let renames: Vec<&str> = recs
        .iter()
        .filter(|r| r["event"] == "rename")
        .map(|r| r["from"].as_str().expect("a rename names its source"))
        .collect();
    assert_eq!(renames, ["Data.txt", "dir"], "{recs:?}");
    // The order the pass did **not** use, computed from the fixture's own
    // names: the pairs sorted by their exact src spelling put the directory
    // first, because `Dir` sorts before `data.txt` byte-wise. The order above
    // is the folded one, so a pass that changed its rule would land on this
    // one and fail the assertion — rather than the case passing whichever of
    // the two the pass happened to use.
    let mut exact = [("Data.txt", "data.txt"), ("dir", "Dir")];
    exact.sort_by(|a, b| a.1.cmp(b.1));
    let exact_order: Vec<&str> = exact.iter().map(|(dst, _)| *dst).collect();
    assert_ne!(
        renames, exact_order,
        "the pass ordered by the exact src key, not the folded one"
    );
    assert_eq!(rfile(&dst, "data.txt"), b"payload", "the rename happened");
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

/// **A directory rename and a descendant's leaf rename in one pass.** The
/// directory step moves first, so the child step begins at the moved parent and
/// also changes its own leaf; a sibling that only differed by the parent is a
/// no-op step that still follows in the cache. Both the dst map and its cache
/// rows must end on src's casing, or the plan after the pass would copy the new
/// spelling and delete the old one.
#[test]
fn a_nested_case_rename_rekeys_the_destination_map_and_cache() {
    let t = temp_root("case_rename_nested");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "Dir/File.txt", b"payload");
    wfile(&src, "Dir/keep.txt", b"same");
    wfile(&dst, "dir/file.txt", b"payload");
    wfile(&dst, "dir/keep.txt", b"same");
    sync_mtime(&src.join("Dir/File.txt"), &dst.join("dir/file.txt"));
    sync_mtime(&src.join("Dir/keep.txt"), &dst.join("dir/keep.txt"));

    // Seed dst's cache on its own casing, so the rename pass has rows to move.
    assert_eq!(cmd_update(update(dst.clone()), &log()).unwrap(), 0);
    let before = recs(&dst);
    for old in ["dir", "dir/file.txt", "dir/keep.txt"] {
        assert!(
            before.contains_key(old),
            "the fixture needs a row for {old}: {:?}",
            before.keys().collect::<Vec<_>>()
        );
    }

    // The binary with the real cache: the rows it leaves are the assertion.
    // Opt out of the memory default — `recs(&dst)` below reads the on-disk rows.
    let out = run_binary_raw(&src, &dst, &[], &[("GIRSYNC_TEST_CACHE", "file")]);
    assert!(
        out.status.success(),
        "sync failed: {}\n{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    let records = parse_ndjson(&out.stdout);
    let steps: Vec<(&str, &str)> = records
        .iter()
        .filter(|r| r["event"] == "rename")
        .map(|r| {
            (
                r["from"].as_str().expect("a rename names its source"),
                r["to"].as_str().expect("a rename names its target"),
            )
        })
        .collect();
    assert_eq!(
        steps,
        [("dir", "Dir"), ("Dir/file.txt", "Dir/File.txt")],
        "the parent moves first, and the changed leaf moves at its new parent: {records:?}"
    );
    assert!(
        !records.iter().any(|r| matches!(
            r["event"].as_str(),
            Some("copy" | "delete" | "mkdir" | "rmdir" | "fix-dir")
        )),
        "a re-keyed map leaves nothing to copy or delete: {records:?}"
    );

    assert_eq!(rfile(&dst, "Dir/File.txt"), b"payload");
    assert_eq!(rfile(&dst, "Dir/keep.txt"), b"same");
    assert_eq!(
        entry_names(&dst),
        vec!["Dir".to_string()],
        "no old spelling of the directory survives"
    );

    let rows = recs(&dst);
    for new in ["Dir", "Dir/File.txt", "Dir/keep.txt"] {
        assert!(
            rows.contains_key(new),
            "the cache row follows the rename to {new}: {:?}",
            rows.keys().collect::<Vec<_>>()
        );
    }
    for old in ["dir", "dir/file.txt", "dir/keep.txt"] {
        assert!(
            !rows.contains_key(old),
            "the cache row did not follow the rename from {old}: {:?}",
            rows.keys().collect::<Vec<_>>()
        );
    }

    assert_eq!(
        cmd_compare_self(compare_self_opts(dst), &log()).unwrap(),
        0,
        "the moved rows describe the tree the run left"
    );
}

/// A case-only rename of a name that is already near the platform's component
/// limit must not fail because of the temporary's name.
///
/// Appending `.girsync_case_tmp.<n>` to the full source name made a 244-byte
/// name into a 262-byte sibling and step1 failed with `ENAMETOOLONG` (Windows:
/// `os error 123`). The fixed short name has no such dependency.
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

    let recs = run_binary(&src, &dst, &[], &[]);
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

/// A failed rename must publish no `RENAME` record: an interrupted case fix
/// must not leave a line claiming an action that did not happen. The injected
/// failure names the step, because a real rename failure (a lock, a volume that
/// went away) cannot be arranged portably.
#[test]
fn a_failed_case_rename_leaves_no_record() {
    let t = temp_root("case_rename_fail");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "Data.txt", b"payload");
    wfile(&dst, "data.txt", b"payload");
    sync_mtime(&src.join("Data.txt"), &dst.join("data.txt"));

    let out = run_binary_raw_with_hooks(
        &src,
        &dst,
        &[],
        TestHooks::default().with_fail_rename_before_claim("data.txt"),
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

/// A step-1 failure happens after the temporary was claimed: the file claim
/// creates an empty placeholder that step 1 replaces only on success, so this
/// branch must remove the tool's own residue. The
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

    let out = run_binary_raw_with_hooks(
        &src,
        &dst,
        &[],
        TestHooks::default().with_fail_rename_step1("data.txt"),
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

/// A step-2 failure happens after step 1 moved the entry into the temporary,
/// so the pass has to put it back where it was.
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

    let out = run_binary_raw_with_hooks(
        &src,
        &dst,
        &[],
        TestHooks::default().with_fail_rename_step2("data.txt"),
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

/// A dst **file link** standing where src has a directory is a type conflict:
/// `fix_dirs` unlinks it and creates the directory before the nested `mkdir`/
/// `copy`, so the link guard must not refuse the descendants the plan already
/// made applicable. This needs a file link, which Windows without developer
/// mode refuses; the case reports the skip rather than passing.
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
    dry.common.set_case_sensitive(false);
    dry.common.dry_run = true;
    assert_eq!(cmd_sync(dry, &log()).unwrap(), 0);
    assert_eq!(
        rfile(&dst, "data.txt"),
        b"PAYLOAD",
        "a dry run copies nothing"
    );

    let mut real = sync(src.clone(), dst.clone());
    real.common.test_cache = TestCacheBackend::Memory;
    real.common.set_case_sensitive(false);
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

/// **A copy carries src's timestamp to dst.**
///
/// The stamp is what makes a copied file *identical* to its source rather than
/// merely equal in content: `compare` treats size+mtime+hash as identity, so
/// a dst whose mtime drifted would read as changed on the next run and be
/// copied again, forever.
///
/// Asserted on the file on disk, not through a `compare` afterwards. A compare
/// answers "do the two sides agree", which several things can make true; the
/// property here is narrower — dst's mtime is src's mtime — and it is the one
/// a copy can lose on its own, since the bytes are written at a different
/// moment than the source's timestamp.
///
/// The source's timestamp is pinned to a fixed past instant first, so "dst
/// kept src's stamp" cannot be confused with "dst got stamped with the time
/// of the copy", which would be a different (and wrong) value.
///
/// **Both flush shapes**, because the stamp is ordered against the flush: the
/// flush puts the bytes on the disk, and the stamp has to land after it and
/// before the handle closes. A shape that stamped before the flush would let
/// the flush's own write-back move it again. The injected `no_fsync` hook picks
/// which shape runs, so both are asked for explicitly rather than inheriting
/// the workspace's choice.
#[test]
fn a_copied_file_carries_the_sources_timestamp() {
    let t = temp_root("copy_mtime");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"payload");

    // A fixed instant well in the past, so the copy's own write time cannot
    // be mistaken for it.
    let stamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    std::fs::OpenOptions::new()
        .write(true)
        .open(src.join("a.txt"))
        .unwrap()
        .set_modified(stamp)
        .unwrap();

    // Both flush shapes are asked for explicitly through the injected hook,
    // rather than by setting a process-global variable on a child.
    for no_fsync in [false, true] {
        let out = run_binary_raw_with_hooks(
            &src,
            &dst,
            &[],
            TestHooks::default().with_no_fsync(no_fsync),
        );
        assert_eq!(
            out.status.code(),
            Some(0),
            "the copy must succeed with GIRSYNC_TEST_NO_FSYNC={no_fsync}; stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        assert_eq!(
            rfile(&dst, "a.txt"),
            b"payload",
            "the bytes were copied (GIRSYNC_TEST_NO_FSYNC={no_fsync})"
        );
        assert_eq!(
            std::fs::metadata(dst.join("a.txt"))
                .unwrap()
                .modified()
                .unwrap(),
            stamp,
            "the copy carries src's timestamp, not the time of the copy \
             (GIRSYNC_TEST_NO_FSYNC={no_fsync})"
        );
    }
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

// --- F24: in-process capture of the copy-phase commit ordering ----------------
//
// A scoped subscriber (`common::run_args_logged`) cannot capture these trace
// events reliably under plain, threaded `cargo test`: tracing caches callsite
// interest process-globally and ANDs it across every live `Dispatch`, so sibling
// tests leave the commit/copy-done callsites at `never()`. The F23 narrow fix
// (pre-register + rebuild) is stable in the quiet `stat_trust` binary but was
// verified to flake here. Instead one process-global subscriber is installed for
// this binary; it captures every event's message and a couple of distinguishing
// fields while a global buffer is armed. Rows are filtered by an op threshold
// injected as a `TestHooks` value unique to this test, so concurrent tests cannot
// pollute the capture.

#[derive(Default, Clone, Debug)]
struct F24Event {
    message: String,
    ops: Option<u64>,
    total: Option<u64>,
}

static F24_CAPTURE: std::sync::Mutex<Option<Vec<F24Event>>> = std::sync::Mutex::new(None);

#[derive(Clone)]
struct F24CaptureLayer;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for F24CaptureLayer {
    fn register_callsite(
        &self,
        _meta: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }

    fn enabled(
        &self,
        _meta: &tracing::Metadata<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        F24_CAPTURE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        #[derive(Default)]
        struct Visit(F24Event);
        impl tracing::field::Visit for Visit {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if field.name() == "message" {
                    self.0.message = value.to_owned();
                }
            }
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0.message = format!("{value:?}");
                }
            }
            fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
                self.record_num(field, value);
            }
            fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
                self.record_num(field, value as u64);
            }
        }
        impl Visit {
            fn record_num(&mut self, field: &tracing::field::Field, value: u64) {
                match field.name() {
                    "ops" => self.0.ops = Some(value),
                    "total" => self.0.total = Some(value),
                    _ => {}
                }
            }
        }
        let mut v = Visit::default();
        event.record(&mut v);
        if !v.0.message.is_empty()
            && let Some(buf) = &mut *F24_CAPTURE.lock().unwrap_or_else(|e| e.into_inner())
        {
            buf.push(v.0);
        }
    }
}

static F24_SUBSCRIBER: std::sync::OnceLock<()> = std::sync::OnceLock::new();

fn install_f24_subscriber() {
    use tracing_subscriber::prelude::*;
    F24_SUBSCRIBER.get_or_init(|| {
        tracing::subscriber::set_global_default(
            tracing_subscriber::registry().with(F24CaptureLayer),
        )
        .expect("install the F24 capture subscriber");
    });
}

/// Run `girsync sync` in-process with an injected commit policy, capturing the
/// tracing events its phases emit via the global F24 layer.
#[allow(unsafe_code)] // `set_var` is unsafe in edition 2024; nextest isolates each test process
fn run_copy_commit_captured(
    src: &Path,
    dst: &Path,
    limits: CommitLimits,
) -> (girsync::RunOutcome, Vec<F24Event>) {
    use clap::Parser;
    use girsync::cli::Cli;

    let argv = [
        std::ffi::OsString::from("girsync"),
        "sync".into(),
        "--src".into(),
        src.as_os_str().to_owned(),
        "--dst".into(),
        dst.as_os_str().to_owned(),
        "--output".into(),
        "json".into(),
        "--rw-threads".into(),
        "8".into(),
        "--hash-all-of".into(),
        "none".into(),
    ];
    let cli = Cli::try_parse_from(argv).expect("valid CLI args");
    let plan = girsync::plan(girsync::logging::Level::Error, None, None, true);
    let hooks = TestHooks::default().with_commit(limits);

    unsafe { std::env::set_var("GIRSYNC_TEST_CACHE", "file") };
    *F24_CAPTURE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Vec::new());
    let out = girsync::run_captured_with_hooks(cli, &plan, hooks).expect("run_captured");
    let events = F24_CAPTURE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .unwrap_or_default();
    (out, events)
}

/// **The copy phase commits its cache rows while it is still copying.**
///
/// `CacheWrite`'s `CommitLimits` are only a bound on the work an interrupted run
/// loses if they are checked as that work happens. `copy_files` once collected
/// every verified record and inserted them only after the pool joined, so during
/// the phase itself no threshold could fire: a run that died mid-copy lost every
/// row, however many files had already landed on disk.
///
/// The trace log is the observable. A threshold tripping is logged as
/// `cache batch committed` (trace), and the phase logs `copy done` after the pool
/// joins, so the last batch commit must come *before* that line. Under the old
/// layout the only batch commit of the phase came after it, with every file
/// already durable.
///
/// The run is in-process (no child spawn). A process-global capture subscriber
/// collects the run's events; the commit policy is injected as a `TestHooks`
/// value with an op limit unique to this test, so its `cache batch committed`
/// events and the `copy done` event with this fixture's file total can be picked
/// out of the shared capture. The fixture is one-sided on purpose: with nothing
/// to hash, the scan and the pre-drop each auto-commit before the copy phase, and
/// nothing after it does — so the last matching commit is the copy phase's.
#[test]
fn the_copy_phase_commits_its_cache_rows_while_it_copies() {
    // An op threshold of 7 and 23 copies: both the scan and the copy phase cross
    // it several times. 7 and 23 are unique to this test (no other case uses them),
    // so the captured `ops=7` commits and `total=23` copy-done are this run's.
    let ops = 7u64;
    let files = 23usize;
    let limits = CommitLimits {
        max_ops: ops as usize,
        max_bytes: u64::MAX,
        // Keep the time trigger out of it: the op count is the only commit this
        // fixture may observe during the copy phase.
        max_interval: std::time::Duration::from_secs(60),
    };
    let t = temp_root("copy_commit");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    for i in 0..files {
        // Empty, so the fixture's cost is file metadata rather than content; the
        // op-count trigger counts rows, not bytes.
        wfile(&src, &format!("f{i:04}.bin"), b"");
    }

    install_f24_subscriber();
    let (out, events) = run_copy_commit_captured(&src, &dst, limits);
    assert_eq!(out.code, 0, "sync failed: {:?}", out.error);
    let recs = common::values(&out);
    assert_eq!(
        paths(&recs, "copy").len(),
        files,
        "the fixture must make the copy phase cross the op-count threshold"
    );

    // Filter the global capture to this run: commits that tripped our op limit,
    // and this run's copy-phase end. Concurrent tests use other thresholds/totals.
    let commits: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.message == "cache batch committed" && e.ops == Some(ops))
        .map(|(i, _)| i)
        .collect();
    assert!(
        !commits.is_empty(),
        "the fixture crossed the op limit, so at least one batch commit must be in the log; \
         an empty list would make the ordering assertion below vacuous: {events:?}"
    );
    let done = events
        .iter()
        .position(|e| e.message == "copy done" && e.total == Some(files as u64))
        .expect("the copy phase logs its end");
    assert!(
        commits.last().is_some_and(|&i| i < done),
        "no batch commit before `copy done` (event {done}): commits were at {commits:?}, \
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
    // The commit threshold and the injected failure are carried as hooks, so
    // both runs happen in-process without a child or an env var.
    let limits = CommitLimits {
        max_ops: 2,
        max_bytes: u64::MAX,
        // Keep the time trigger out of it: the op count is the only commit this
        // fixture may observe during the copy phase.
        max_interval: std::time::Duration::from_secs(60),
    };
    let run = |fail: Option<&str>| {
        let mut hooks = TestHooks::default().with_commit(limits);
        if let Some(rel) = fail {
            hooks = hooks.with_fail_copy(rel);
        }
        run_binary_raw_with_hooks(&src, &dst, &[], hooks)
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
/// file. The verify re-reads what the flush wrote through, so a flush that
/// never completed must not leave a digest the rerun would trust. The injector
/// is the only portable way to make the flush fail.
#[test]
fn a_failed_flush_aborts_before_the_copy_is_cached() {
    let t = temp_root("flush_fail");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.bin", b"payload");

    // `no_fsync` is explicitly false because the workspace's nextest setup
    // turns the flush off for speed; this case is about the flush itself. The
    // on-disk cache rows are read via `recs(&dst)`, and the harness pins the
    // file backend for in-process runs.
    let out = run_binary_raw_with_hooks(
        &src,
        &dst,
        &[],
        TestHooks::default()
            .with_no_fsync(false)
            .with_fail_flush("a.bin"),
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

    // No resume: the rerun copies it and caches it. Still reads `recs(&dst)`.
    let out = run_binary_raw_with_hooks(&src, &dst, &[], TestHooks::default().with_no_fsync(false));
    assert_eq!(
        out.status.code(),
        Some(0),
        "the rerun converges; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(rfile(&dst, "a.bin"), b"payload");
    assert!(recs(&dst).contains_key("a.bin"));
}

/// The failure a run reports is the **first copy in the plan**, not the first
/// copy to fail.
///
/// Two copies fail here, and they are arranged so that the two orders are not
/// the same order: `mmm.bin` fails before it opens anything, while `aaa.bin`
/// is copied in full and fails at the flush. The first failure to *finish* is
/// therefore `mmm.bin`, and the first copy *in the plan* is `aaa.bin`. A
/// phase that reported the first failure to finish would name `mmm.bin`; the
/// plan's order says `aaa.bin`, and that is what has to be reported.
///
/// **The plan lists its copies in path order**, so the copy that comes first
/// in the plan is the one whose path sorts first: `aaa.bin`. That order is
/// the plan builder's rule — pinned by the builder's own case,
/// `the_copy_list_is_in_path_order` — and this case leans on it, so it is
/// said here rather than left for a reader to infer from the fixture.
///
/// **The order the two failures are recorded in is put in place by a barrier,
/// not by a race.** `aaa.bin` waits at a gate until `mmm.bin` has failed, so
/// the recording order is the opposite of the plan's whatever the workers
/// happen to do and however fast the host is. A fixture that merely made
/// `aaa.bin` slow would say the same thing only while the host stayed slow
/// enough: the case would pass for the wrong reason on a fast machine, and
/// fail for the wrong reason on a loaded one.
///
/// The third file is the control: it succeeds, and it was still copied, so
/// the failure did not cut the phase short.
#[test]
fn the_reported_copy_failure_is_the_first_copy_in_the_plan() {
    let t = temp_root("copy_fail_order");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "aaa.bin", b"first in the plan");
    wfile(&src, "mmm.bin", b"fails first");
    wfile(&src, "zzz.bin", b"the control");

    let out = run_binary_raw_with_hooks(
        &src,
        &dst,
        &["--rw-threads", "2"],
        TestHooks::default()
            .with_fail_copy("mmm.bin")
            .with_fail_flush("aaa.bin")
            // The workspace's nextest setup turns the flush off for speed; this
            // case is about the flush.
            .with_no_fsync(false)
            // The barrier: `aaa.bin` does not begin until `mmm.bin` has failed,
            // so the failure recorded first is the one that comes *later* in
            // the plan.
            .with_gate_copy("aaa.bin:mmm.bin"),
    );
    assert_eq!(
        out.status.code(),
        Some(3),
        "two copies failed, so the run failed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // In-process, the raw `anyhow` chain is exposed as the error string (the
    // binary's `error="…"` wrapper is added by `main`'s fatal event, which a
    // captured run does not emit). The reported copy is named as the path
    // inside the mirror, and the whole line starts with `copy <rel>: `.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("copy aaa.bin: "),
        "the failure reported is the first copy in the plan: {stderr}"
    );
    assert!(
        !stderr.contains("mmm.bin"),
        "the copy that failed first is not the one reported: {stderr}"
    );

    // Every copy was attempted: the one that could succeed was copied and
    // announced, and neither failure announced anything.
    let recs = parse_ndjson(&out.stdout);
    assert_eq!(
        paths(&recs, "copy"),
        vec!["zzz.bin"],
        "the successful copy was announced and the failed ones were not: {recs:?}"
    );
    assert!(dst.join("zzz.bin").is_file(), "the successful copy landed");
}

/// **The failure a user reads is `copy <rel>: <cause>`, and it is exactly that.**
///
/// The reported failure reaches the user as one line: the copy that failed,
/// and why it failed. That line is the tool's contract with whoever reads it,
/// and a check for a substring does not pin it: a cause that grew a prefix, or
/// a copy named by its full path rather than by its path inside the mirror,
/// would pass a check for `copy mmm.bin` and change what the user reads.
///
/// The cause here is the injected one, which names no path of its own, so the
/// whole line is fixed — the phase, the copy, the separator, and the cause,
/// with nothing else in it.
#[test]
fn a_reported_copy_failure_is_the_copy_and_the_cause() {
    let t = temp_root("copy_fail_text");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "mmm.bin", b"payload");

    let out = run_binary_raw_with_hooks(
        &src,
        &dst,
        &[],
        TestHooks::default().with_fail_copy("mmm.bin"),
    );
    assert_eq!(
        out.status.code(),
        Some(3),
        "a failed copy is a failed run; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The in-process error is the raw `anyhow` chain: the phase, the relative
    // path and the injected cause, with nothing else around it.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("copy mmm.bin: test: injected copy failure for mmm.bin"),
        "the failure names the copy and the cause, and nothing else: {stderr}"
    );
}

/// A case-only rename of a directory must move the subtree once, not try to
/// rename its children at the old path: on a case-sensitive host that path no
/// longer exists after the parent move. Here the child's leaf casing is already
/// src's, so the parent move is the whole job.
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
    // The setter, not the field: the filter caches the options it matches with,
    // and the case mode is one of them, so assigning the field alone would leave
    // the filter matching under the old mode.
    o.common.set_case_sensitive(false);
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    assert_eq!(entry_names(&dst), vec!["Dir".to_string()]);
    assert_eq!(rfile(&dst, "Dir/file.txt"), b"payload");
    // The subtree was moved, not duplicated under the new parent.
    assert!(!dst.join("Dir/Dir").exists());

    let mut same = compare_mem(src, dst);
    same.common.set_case_sensitive(false);
    assert_eq!(cmd_compare(same, &log()).unwrap(), 0);
}

/// Two spellings of one root that differ only by case are the same cache.
///
/// The pre-open identity check compares roots canonically: `root` and `ROOT`
/// are one folder before either cache is opened, so a stale "--ignore-cache"
/// hint cannot delete the cache the src side just made.
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

/// On Windows a case-only rename of `girpr-cache` is the same live file, so
/// the walk must keep excluding it: the prefix is folded case-insensitively, so
/// the renamed cache is tool state again and both the dry run and the real run
/// leave it alone.
#[cfg(windows)]
#[test]
fn run_sync_excludes_a_case_renamed_cache_directory() {
    let t = temp_root("case_cache");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    if is_case_sensitive(&src) {
        eprintln!("skip: this filesystem keeps `GIRPR-CACHE` distinct from `girpr-cache`");
        return;
    }
    wfile(&src, "a.txt", b"payload");
    wfile(&dst, "a.txt", b"payload");
    sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));

    // Real file caches: with the in-memory backend there is no `girpr-cache`
    // entry on disk to rename, so opt out of the memory default throughout.
    let envs = [("GIRSYNC_TEST_CACHE", "file")];
    let out = run_binary_raw(&src, &dst, &[], &envs);
    assert!(
        out.status.success(),
        "initial sync failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(src.join(CACHE_PREFIX).is_file(), "src cache was created");
    assert!(dst.join(CACHE_PREFIX).is_file(), "dst cache was created");

    // Case-only through a temporary, as `tests/helpers.rs` does: the two
    // spellings name the same entry on this filesystem, so the rename has to
    // leave and come back.
    let tmp = src.join("girsync_case_cache_tmp");
    std::fs::rename(src.join(CACHE_PREFIX), &tmp).unwrap();
    std::fs::rename(&tmp, src.join("GIRPR-CACHE")).unwrap();
    let cache_name = std::fs::read_dir(&src)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.eq_ignore_ascii_case(CACHE_PREFIX))
        .expect("the live cache is still on disk");
    assert_eq!(cache_name, "GIRPR-CACHE", "the fixture renamed the cache");

    // Dry run: the cache is tool state, so nothing is planned for it.
    let out = run_binary_raw(&src, &dst, &["--dry-run"], &envs);
    assert!(
        out.status.success(),
        "dry run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let recs = parse_ndjson(&out.stdout);
    for rec in &recs {
        if let Some(p) = rec["path"].as_str() {
            assert!(
                !p.to_ascii_lowercase().starts_with(CACHE_PREFIX),
                "a case-renamed cache is not mirror content: {rec}"
            );
        }
    }
    let summary = common::summary_of(&recs);
    assert_eq!(summary["copied"], 0, "the cache is not copied: {summary}");
    assert_eq!(summary["deleted"], 0, "the cache is not deleted: {summary}");

    // A real run neither copies the cache nor damages dst's live cache.
    let out = run_binary_raw(&src, &dst, &[], &envs);
    assert!(
        out.status.success(),
        "sync after the rename failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(rfile(&dst, "a.txt"), b"payload");
    // Both caches still open; a dst cache readable before the run stays readable.
    assert_eq!(cmd_update(update(src.clone()), &log()).unwrap(), 0);
    assert_eq!(cmd_update(update(dst.clone()), &log()).unwrap(), 0);
}

/// A sync backs up src's cache and snapshots dst's pre-sync state.
///
/// dst gets the snapshot only. The two would be copies of the same bytes
/// taken at the same moment, so the second name is a second copy of one file
/// and nothing else.
#[test]
fn run_sync_backs_up_src_and_snapshots_dst() {
    let t = temp_root("bakpolicy");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"one");
    wfile(&dst, "a.txt", b"two");
    // Both sides have a cache, so both have something to copy aside.
    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(dst.clone()), &log()).unwrap();

    assert_eq!(cmd_sync(sync(src.clone(), dst.clone()), &log()).unwrap(), 0);

    assert_eq!(
        siblings(&src, "girpr-cache-backup-").len(),
        1,
        "src keeps one copy of its cache"
    );
    assert_eq!(
        siblings(&dst, "girpr-cache-old-").len(),
        1,
        "dst keeps one snapshot of its pre-sync state"
    );
    assert!(
        siblings(&dst, "girpr-cache-backup-").is_empty(),
        "dst does not get a second copy of the same bytes"
    );
}

/// The bound leaves a copy that is still being written alone.
///
/// A copy is written under a working name and published under its real name
/// once it is complete, so a run that merely opens a folder cannot take away
/// the copy another run is making there — however often the bound is applied.
#[test]
fn a_run_that_bounds_a_folder_leaves_a_copy_that_is_being_written_alone() {
    let t = temp_root("retain_working");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"one");
    // The copy another run is making: the working name carries neither of the
    // two published prefixes.
    wfile(
        &src,
        "girpr-cache-partial-20240101T000000.000000Z",
        b"half a copy",
    );
    // Copies from earlier runs, which the bound *is* about.
    wfile(&src, "girpr-cache-backup-20240101T000000.000000Z", b"old");
    wfile(&src, "girpr-cache-backup-20240102T000000.000000Z", b"newer");

    let mut o = sync(src.clone(), dst.clone());
    o.common.backup = BackupPolicy {
        disabled: false,
        retain: NonZeroUsize::new(1),
    };
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    assert!(
        src.join("girpr-cache-partial-20240101T000000.000000Z")
            .exists(),
        "the copy that is being written is left alone"
    );
    assert_eq!(
        siblings(&src, "girpr-cache-backup-").len(),
        1,
        "the published copies are still bounded"
    );
}

/// `--no-backup` leaves no copy and no snapshot behind.
///
/// It is a policy about what a run leaves behind, so it cannot change what
/// the run answers: the mirror is the same either way.
#[test]
fn run_sync_with_no_backup_leaves_no_copy() {
    let t = temp_root("nobak");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"one");
    wfile(&dst, "a.txt", b"two");
    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(dst.clone()), &log()).unwrap();

    let mut o = sync(src.clone(), dst.clone());
    o.common.backup = BackupPolicy {
        disabled: true,
        retain: None,
    };
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    for dir in [&src, &dst] {
        assert!(
            siblings(dir, "girpr-cache-backup-").is_empty(),
            "{} keeps no copy",
            dir.display()
        );
        assert!(
            siblings(dir, "girpr-cache-old-").is_empty(),
            "{} keeps no snapshot",
            dir.display()
        );
    }
    // The run still did its work.
    assert_eq!(rfile(&dst, "a.txt"), b"one");
}

/// `--backup-retain N` holds each prefix to N siblings.
///
/// The prefixes are counted separately: src's copies and dst's snapshots are
/// two lists, each with its own bound.
///
/// What the bound takes is the oldest copies it may remove — a copy a live
/// run is still using is left alone and does not spend the removal budget,
/// so the bound walks on to the next copy in age order and can take one that
/// is among the newest N. None is spoken for here, so the oldest of each
/// list goes and what is left is the two newest of the three. That is what
/// this case shows, not what the bound promises: the copies a folder keeps
/// are not necessarily its newest.
#[test]
fn run_sync_backup_retain_bounds_each_prefix() {
    let t = temp_root("retain");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"one");
    wfile(&dst, "a.txt", b"two");
    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(dst.clone()), &log()).unwrap();

    for _ in 0..3 {
        let mut o = sync(src.clone(), dst.clone());
        o.common.backup = BackupPolicy {
            disabled: false,
            retain: NonZeroUsize::new(2),
        };
        assert_eq!(cmd_sync(o, &log()).unwrap(), 0);
    }

    assert_eq!(
        siblings(&src, "girpr-cache-backup-").len(),
        2,
        "two copies kept, the oldest one gone"
    );
    assert_eq!(
        siblings(&dst, "girpr-cache-old-").len(),
        2,
        "two snapshots kept, the oldest one gone"
    );
    assert!(
        siblings(&dst, "girpr-cache-backup-").is_empty(),
        "dst keeps no copy of its own"
    );
}

/// A conversion that was killed before it could clean up after itself leaves a
/// file in the folder — and it is the tool's own residue, not the user's
/// content.
///
/// The conversion's temporary is named inside the tool's namespace, so the
/// scan does not see it: it is neither copied out of a source folder nor
/// deleted as an extra in a destination one.
#[test]
fn run_sync_leaves_a_leftover_conversion_temporary_alone() {
    let t = temp_root("convtmp");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"one");
    // A leftover in each folder, as a killed conversion in either would leave.
    wfile(
        &src,
        "girpr-cache-convert-1234-20240101T000000.000000Z.redb",
        b"half a database",
    );
    wfile(
        &dst,
        "girpr-cache-convert-5678-20240101T000000.000000Z.redb",
        b"half a database",
    );

    assert_eq!(cmd_sync(sync(src.clone(), dst.clone()), &log()).unwrap(), 0);

    assert!(
        !dst.join("girpr-cache-convert-1234-20240101T000000.000000Z.redb")
            .exists(),
        "the tool's own residue is not mirrored as content"
    );
    assert!(
        dst.join("girpr-cache-convert-5678-20240101T000000.000000Z.redb")
            .exists(),
        "and an extra in the destination is left alone: it is tool state"
    );
}

/// `--backup-retain N` bounds the copies beside a folder, whether or not this
/// run copied one.
///
/// The copies are what accumulates in a folder over time, so the bound is
/// about the folder rather than about the run that happens to be looking at
/// it. A folder whose cache is absent is bounded just the same — otherwise
/// the copies would only ever be pruned by a run that happened to make
/// another one.
#[test]
fn run_sync_backup_retain_bounds_a_folder_with_no_cache() {
    let t = temp_root("retain_nocache");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"one");
    // Copies from earlier runs, sitting beside caches these folders do not
    // have. Both lists are bounded on their own.
    wfile(&src, "girpr-cache-backup-20240101T000000.000000Z", b"old");
    wfile(&src, "girpr-cache-backup-20240102T000000.000000Z", b"newer");
    wfile(&dst, "girpr-cache-old-20240101T000000.000000Z", b"old");
    wfile(&dst, "girpr-cache-old-20240102T000000.000000Z", b"newer");
    assert!(!src.join(CACHE_PREFIX).exists(), "src has no cache");
    assert!(!dst.join(CACHE_PREFIX).exists(), "dst has no cache");

    let mut o = sync(src.clone(), dst.clone());
    o.common.backup = BackupPolicy {
        disabled: false,
        retain: NonZeroUsize::new(1),
    };
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    assert_eq!(
        siblings(&src, "girpr-cache-backup-"),
        vec!["girpr-cache-backup-20240102T000000.000000Z".to_string()],
        "the oldest copy beside src's cache is gone"
    );
    assert_eq!(
        siblings(&dst, "girpr-cache-old-"),
        vec!["girpr-cache-old-20240102T000000.000000Z".to_string()],
        "and the oldest snapshot beside dst's cache is gone"
    );
}

/// `--no-backup` skips the copies; it does not skip the bound.
///
/// The two are separate questions: the copies are what this run leaves
/// behind, and the bound is what the folder keeps. A run that copies nothing
/// aside still has to leave the folder within the bound the user asked for.
#[test]
fn run_sync_with_no_backup_still_bounds_the_copies() {
    let t = temp_root("retain_nobak");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"one");
    // A cache, so a run that copied aside would have copied this one.
    let mut up = update(src.clone());
    up.common.backup = BackupPolicy {
        disabled: true,
        retain: None,
    };
    cmd_update(up, &log()).unwrap();
    assert!(src.join(CACHE_PREFIX).exists(), "src has a cache");
    wfile(&src, "girpr-cache-backup-20240101T000000.000000Z", b"old");
    wfile(&src, "girpr-cache-backup-20240102T000000.000000Z", b"newer");

    let mut o = sync(src.clone(), dst.clone());
    o.common.backup = BackupPolicy {
        disabled: true,
        retain: NonZeroUsize::new(1),
    };
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    assert_eq!(
        siblings(&src, "girpr-cache-backup-"),
        vec!["girpr-cache-backup-20240102T000000.000000Z".to_string()],
        "the bound holds even though this run copied nothing aside"
    );
}

/// A dry run writes nothing, and a bound is a removal.
#[test]
fn run_sync_dry_run_bounds_nothing() {
    let t = temp_root("retain_dry");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"one");
    wfile(&src, "girpr-cache-backup-20240101T000000.000000Z", b"old");
    wfile(&src, "girpr-cache-backup-20240102T000000.000000Z", b"newer");

    let mut o = sync_dry(src.clone(), dst.clone());
    o.common.backup = BackupPolicy {
        disabled: false,
        retain: NonZeroUsize::new(1),
    };
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    assert_eq!(
        siblings(&src, "girpr-cache-backup-").len(),
        2,
        "a dry run removes nothing"
    );
}

/// A dry run writes nothing, and a copy is a write.
#[test]
fn run_sync_dry_run_leaves_no_copy() {
    let t = temp_root("drycopy");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"one");
    wfile(&dst, "a.txt", b"two");
    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(dst.clone()), &log()).unwrap();

    assert_eq!(
        cmd_sync(sync_dry(src.clone(), dst.clone()), &log()).unwrap(),
        0
    );

    for dir in [&src, &dst] {
        assert!(
            siblings(dir, "girpr-cache-backup-").is_empty(),
            "{} keeps no copy",
            dir.display()
        );
        assert!(
            siblings(dir, "girpr-cache-old-").is_empty(),
            "{} keeps no snapshot",
            dir.display()
        );
    }
}

/// The copy is taken **before** the cache is removed, so the copy holds the
/// cache as it was and the live cache is a new one.
///
/// The order is what makes the copy worth having: a copy taken after the
/// removal would be a copy of the rebuilt cache, which is the thing the run
/// could always make again.
#[test]
fn run_sync_copies_the_cache_before_ignore_cache_removes_it() {
    let t = temp_root("copyorder");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"one");
    wfile(&dst, "a.txt", b"one");
    sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));
    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(dst.clone()), &log()).unwrap();
    // A digest that is wrong but the right length, so it is stored as it is:
    // the row's stat still matches the file, so a run that keeps the cache
    // trusts it and leaves it alone.
    common::poison_digest(&src, "a.txt", "md5");
    let planted = md5_in(&src, "a.txt");
    assert!(
        planted.iter().all(|b| *b == 0xAB),
        "the planted digest is in the cache"
    );

    let mut o = sync(src.clone(), dst.clone());
    o.common.ignore_cache = true;
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    let copies = siblings(&src, "girpr-cache-backup-");
    assert_eq!(
        copies.len(),
        1,
        "the cache was copied before it was removed"
    );
    assert_eq!(
        md5_in_file(&src.join(&copies[0]), "a.txt"),
        planted,
        "the copy holds the cache as it was before the run"
    );
    assert_ne!(
        md5_in(&src, "a.txt"),
        planted,
        "the live cache was removed and rebuilt from the disk"
    );
}

/// `--ignore-cache` still removes the cache when the copy is skipped: the
/// flag is about the copy, not about the removal.
///
/// The removal is what the planted digest shows — a cache that was removed
/// and rebuilt does not hold what the old one held.
#[test]
fn run_sync_with_no_backup_still_removes_the_cache() {
    let t = temp_root("nobakignore");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"one");
    wfile(&dst, "a.txt", b"one");
    sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));
    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(dst.clone()), &log()).unwrap();
    common::poison_digest(&src, "a.txt", "md5");
    let planted = md5_in(&src, "a.txt");

    let mut o = sync(src.clone(), dst.clone());
    o.common.backup = BackupPolicy {
        disabled: true,
        retain: None,
    };
    o.common.ignore_cache = true;
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    assert!(
        siblings(&src, "girpr-cache-backup-").is_empty(),
        "no copy was made"
    );
    assert!(
        siblings(&dst, "girpr-cache-old-").is_empty(),
        "no snapshot was made"
    );
    assert_ne!(
        md5_in(&src, "a.txt"),
        planted,
        "the cache was removed and rebuilt from the disk anyway"
    );
}

/// `--backup-retain 0` is refused, and the refusal names the flag.
///
/// Keeping none of the copies is what `--no-backup` is for, so a bound of
/// zero is a contradiction rather than a request — and it is a runtime usage
/// mistake, which is exit `3` rather than the `2` a parse error gets.
#[test]
fn run_backup_retain_zero_is_refused() {
    let t = temp_root("retain0");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"one");
    wfile(&dst, "a.txt", b"two");

    let (code, stderr) = common::run_error_text([
        "sync",
        "--src",
        &src.display().to_string(),
        "--dst",
        &dst.display().to_string(),
        "--backup-retain",
        "0",
    ]);
    assert_eq!(code, 3, "a runtime usage mistake exits 3: {stderr}");
    assert!(
        stderr.contains("--backup-retain"),
        "the message names the flag: {stderr}"
    );
}
