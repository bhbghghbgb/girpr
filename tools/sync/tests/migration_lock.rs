//! Backend-agnostic behavior lock for the sled -> redb migration.
//!
//! These tests use only the public `cmd_*` API plus the public cache helpers
//! (`open_db` / `load_all_records`), never the backend directly, so they must
//! pass unchanged (modulo backend-specific assertions, which are deliberately
//! avoided here) before AND after the migration.
//!
//! Deliberately avoided here:
//! - asserting the cache path is a dir vs a file (sled dir -> redb file),
//! - asserting exact hash encodings (hex today, raw bytes after).
//!   Hash expectations are always derived from `hash_file`, which moves with
//!   the codebase.

mod common;

use common::{TempRoot, compare, log, sync, update, wfile};
use girsync::cache::{CACHE_PREFIX, get_rec, load_all_records, open_db, put_rec};
use girsync::hash::hash_file;
use girsync::{CommonOpts, cmd_compare, cmd_sync, cmd_update};
use std::collections::HashMap;

fn md5s() -> Vec<String> {
    vec!["md5".to_string()]
}

fn sha256s() -> Vec<String> {
    vec!["sha256".to_string()]
}

fn both() -> Vec<String> {
    vec!["md5".to_string(), "sha256".to_string()]
}

fn with_algos(dir: std::path::PathBuf, algos: Vec<String>) -> girsync::UpdateOpts {
    girsync::UpdateOpts {
        dir,
        common: CommonOpts {
            algos,
            ..common::opts()
        },
    }
}

fn recs(dir: &std::path::Path) -> HashMap<String, girsync::cache::FileRec> {
    let db = open_db(&dir.join(CACHE_PREFIX), true, false, false).unwrap();
    load_all_records(&db).unwrap()
}

/// Cache path exists after update (dir today, file after migration: only
/// assert existence), and files + dirs are recorded with hashes.
#[test]
fn lock_update_records_files_dirs_and_hashes() {
    let t = TempRoot::new("lock_basic");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.txt", b"world");
    std::fs::create_dir_all(dir.join("empty")).unwrap();

    assert_eq!(cmd_update(update(dir.clone()), &log()).unwrap(), 0);
    assert!(dir.join(CACHE_PREFIX).exists(), "update creates the cache");

    let r = recs(&dir);
    assert!(r.contains_key("a.txt"));
    assert!(r.contains_key("sub/b.txt"));
    assert!(r.contains_key("empty"), "empty dirs are presence-only rows");
    assert_eq!(r["empty"].kind, "dir");
    assert_eq!(r["a.txt"].kind, "file");

    let want = hash_file(&dir.join("a.txt"), &md5s()).unwrap();
    assert_eq!(r["a.txt"].hashes, want, "cached hash matches hash_file");
    assert!(!r["a.txt"].hashes.is_empty());
    assert!(r["empty"].hashes.is_empty());
}

/// A second update with identical content keeps identical cache values
/// (the cache-hit path reuses entries instead of rewriting them).
#[test]
fn lock_update_is_idempotent() {
    let t = TempRoot::new("lock_idem");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"stable content");

    cmd_update(update(dir.clone()), &log()).unwrap();
    let first = recs(&dir);
    cmd_update(update(dir.clone()), &log()).unwrap();
    let second = recs(&dir);

    assert_eq!(first.len(), second.len());
    for (k, v) in &first {
        let w = &second[k];
        assert_eq!(v.kind, w.kind, "{k}");
        assert_eq!(v.size, w.size, "{k}");
        assert_eq!(v.mtime_ns, w.mtime_ns, "{k}");
        assert_eq!(v.hashes, w.hashes, "{k}");
    }
}

/// Updating with an additional algorithm backfills it; both digests then
/// match a fresh `hash_file` call.
#[test]
fn lock_multi_algo_backfill() {
    let t = TempRoot::new("lock_algos");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"digest me");

    cmd_update(with_algos(dir.clone(), md5s()), &log()).unwrap();
    assert!(recs(&dir)["a.txt"].hashes.contains_key("md5"));

    cmd_update(with_algos(dir.clone(), both()), &log()).unwrap();
    let r = recs(&dir);
    let got = &r["a.txt"].hashes;
    assert!(got.contains_key("md5"));
    assert!(got.contains_key("sha256"));
    assert_eq!(
        *got,
        hash_file(&dir.join("a.txt"), &both()).unwrap(),
        "backfilled hashes match fresh digests"
    );
}

/// A cache write never drops a digest it did not recompute, unless the file's
/// stat changed.
///
/// The write shape keys on the stat comparison alone. Every `update` call also
/// exercises `no_trust_cached_hashes` (update always sets it), so these cover
/// both inputs to the decision: which algos were requested, and whether the
/// stat still matches.
#[test]
fn lock_cache_writes_merge_unless_stat_changed() {
    let t = TempRoot::new("lock_merge");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"digest me");
    wfile(&dir, "sub/b.txt", b"nested payload");
    let full = hash_file(&dir.join("a.txt"), &both()).unwrap();

    // Baseline: every algorithm on the row.
    cmd_update(with_algos(dir.clone(), both()), &log()).unwrap();
    assert_eq!(recs(&dir)["a.txt"].hashes, full);

    // stat unchanged + `--hash none` -> nothing recomputed, nothing lost.
    cmd_update(with_algos(dir.clone(), vec![]), &log()).unwrap();
    assert_eq!(
        recs(&dir)["a.txt"].hashes,
        full,
        "--hash none must not destroy existing digests"
    );

    // stat unchanged + narrower --hash -> unrequested algo rides along.
    cmd_update(with_algos(dir.clone(), md5s()), &log()).unwrap();
    let kept = recs(&dir)["a.txt"].hashes.clone();
    assert!(
        kept.contains_key("sha256"),
        "unrequested sha256 survives an md5-only update"
    );
    assert_eq!(kept["md5"], full["md5"]);

    // stat changed -> every stored digest is suspect, so all are dropped and
    // only the algos requested this run are kept.
    wfile(&dir, "a.txt", b"different bytes entirely");
    cmd_update(with_algos(dir.clone(), md5s()), &log()).unwrap();
    let after = &recs(&dir)["a.txt"].hashes;
    assert_eq!(
        after.len(),
        1,
        "a stat change drops digests the run did not recompute, got {:?}",
        after.keys().collect::<Vec<_>>()
    );
    assert!(
        after.contains_key("md5"),
        "the requested algo is present after the drop"
    );
    assert_eq!(
        *after,
        hash_file(&dir.join("a.txt"), &md5s()).unwrap(),
        "recomputed digest still matches the file"
    );
    // The untouched sibling proves this is per-path, not a whole-cache wipe.
    assert_eq!(
        recs(&dir)["sub/b.txt"].hashes,
        hash_file(&dir.join("sub/b.txt"), &both()).unwrap(),
        "unchanged path keeps its digests"
    );
}

/// `--hash none` records stat rows with no hashes, and they still compare
/// equal and sync cleanly.
#[test]
fn lock_hash_none_tracks_without_digests() {
    let t = TempRoot::new("lock_none");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"same bytes");
    wfile(&dst, "a.txt", b"same bytes");
    // Same size but fresh writes differ by mtime; pin it so size+mtime
    // equality (the only signal under --hash none) holds.
    common::sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));

    let none_opts = |d: std::path::PathBuf| girsync::UpdateOpts {
        dir: d,
        common: CommonOpts {
            algos: vec![],
            ..common::opts()
        },
    };
    cmd_update(none_opts(src.clone()), &log()).unwrap();
    let r = recs(&src);
    assert!(
        r["a.txt"].hashes.is_empty(),
        "--hash none stores no digests"
    );
    assert_eq!(r["a.txt"].size, 10);

    // Compare + sync under --hash none converge.
    let mut c = compare(src.clone(), dst.clone());
    c.common.algos = vec![];
    // dst has no cache yet; folder side builds one lazily under --hash none.
    assert_eq!(cmd_compare(c, &log()).unwrap(), 0);

    let mut s = sync(src.clone(), dst.clone());
    s.common.algos = vec![];
    assert_eq!(cmd_sync(s, &log()).unwrap(), 0);
}

/// sha256-only sync converges and the cache holds sha256 digests.
#[test]
fn lock_sha256_sync_converges() {
    let t = TempRoot::new("lock_sha");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"sha payload v2 longer");
    wfile(&dst, "a.txt", b"old");

    let mut s = sync(src.clone(), dst.clone());
    s.common.algos = sha256s();
    assert_eq!(cmd_sync(s, &log()).unwrap(), 0);
    assert_eq!(common::rfile(&dst, "a.txt"), b"sha payload v2 longer");

    let got = recs(&dst)["a.txt"].hashes.clone();
    assert_eq!(got, hash_file(&dst.join("a.txt"), &sha256s()).unwrap());

    let mut c = compare(src.clone(), dst.clone());
    c.common.algos = sha256s();
    assert_eq!(cmd_compare(c, &log()).unwrap(), 0);
}

/// `--no-trust-cached-hashes` makes a side rehash despite a size+mtime match,
/// and it is selectable per side.
///
/// A wrong digest is planted in one side's cache while the file's size and
/// mtime still match the other side. That isolates the trust decision: the
/// planted digest can only surface as a diff if the side actually trusted the
/// cache, so exit 0 under the flag proves the rehash happened.
#[test]
fn lock_no_trust_cached_hashes_is_per_side() {
    let t = TempRoot::new("lock_notrust");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"identical bytes");
    wfile(&dst, "a.txt", b"identical bytes");
    common::sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));

    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(dst.clone()), &log()).unwrap();

    // Plant a wrong md5 for dst, leaving size/mtime intact.
    poison(&dst, "a.txt");

    // Default trusts the cache, so the planted digest surfaces as a diff.
    assert_eq!(
        cmd_compare(compare(src.clone(), dst.clone()), &log()).unwrap(),
        4
    );

    // Distrusting dst forces a rehash, which repairs the entry in place.
    let mut c = compare(src.clone(), dst.clone());
    c.trust.no_trust_dst = true;
    assert_eq!(cmd_compare(c, &log()).unwrap(), 0);

    // dst is repaired now, so re-poison and check the flag is really
    // per-side: distrusting src alone must not repair dst.
    poison(&dst, "a.txt");
    let mut c = compare(src.clone(), dst.clone());
    c.trust.no_trust_src = true;
    assert_eq!(
        cmd_compare(c, &log()).unwrap(),
        4,
        "distrusting src must not repair dst's cached digest"
    );

    // Distrusting both sides repairs both.
    poison(&src, "a.txt");
    poison(&dst, "a.txt");
    let mut c = compare(src.clone(), dst.clone());
    c.trust.no_trust_src = true;
    c.trust.no_trust_dst = true;
    assert_eq!(cmd_compare(c, &log()).unwrap(), 0);
}

/// Overwrite a cached digest with a wrong one, keeping size and mtime as-is.
fn poison(dir: &std::path::Path, rel: &str) {
    let db = open_db(&dir.join(CACHE_PREFIX), true, false, false).unwrap();
    let mut rec = get_rec(&db, rel).unwrap().expect("row exists after update");
    let real = hash_file(&dir.join(rel), &md5s()).unwrap();
    assert_eq!(real["md5"].len(), 16, "md5 digest is 16 raw bytes");
    rec.hashes.insert("md5".into(), vec![0u8; 16]);
    assert_ne!(rec.hashes["md5"], real["md5"], "planted digest differs");
    put_rec(&db, rel, &rec).unwrap();
}

/// Second update leaves a `girpr-cache-backup-*` sibling; sync leaves a
/// `girpr-cache-old-*` snapshot of dst. Both are keep-all, so one is enough.
#[test]
fn lock_backups_and_snapshots_created() {
    let t = TempRoot::new("lock_bak");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"x");
    wfile(&dst, "a.txt", b"x");

    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(src.clone()), &log()).unwrap();
    assert!(
        common::has_backup_sibling(&src.join(CACHE_PREFIX)),
        "second update backs up the old cache"
    );

    // Snapshots only fire when dst already has a cache: create one first.
    cmd_update(update(dst.clone()), &log()).unwrap();
    cmd_sync(sync(src.clone(), dst.clone()), &log()).unwrap();
    // Snapshots live next to the cache itself, i.e. inside dst.
    let has_old_next_to_dst = std::fs::read_dir(&dst)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .any(|n| n.starts_with("girpr-cache-old-"));
    assert!(
        has_old_next_to_dst,
        "sync snapshots dst pre-state next to the cache"
    );
}

/// A record side is read as-is: comparing record vs folder adds no rows to
/// the record.
///
/// The folder must be a *different* root. `compare` refuses a record paired
/// with the folder that holds it — a folder side rewrites its cache as it
/// scans, so the "before" snapshot would be repaired by the run under test and
/// the assertion would hold vacuously.
#[test]
fn lock_record_side_is_read_only() {
    let t = TempRoot::new("lock_rec");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    cmd_update(update(dir.clone()), &log()).unwrap();

    // Same content, different root.
    let live = t.mkdirs("live");
    wfile(&live, "a.txt", b"hello");
    common::sync_mtime(&dir.join("a.txt"), &live.join("a.txt"));

    let record = dir.join(CACHE_PREFIX);
    let before = recs(&dir);
    let code = cmd_compare(compare(record, live), &log()).unwrap();
    assert_eq!(code, 0);
    let after = recs(&dir);
    assert_eq!(before.len(), after.len(), "record compare writes no rows");
    for (k, v) in &before {
        assert_eq!(v.hashes, after[k].hashes);
    }
}

/// Dry-run sync changes no disk bytes, creates no backups, and leaves the
/// caches' logical content identical.
#[test]
fn lock_dry_run_changes_nothing() {
    let t = TempRoot::new("lock_dry");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"new content here");
    wfile(&dst, "a.txt", b"old");
    // Pre-create both caches so dry-run has no reason to create anything.
    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(dst.clone()), &log()).unwrap();
    let src_before = recs(&src);
    let dst_before = recs(&dst);

    let mut o = sync(src.clone(), dst.clone());
    o.dry_run = true;
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    assert_eq!(common::rfile(&dst, "a.txt"), b"old");
    assert!(
        !common::has_backup_sibling(&dst.join(CACHE_PREFIX)),
        "dry-run makes no backups"
    );
    assert_eq!(recs(&src).len(), src_before.len());
    assert_eq!(recs(&dst).len(), dst_before.len());
}

/// Empty dirs and nesting survive a full sync and compare equal after.
#[test]
fn lock_empty_dirs_and_nesting_sync() {
    let t = TempRoot::new("lock_dirs");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a/b/c/deep.txt", b"deep");
    std::fs::create_dir_all(src.join("a/empty1")).unwrap();
    std::fs::create_dir_all(src.join("lonely")).unwrap();

    assert_eq!(cmd_sync(sync(src.clone(), dst.clone()), &log()).unwrap(), 0);
    assert!(dst.join("a/empty1").is_dir());
    assert!(dst.join("lonely").is_dir());
    assert_eq!(common::rfile(&dst, "a/b/c/deep.txt"), b"deep");
    assert_eq!(cmd_compare(compare(src, dst), &log()).unwrap(), 0);
}
