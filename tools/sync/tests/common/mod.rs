//! Shared fixtures for the integration tests.
//!
//! Each test binary gets its own copy of this module, so not every helper is
//! used in every binary.

#![allow(dead_code)]

use girsync::cache::{
    CACHE_PREFIX, CacheDb, CacheOpen, FileRec, TestCacheBackend, load_all_records, open_db,
};
use girsync::effective::{SideScan, classify, ensure_distinct_sides, open_side, resolve_side};
use girsync::hash::{DigestSet, HashAlg};
use girsync::planner::{CasePairing, HashMode, SideRequest, StatTrust, plan_pairs};
use girsync::report::OutputFormat;
use girsync::rw::{RwLimits, RwRuntime, RwSide};
use girsync::{
    CommonOpts, CompareOpts, CompareSelfOpts, LogCtx, ScanMode, SyncOpts, TrustOpts, UpdateOpts,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// The scratch-directory helper, from the shared `girpr-tempdir` crate, so the
// tree a test writes into is named and removed exactly the way it is in the
// other two projects.
pub use girpr_tempdir::TempDir;

/// The prefix every scratch directory in this suite carries: it says which
/// suite leaked a directory if one survives a crash.
pub const TEMP_PREFIX: &str = "girsync";

/// The console's records, with the colour taken out.
///
/// `tracing`'s human format colours the level and the target separately, and
/// the escape sequences land *between* them: the text is `DEBUG`, then an
/// escape, then the target. A substring that spans one is not a substring at
/// all — and an assertion that a coloured string does *not* contain one passes
/// for the wrong reason, which is worse than no assertion.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // Skip the escape sequence: ESC, '[', then up to its final byte.
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() || c == '~' {
                break;
            }
        }
    }
    out
}

/// The plain read/write open most tests want: no rebuild, no backup.
pub fn rw() -> CacheOpen {
    CacheOpen::ReadWrite {
        ignore_cache: false,
        backup_first: false,
    }
}

/// Isolated temp directory per test, so parallel `cargo nextest run` workers
/// never share a cache file.
///
/// The tree is named so nothing can collide with it and removed when the
/// guard drops; see `girpr-tempdir` for why each part of the name is there.
pub fn temp_root(tag: &str) -> TempDir {
    TempDir::new(TEMP_PREFIX, tag)
}

pub fn wfile(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, bytes).unwrap();
}

// ---------------------------------------------------------------------------
// Links
// ---------------------------------------------------------------------------
//
// Tests that need a link call these; a platform that refuses the creation
// (Windows without developer mode refuses `symlink_file` and `symlink_dir`) gets
// an `Err` and the test skips with the OS message. A directory link has a
// fallback that needs no privilege - a junction - because that is the shape a
// game tree on Windows is most likely to contain anyway.

/// Create a link to a file.
pub fn file_link(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "no link API",
        ))
    }
}

/// Create a link to a directory, falling back to a junction on Windows.
///
/// `cmd /c mklink /J` creates a directory reparse point without elevation, and
/// the walk and the deletion guard treat it as a link either way
/// (`girsync::link::is_link`). If both calls fail, the original error is
/// returned so the caller can report why it skipped.
pub fn dir_link(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        let first = std::os::windows::fs::symlink_dir(target, link);
        if first.is_ok() {
            return first;
        }
        let created = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output();
        match created {
            Ok(out) if out.status.success() => Ok(()),
            _ => first,
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "no link API",
        ))
    }
}

/// Skip the calling test, with the reason, when the platform refused the link.
///
/// A macro rather than a helper so the `return` belongs to the test. A silent
/// skip is not acceptable: the point of the link cases is to pin behaviour, and
/// a run that quietly covered nothing would read as coverage.
#[macro_export]
macro_rules! skip_if_no_link {
    ($what:expr, $res:expr) => {
        match $res {
            Ok(()) => {}
            Err(e) => {
                eprintln!("skip: this platform refused a {} link: {e}", $what);
                return;
            }
        }
    };
}

/// True when `path` is itself a link or junction (the test-side mirror of
/// `girsync::link::is_link`, which is not exported from the public surface).
pub fn is_link(path: &Path) -> bool {
    if path.is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        std::fs::symlink_metadata(path)
            .map(|md| md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        false
    }
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
        algos: vec![HashAlg::Md5],
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
        // The unflagged run, which is `Shared(1)` -- see `RwLimits::default`.
        rw: RwLimits::default(),
        test_cache: TestCacheBackend::default(),
    }
}

/// `opts()` with an in-memory test cache, for semantic suites that assert sync
/// results rather than the cache file itself. Skips the per-command 1 MB create
/// and fsync on the HDD. Cache-focused suites (compare_self, update, sync/cache,
/// compare) keep `opts()` and its `File` backend.
pub fn opts_mem() -> CommonOpts {
    CommonOpts {
        test_cache: TestCacheBackend::Memory,
        ..opts()
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
        level: girsync::logging::Level::Error,
        file_level: girsync::logging::Level::Debug,
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
/// one rw operation at a time, which is what an unflagged command resolves to.
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

/// [`update`] against the shared in-memory cache, for a case that will read the
/// rows back through [`recs_of`].
pub fn update_mem(dir: PathBuf) -> UpdateOpts {
    UpdateOpts {
        dir,
        common: opts_mem(),
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

/// [`compare`] served from the shared memory cache, for a case that asserts a
/// verdict rather than the cache file.
pub fn compare_mem(src: PathBuf, dst: PathBuf) -> CompareOpts {
    CompareOpts {
        common: opts_mem(),
        ..compare(src, dst)
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
        common: opts(),
    }
}

/// [`sync`] served from the shared memory cache, for a case that asserts the
/// tree or the rows rather than the cache file.
pub fn sync_mem(src: PathBuf, dst: PathBuf) -> SyncOpts {
    SyncOpts {
        common: opts_mem(),
        ..sync(src, dst)
    }
}

/// [`sync`] at a chosen rw limit.
///
/// A limit rather than a flag string, so a case can reach a *split* shape — which
/// no single command-line flag spelling maps to on its own, and which is the shape
/// where a copy takes two permits.
pub fn sync_at(src: PathBuf, dst: PathBuf, rw: RwLimits) -> SyncOpts {
    SyncOpts {
        common: CommonOpts { rw, ..opts() },
        ..sync(src, dst)
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

pub fn md5arg() -> Vec<HashAlg> {
    vec![HashAlg::Md5]
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
// whose stat matches and which is missing an algorithm is a state a stat-only
// `--hash-all-of none` scan leaves behind, but the binary will not create it on
// request — so the cases build it directly, through the public cache API.

/// A cache row edited in place, or a path to one.
/// Every row a side's cache currently holds, for asserting what a run left
/// behind or dropped.
pub fn recs_of(dir: &Path) -> HashMap<String, FileRec> {
    load_all_records(&cache_at(dir)).unwrap()
}

/// The cache a run in this process left at `dir`.
///
/// A suite that ran with [`opts_mem`] has its rows in the shared in-memory cache
/// for `<dir>/girpr-cache`; every other suite has the real file. Preferring the
/// memory entry when one exists keeps the assertions identical while letting a
/// cache-content case leave the HDD.
fn cache_at(dir: &Path) -> CacheDb {
    let p = dir.join(CACHE_PREFIX);
    CacheDb::memory_cache_at(&p).unwrap_or_else(|| open_db(&p, true, rw()).unwrap())
}

/// Edit one row of whichever cache a run left at `dir`.
///
/// Public because a suite that builds a cache state by hand needs the same
/// memory-or-file decision `recs_of` makes; opening the file directly would
/// invent an empty cache beside the in-memory one.
pub fn edit_row<F: FnOnce(&mut FileRec)>(dir: &Path, rel: &str, f: F) {
    let db = cache_at(dir);
    let mut rec = db
        .get(rel)
        .unwrap()
        .unwrap_or_else(|| panic!("no row for {rel}"));
    f(&mut rec);
    db.put(rel, &rec).unwrap();
}

/// Drop one algorithm from a row, leaving its stat intact.
///
/// Rebuilt rather than deleted in place: `DigestSet` has no `remove`, and the reason
/// it has none is that a name this build knows can be sitting in the foreign arm, so a
/// caller deleting "md5" has to look in both places. The rebuild does that by
/// construction rather than by remembering to.
pub fn strip_algo(dir: &Path, rel: &str, algo: &str) {
    edit_row(dir, rel, |r| {
        let mut kept = DigestSet::default();
        for (a, d) in r.hashes.iter() {
            if a.as_str() != algo {
                kept.insert_named(a.as_str(), d);
            }
        }
        for (n, d) in r.hashes.foreign_iter() {
            if n != algo {
                kept.insert_named(n, d);
            }
        }
        r.hashes = kept;
    });
}

/// Overwrite a digest with a wrong one of the same length, keeping size and
/// mtime. The stat still matches, so only the content disagrees — the case where
/// a digest is the only thing that can decide the pair.
pub fn poison_digest(dir: &Path, rel: &str, algo: &str) {
    edit_row(dir, rel, |r| {
        // By *name*, so the length that gets replaced is whatever the row really
        // holds -- including the unexpected length a corrupt row would carry.
        let n = r.hashes.get_named(algo).map(|d| d.len()).unwrap_or(32);
        r.hashes.insert_named(algo, &vec![0xABu8; n]);
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
///
/// Known and foreign names together, sorted, so a case asserting "the row still holds
/// md5" cannot pass because the foreign arm happened to be checked first.
pub fn digests_of(dir: &Path, rel: &str) -> Vec<String> {
    let hashes = &recs_of(dir)[rel].hashes;
    let mut v: Vec<String> = hashes.iter().map(|(a, _)| a.as_str().to_string()).collect();
    v.extend(hashes.foreign_iter().map(|(n, _)| n.to_string()));
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
pub fn pair(t: &TempDir, spec: &[Spec], algos: &[&str], warmed: bool) -> (PathBuf, PathBuf) {
    pair_with(t, spec, algos, warmed, TestCacheBackend::File)
}

/// [`pair`] warmed into the shared in-memory cache.
///
/// Only the fixture's own rows move: the suite still calls `recs_of`, which
/// finds the memory entry, so a case can assert the same record contents it did
/// against a file cache. Use it together with [`opts_mem`] — a file-backed run
/// cannot see rows written to memory.
pub fn pair_mem(t: &TempDir, spec: &[Spec], algos: &[&str], warmed: bool) -> (PathBuf, PathBuf) {
    pair_with(t, spec, algos, warmed, TestCacheBackend::Memory)
}

fn pair_with(
    t: &TempDir,
    spec: &[Spec],
    algos: &[&str],
    warmed: bool,
    backend: TestCacheBackend,
) -> (PathBuf, PathBuf) {
    let s = t.subdir("src");
    let d = t.subdir("dst");
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
            let o = CommonOpts {
                test_cache: backend,
                ..with_algos(algos)
            };
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
///
/// Takes the spellings rather than the algorithms, so a case reads the way the
/// flag does and a typo is a compile-adjacent `unwrap` rather than a silent
/// empty plan.
pub fn with_algos(algos: &[&str]) -> CommonOpts {
    CommonOpts {
        algos: algos.iter().map(|a| a.parse().unwrap()).collect(),
        ..opts()
    }
}

/// [`with_algos`] backed by the shared in-memory cache, for a fixture that was
/// warmed with [`pair_mem`].
pub fn with_algos_mem(algos: &[&str]) -> CommonOpts {
    CommonOpts {
        test_cache: TestCacheBackend::Memory,
        ..with_algos(algos)
    }
}

/// The exact phase sequence `cmd_compare` runs — open both sides, one
/// `plan_pairs`, resolve both — with the per-side trust flags applied.
///
/// Driven by the tests rather than delegated to, so that a case can assert what
/// the planner decided *and* what the sides ended up reading. Every phase
/// happens here, in this order; that is the property under test.
pub fn resolve_both(src: &Path, dst: &Path, trust: TrustOpts) -> (SideScan, SideScan) {
    // Semantic harness: the callers assert maps and verdicts, so the caches can
    // live in memory. A caller with a warmed fixture must have warmed it with
    // `pair_mem`, or this resolve starts cold.
    let common = CommonOpts {
        test_cache: TestCacheBackend::Memory,
        ..with_algos(&["md5"])
    };
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
        CasePairing::from(common.case_sensitive),
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
