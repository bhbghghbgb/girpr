//! Shared girsync test fixtures: the option, cache and planner builders that
//! used to live in `girpr-tempdir::test_support`.
//!
//! This is **test support**, not a shipped surface. It holds the helpers that
//! construct girsync's own types, so it depends on girsync directly. Keeping
//! that dependency here — rather than in the scratch-directory crate — is the
//! point of the split: `girpr-tempdir` stays a girsync-free leaf, and only the
//! two programs that already depend on girsync (`girsync` itself and
//! `sled2redb`) take this crate as a dev-dependency. The repair binary and
//! `gircap` therefore no longer inherit girsync through their scratch dirs.
//!
//! The girsync-free helpers (temp dirs, file/link fixtures, NDJSON-free file
//! mechanics) stay in `girpr-tempdir::test_support` and are re-exported here, so
//! a test that does `use common::*` sees exactly the same surface as before.
//! Scratch trees are still made through `girpr_tempdir::TempDir`, so the naming
//! and cleanup policy is unchanged.

#![allow(dead_code)]
// each test binary used only a subset of these helpers
// These are test fixtures: panicking on a broken fixture is the point. Keep the
// crate-level lint gate (which exists for code a user runs) out of this crate.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use girsync::cache::{
    CACHE_PREFIX, CacheDb, CacheOpen, FileRec, TestCacheBackend, load_all_records, open_db,
};
use girsync::cli::Cli;
use girsync::effective::SideScan;
use girsync::hash::{DigestSet, HashAlg};
use girsync::logging::Level;
use girsync::planner::{HashMode, StatTrust};
use girsync::report::OutputFormat;
use girsync::rw::{RwLimits, RwRuntime};
use girsync::{
    BackupPolicy, CommonOpts, CompareOpts, CompareSelfOpts, Filter, LogCtx, Report, RunOutcome,
    ScanMode, SyncOpts, TestHooks, TrustOpts, UpdateOpts, plan,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clap::Parser;
use serde_json::Value;

// The girsync-free half of the old module and the scratch-directory guard,
// re-exported so `use common::*` keeps every call site unchanged.
pub use girpr_tempdir::TempDir;
pub use girpr_tempdir::test_support::*;
pub use girpr_tempdir::{skip_if_case_insensitive, skip_if_no_link};

/// The plain read/write open most tests want: no rebuild, no backup.
pub fn rw() -> CacheOpen {
    CacheOpen::ReadWrite {
        ignore_cache: false,
        backup_first: false,
        retain: None,
    }
}

/// Default options for the end-to-end tests: md5, no filters, case-sensitive.
pub fn opts() -> CommonOpts {
    CommonOpts {
        algos: vec![HashAlg::Md5],
        hash_mode: HashMode::default(),
        stat: StatTrust::default(),
        why: false,
        show_identical: false,
        filter: Filter::new(vec![], vec![], true),
        case_sensitive: true,
        max_depth: 10,
        ignore_cache: false,
        dry_run: false,
        // The unflagged run, which is `Shared(1)` -- see `RwLimits::default`.
        rw: RwLimits::default(),
        test_cache: TestCacheBackend::default(),
        // The unflagged run backs up and keeps everything.
        backup: BackupPolicy::default(),
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
        report_sink: Default::default(),
    }
}

/// Scan mode for tests: `no_trust_cached_hashes` and `dry_run`.
pub fn scan(no_trust_cached_hashes: bool, dry_run: bool) -> ScanMode {
    ScanMode::new(no_trust_cached_hashes, dry_run)
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
/// Delegated to the command's own `compare_detailed` so a caller asserts on the
/// maps the command produces, not a second derivation of them. The semantic suites
/// assert maps and counters, so the caches can live in memory; a caller with a
/// warmed fixture must have warmed it with `pair_mem`, or this resolve starts cold.
pub fn resolve_both(src: &Path, dst: &Path, trust: TrustOpts) -> (SideScan, SideScan) {
    let common = CommonOpts {
        test_cache: TestCacheBackend::Memory,
        ..with_algos(&["md5"])
    };
    let opts = CompareOpts {
        src: src.to_path_buf(),
        dst: dst.to_path_buf(),
        trust,
        common,
    };
    let outcome = girsync::commands::compare_detailed(opts, &log()).expect("compare two folders");
    (outcome.src, outcome.dst)
}

// ---------------------------------------------------------------------------
// In-process CLI runner glue, moved here from tools/sync/tests/common/mod.rs
// so it compiles once instead of in every girsync integration binary.
// ---------------------------------------------------------------------------
// (imports merged into the crate-level list above)

/// The quiet log policy an in-process run is dispatched under: no console
/// events, no log file. The report's records are captured regardless of level.
fn capture_plan() -> girsync::girpr_log::LogPlan {
    plan(Level::Error, None, None, true)
}

/// Run a parsed CLI in-process, capturing the records, exit code and error
/// string a process test used to read off the binary's stdout/stderr.
///
/// `GIRSYNC_TEST_CACHE=file` is pinned here: a spawned child got a fresh
/// in-memory cache per process, but the memory registry is shared across
/// in-process runs on the same paths. Forcing the file backend keeps each run's
/// cache local to its fixture and makes the result independent of an ambient
/// `GIRSYNC_TEST_CACHE`. Nextest isolates each test in its own process, so the
/// `set_var` cannot leak to another test.
#[allow(unsafe_code, dead_code)] // `set_var` is unsafe in edition 2024; nextest isolates each test process
pub fn run_cli(cli: Cli) -> RunOutcome {
    run_cli_with_hooks(cli, TestHooks::from_env())
}

/// Run a parsed CLI in-process with the given hooks injected, capturing records,
/// exit code and error string exactly as [`run_cli`] does.
///
/// This is the seam the failure-path cases use instead of spawning a child just
/// to set a `GIRSYNC_TEST_*` variable: the hooks are data, so two runs in one
/// process can ask for different failures without a `OnceLock` or a subprocess.
#[allow(unsafe_code, dead_code)] // `set_var` is unsafe in edition 2024; nextest isolates each test process
pub fn run_cli_with_hooks(cli: Cli, hooks: TestHooks) -> RunOutcome {
    unsafe { std::env::set_var("GIRSYNC_TEST_CACHE", "file") };
    girsync::run_captured_with_hooks(cli, &capture_plan(), hooks).expect("run_captured")
}

/// Parse arguments (as passed to `Command::args`, without a program name) and
/// run the CLI in-process. The binary name is supplied as `argv[0]` for clap.
#[allow(dead_code)]
pub fn run_args<I, S>(args: I) -> RunOutcome
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let argv = std::iter::once(std::ffi::OsString::from("girsync"))
        .chain(args.into_iter().map(Into::into));
    run_cli(Cli::try_parse_from(argv).expect("valid CLI args"))
}

/// [`run_args`] with hooks injected through [`run_cli_with_hooks`].
#[allow(dead_code)]
pub fn run_args_with_hooks<I, S>(args: I, hooks: TestHooks) -> RunOutcome
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let argv = std::iter::once(std::ffi::OsString::from("girsync"))
        .chain(args.into_iter().map(Into::into));
    run_cli_with_hooks(Cli::try_parse_from(argv).expect("valid CLI args"), hooks)
}

/// Parse argv, run in-process, and return the exit code plus the error message
/// a caller would read off stderr: clap's own text for a parse error (exit 2),
/// or the `anyhow` string `main` logs as `fatal` (exit 3). Empty on success.
#[allow(dead_code)]
pub fn run_error_text<I, S>(args: I) -> (i32, String)
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let argv = std::iter::once(std::ffi::OsString::from("girsync"))
        .chain(args.into_iter().map(Into::into));
    match Cli::try_parse_from(argv) {
        Err(e) => (2, e.to_string()),
        Ok(cli) => {
            let out = run_cli(cli);
            (out.code, out.error.unwrap_or_default())
        }
    }
}

/// The captured records as the JSON values a `--output json` parser would read.
#[allow(dead_code)]
pub fn values(out: &RunOutcome) -> Vec<Value> {
    out.records.iter().map(|r| r.json()).collect()
}

/// The captured records rendered as the text a stdout consumer would read, in
/// the run's `--output` format, one line per record with a trailing newline.
#[allow(dead_code)]
pub fn rendered(out: &RunOutcome, format: OutputFormat) -> String {
    let report = Report::new(format);
    let mut text = String::new();
    for rec in &out.records {
        text.push_str(&report.render(rec));
        text.push('\n');
    }
    text
}

/// Parse argv, run in-process, and return the exit code plus rendered stdout —
/// the drop-in for a binary helper that returned `(code, text)`.
#[allow(dead_code)]
pub fn run_text<I, S>(args: I) -> (i32, String)
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let argv = std::iter::once(std::ffi::OsString::from("girsync"))
        .chain(args.into_iter().map(Into::into));
    let cli = Cli::try_parse_from(argv).expect("valid CLI args");
    let format = cli.output;
    let out = run_cli(cli);
    (out.code, rendered(&out, format))
}

/// A console sink a test can read back: each event's formatted bytes are
/// appended to one shared buffer, so a scoped subscriber's stderr is available
/// even though no child process was spawned.
#[derive(Clone, Default)]
pub struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

#[allow(dead_code)] // each integration target gets its own copy of `common`
impl SharedBuffer {
    /// Everything the console layer has rendered so far, as text.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(|e| e.into_inner())).into_owned()
    }
}

pub struct BufferWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for BufferWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedBuffer {
    type Writer = BufferWriter;

    fn make_writer(&'a self) -> Self::Writer {
        BufferWriter(Arc::clone(&self.0))
    }
}

/// Parse argv and run the CLI in-process under a scoped copy of the binary's
/// tracing stack, with the console rendered into a returned buffer.
///
/// This is the seam for the tests that used to spawn the binary to read its
/// log: the plan is resolved from the same CLI as `main` resolves it, the layer
/// stack is the one [`girsync::subscriber_for`] builds, and the non-blocking
/// file guard is held in a binding that outlives the `with_default` closure so
/// the log file's tail is flushed before this returns. `env` lists any variable
/// the plan must see (`RUST_LOG`, `<PROJECT>_LOG_FILE_LEVEL`); each is removed
/// afterwards so a second run in the same test starts clean.
#[allow(unsafe_code, dead_code)] // `set_var`/`remove_var` are unsafe in edition 2024; nextest isolates each test process
pub fn run_args_logged<I, S>(args: I, env: &[(&str, &str)]) -> (RunOutcome, SharedBuffer)
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let argv = std::iter::once(std::ffi::OsString::from("girsync"))
        .chain(args.into_iter().map(Into::into));
    let cli = Cli::try_parse_from(argv).expect("valid CLI args");
    for (key, value) in env {
        unsafe { std::env::set_var(key, value) };
    }
    let plan = plan(
        cli.log_level,
        cli.log_file_level,
        cli.log_file.as_deref(),
        cli.no_log_file,
    );
    unsafe { std::env::set_var("GIRSYNC_TEST_CACHE", "file") };
    let console = SharedBuffer::default();
    let (subscriber, _guard) = girsync::subscriber_for(&plan, console.clone());
    let out = tracing::subscriber::with_default(subscriber, || {
        girsync::run_captured(cli, &plan).expect("run_captured")
    });
    for (key, _) in env {
        unsafe { std::env::remove_var(key) };
    }
    (out, console)
}
