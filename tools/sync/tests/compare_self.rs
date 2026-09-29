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

/// A file whose size+mtime still match keeps its recorded digest, so the default
/// is a stat-level check. Restoring both hides a content change from it;
/// `--no-trust-cached-hashes` is the only thing that surfaces it.
#[test]
fn run_compare_self_no_trust_cached_hashes_surfaces_hidden_content_drift() {
    let t = TempRoot::new("cs_notrust");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    cmd_update(update(dir.clone()), &log()).unwrap();
    let before = recs(&dir);

    // Same length, mtime put back: nothing but the bytes differ.
    wfile(&dir, "a.txt", b"world");
    restore_recorded_mtime(&dir, "a.txt");

    assert_eq!(
        cmd_compare_self(self_compare(dir.clone()), &log()).unwrap(),
        0,
        "stat still matches, so the default sees nothing"
    );

    let mut o = self_compare(dir.clone());
    o.no_trust_cached_hashes = true;
    assert_eq!(
        cmd_compare_self(o, &log()).unwrap(),
        4,
        "a rehash sees the content change"
    );
    assert_eq!(recs(&dir), before, "the rehash was not backfilled");
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
