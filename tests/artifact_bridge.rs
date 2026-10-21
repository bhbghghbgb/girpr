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
//! Not "it would be rude under `cargo test`". These cases are **expensive**.
//! One capture is a `--print-plan` pre-pass plus a real run, and each run
//! fetches four manifests — ~20 MB of real bundle bytes — before it can plan a
//! single file. The whole suite is a handful of cases and **minutes** of
//! wall-clock, which is a price worth paying *when something has gone wrong*
//! and not worth paying on every `cargo test`.
//!
//! So it is run **by hand, deliberately**, and only when the repair spec is
//! suspected to have regressed:
//!
//! ```text
//! cargo test -p girpr --test artifact_bridge -- --ignored --nocapture
//! ```
//!
//! Nothing else runs it. `cargo test` still *compiles* this file, which is what
//! keeps it from rotting against `RunCtx`, `PathScope` and the `SUMMARY`
//! contract — a broken bridge would fail to build rather than fail silently.
//!
//! # Why a missing bundle is an error, not a skip
//!
//! Because this file is only ever run intentionally. A skip on a missing
//! prerequisite would turn "I ran the check and it found nothing to check" into
//! a green line — and that is the one reading this harness must never permit.
//! Nobody types `cargo test -- --ignored` here by accident, so the hazard a
//! skip guards against (people run it with `--ignored` forever and stop reading)
//! does not exist; its opposite does. If the bundle is missing, the run
//! **fails**, and says exactly what to run to produce one:
//!
//! ```text
//! cargo run -p gircap -- capture --out artifacts
//! # or, for a bundle elsewhere:
//! set GIRPR_ARTIFACT_BUNDLE=<dir>
//! ```
//!
//! Resolution order: `GIRPR_ARTIFACT_BUNDLE` always wins, then
//! `gircap list --root artifacts`, then a directory scan.
//!
//! # What this file is allowed to touch
//!
//! The bundle, read-only. The working tree is a scratch *copy* of the one
//! file the capture is about — pointing `--game-path` at the bundle in place
//! is what turns a cleanup flag into a way to destroy the evidence, and the
//! bundle has to still pass `gircap verify` afterwards.

#[path = "artifact_bridge/bundle.rs"]
mod bundle;
#[path = "artifact_bridge/server.rs"]
mod server;
#[path = "common/tempdir.rs"]
mod tempdir;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use bundle::{Capture, Patchmaps};
use server::{Bridge, Bundle, Origins};

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
             \x20   produce one with:  cargo run -p gircap -- capture --out {}\n\
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

/// `gircap verify --artifact <dir>`, exit `0`. **The precondition** of every
/// run (`docs/06` §3): it makes a failing check rule the bundle out as the
/// cause instead of leaving it as the first suspect.
///
/// Deliberately **not** re-implemented here — `gircap` owns that verdict, and
/// a second implementation would be a second thing to be wrong. When `gircap`
/// is not built the precondition is *skipped*, with its reason printed, rather
/// than replaced by a substitute.
///
/// A bundle that fails verification is a hard failure, not a skip: the
/// precondition is about an *absent* input turning people off `--ignored`
/// forever, and a corrupt bundle is a real problem that must not hide behind
/// a green run.
async fn assert_bundle_verifies(bundle: &Bundle) {
    let Some(exe) = gircap_exe() else {
        println!(
            "SKIP precondition: gircap is not built beside {}; skipping `gircap verify`",
            girpr_exe_dir().display()
        );
        return;
    };
    let out = tokio::process::Command::new(&exe)
        .args(["verify", "--artifact"])
        .arg(&bundle.root)
        .output()
        .await
        .expect("run gircap verify");
    let code = out.status.code().unwrap_or(-1);
    assert_eq!(
        code,
        0,
        "`gircap verify` must pass before the bridge runs anything against this bundle.\n\
         stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    println!(
        "precondition: gircap verify {} -> exit 0",
        bundle.root.display()
    );
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

/// Start the four listeners over the bundle under test.
async fn bridge() -> Bridge {
    let bundle = bundle();
    assert_bundle_verifies(&bundle).await;
    Bridge::start(bundle).await
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
    _guard: tempdir::TempDir,
}

impl Scratch {
    fn new(bundle: &Bundle, capture: &Capture) -> Self {
        let root = tempdir::TempDir::new("girpr_artifact_bridge", "scratch");
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
        "--io-threads".into(),
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
async fn drive_capture(bridge: &Bridge, capture: &Capture) {
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
    } else {
        let bytes = std::fs::read(&target)
            .unwrap_or_else(|e| panic!("{}: read patched {}: {e}", capture.id, capture.path));
        assert_eq!(
            format!("{:x}", md5::compute(&bytes)),
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
    let asked: BTreeSet<String> = bridge.chunk_ids().into_iter().collect();
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
async fn drive_named(bridge: &Bridge, id: &str) {
    let records = patchmaps(bridge.bundle());
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
    drive_capture(bridge, &capture).await;
}

macro_rules! capture_test {
    ($name:ident, $id:literal, $doc:literal) => {
        #[doc = $doc]
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "expensive: real girpr runs over bundle bytes. Run by hand only when a repair-spec change is suspected - cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
        async fn $name() {
            let bridge = bridge().await;
            drive_named(&bridge, $id).await;
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
    "zero_reuse_multi",
    "`zero_reuse_multi`: no chunk has a local source, so every chunk is \
     fetched and the file is written whole. A path with a space in it, \
     which the `--include` glob has to carry verbatim."
);

capture_test!(
    the_reuse_in_place_capture_fetches_only_what_it_must,
    "kept_in_place",
    "`kept_in_place`: the load-bearing reuse case — three chunks, one of \
     them already correct where it belongs and two fetched. The `REUSE` set \
     names the kept chunk, and the run must not ask for it."
);

capture_test!(
    the_reuse_moved_capture_fetches_only_what_it_must,
    "moved_ascending",
    "`moved_ascending`: the load-bearing reuse case at scale — 10 chunks, \
     8 reused locally and 2 fetched, and the 4 that move go *earlier*. The \
     `REUSE` set names all 8."
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
