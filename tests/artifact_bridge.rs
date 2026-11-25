//! The cross-tool claim, against the real bundle.
//!
//! Spec: [`docs/06-artifact-bridge-spec.md`](../docs/06-artifact-bridge-spec.md).
//!
//! **What this file proves, and nothing else.** `girpr`'s output is identical
//! to what another repair tool computes, whatever `girpr`'s internal
//! implementation happens to be. `gircap` computes the patch independently —
//! its own manifest diff, its own reuse scoring, its own arithmetic — and
//! records the ground truth in the bundle; `girpr` is then run against the
//! same inputs and its result is compared. That is a **cross-tool** claim,
//! and it is **three assertions** per capture:
//!
//! 1. the patched file's bytes are the reference bytes, and its length is the
//!    reference length;
//! 2. the chunks `girpr` asked for are exactly the chunks the patch says
//!    must be fetched — and **none** of the ones it says can be reused;
//! 3. the bytes `girpr` reports downloading are the bytes the patch says the
//!    patch costs.
//!
//! Everything else about `girpr` — which tier it picked, which direction it
//! wrote in, whether it trimmed eagerly, what its `SUMMARY` line said — is
//! **`girpr`'s own business**, it is not shared with any other repair tool,
//! and it is tested where it belongs: on crafted fixtures, in
//! `tests/repair_machinery.rs`. The replay server's own behaviour is tested
//! in `tests/artifact_server.rs`.
//!
//! # Why every test here is `#[ignore]`d: cost
//!
//! Not "it would be rude under `cargo nextest run`". These cases are
//! **expensive**. One capture is a `--print-plan` pre-pass plus a real run, and
//! each run fetches four manifests — ~19 MiB of real bundle bytes — before it
//! can plan a single file. The six cases take roughly a minute of wall-clock
//! in a debug build (measured: 63 s wall for the whole six-test run — the
//! cases run in parallel, so that is not the sum of their durations — and 67 s
//! wall including the rebuild, on a 4-core Windows x86_64 host with
//! `cargo nextest run -p girpr --test artifact_bridge --run-ignored only`
//! against one 7.0.0 → 7.1.0 bundle); the figure depends on the profile and
//! host, and a release build was not measured here. That is a price worth
//! paying **before a release that changes the repair spec** and *after
//! something has gone wrong*, and not worth paying on every `cargo nextest
//! run`.
//!
//! So it is run **by hand, deliberately** — before any release that changes
//! the repair spec, and whenever that spec is suspected to have regressed.
//! The persistent cases need the small v4 bundle produced by the hand-run C4
//! producer; the patch-shape cases need the ordinary captured bundle:
//!
//! ```text
//! # produce the v4 bundle once (C4's hand-run producer)
//! cargo nextest run -p gircap --test persistent_capture_e2e --run-ignored only -E 'test(write_persistent_v4_bundle)'
//! # then run the bridge against it
//! set GIRPR_ARTIFACT_BUNDLE=artifacts/persistent_v4
//! cargo nextest run -p girpr --run-ignored only -E 'binary(artifact_bridge)'
//! ```
//!
//! No `--no-capture` or `--test-threads` flag is needed: each capture gets its
//! own scratch tree (§6.3) and its own listeners (§6.2), so the cases are
//! parallel-safe, and the `[[profile.default.overrides]]` in
//! `.config/nextest.toml` whose `filter` names these cases sets
//! `success-output = "immediate"`, so the per-capture lines are printed even
//! though the cases pass.
//!
//! To reproduce one failure, put the case in the filter of the same command:
//!
//! ```text
//! cargo nextest run -p girpr --run-ignored only -E 'test(the_reuse_in_place_capture_fetches_only_what_it_must)'
//! ```
//!
//! The `-E` expression narrows the set nextest has already selected; it does
//! not add `#[ignore]`d cases to it. A command without `--run-ignored only`
//! (or `all`) runs none of them, and `cargo nextest list` needs
//! `--run-ignored only` (or `all`) to list them at all.
//!
//! Nothing else runs it. `cargo nextest run` still *compiles* this file, which
//! is what keeps it from rotting against `RunCtx`, `PathScope` and the `SUMMARY`
//! contract — a broken bridge would fail to build rather than fail silently.
//!
//! # Why a missing bundle is an error, not a skip
//!
//! Because this file is only ever run intentionally. A skip on a missing
//! prerequisite would turn "I ran the check and it found nothing to check" into
//! a green line — and that is the one reading this harness must never permit.
//! Nobody types `cargo nextest run --run-ignored only` here by accident, so the
//! hazard a skip guards against (people run it with `--run-ignored only` forever
//! and stop reading) does not exist; its opposite does. If the bundle is
//! missing, the run **fails**, and says exactly what to run to produce one:
//!
//! ```text
//! cargo nextest run -p gircap --test persistent_capture_e2e --run-ignored only -E 'test(write_persistent_v4_bundle)'
//! set GIRPR_ARTIFACT_BUNDLE=artifacts/persistent_v4
//! # or, for the ordinary patch-shape bundle / a bundle elsewhere:
//! cargo run -p gircap -- capture --out artifacts
//! set GIRPR_ARTIFACT_BUNDLE=<dir>
//! ```
//!
//! Resolution order: `GIRPR_ARTIFACT_BUNDLE` always wins, then the search
//! root (`GIRPR_ARTIFACT_ROOT` when set, else `artifacts/`), then
//! `gircap list --root <root>`, and a directory scan only as the fallback when
//! that listing is unavailable or fails.
//!
//! # What this file is allowed to touch
//!
//! The bundle, read-only. The working tree is a scratch *copy* of the one
//! file the capture is about — pointing `--game-path` at the bundle in place
//! is what turns a cleanup flag into a way to destroy the evidence. Every
//! rewrite the listeners apply lives in memory; nothing is written back.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "artifact_bridge/bundle.rs"]
mod bundle;
#[path = "common/run_lock.rs"]
mod run_lock;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use bundle::{Capture, Patchmaps, PersistentRecords};
use girpr::test_support::{BackoffPolicy, Limits};
use girpr::{Biz, OutputFormat, RunCtx, RunExit};
use girpr_repair_plan::PathScope;
use girpr_repair_policy::RepairMode;
use girpr_tempdir::TempDir;
use girpr_test_fixtures::artifact_server as server;
use girpr_test_fixtures::md5hex;
use girpr_test_support::run as run_library;
use server::{Bridge, Bundle, HitKind, Origins};

// ---------------------------------------------------------------------------
// gate: locating a bundle
// ---------------------------------------------------------------------------

/// The `artifacts/` root beside the package manifest, or
/// `GIRPR_ARTIFACT_ROOT` when one is set.
fn artifacts_root() -> PathBuf {
    match std::env::var_os("GIRPR_ARTIFACT_ROOT") {
        Some(v) => PathBuf::from(v),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts"),
    }
}

/// Bundles are laid out `<root>/<biz>/<old>__<new>/<capture-id>/`, so a bundle
/// is any directory holding a `MANIFEST.json`. The fallback for when `gircap` is
/// not built — the primary is [`gircap_list`].
fn scan_for_bundles(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(biz) = std::fs::read_dir(root) else {
        return out;
    };
    for b in biz.filter_map(Result::ok) {
        let Ok(pair) = std::fs::read_dir(b.path()) else {
            continue;
        };
        for p in pair.filter_map(Result::ok) {
            let Ok(id) = std::fs::read_dir(p.path()) else {
                continue;
            };
            for c in id.filter_map(Result::ok) {
                if c.path().join("MANIFEST.json").is_file() {
                    out.push(c.path());
                }
            }
        }
    }
    out.sort();
    out
}

/// `gircap list --root <root>` — the discovery primitive `docs/06` §3 names.
///
/// Returns `None` when `gircap` is not built or the call fails, so the caller
/// can fall back to a plain directory scan. Discovery only has to *name* a
/// directory, and refusing to find a bundle that is sitting right there because
/// a sibling workspace member has not been built is a worse failure than not
/// asking the tool that owns the layout.
fn gircap_list(root: &Path) -> Option<Vec<PathBuf>> {
    let exe = gircap_exe()?;
    let out = std::process::Command::new(exe)
        .args(["list", "--root"])
        .arg(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut found: Vec<PathBuf> = stdout
        .lines()
        .filter_map(|line| {
            // `BUNDLE tool=gircap version=0.1.0 path=<biz>/<old>__<new>/<id> …`.
            // Only the `bundle` records: the terminal `summary` carries
            // `first=`/`last=` paths of its own, which are not bundles.
            let rest = line.strip_prefix("BUNDLE ")?;
            let tail = rest.split_once(" path=")?.1;
            let rel = tail.split_whitespace().next()?;
            // `gircap list` reports `/`-separated paths on every platform;
            // normalize so a printed bundle path does not mix separators.
            Some(root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR)))
        })
        .collect();
    found.sort();
    Some(found)
}

/// Resolve the bundle under test.
///
/// **A missing bundle is an error, not a skip.** This suite only ever runs
/// deliberately, so "nothing to check" must never render as a green line —
/// that is the one reading it must not permit. The message names the exact
/// command that produces a bundle, because a failure here is almost always
/// someone who has not captured one yet, not a broken bundle.
fn locate_bundle() -> PathBuf {
    if let Some(explicit) = std::env::var_os("GIRPR_ARTIFACT_BUNDLE") {
        let p = PathBuf::from(explicit);
        assert!(
            p.join("MANIFEST.json").is_file(),
            "GIRPR_ARTIFACT_BUNDLE={} is not a bundle (no MANIFEST.json)",
            p.display()
        );
        return p;
    }
    let root = artifacts_root();
    let (found, how) = match gircap_list(&root) {
        Some(list) => (list, "gircap list"),
        None => (scan_for_bundles(&root), "a directory scan"),
    };
    match found.len() {
        1 => found.into_iter().next().expect("len 1"),
        0 => panic!(
            "no gircap bundle under {} (via {how}).\n\
             \x20 This suite is #[ignore]d because it is expensive and is meant to be run by \
             hand, so a missing input is a failure rather than a silent pass.\n\
             \x20   produce the v4 bundle with:  cargo nextest run -p gircap --test \
             persistent_capture_e2e --run-ignored only -E 'test(write_persistent_v4_bundle)'\n\
             \x20   then point at it with:  set GIRPR_ARTIFACT_BUNDLE=artifacts/persistent_v4\n\
             \x20   or capture an ordinary bundle:  cargo run -p gircap -- capture --out {}\n\
             \x20   or point at an existing one:  set GIRPR_ARTIFACT_BUNDLE=<dir>",
            root.display(),
            root.display(),
        ),
        n => panic!(
            "{n} bundles under {} (via {how}); this suite needs exactly one.\n{}\
             \x20   name one with:  set GIRPR_ARTIFACT_BUNDLE=<dir>",
            root.display(),
            found
                .iter()
                .map(|p| format!("\x20     {}\n", p.display()))
                .collect::<String>(),
        ),
    }
}

/// The bundle under test.
fn bundle() -> Bundle {
    Bundle::open(locate_bundle())
}

/// The bundle's records: the matrix and the patchmaps.
fn patchmaps(bundle: &Bundle) -> Patchmaps {
    Patchmaps::read(&bundle.root)
}

fn girpr_exe_dir() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_girpr"))
        .parent()
        .expect("the binary has a parent dir")
        .to_path_buf()
}

/// `gircap` is a workspace member, so Cargo builds it into the same target
/// directory as `girpr` — which is exactly where `CARGO_BIN_EXE_girpr` points.
fn gircap_exe() -> Option<PathBuf> {
    let dir = girpr_exe_dir();
    for name in ["gircap.exe", "gircap"] {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// the suite fixture: one bundle, one listener set
// ---------------------------------------------------------------------------

/// The one fixture every capture test in this process shares.
///
/// The bundle is read-only and immutable for the whole suite, so opening it and
/// binding the listeners are suite-level work, not per-test work. The capture
/// *records* are read here too (`bundle()` already parses `MANIFEST.json`, and
/// `patchmaps()` is a one-time parse of the patchmaps), so a test reads them
/// from here rather than from the disk again. The v4 records get a cheap shape
/// check per persistent case (`PersistentRecords::assert_v4`); `gircap verify`
/// is not invoked and is not re-implemented here.
///
/// Per-capture state is deliberately **not** shared: each test still makes its
/// own [`Scratch`] tree and runs its own pre-pass and real run, and each run
/// still cleans up its own run lock. Only the immutable inputs and the replay
/// server are shared.
struct Suite {
    bridge: Bridge,
    patchmaps: Patchmaps,
    /// The bridge's accept loops are `tokio::spawn`ed, so they belong to a
    /// runtime. Every `#[tokio::test]` gets a runtime of its own, dropped when
    /// that test ends; a bridge started on one test's runtime would be dead —
    /// its sockets closed — by the time the next test used it. The bridge is
    /// therefore started on this dedicated multi-thread runtime, which stays
    /// alive (with the fixture) for the life of the process.
    _runtime: tokio::runtime::Runtime,
}

/// Serialize the capture tests against the shared bridge in a single-process
/// run. A request-log checkpoint marks where one run begins; it can only mark
/// that boundary between runs, not between requests arriving concurrently from
/// two tests. Under nextest each test has its own process and this lock is
/// uncontended.
static RUN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// One test's checkpointed handle to the shared fixture.
///
/// The handle is per test even though the fixture is shared: each test gets its
/// own scratch tree and runs its own pre-pass and real run through it, and
/// `start` marks where this test's requests begin in the shared log.
struct SuiteRun {
    suite: &'static Suite,
    start: usize,
    /// Held for the whole test: see [`RUN_LOCK`].
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

impl SuiteRun {
    fn bridge(&self) -> &Bridge {
        &self.suite.bridge
    }

    /// The chunk ids this run asked for, not the ids earlier runs in this
    /// process asked for.
    fn chunk_ids(&self) -> Vec<String> {
        self.bridge().chunk_ids_since(self.start)
    }
}

/// The suite fixture, initialised once per process.
///
/// A process-wide `OnceCell` means "once per suite" under a single-process
/// `cargo test` and "once per case" under nextest, where each test gets its own
/// process. Both modes are correct: the fixture is immutable, so sharing it
/// changes no per-capture assertion.
async fn suite() -> &'static Suite {
    static SUITE: tokio::sync::OnceCell<Suite> = tokio::sync::OnceCell::const_new();
    SUITE
        .get_or_init(|| async {
            // The fixture needs a runtime that outlives the test runtimes, and
            // `Runtime::block_on` cannot be called from inside an async
            // context. Build it on a plain thread, off any test runtime.
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("build the bridge runtime");
                let bundle = bundle();
                let patchmaps = patchmaps(&bundle);
                let bridge = runtime.block_on(Bridge::start(bundle));
                println!(
                    "bridge fixture: opened {} and bound seven listeners (once per process)",
                    bridge.bundle().root.display()
                );
                tx.send(Suite {
                    bridge,
                    patchmaps,
                    _runtime: runtime,
                })
                .expect("send the bridge fixture")
            });
            rx.recv().expect("initialise the bridge fixture")
        })
        .await
}

/// Take a per-test handle on the shared fixture, checkpointing the request log.
async fn suite_run() -> SuiteRun {
    let guard = RUN_LOCK.lock().await;
    let suite = suite().await;
    SuiteRun {
        start: suite.bridge.log_len(),
        suite,
        _guard: guard,
    }
}

// ---------------------------------------------------------------------------
// the driver
// ---------------------------------------------------------------------------

/// A scratch copy of the bundle's install, reduced to what **one** capture
/// needs.
///
/// The copy is of the one file the capture is about, plus the install's own
/// state — the version file and the audio scan file — and not of the whole
/// fixture. The other captures' files are not what this capture is about, and
/// "every path outside the scope is untouched" is `girpr`'s own claim, tested
/// on a crafted multi-file fixture in `tests/repair_machinery.rs`.
///
/// `docs/06` §6.3 — the bundle is the input, a copy is the working tree.
/// Pointing `--game-path` at the bundle in place is what turns a cleanup flag
/// into a way to destroy the evidence.
struct Scratch {
    root: PathBuf,
    /// Removes the tree on drop. Named so the owned `root` path can stay a
    /// plain `PathBuf` for the call sites that hand it to the binary.
    _guard: TempDir,
}

impl Scratch {
    fn new(bundle: &Bundle, capture: &Capture) -> Self {
        let root = TempDir::new("girpr_artifact_bridge", "scratch");
        let game = root.join("game");
        // The install's own state.
        copy_file(
            &bundle.root.join("game").join("config.ini"),
            &game.join("config.ini"),
        );
        if let Some(scan) = audio_scan_rel(bundle) {
            copy_file(&bundle.root.join("game").join(&scan), &game.join(&scan));
        }
        // The capture's file, in the state the bundle recorded. A capture
        // whose file is *new* has none: the repair creates it.
        let src = bundle.root.join("game").join(&capture.path);
        if src.is_file() {
            copy_file(&src, &game.join(&capture.path));
        }
        Self {
            root: root.to_path_buf(),
            _guard: root,
        }
    }

    fn game(&self) -> PathBuf {
        self.root.join("game")
    }

    /// Somewhere for the binary to put its trace log, so the run does not
    /// scatter `logs/` under the build directory.
    ///
    /// The pre-pass gets its **own** file: `logging::open_log_file` opens with
    /// `append`, so one shared file would carry both runs' records.
    fn log_file(&self) -> PathBuf {
        self.root.join("girpr.log")
    }

    fn pre_log_file(&self) -> PathBuf {
        self.root.join("girpr-pre.log")
    }

    fn plan_out(&self) -> PathBuf {
        self.root.join("plan.json")
    }
}

/// Copy one file, creating the parents.
fn copy_file(src: &Path, dst: &Path) {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).expect("create the scratch parent");
    }
    std::fs::copy(src, dst).unwrap_or_else(|e| panic!("copy {}: {e}", src.display()));
}

/// The audio scan file's path, as the bundle's own `getGameConfigs` body names
/// it.
///
/// The run reads that file to resolve the audio selection, so the scratch
/// install has to carry it: a scratch tree without it would resolve the
/// selection differently from the bundle it is standing in for.
fn audio_scan_rel(bundle: &Bundle) -> Option<String> {
    let body: serde_json::Value =
        serde_json::from_slice(&bundle.api_body("getGameConfigs.json")).ok()?;
    let dir = body["data"]["launch_configs"][0]["audio_pkg_scan_dir"]
        .as_str()?
        .replace('\\', "/");
    if dir.is_empty() { None } else { Some(dir) }
}

/// One `girpr` invocation's result, with its NDJSON records indexed by name.
struct Run {
    code: i32,
    records: BTreeMap<String, serde_json::Value>,
    stdout: String,
    stderr: String,
}

impl Run {
    fn record(&self, event: &str) -> &serde_json::Value {
        self.records
            .get(event)
            .unwrap_or_else(|| panic!("no {event} record in:\n{}", self.stdout))
    }
    fn num(&self, event: &str, key: &str) -> u64 {
        self.record(event)[key]
            .as_u64()
            .unwrap_or_else(|| panic!("{event}.{key} is not a number in {:?}", self.record(event)))
    }
    fn word(&self, event: &str, key: &str) -> String {
        self.record(event)[key]
            .as_str()
            .unwrap_or_else(|| panic!("{event}.{key} is not a string in {:?}", self.record(event)))
            .to_string()
    }
}

/// Run the real binary (`CARGO_BIN_EXE_girpr` — this is the whole reason the
/// bridge is an integration test in this package and not a separate crate).
async fn girpr(args: &[String]) -> Run {
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_girpr"))
        .args(args)
        .output()
        .await
        .expect("spawn girpr");
    run_lock::remove_run_lock_for_args(args);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let mut records = BTreeMap::new();
    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("--output json line is not JSON ({e}): {line}"));
        // Last one wins: `PROGRESS` ticks repeatedly and `SUMMARY` is terminal,
        // so the newest record of a name is the one that describes the run.
        if let Some(event) = v["event"].as_str() {
            records.insert(event.to_string(), v);
        }
    }
    Run {
        code: out.status.code().expect("process must exit normally"),
        records,
        stdout,
        stderr,
    }
}

/// The flags every run carries (`docs/06` §5).
///
/// `--audio` is omitted **deliberately**: an explicit audio selection under an
/// active path scope is refused when the scan file falls outside that scope,
/// and the scan file does. Omitting it lets the run autodetect from the scan
/// file and resolve to the language the bundle was captured in.
///
/// No cleanup flag either: a scoped cleanup sees only the one selected path,
/// which the manifest contains, so it has nothing to delete. The one capture
/// that *wants* a cleanup is `removed`, and that is the row that says so.
fn base_args(game: &Path, bundle: &Bundle, o: &Origins, log: &Path) -> Vec<String> {
    vec![
        "--game-path".into(),
        game.display().to_string(),
        // `MANIFEST.json -> biz.id`, not a hardcoded region.
        "--biz".into(),
        bundle.biz.clone(),
        "--hyp-base".into(),
        o.hyp_base.clone(),
        "--sophon-base".into(),
        o.sophon_base.clone(),
        "--jobs".into(),
        "1".into(),
        "--output".into(),
        "json".into(),
        "--log-file".into(),
        log.display().to_string(),
    ]
}

/// The flags one capture's runs carry, on top of [`base_args`].
///
/// The repair mode is asked for explicitly, so the run's path is the one the
/// claim is about rather than the one the mode resolver happens to pick. The
/// **staged** tier is the default, and that is what the claim needs: the
/// staged tier keeps every source alive, so the chunks the repair asks for
/// are exactly the chunks the patch says have no local source. The ordered
/// tier is `girpr`'s business, and it is tested on a crafted fixture.
fn capture_args(capture: &Capture) -> Vec<String> {
    let mut args = vec![
        // Exactly one path: the patchmap's own `path` field. A path with a
        // space in it is carried verbatim — the glob is a path, not a
        // pattern.
        "--include".into(),
        capture.path.clone(),
        // A shim fault should stop the run, not spend the budget discovering
        // the same 404 on every remaining file.
        "--fail-fast".into(),
    ];
    if capture.is_purge_only() {
        // `removed`: the files-cleanup without the repair loop. No
        // `--max-files` — the repair plan is empty by construction, so the
        // ceiling would be vacuous and `limited=1` would be a lie about it.
        args.push("--purge-only".into());
    } else {
        args.push("--repair-mode".into());
        args.push("inplace".into());
    }
    args
}

/// Drive one capture end to end and assert the **cross-tool claim**.
///
/// **One run per capture**, always: a `download_bytes` total is only
/// attributable to a single capture.
async fn drive_capture(handle: &SuiteRun, capture: &Capture) {
    let bridge = handle.bridge();
    let bundle = bridge.bundle().clone();
    let scratch = Scratch::new(&bundle, capture);

    // ---- the free pre-pass (docs/06 §6.1) --------------------------------
    //
    // `--print-plan` returns before `create_dir_all`, so it costs no disk and
    // separates the two failure modes that otherwise arrive tangled in one red
    // run: when `files=1` fails the glob is wrong; when the real run's
    // `download_bytes` fails, the reuse is wrong.
    let mut pre = base_args(
        &scratch.game(),
        &bundle,
        bridge.origins(),
        &scratch.pre_log_file(),
    );
    pre.push("--print-plan".into());
    pre.push("--plan-out".into());
    pre.push(scratch.plan_out().display().to_string());
    pre.extend(capture_args(capture));
    let plan = girpr(&pre).await;
    assert_eq!(plan.code, 0, "--print-plan must exit 0\n{}", plan.stderr);
    assert_eq!(
        plan.num("plan", "files"),
        if capture.is_purge_only() { 0 } else { 1 },
        "the scope must select exactly the capture's path ({})\n{}",
        capture.path,
        plan.stdout
    );
    assert_eq!(
        plan.num("plan", "scoped"),
        1,
        "every capture is scoped, `removed` included: src/repair/mod.rs 4.2 makes \
         `--purge-only --include <path>` the request that deletes it\n{}",
        plan.stdout
    );

    // ---- the real run ---------------------------------------------------
    let mut args = base_args(
        &scratch.game(),
        &bundle,
        bridge.origins(),
        &scratch.log_file(),
    );
    args.extend(capture_args(capture));
    let run = girpr(&args).await;

    // ---- the preconditions ----------------------------------------------
    //
    // These are about the run, not about the patch: without them the
    // comparison below means nothing. A run that degraded but stayed green —
    // an empty reuse map, every chunk downloaded — is exactly what
    // `local_manifest` is there to rule out.
    assert_eq!(
        run.code, 0,
        "{}: exit must be 0\n{}\n{}",
        capture.id, run.stdout, run.stderr
    );
    assert_eq!(run.num("summary", "exit"), 0, "{}", capture.id);
    assert_eq!(
        run.word("summary", "local_manifest"),
        "ok",
        "{}: the local build's manifests must have loaded",
        capture.id
    );
    assert_eq!(run.num("summary", "scoped"), 1, "{}", capture.id);
    assert_eq!(
        run.num("summary", "config_bumped"),
        0,
        "{}: a scoped run deliberately did not put the bytes in place",
        capture.id
    );

    // ---- 1: the bytes are the reference bytes --------------------------
    let target = scratch.game().join(&capture.path);
    if capture.absent_after {
        assert!(
            !target.exists(),
            "{}: {} must be gone after the cleanup",
            capture.id,
            capture.path
        );
        // The cleanup's byte count is a **cross-tool** fact, not `girpr`'s own:
        // the bytes the run deleted are the old file's bytes, and the
        // patchmap states the old file's size. A run that deleted a different
        // number would still have removed the path, and this is what says so.
        assert_eq!(
            run.num("summary", "deleted_extra_bytes"),
            capture.old_size,
            "{}: the cleanup must delete exactly the old side's bytes",
            capture.id
        );
        // And a deletion downloads nothing. The patchmap has no `totals` for
        // this capture — there is no target to patch — so the bundle states
        // the figure as zero and this is where it is checked.
        assert_eq!(
            run.num("summary", "download_bytes"),
            0,
            "{}: a deletion must download nothing",
            capture.id
        );
    } else {
        let bytes = std::fs::read(&target)
            .unwrap_or_else(|e| panic!("{}: read patched {}: {e}", capture.id, capture.path));
        assert_eq!(
            md5hex(&bytes),
            capture.result_md5,
            "{}: the patched file must equal expectations.result_md5",
            capture.id
        );
        assert_eq!(
            bytes.len() as u64,
            capture.new_size,
            "{}: the patched file must be the new side's length",
            capture.id
        );
    }

    // ---- 2: the fetch set is the patch's fetch set ----------------------
    //
    // The load-bearing assertion of the whole suite. The patchmap's `plan[]`
    // says, per chunk, whether the bytes are already in the old file
    // (`REUSE`) or have to come off the network (`FETCH`). The repair asked
    // for exactly the `FETCH` ones — and for **none** of the `REUSE` ones.
    //
    // The negative half is the one that catches a broken reuse path: with an
    // empty reuse map every chunk is fetched, `local_manifest` says
    // `unavailable`, and the byte total is the whole file instead of the
    // patch.
    let asked: BTreeSet<String> = handle.chunk_ids().into_iter().collect();
    let want_fetch: BTreeSet<String> = capture.fetch_set().iter().map(|s| s.to_string()).collect();
    let want_reuse: BTreeSet<String> = capture.reuse_set().iter().map(|s| s.to_string()).collect();
    assert_eq!(
        asked, want_fetch,
        "{}: the chunks asked for must be exactly the ones the patch says have \
         to be fetched",
        capture.id
    );
    let forbidden: Vec<&String> = want_reuse.intersection(&asked).collect();
    assert!(
        forbidden.is_empty(),
        "{}: these ids are in the REUSE set, so they must not have been \
         requested (reuse was broken): {:?}",
        capture.id,
        forbidden
    );

    // ---- 3: the byte total is the patch's byte total --------------------
    if let Some(total) = capture.download_bytes {
        assert_eq!(
            run.num("summary", "download_bytes"),
            total,
            "{}: girpr's download_bytes counts the bytes of files it promoted; the \
             patchmap total is the sum of compressed_size over the chunks with no old-side \
             match, and the two are equal by construction",
            capture.id
        );
    }

    println!(
        "bridge {:<24} plan files={} | run total={} skipped={} repaired={} failed={} \
         dl={} | chunks requested={} (fetch={} reuse={})",
        capture.id,
        plan.num("plan", "files"),
        run.num("summary", "total"),
        run.num("summary", "skipped"),
        run.num("summary", "repaired"),
        run.num("summary", "failed"),
        run.num("summary", "download_bytes"),
        asked.len(),
        want_fetch.len(),
        want_reuse.len(),
    );
}

// ---------------------------------------------------------------------------
// the cases
// ---------------------------------------------------------------------------

/// Drive one named capture.
///
/// The capture is resolved **through the matrix**, not by looking for a
/// patchmap named after it: the matrix is the only place that says which
/// capture answers which class.
///
/// A class the matrix records as **not** answered is a fact rather than a
/// missing prerequisite, so the case stands down out loud. A class the matrix
/// does not mention at all is a **missing prerequisite** and fails: this suite
/// only runs deliberately, so it must never report "checked nothing" as a
/// pass.
async fn drive_named(handle: &SuiteRun, id: &str) {
    if let Some(kind) = id.strip_prefix("persistent_") {
        drive_persistent(handle, kind).await;
        return;
    }
    let records = &handle.suite.patchmaps;
    let Some(capture) = records.capture_for_class(id).cloned() else {
        match records.matrix_class(id) {
            Some(m) if m.absent_by_record() => {
                println!("{id}: not captured by this bundle — {}", m.reason);
                return;
            }
            Some(m) => panic!(
                "{id}: the matrix records it as not captured but gives no reason ({:?}), \
                 so a case for it cannot be reasoned about",
                m.reason
            ),
            None => panic!(
                "{id}: not in this bundle and not in its matrix (it has: {:?}). \
                 This suite is #[ignore]d because it is expensive and run by hand, so a \
                 missing capture is a failure, not a pass.",
                records
                    .captures
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>()
            ),
        }
    };
    drive_capture(handle, &capture).await;
}

// ---------------------------------------------------------------------------
// the three persistent cases
// ---------------------------------------------------------------------------

/// The bundle-derived facts one persistent case owns.
///
/// girpr always fetches all three manifests and replays all three kinds; "one
/// manifest per case" means the case owns **one manifest's assertions** — its
/// mirror, its blob(s) and its revision marker — not that the run fetches one.
struct PersistentCase {
    /// The `persistent_<kind>` id from `drive_named`.
    id: &'static str,
    /// The `index.json` kind (`silence` in the id is `silence_data` here).
    kind: &'static str,
    /// This kind's mirror under `GenshinImpact_Data/Persistent/`.
    mirror: &'static str,
    /// The blob's basename in `index.json` paired with where girpr publishes it
    /// under `game/`. The bytes are always read from the bundle, never written
    /// into this table.
    publishes: &'static [(&'static str, &'static str)],
    /// This kind's revision marker.
    marker: &'static str,
}

const PERSISTENT_CASES: [PersistentCase; 3] = [
    PersistentCase {
        id: "persistent_res",
        kind: "res",
        mirror: "res_versions_persist",
        publishes: &[
            (
                "11111111.blk",
                "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/11111111.blk",
            ),
            (
                "VO_TEST.pck",
                "GenshinImpact_Data/StreamingAssets/AudioAssets/English(US)/VO_TEST.pck",
            ),
        ],
        marker: "res_revision",
    },
    PersistentCase {
        id: "persistent_data",
        kind: "data",
        mirror: "data_versions_persist",
        publishes: &[
            (
                "22222222.blk",
                "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/22222222.blk",
            ),
            (
                "hardware_model_config.json",
                "GenshinImpact_Data/StreamingAssets/hardware_model_config.json",
            ),
        ],
        marker: "data_revision",
    },
    PersistentCase {
        id: "persistent_silence",
        kind: "silence_data",
        mirror: "silence_data_versions_persist",
        publishes: &[(
            "33333333.blk",
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/33333333.blk",
        )],
        marker: "silence_revision",
    },
];

/// A scratch copy of the whole (small, synthetic) `game/` tree. The persistent
/// tier writes install-wide markers, so it cannot run under the `--include`
/// scope the patch cases use: a marker outside that scope is a usage error.
fn persistent_scratch(bundle: &Bundle) -> TempDir {
    let guard = TempDir::new("girpr_artifact_bridge", "persistent");
    copy_tree(&bundle.root.join("game"), &guard.join("game"));
    // The persistent tier is what this case is about. Put the install's base
    // files at the new version too, so the base repair has nothing to fetch
    // and cannot mask a persistent failure with an unrelated chunk 404.
    copy_tree(&bundle.root.join("expected"), &guard.join("game"));
    // The producer records the install's Persistent AV folder as a one-line
    // placeholder *file*; a real install, and the repair's own prerequisites,
    // need a directory there. Normalize the scratch copy only — the bundle is
    // read-only.
    let persistent = guard.join("game/GenshinImpact_Data/Persistent");
    if persistent.is_file() {
        std::fs::remove_file(&persistent).expect("drop the bundled placeholder");
    }
    guard
}

fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create the scratch tree");
    for entry in std::fs::read_dir(src).expect("read the bundle game tree") {
        let entry = entry.expect("a tree entry");
        let to = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), &to).expect("copy a game file");
        }
    }
}

/// Write the scratch `dispatch/{key.pem,record.json,response.json}` a
/// key-and-record run needs. This is C2's `write_dispatch_inputs` pattern
/// (`tests/repair_persistent.rs`), with every byte taken from this bundle: the
/// key and envelope are the record's own bundle-relative files, and the record
/// is `Served::load`'s in-memory copy, whose CDN authorities name this
/// process's listeners. The bundle on disk is never written.
fn write_dispatch_inputs(
    scratch: &Path,
    records: &PersistentRecords,
    bundle_root: &Path,
    record: &serde_json::Value,
) -> (PathBuf, PathBuf) {
    let dispatch = scratch.join("dispatch");
    std::fs::create_dir_all(&dispatch).expect("scratch dispatch dir");
    let key = dispatch.join("key.pem");
    let record_path = dispatch.join("record.json");
    std::fs::write(&key, records.key_bytes(bundle_root)).expect("scratch key");
    std::fs::write(
        &record_path,
        serde_json::to_vec(record).expect("serialize record"),
    )
    .expect("scratch record");
    std::fs::write(
        dispatch.join("response.json"),
        records.envelope_bytes(bundle_root),
    )
    .expect("scratch envelope");
    (key, record_path)
}

/// A run context against the bridge's listeners, obtaining `DispatchInfo` from
/// the scratch key+record pair. No override is set: the file path is the claim.
/// It performs no dispatcher GET, so the production `dispatch_url` in the
/// record is never contacted.
fn persistent_ctx(
    scratch: &TempDir,
    origins: &Origins,
    key: &Path,
    record: &Path,
    dry_run: bool,
    purge_only: bool,
) -> RunCtx {
    RunCtx {
        game_dir: scratch.join("game"),
        biz: Biz::Hk4eGlobal,
        audio: std::collections::HashSet::new(),
        audio_explicit: false,
        // Everything is in scope (so the install-wide markers are admissible),
        // except the one base file whose chunks this synthetic capture never
        // stored. That file is not part of the persistent claim.
        path_scope: PathScope::new(
            &["**".to_string()],
            &[
                "GenshinImpact_Data/Resources/browser_assets".to_string(),
                // New base files whose chunks this synthetic capture did not
                // store: they are not part of the persistent claim.
                "GenshinImpact_Data/StreamingAssets/blocks/eager.blk".to_string(),
                "GenshinImpact_Data/StreamingAssets/blocks/grow.blk".to_string(),
            ],
        )
        .expect("the persistent scope compiles"),
        limits: Limits::default(),
        repair_mode: RepairMode::Auto,
        repair_mode_explicit: false,
        jobs: 1,
        with_persistent: true,
        persistent_dispatch_override: None,
        dispatch_key_path: Some(key.to_path_buf()),
        dispatch_record_path: Some(record.to_path_buf()),
        check_only: false,
        scan: false,
        purge_only,
        fail_fast: true,
        dry_run,
        print_plan: false,
        plan_out: None,
        purge_after: false,
        purge_before: false,
        output: OutputFormat::Text,
        hyp_base_override: Some(origins.hyp_base.clone()),
        sophon_base_override: Some(origins.sophon_base.clone()),
        backoff: BackoffPolicy::none(),
    }
}

/// Replay the three manifests and the full-file blobs from the scratch
/// key+record pair, then assert **this kind's** mirror, blob(s) and revision
/// marker. All three cases use the one bundle and the one listener set; each
/// owns a different slice of the published state.
async fn drive_persistent(handle: &SuiteRun, kind: &str) {
    let case = PERSISTENT_CASES
        .iter()
        .find(|c| c.id.strip_prefix("persistent_") == Some(kind))
        .unwrap_or_else(|| panic!("no persistent case for {kind:?}"));
    let bridge = handle.bridge();
    let bundle = bridge.bundle();
    let Some(records) = PersistentRecords::read(&bundle.root) else {
        panic!(
            "{} carries no dispatch/ + persistent/ records; produce the v4 bundle with \
             cargo nextest run -p gircap --test persistent_capture_e2e --run-ignored only \
             -E 'test(write_persistent_v4_bundle)' and set \
             GIRPR_ARTIFACT_BUNDLE=artifacts/persistent_v4",
            bundle.root.display()
        );
    };
    records.assert_v4(&bundle.root);
    let Some(record) = bridge.served_dispatch_record() else {
        panic!("the bridge did not load the dispatch record");
    };

    let scratch = persistent_scratch(bundle);
    let (key, record_path) = write_dispatch_inputs(&scratch, &records, &bundle.root, &record);
    let (summary, code) = run_library(persistent_ctx(
        &scratch,
        bridge.origins(),
        &key,
        &record_path,
        false,
        false,
    ))
    .await
    .unwrap_or_else(|e| panic!("{}: a clean flagged run failed: {e}", case.id));
    assert_eq!(code, RunExit::Success, "{}", case.id);
    assert_eq!(summary.flags.persistent, Some("published"), "{}", case.id);
    assert!(
        summary.flags.persistent_repaired.unwrap_or(0) > 0,
        "{}: the dispatch record must have driven a real repair",
        case.id
    );

    let persistent_dir = scratch.join("game").join("GenshinImpact_Data/Persistent");

    // This kind's mirror is the raw manifest bytes from `index.json`, not a
    // re-serialization. The other two mirrors are published by the same run
    // but belong to their own cases.
    assert_eq!(
        std::fs::read(persistent_dir.join(case.mirror)).expect("the mirrored manifest"),
        records.manifest(&bundle.root, case.kind),
        "{}: {mirror} must be the raw manifest bytes",
        case.id,
        mirror = case.mirror
    );

    // The request log is the proof of which hotfix route was exercised. A
    // source_kind is a bundle fact: the hit's basename must name one of this
    // kind's blobs in `persistent/index.json`. The file path performs no
    // dispatcher GET at all — there must be no Dispatch hit to lean on.
    let hits = handle.bridge().hits_since(handle.start);
    assert!(
        !hits.iter().any(|h| h.kind == HitKind::Dispatch),
        "{}: the key+record path must not query the dispatcher",
        case.id
    );
    let blobs = records.blobs_for_kind(case.kind);
    assert!(
        hits.iter().any(|h| {
            h.kind == HitKind::Hotfix
                && blobs.iter().any(|b| {
                    b["remote_name"]
                        .as_str()
                        .is_some_and(|r| r.rsplit('/').next() == Some(h.id.as_str()))
                })
        }),
        "{}: no Hotfix hit names a blob of kind {} (hits: {hits:?})",
        case.id,
        case.kind
    );

    // The hotfix bytes came off the replayed full-file route and are exactly
    // the bundle's blob bytes, byte for byte.
    for (blob_name, publish_rel) in case.publishes {
        assert_eq!(
            std::fs::read(scratch.join("game").join(publish_rel))
                .unwrap_or_else(|e| panic!("{}: read {publish_rel}: {e}", case.id)),
            records.blob(&bundle.root, blob_name),
            "{}: {publish_rel} must be the bundle's {blob_name} blob",
            case.id
        );
    }

    // This kind's revision marker equals the revision the record decoded.
    let decoded = &records.dispatch["decoded"];
    let want = decoded[case.marker]
        .as_u64()
        .unwrap_or_else(|| panic!("decoded has no {}", case.marker))
        .to_string();
    assert_eq!(
        std::fs::read(persistent_dir.join(case.marker)).expect("the revision marker"),
        want.as_bytes(),
        "{}: {} is exact plain text with no trailing newline",
        case.id,
        case.marker
    );

    if case.kind == "res" {
        // The eternal res marker is the second half of the res claim.
        let eternal = decoded["res_revision"].as_u64().unwrap().to_string();
        assert_eq!(
            std::fs::read(persistent_dir.join("res_revision_eternal")).expect("eternal marker"),
            eternal.as_bytes(),
            "res_revision_eternal is the decoded res_revision"
        );

        // The whole marker block lives in the res case only, once, so the three
        // cases each own a slice rather than all re-asserting the world. The
        // compact v4 producer records no `res_versions_streaming` source, so
        // C16's omit branch is the ninth marker's expected state here: eight
        // markers carry bundle-derived bytes and the ninth is pinned absent.
        let num = |key: &str| decoded[key].as_u64().unwrap().to_string();
        let text = |key: &str| decoded[key].as_str().unwrap().to_string();
        let mut expected: Vec<(&str, String)> = vec![
            ("res_revision", num("res_revision")),
            ("res_revision_eternal", num("res_revision")),
            ("data_revision", num("data_revision")),
            ("silence_revision", num("silence_revision")),
            ("audio_revision", num("audio_revision")),
            ("PatchDone", num("audio_revision")),
            ("ChannelName", text("channel_name")),
            ("ScriptVersion", text("game_version")),
        ];
        expected.sort_by_key(|(name, _)| *name);
        for (name, value) in expected {
            assert_eq!(
                std::fs::read(persistent_dir.join(name)).expect("the marker"),
                value.as_bytes(),
                "{name} is exact plain text with no trailing newline"
            );
        }
        assert!(
            !persistent_dir.join("base_res_version_hash").exists(),
            "the bundle has no hash source, so C16 must omit the ninth marker"
        );
        assert_eq!(
            summary.flags.persistent_base_hash.as_deref(),
            Some("omitted"),
            "the omission must be reported, not silently skipped"
        );

        // C4/C20: neither a dry run nor a cleanup-only run publishes a marker,
        // a mirror or a hotfix file. Each gets its own fresh copy of the tree.
        for (dry_run, purge_only) in [(true, false), (false, true)] {
            let ro = persistent_scratch(bundle);
            let (key, record_path) = write_dispatch_inputs(&ro, &records, &bundle.root, &record);
            let (summary, code) = run_library(persistent_ctx(
                &ro,
                bridge.origins(),
                &key,
                &record_path,
                dry_run,
                purge_only,
            ))
            .await
            .expect("a read-only flagged run");
            assert_eq!(code, RunExit::Success);
            assert_eq!(
                summary.flags.persistent,
                Some(if dry_run { "listed" } else { "skipped" })
            );
            let ro_persistent = ro.join("game").join("GenshinImpact_Data/Persistent");
            for name in [
                "base_res_version_hash",
                "res_revision",
                "res_revision_eternal",
                "data_revision",
                "silence_revision",
                "audio_revision",
                "PatchDone",
                "ChannelName",
                "ScriptVersion",
                "res_versions_persist",
                "data_versions_persist",
                "silence_data_versions_persist",
            ] {
                assert!(
                    !ro_persistent.join(name).exists(),
                    "{name} must not be written under dry_run={dry_run}, purge_only={purge_only}"
                );
            }
            for (_, publish_rel) in PERSISTENT_CASES.iter().flat_map(|c| c.publishes) {
                assert!(
                    !ro.join("game").join(publish_rel).exists(),
                    "a read-only run downloads no {publish_rel}"
                );
            }
        }
    }

    println!(
        "bridge {:<20} stored key+record -> mirror={} blob(s)={} hotfix hits={} repaired={}",
        case.id,
        case.mirror,
        case.publishes.len(),
        hits.iter().filter(|h| h.kind == HitKind::Hotfix).count(),
        summary.flags.persistent_repaired.unwrap_or(0)
    );
}

macro_rules! capture_test {
    ($name:ident, $id:literal, $doc:literal) => {
        #[doc = $doc]
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "expensive: real girpr runs over bundle bytes. Run by hand before any release that changes the repair spec, or when a regression is suspected. The persistent cases need GIRPR_ARTIFACT_BUNDLE=artifacts/persistent_v4 from C4's hand-run producer; then: cargo nextest run -p girpr --test artifact_bridge --run-ignored only (its output is visible without --no-capture; the per-case success-output override in .config/nextest.toml names it)"]
        async fn $name() {
            let run = suite_run().await;
            drive_named(&run, $id).await;
        }
    };
}

capture_test!(
    the_identical_capture_fetches_nothing,
    "identical",
    "`identical`: the file on disk is already the target, so the plan is a \
     skip, nothing is fetched, and the byte total is zero."
);

capture_test!(
    the_no_reuse_capture_fetches_the_whole_file,
    "no_reuse",
    "`no_reuse`: no chunk has a local source, so every chunk is fetched and \
     the file is written whole. A path with a space in it, which the \
     `--include` glob has to carry verbatim."
);

capture_test!(
    the_reuse_in_place_capture_fetches_only_what_it_must,
    "reuse_in_place",
    "`reuse_in_place`: the load-bearing reuse case — three chunks, one of \
     them already correct where it belongs and two fetched. The `REUSE` set \
     names the kept chunk, and the run must not ask for it."
);

capture_test!(
    the_reuse_moved_capture_fetches_only_what_it_must,
    "reuse_moved",
    "`reuse_moved`: the load-bearing reuse case at scale — 10 chunks, 8 reused \
     locally and 2 fetched, and the 4 that move go *earlier*. The `REUSE` set \
     names all 8."
);

capture_test!(
    the_added_capture_writes_the_new_file,
    "added",
    "`added`: the path is absent from `game/` and present in the new \
     manifest, so it is created and fetched whole."
);

capture_test!(
    the_removed_capture_purges_the_file,
    "removed",
    "`removed`: no repair, so no byte total — `totals: null`. It gets the \
     `--purge-only` run, and the file is gone afterwards."
);

capture_test!(
    persistent_res,
    "persistent_res",
    "`persistent_res`: the v4 bundle's stored key and record drive the run; \
     the res mirror, res blobs and res revision markers (including the eternal \
     one) are asserted, plus the full marker block and the read-only modes."
);

capture_test!(
    persistent_data,
    "persistent_data",
    "`persistent_data`: the same stored key and record, asserting the data \
     mirror, data blobs and data revision marker."
);

capture_test!(
    persistent_silence,
    "persistent_silence",
    "`persistent_silence`: the same stored key and record, asserting the \
     silence_data mirror, silence blob and silence revision marker."
);
