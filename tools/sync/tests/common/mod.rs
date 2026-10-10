//! Shared fixtures for the integration tests.
//!
//! Each test binary gets its own copy of this module, so not every helper is
//! used in every binary.

#![allow(dead_code)]

use girsync::cache::{CACHE_PREFIX, CacheOpen, FileRec, load_all_records, open_db};
use girsync::effective::{SideScan, classify, ensure_distinct_sides, open_side, resolve_side};
use girsync::planner::{HashMode, SideRequest, StatTrust, plan_pairs};
use girsync::report::OutputFormat;
use girsync::rw::{RwRuntime, RwSide};
use girsync::{
    CommonOpts, CompareOpts, CompareSelfOpts, LogCtx, ScanMode, SyncOpts, TrustOpts, UpdateOpts,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static RUN_SEQ: AtomicU64 = AtomicU64::new(0);

/// The plain read/write open most tests want: no rebuild, no backup.
pub fn rw() -> CacheOpen {
    CacheOpen::ReadWrite {
        ignore_cache: false,
        backup_first: false,
    }
}

/// Isolated temp directory per test, so parallel `cargo test` workers never
/// share a cache file.
pub struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    pub fn new(tag: &str) -> Self {
        let n = RUN_SEQ.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "girsync_run_{}_{}_{}_{}",
            std::process::id(),
            nanos,
            n,
            tag
        ));
        std::fs::remove_dir_all(&path).ok();
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub fn mkdirs(&self, rel: &str) -> PathBuf {
        let p = self.path.join(rel);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    pub fn root(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        // Cache handles opened by cmd_* are dropped by then; best-effort cleanup.
        std::fs::remove_dir_all(&self.path).ok();
    }
}

pub fn wfile(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, bytes).unwrap();
}

pub fn rfile(root: &Path, rel: &str) -> Vec<u8> {
    std::fs::read(root.join(rel)).unwrap()
}

/// Actual directory entry names, excluding `girpr-cache*`.
///
/// Path existence cannot answer casing questions on a case-insensitive
/// filesystem: `data.txt` and `Data.txt` resolve to the same entry there.
pub fn entry_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with("girpr-cache"))
        .collect();
    names.sort();
    names
}

/// Pin `target`'s mtime to `source`'s, so two identical-content files compare
/// equal (compare treats size+mtime+hash as identity).
pub fn sync_mtime(source: &Path, target: &Path) {
    let mtime = std::fs::metadata(source).unwrap().modified().unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(target)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
}

/// Default options for the end-to-end tests: md5, no filters, case-sensitive.
pub fn opts() -> CommonOpts {
    CommonOpts {
        algos: vec!["md5".to_string()],
        hash_mode: HashMode::default(),
        stat: StatTrust::default(),
        why: false,
        show_identical: false,
        includes: vec![],
        excludes: vec![],
        case_sensitive: true,
        max_depth: 10,
        ignore_cache: false,
        dry_run: false,
    }
}

/// `opts()` with `--dry-run`, for the cases that assert a run writes nothing.
pub fn opts_dry() -> CommonOpts {
    CommonOpts {
        dry_run: true,
        ..opts()
    }
}

/// Quiet log context; tracing is a no-op without a subscriber anyway.
///
/// `output` is left at the default text format. Tests that assert on what a run
/// *reports* assert on records from `girsync::report` (or, where they need the
/// binary, on `--output json`); the format here only decides how those records
/// would have been rendered, which no in-process test observes.
pub fn log() -> LogCtx {
    LogCtx {
        level: "error".to_string(),
        file: None,
        output: OutputFormat::Text,
    }
}

/// Scan mode for tests: `no_trust_cached_hashes` and `dry_run`.
pub fn scan(no_trust_cached_hashes: bool, dry_run: bool) -> ScanMode {
    ScanMode {
        no_trust_cached_hashes,
        dry_run,
    }
}

/// The limiter a test gets when concurrency is not what it is testing: strictly
/// one rw operation at a time, which is what a command built for itself before
/// the rw flags existed.
///
/// A `RwRuntime` rather than a thread count, so a test calling `resolve_folder`
/// directly threads through exactly the same gate a command does and cannot
/// accidentally exercise a shape no command produces.
pub fn serial_rw() -> RwRuntime {
    RwRuntime::serial().unwrap()
}

pub fn update(dir: PathBuf) -> UpdateOpts {
    UpdateOpts {
        dir,
        common: opts(),
    }
}

pub fn compare(src: PathBuf, dst: PathBuf) -> CompareOpts {
    CompareOpts {
        src,
        dst,
        trust: TrustOpts::default(),
        common: opts(),
    }
}

/// [`compare`] with `--dry-run`.
pub fn compare_dry(src: PathBuf, dst: PathBuf) -> CompareOpts {
    CompareOpts {
        common: opts_dry(),
        ..compare(src, dst)
    }
}

/// [`update`] with `--dry-run`.
pub fn update_dry(dir: PathBuf) -> UpdateOpts {
    UpdateOpts {
        common: opts_dry(),
        ..update(dir)
    }
}

pub fn compare_self_opts(dir: PathBuf) -> CompareSelfOpts {
    CompareSelfOpts {
        dir,
        no_trust_cached_hashes: false,
        common: opts(),
    }
}

pub fn sync(src: PathBuf, dst: PathBuf) -> SyncOpts {
    SyncOpts {
        src,
        dst,
        trust: TrustOpts::default(),
        missing_only: false,
        keep_extra: false,
        jobs: 1,
        common: opts(),
    }
}

/// [`sync`] with `--dry-run`. The flag lives on `common`, since one flag drives
/// both the cache half and the filesystem half.
pub fn sync_dry(src: PathBuf, dst: PathBuf) -> SyncOpts {
    SyncOpts {
        common: opts_dry(),
        ..sync(src, dst)
    }
}

pub fn md5arg() -> Vec<String> {
    vec!["md5".to_string()]
}

/// True when a `girpr-cache-backup-*` sibling exists next to `dir`.
pub fn has_backup_sibling(dir: &Path) -> bool {
    let parent = dir.parent().unwrap_or_else(|| Path::new("."));
    let Ok(rd) = std::fs::read_dir(parent) else {
        return false;
    };
    let needle = format!("{}-backup-", girsync::cache::CACHE_PREFIX);
    rd.filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .any(|n| n.starts_with(&needle))
}

// ---------------------------------------------------------------------------
// Reading what a run reported
// ---------------------------------------------------------------------------

/// Parse a run's `--output json` stdout into one value per line.
///
/// This is the reader every stdout assertion in the suite goes through. It is
/// here rather than duplicated per binary because "stdout is newline-delimited
/// JSON, one record per line" is a property of the tool, and a test that parses
/// it any other way is asserting a different contract.
pub fn parse_ndjson(bytes: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8(bytes.to_vec())
        .expect("stdout is utf-8")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON: {e}\n  line: {l}")))
        .collect()
}

/// The single `summary` record a run reported.
///
/// Every command ends with exactly one, in every format, so it is the one record
/// a caller can rely on being last. Panicking on anything else is deliberate: a
/// run that reported no summary, or two, has changed its contract.
pub fn summary_of(recs: &[serde_json::Value]) -> serde_json::Value {
    let summaries: Vec<_> = recs.iter().filter(|r| r["event"] == "summary").collect();
    assert_eq!(
        summaries.len(),
        1,
        "exactly one summary record, got {summaries:?} of {}",
        recs.len()
    );
    summaries[0].clone()
}

// ---------------------------------------------------------------------------
// Fixture helpers for the lazy-resolution cases
// ---------------------------------------------------------------------------
//
// These arrange a cache state rather than hoping the binary produces it. A row
// whose stat matches and which is missing an algorithm is a state a real
// `--hash none` history leaves behind, but the binary will not create it on
// request — so the cases build it directly, through the public cache API.

/// A cache row edited in place, or a path to one.
/// Every row a side's cache currently holds, for asserting what a run left
/// behind or dropped.
pub fn recs_of(dir: &Path) -> HashMap<String, FileRec> {
    load_all_records(&open_db(&dir.join(CACHE_PREFIX), true, rw()).unwrap()).unwrap()
}

fn edit_rec<F: FnOnce(&mut FileRec)>(dir: &Path, rel: &str, f: F) {
    let p = dir.join(CACHE_PREFIX);
    let db = open_db(&p, true, rw()).unwrap();
    let mut rec = db
        .get(rel)
        .unwrap()
        .unwrap_or_else(|| panic!("no row for {rel}"));
    f(&mut rec);
    db.put(rel, &rec).unwrap();
}

/// Drop one algorithm from a row, leaving its stat intact.
pub fn strip_algo(dir: &Path, rel: &str, algo: &str) {
    edit_rec(dir, rel, |r| {
        r.hashes.remove(algo);
    });
}

/// Overwrite a digest with a wrong one of the same length, keeping size and
/// mtime. The stat still matches, so only the content disagrees — the case where
/// a digest is the only thing that can decide the pair.
pub fn poison_digest(dir: &Path, rel: &str, algo: &str) {
    edit_rec(dir, rel, |r| {
        let n = r.hashes.get(algo).map(|d| d.len()).unwrap_or(32);
        r.hashes.insert(algo.to_string(), vec![0xABu8; n]);
    });
}

/// Push a file's mtime forward, so a stat-equal pair stops being equal.
pub fn age(dir: &Path, rel: &str, secs: u64) {
    let p = dir.join(rel);
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(secs);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&p)
        .unwrap()
        .set_modified(later)
        .unwrap();
}

/// The digest rows currently hold, for asserting what a case left behind.
pub fn digests_of(dir: &Path, rel: &str) -> Vec<String> {
    let mut v: Vec<String> = recs_of(dir)[rel].hashes.keys().cloned().collect();
    v.sort();
    v
}

/// One side of a fixture path: `None` absent, `Some(&[])` a directory, anything
/// else a file's bytes.
pub type Side = Option<&'static [u8]>;

/// One fixture path: `(rel, src, dst)`. See [`Side`] for the shapes.
pub type Spec = (&'static str, Side, Side);

/// A pair of folders laid out from `spec`, each warmed with `algos`.
///
/// `None` on a side means the path is **absent** there; `Some(&[])` means an
/// **empty directory** — different shapes, and cases need both, since
/// absent-there is a `MISSING`/`EXTRA` while a directory there is a
/// `TYPE-CONFLICT`. When both sides carry content, dst's mtime is pinned to
/// src's, so the pair starts out stat-equal and a case can break exactly the
/// dimension it is about.
///
/// With `warmed: false` both caches are left cold.
pub fn pair(t: &TempRoot, spec: &[Spec], algos: &[&str], warmed: bool) -> (PathBuf, PathBuf) {
    let s = t.mkdirs("src");
    let d = t.mkdirs("dst");
    for (rel, sb, db) in spec {
        for (dir, bytes) in [(&s, *sb), (&d, *db)] {
            match bytes {
                None => continue,
                // An empty byte slice is a directory, not an empty file: a
                // fixture asking for one means it.
                Some([]) => {
                    std::fs::create_dir_all(dir.join(rel)).unwrap();
                }
                Some(b) => wfile(dir, rel, b),
            }
        }
        if let (Some(sb), Some(db)) = (sb, db)
            && !sb.is_empty()
            && !db.is_empty()
        {
            sync_mtime(&s.join(rel), &d.join(rel));
        }
    }
    if warmed {
        for dir in [&s, &d] {
            let o = with_algos(algos);
            girsync::cmd_update(
                girsync::UpdateOpts {
                    dir: dir.clone(),
                    common: o,
                },
                &log(),
            )
            .unwrap();
        }
    }
    (s, d)
}

/// Pin a case's algorithm set on an existing pair, so the caches were warmed
/// with one set and the run is asked for another.
pub fn with_algos(algos: &[&str]) -> CommonOpts {
    CommonOpts {
        algos: algos.iter().map(|a| a.to_string()).collect(),
        ..opts()
    }
}

/// The exact phase sequence `cmd_compare` runs — open both sides, one
/// `plan_pairs`, resolve both — with the per-side trust flags applied.
///
/// Driven by the tests rather than delegated to, so that a case can assert what
/// the planner decided *and* what the sides ended up reading. Every phase
/// happens here, in this order; that is the property under test.
pub fn resolve_both(src: &Path, dst: &Path, trust: TrustOpts) -> (SideScan, SideScan) {
    let common = with_algos(&["md5"]);
    let src = classify(src);
    let dst = classify(dst);
    ensure_distinct_sides(&src, &dst).unwrap();
    let mut s = open_side(
        &src,
        &common,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_src,
            dry_run: false,
        },
    )
    .unwrap();
    let mut d = open_side(
        &dst,
        &common,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_dst,
            dry_run: false,
        },
    )
    .unwrap();
    // The labels exist only so a coverage error can name a side; a fixture that
    // reaches one is a test failure, not something to read.
    let s_label = format!("src {}", src.cache_path().display());
    let d_label = format!("dst {}", dst.cache_path().display());
    let plans = plan_pairs(
        SideRequest {
            entries: &s.phase_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_src,
            cap: s.cap,
            label: &s_label,
        },
        SideRequest {
            entries: &d.phase_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_dst,
            cap: d.cap,
            label: &d_label,
        },
        common.case_sensitive,
        common.hash_mode,
        common.stat,
    )
    .unwrap();
    let sm = resolve_side(
        &mut s,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_src,
            dry_run: false,
        },
        &plans.src,
        RwSide::Src,
        &serial_rw(),
    )
    .unwrap();
    let dm = resolve_side(
        &mut d,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_dst,
            dry_run: false,
        },
        &plans.dst,
        RwSide::Dst,
        &serial_rw(),
    )
    .unwrap();
    (sm, dm)
}
