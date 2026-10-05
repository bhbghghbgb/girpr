//! End-to-end runs of `cmd_compare_self` (real FS + real cache).

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use common::{TempRoot, log, rw, update, wfile};
use girsync::cache::{CACHE_PREFIX, FileRec, load_all_records, open_db};
use girsync::{CommonOpts, CompareSelfOpts, cmd_compare_self, cmd_update};

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

/// `--no-trust-cached-hashes` used to be the *only* way to see that drift: the
/// default audit served the disk side from the record's own cache, so a preserved
/// stat meant the record's digest was compared with itself and always agreed.
///
/// The disk side is now cache-free, so the flag has nothing left to change. It
/// stays on the CLI because a script passing it everywhere must keep working —
/// which is a claim about *accepting* it, so that is what this asserts: same
/// verdict either way, and still no writes.
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

/// Case-only pairing, insensitive mode. The disk side no longer adopts the
/// record's alternate-cased row, so `Case.txt` on disk and `case.txt` in the
/// record are two independent sides of a pair the planner matches by lowercase.
///
/// The content differs at the same length and the mtime is restored, so size and
/// mtime agree and only a digest can decide: the pair is both a `CASE-MISMATCH`
/// *and* `CHANGED`, from the one `diff_maps` pass. Before the disk side was made
/// cache-free it settled this pair from the record's own md5, so only the
/// `CASE-MISMATCH` could ever appear — which is why this uses different bytes
/// rather than the identical bytes every other case-only fixture in the suite
/// uses.
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
