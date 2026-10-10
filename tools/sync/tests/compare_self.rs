//! End-to-end runs of `cmd_compare_self` (real FS + real cache).

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use common::{TempRoot, age, log, rw, strip_algo, sync, sync_mtime, update, wfile};
use girsync::cache::{CACHE_PREFIX, FileRec, load_all_records, open_db};
use girsync::{CommonOpts, CompareSelfOpts, cmd_compare_self, cmd_sync, cmd_update};

fn recs(dir: &Path) -> HashMap<String, FileRec> {
    load_all_records(&open_db(&dir.join(CACHE_PREFIX), true, rw()).unwrap()).unwrap()
}

fn self_compare(dir: PathBuf) -> CompareSelfOpts {
    CompareSelfOpts {
        dir,
        no_trust_cached_hashes: false,
        common: common::opts(),
    }
}

/// Re-pin a file's mtime to the stamp the cache recorded, so size+mtime match
/// again after a rewrite of the same length.
fn restore_recorded_mtime(dir: &Path, rel: &str) {
    let want = recs(dir)[rel].mtime_ns;
    let stamp = std::time::UNIX_EPOCH + std::time::Duration::from_nanos(want as u64);
    std::fs::OpenOptions::new()
        .write(true)
        .open(dir.join(rel))
        .unwrap()
        .set_modified(stamp)
        .unwrap();
}

#[test]
fn run_compare_self_is_clean_after_update() {
    let t = TempRoot::new("cs_clean");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.txt", b"world");
    cmd_update(update(dir.clone()), &log()).unwrap();

    assert_eq!(cmd_compare_self(self_compare(dir), &log()).unwrap(), 0);
}

#[test]
fn run_compare_self_reports_drift_with_record_as_src() {
    let t = TempRoot::new("cs_drift");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.txt", b"world");
    std::fs::create_dir_all(dir.join("empty")).unwrap();
    cmd_update(update(dir.clone()), &log()).unwrap();

    // A new file and a deleted one. The record is the src side, so the path it
    // never learned is EXTRA and the one it still holds is MISSING.
    wfile(&dir, "added.txt", b"new");
    std::fs::remove_file(dir.join("sub/b.txt")).unwrap();

    assert_eq!(cmd_compare_self(self_compare(dir), &log()).unwrap(), 4);
}

/// The whole point of the command: reporting drift must not repair it. A plain
/// `compare` of a record against its own folder used to do exactly that, so the
/// cache came back clean and the next run had nothing left to report.
#[test]
fn run_compare_self_leaves_the_cache_exactly_as_it_found_it() {
    let t = TempRoot::new("cs_nowrite");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    cmd_update(update(dir.clone()), &log()).unwrap();
    let before = recs(&dir);

    wfile(&dir, "added.txt", b"new");
    std::fs::remove_file(dir.join("a.txt")).unwrap();
    assert_eq!(
        cmd_compare_self(self_compare(dir.clone()), &log()).unwrap(),
        4
    );

    assert_eq!(recs(&dir), before, "cache untouched");
    // Reporting it a second time must report it again, not find it fixed.
    assert_eq!(
        cmd_compare_self(self_compare(dir.clone()), &log()).unwrap(),
        4,
        "the same drift is still there on a rerun"
    );
    assert_eq!(recs(&dir), before);
}

/// A file whose size+mtime still match used to keep its recorded digest on *both*
/// sides of the audit — the disk side read the record's own cache — so the content
/// comparison was the record compared with itself and this reported "in step".
///
/// Same length, mtime put back: nothing but the bytes differ, and no stat can
/// tell. This is the pair the command exists for, and it is why the disk side is
/// now scanned against an empty handle rather than the record's.
#[test]
fn hidden_content_drift_is_reported_without_a_flag() {
    let t = TempRoot::new("cs_hidden");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    cmd_update(update(dir.clone()), &log()).unwrap();
    let before = recs(&dir);

    wfile(&dir, "a.txt", b"world");
    restore_recorded_mtime(&dir, "a.txt");

    assert_eq!(
        cmd_compare_self(self_compare(dir.clone()), &log()).unwrap(),
        4,
        "the audit must read the folder, not re-read the record"
    );
    assert_eq!(recs(&dir), before, "reading it changes nothing");
}

/// The disk side is cache-free, so a preserved stat means the file is rehashed and
/// that drift is already reported — `--no-trust-cached-hashes` has nothing left to
/// change.
///
/// The flag stays on the CLI because a script passing it everywhere must keep
/// working, which is a claim about *accepting* it, so that is what this asserts:
/// same verdict either way, and still no writes.
#[test]
fn the_no_trust_flag_is_redundant_here_rather_than_a_second_mode() {
    let t = TempRoot::new("cs_flag");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    cmd_update(update(dir.clone()), &log()).unwrap();
    let before = recs(&dir);
    wfile(&dir, "a.txt", b"world");
    restore_recorded_mtime(&dir, "a.txt");

    let mut o = self_compare(dir.clone());
    o.no_trust_cached_hashes = true;
    assert_eq!(
        cmd_compare_self(o, &log()).unwrap(),
        4,
        "accepted, and the same answer the default gives"
    );
    assert_eq!(recs(&dir), before, "still writes nothing");
}

/// The records the real binary reported for a `compare-self` run.
///
/// `cmd_compare_self` returns an exit code, so the records *are* the result, and a
/// case that cares which ones appeared has to read them from somewhere. The binary
/// also differs from the in-process default in one way that matters here:
/// `--case-sensitive` defaults to **off**, so this is insensitive mode.
fn records(dir: &Path) -> Vec<serde_json::Value> {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .arg("compare-self")
        .arg("--dir")
        .arg(dir)
        .arg("--output")
        .arg("json")
        .output()
        .expect("spawn girsync");
    // Exit 4 is the *answer* here, not a failure: it means "the cache has drifted",
    // which is exactly what these fixtures arrange. Only 0 and 4 are success here;
    // anything else is the command refusing to run (3), and stdout is then not a
    // report at all.
    assert!(
        matches!(out.status.code(), Some(0) | Some(4)),
        "compare-self failed: {}\n{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    common::parse_ndjson(&out.stdout)
}

/// The `path` of every `event`, sorted, so an assertion names what was reported
/// without restating the report order.
fn paths(recs: &[serde_json::Value], event: &str) -> Vec<String> {
    let mut v: Vec<String> = recs
        .iter()
        .filter(|r| r["event"] == event)
        .map(|r| r["path"].as_str().unwrap_or_default().to_string())
        .collect();
    v.sort();
    v
}

/// Case-only pairing, insensitive mode. The disk side does not adopt the record's
/// alternate-cased row, so `Case.txt` on disk and `case.txt` in the record are two
/// independent sides of a pair the planner matches by lowercase.
///
/// The content differs at the same length and the mtime is restored, so size and
/// mtime agree and only a digest can decide: the pair is both a `CASE-MISMATCH`
/// *and* `CHANGED`, from the one `diff_maps` pass. Which is why this uses different
/// bytes rather than the identical bytes every other case-only fixture in the
/// suite uses — identical bytes would make the digest agree and prove nothing about
/// the `CHANGED` half.
#[test]
fn a_case_only_pair_with_differing_content_is_changed_as_well_as_case_mismatched() {
    let t = TempRoot::new("cs_case");
    let dir = t.mkdirs("w");
    wfile(&dir, "case.txt", b"payload");
    cmd_update(update(dir.clone()), &log()).unwrap();
    let before = recs(&dir);

    // A rename on disk plus a same-length rewrite, with the stat put back, so the
    // pair is undecided and the digest is the only thing that can settle it.
    std::fs::rename(dir.join("case.txt"), dir.join("Case.txt")).unwrap();
    wfile(&dir, "Case.txt", b"PAYLOAD");
    restore_recorded_mtime(&dir, "case.txt");

    let recs_out = records(&dir);
    let mismatch: Vec<_> = recs_out
        .iter()
        .filter(|r| r["event"] == "case-mismatch")
        .map(|r| (r["src"].as_str().unwrap(), r["dst"].as_str().unwrap()))
        .collect();
    assert_eq!(
        mismatch,
        vec![("case.txt", "Case.txt")],
        "the casing difference is still reported: {recs_out:?}"
    );
    assert_eq!(
        paths(&recs_out, "changed"),
        vec!["case.txt"],
        "and so is the content disagreement, which only a digest can see"
    );
    assert_eq!(recs(&dir), before, "still writes nothing");
}

/// `--hash none` still means "decide by size+mtime", and it is now the only cheap
/// way to run this audit — an in-step folder is otherwise read in full, because
/// every one of its pairs is undecided.
///
/// The contrast with `hidden_content_drift_is_reported_without_a_flag` is the
/// point: on the same fixture the default reports `CHANGED` and this reports
/// clean, and both are correct, because `--hash none` is a request for a
/// stat-only audit rather than a weaker version of the same one.
#[test]
fn hash_none_is_a_stat_only_audit_and_still_runs() {
    let t = TempRoot::new("cs_nonone");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.txt", b"world");
    cmd_update(update(dir.clone()), &log()).unwrap();
    let before = recs(&dir);

    // Content drift under a preserved stat: invisible to a stat-only audit.
    wfile(&dir, "a.txt", b"world");
    restore_recorded_mtime(&dir, "a.txt");

    let mut o = self_compare(dir.clone());
    o.common = CommonOpts {
        algos: vec![],
        ..common::opts()
    };
    assert_eq!(
        cmd_compare_self(o, &log()).unwrap(),
        0,
        "size and mtime agree, which is the whole of what was asked for"
    );

    // And it still sees the drift it is meant to see.
    wfile(&dir, "a.txt", b"a much longer body");
    assert_eq!(
        cmd_compare_self(self_compare(dir.clone()), &log()).unwrap(),
        4,
        "a size difference needs no digest either way"
    );
    assert_eq!(recs(&dir), before, "still writes nothing");
}

/// A record that cannot supply a digest the run asked for must **fail**, not
/// degrade.
///
/// The degradation is the dangerous part and it is silent: `hashes_differ` skips
/// any algorithm either side lacks, so an uncovered pair falls back to the size
/// and mtime that already agreed, and the audit reports a confident "in step"
/// about content it never read. With the disk side cache-free that silence is not
/// masked by a tautology, so it is visible — and visible as a *wrong answer*,
/// which is worse than an error.
#[test]
fn an_uncovered_record_fails_rather_than_degrading_to_size_and_mtime() {
    let t = TempRoot::new("cs_cover");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    cmd_update(update(dir.clone()), &log()).unwrap();
    strip_algo(&dir, "a.txt", "md5");

    let err = cmd_compare_self(self_compare(dir.clone()), &log()).unwrap_err();
    let msg = format!("{:#}", err);
    // Four signals, or the user cannot act: which record, how many paths,
    // one example path with the algorithm it lacks, per-algorithm coverage, and
    // a remedy.
    for (what, needle) in [
        ("the record under audit", "girpr-cache"),
        ("an example path", "a.txt"),
        ("the algorithm it lacks", "md5"),
        ("a remedy", "girsync update"),
    ] {
        assert!(msg.contains(needle), "message should name {what}:\n{msg}");
    }
    assert!(
        msg.contains("0/1"),
        "per-algorithm coverage, and it says zero covered:\n{msg}"
    );
}

/// **Scoped to the undecided set**, which is the rule that keeps this check from
/// failing runs that are perfectly answerable.
///
/// The row is stripped of its digest *and* the file is aged, so the pair differs
/// in stat — which means size and mtime have already decided it and no digest was
/// ever required. A whole-record preflight would fail here; the coverage
/// requirement is a property of a (path, side) pair inside the undecided set, never
/// of the record as a whole.
#[test]
fn a_stat_differing_pair_needs_no_coverage_so_a_stripped_row_is_just_changed() {
    let t = TempRoot::new("cs_cover_differ");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "b.txt", b"world");
    cmd_update(update(dir.clone()), &log()).unwrap();
    // `b.txt` loses its digest *and* its stat, so it is already decided.
    strip_algo(&dir, "b.txt", "md5");
    age(&dir, "b.txt", 60);

    assert_eq!(
        cmd_compare_self(self_compare(dir.clone()), &log()).unwrap(),
        4,
        "b.txt is CHANGED by stat, so the missing digest is not in question"
    );
    assert_eq!(
        recs(&dir)["b.txt"].hashes.len(),
        0,
        "and the row is untouched"
    );
}

/// **The failure mode is reachable by an ordinary workflow, and this is the route.**
///
/// `update` the folder, let a file appear, then `sync` the folder *as src*. The
/// sync's phase A has no cached row for the newcomer, so it records the current
/// stat and **no digest** — correctly, because the pair was decided by presence
/// and needed no digest. The row is now stat-only and *fresh*, so nothing will
/// ever fill it in on its own. The next `compare-self` finds a stat-equal,
/// undecided pair on a record that cannot answer it.
///
/// Before the coverage rule this exited **0**: `hashes_differ` skips an algorithm
/// either side lacks, so the pair fell back to the size+mtime that already agreed
/// and the audit reported the newcomer as in step. That is the whole false-clean
/// this check exists to stop, so the test uses a real `sync` rather than
/// `strip_algo` — a hand-stripped digest is a hypothetical, a stat-only row
/// written by a lazy scan is Tuesday.
#[test]
fn a_lazy_sync_can_leave_a_record_that_cannot_answer_and_that_is_now_an_error() {
    let t = TempRoot::new("cs_lazy_route");
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

    // The row exists, carries the file's current stat, and has no digest. Fresh,
    // because it was taken from disk: this is not a stale entry.
    let late = &recs(&src)["late.txt"];
    assert!(
        late.hashes.is_empty(),
        "phase A recorded stat only: {late:?}"
    );
    assert_eq!(late.size, 12, "and the stat it recorded is the file's own");

    let err = cmd_compare_self(self_compare(src.clone()), &log())
        .expect_err("the audit cannot answer for the newcomer, and says so");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("late.txt"),
        "names the one uncovered path:\n{msg}"
    );
    assert!(
        msg.contains("girsync update --dir"),
        "and the remedy is one command:\n{msg}"
    );
    assert!(
        msg.contains(&src.display().to_string()),
        "and the label resolves to the folder to run it on:\n{msg}"
    );
    // And the covered paths are counted, so the user can see this is 1 of N
    // rather than a whole broken record.
    assert!(msg.contains("2/3"), "per-algorithm coverage:\n{msg}");

    // The remedy works, and this is the point of the test: one command, and the
    // audit answers again.
    cmd_update(update(src.clone()), &log()).unwrap();
    assert_eq!(
        cmd_compare_self(self_compare(src), &log()).unwrap(),
        0,
        "after update the record covers every undecided pair"
    );
}

/// `--hash none` is a request for a stat-only audit, not a coverage failure — so
/// the same stripped record that fails above passes here.
///
/// This needs no special case in the planner: an empty request means there is
/// nothing to cover, and the early return for `--hash none` is reached before the
/// coverage check could fire. The test is here because "no special case" is the
/// kind of claim that stops being true without one.
#[test]
fn hash_none_is_not_a_coverage_failure() {
    let t = TempRoot::new("cs_cover_none");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    cmd_update(update(dir.clone()), &log()).unwrap();
    strip_algo(&dir, "a.txt", "md5");

    let mut o = self_compare(dir.clone());
    o.common = CommonOpts {
        algos: vec![],
        ..common::opts()
    };
    assert_eq!(
        cmd_compare_self(o, &log()).unwrap(),
        0,
        "nothing was asked for, so nothing is uncovered"
    );
}

#[test]
fn run_compare_self_rejects_a_missing_cache_and_ignore_cache() {
    let t = TempRoot::new("cs_reject");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");

    // No cache yet: an audit with no subject, and creating one would make the
    // first run vacuously clean.
    let err = cmd_compare_self(self_compare(dir.clone()), &log()).unwrap_err();
    assert!(
        format!("{:#}", err).contains("update"),
        "the error should say how to make one: {:#}",
        err
    );
    assert!(!dir.join(CACHE_PREFIX).exists(), "nothing was created");

    cmd_update(update(dir.clone()), &log()).unwrap();

    // --ignore-cache would delete the record under audit.
    let mut o = self_compare(dir.clone());
    o.common = CommonOpts {
        ignore_cache: true,
        ..common::opts()
    };
    assert!(cmd_compare_self(o, &log()).is_err());
    assert!(dir.join(CACHE_PREFIX).exists(), "the cache survived");

    // Not a folder.
    let file = t.mkdirs("afile");
    wfile(&file, "x.txt", b"x");
    assert!(cmd_compare_self(self_compare(file.join("x.txt")), &log()).is_err());
}
