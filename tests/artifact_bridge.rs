//! The `gircap`→`girpr` end-to-end artifact bridge.
//!
//! Spec: [`docs/06-artifact-bridge-spec.md`](../docs/06-artifact-bridge-spec.md).
//! Plan: [`docs/07-artifact-bridge-implementation-plan.md`](../docs/07-artifact-bridge-implementation-plan.md).
//!
//! `gircap capture` writes a self-contained bundle — a real `game/` fixture,
//! the API bodies and manifests behind it, every chunk blob, `expected/` ground
//! truth and a `patchmaps/<id>.json` per test-matrix variant. Three consumption
//! paths work (inspect the metadata, rebuild old/new offline, prove the
//! bundle's own integrity). **This file is the fourth**: run the real `girpr`
//! binary against the bundle's bytes, over loopback, and check it patches to
//! the new version.
//!
//! **Gate 1 — every test here is `#[ignore]`d.** Never runs under a bare
//! `cargo test`; still *compiles*, which is what keeps it from rotting against
//! `RunCtx`, `PathScope` and the `SUMMARY` contract. Invoke as:
//!
//! ```text
//! cargo test -p girpr --test artifact_bridge -- --ignored --nocapture
//! ```
//!
//! **Gate 2 — no bundle means skip, never fail.** `cargo test -- --ignored` is
//! a thing people type, and it must not turn red because nobody has captured
//! anything. Resolution order is `GIRPR_ARTIFACT_BUNDLE`, then a bundle
//! discovered under `artifacts/`, then a printed reason plus the command that
//! would produce one. The skip message is part of the contract: a bridge that
//! fails on a missing input teaches people to run it with `--ignored` forever
//! and stop reading.
//!
//! **What this file is allowed to touch.** The bundle, read-only. The working
//! tree is a scratch *copy* of the bundle's `game/` — pointing `--game-path`
//! at the bundle in place is what turns a cleanup flag into a way to destroy
//! the evidence, and the bundle has to still pass `gircap verify` afterwards.

// The server's surface is wider than M1's half of it exercises — the request
// log, the per-capture lookups and the `--purge-only` predicate are read by the
// driver milestones (M2/M3), so the allowance is about the milestone split and
// not about code nothing reaches.
#[allow(dead_code)]
#[path = "artifact_bridge/server.rs"]
mod server;

use std::path::{Path, PathBuf};

use server::{Bridge, Bundle};

// ---------------------------------------------------------------------------
// gate 2: locating a bundle
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
/// is any directory holding a `MANIFEST.json`.
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

/// Resolve the bundle under test, or `None` with the reason and the exact
/// command that would produce one.
fn locate_bundle() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("GIRPR_ARTIFACT_BUNDLE") {
        let p = PathBuf::from(explicit);
        assert!(
            p.join("MANIFEST.json").is_file(),
            "GIRPR_ARTIFACT_BUNDLE={} is not a bundle (no MANIFEST.json)",
            p.display()
        );
        return Some(p);
    }
    let root = artifacts_root();
    let found = scan_for_bundles(&root);
    match found.len() {
        0 => {
            println!(
                "SKIP artifact_bridge: no bundle under {}\n\
                 \x20     produce one with:  cargo run -p gircap -- capture --out {}\n\
                 \x20     or point at an existing one:  set GIRPR_ARTIFACT_BUNDLE=<dir>",
                root.display(),
                root.display()
            );
            None
        }
        1 => Some(found.into_iter().next().expect("len 1")),
        n => {
            let list: Vec<String> = found.iter().map(|p| format!("  {}", p.display())).collect();
            println!(
                "SKIP artifact_bridge: {n} bundles under {}; name one with GIRPR_ARTIFACT_BUNDLE\n{}",
                root.display(),
                list.join("\n")
            );
            None
        }
    }
}

/// The bundle under test, or a skip. Gate 2.
fn bundle_or_skip() -> Option<Bundle> {
    locate_bundle().map(Bundle::open)
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
/// A bundle that fails verification is a hard failure, not a skip: Gate 2 is
/// about an *absent* input turning people off `--ignored` forever, and a
/// corrupt bundle is a real problem that must not hide behind a green run.
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

/// Start the four listeners over `bundle`.
async fn bridge_or_skip() -> Option<Bridge> {
    let bundle = bundle_or_skip()?;
    assert_bundle_verifies(&bundle).await;
    Some(Bridge::start(bundle).await)
}

/// A one-shot HTTP GET against the bridge, returning `(status, content-type,
/// body)`. Status is the number, so a test compares it against a recorded shape
/// rather than against a rendered status line.
async fn get(url: &str) -> (u16, String, Vec<u8>) {
    let resp = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET {url}: {e}"));
    let status = resp.status().as_u16();
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = resp.bytes().await.expect("body").to_vec();
    (status, ctype, body)
}

// ---------------------------------------------------------------------------
// M2 — the driver
// ---------------------------------------------------------------------------

use std::collections::{BTreeMap, BTreeSet};

use server::{Capture, Origins};

/// A scratch copy of the bundle's **whole** `game/` fixture, removed on drop.
///
/// The copy is of the whole fixture rather than of the one selected path
/// because a capture is one path *out of* the fixture: the other captures' files
/// are what "every path outside the scope is untouched" is measured against.
/// `docs/06` §6.4 — the bundle is the input, a copy is the working tree.
/// Pointing `--game-path` at the bundle in place is what turns a cleanup flag
/// into a way to destroy the evidence.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(bundle: &Bundle) -> Self {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("girpr_artifact_bridge_{}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create scratch root");
        copy_tree(&bundle.root.join("game"), &root.join("game"));
        Self { root }
    }

    fn game(&self) -> PathBuf {
        self.root.join("game")
    }

    /// Somewhere for the binary to put its trace log, so the run does not
    /// scatter `logs/` under the build directory.
    ///
    /// The pre-pass gets its **own** file: `logging::open_log_file` opens with
    /// `append`, so one shared file would carry both runs' records and the
    /// log-file cross-check could not say which run downloaded what.
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

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create dst");
    for entry in std::fs::read_dir(src).unwrap_or_else(|e| panic!("read {}: {e}", src.display())) {
        let entry = entry.expect("read dir entry");
        let to = dst.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), &to).expect("copy file");
        }
    }
}

/// Every file under `game/` as `rel -> (bytes, md5)`, `/`-separated.
///
/// MD5 rather than length and timestamp, because "untouched" has to mean
/// *byte-identical*: a run that rewrote a file to the same length would
/// otherwise pass.
fn snapshot(game: &Path) -> BTreeMap<String, (u64, String)> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, (u64, String)>) {
        for entry in
            std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        {
            let entry = entry.expect("read dir entry");
            let path = entry.path();
            if entry.file_type().expect("file type").is_dir() {
                walk(&path, base, out);
                continue;
            }
            let rel = path
                .strip_prefix(base)
                .expect("under the base")
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(&path).expect("read file");
            out.insert(
                rel,
                (bytes.len() as u64, format!("{:x}", md5::compute(&bytes))),
            );
        }
    }
    let mut out = BTreeMap::new();
    walk(game, game, &mut out);
    out
}

/// The chunk ids a trace log records as **downloaded** (`docs/06` §4.7's
/// documented last resort).
///
/// Only the two download records are matched:
///
/// | record | site | when |
/// |---|---|---|
/// | `chunk downloaded chunk=<id>` | `sophon::try_fetch_chunk` | the body was read — a real download |
/// | `no local reuse candidate -> download chunk=<id>` | `repair::file::try_reuse_slice` | the decision, before the fetch |
///
/// **Never the bare id.** A reused chunk is logged too — `reuse slice md5 ok
/// -> copy chunk=<id>` and `chunk already complete -> keep chunk=<id>` — so a
/// substring search for an id finds it in the "reused it" line and
/// false-positives.
fn downloaded_chunks(log: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in log.lines() {
        for marker in ["chunk downloaded ", "no local reuse candidate -> download "] {
            let Some(at) = line.find(marker) else {
                continue;
            };
            let rest = &line[at + marker.len()..];
            let Some(id) = rest.strip_prefix("chunk=") else {
                continue;
            };
            let id = id.split_whitespace().next().unwrap_or_default();
            if !id.is_empty() {
                out.insert(id.to_string());
            }
        }
    }
    out
}

/// Which paths a before/after pair disagrees about, added and removed.
fn tree_delta(
    before: &BTreeMap<String, (u64, String)>,
    after: &BTreeMap<String, (u64, String)>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let added: BTreeSet<String> = after
        .keys()
        .filter(|k| !before.contains_key(*k))
        .cloned()
        .collect();
    let removed: BTreeSet<String> = before
        .keys()
        .filter(|k| !after.contains_key(*k))
        .cloned()
        .collect();
    (added, removed)
}

/// Paths whose bytes moved: same key, different `(len, md5)`.
fn tree_changed(
    before: &BTreeMap<String, (u64, String)>,
    after: &BTreeMap<String, (u64, String)>,
) -> BTreeSet<String> {
    before
        .iter()
        .filter(|(k, v)| after.get(*k).is_some_and(|w| w != *v))
        .map(|(k, _)| k.clone())
        .collect()
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
/// and `GenshinImpact_Data/Persistent/audio_lang_14` does. Omitting it lets the
/// run autodetect from the scan file and resolve to `en-us` — the state the
/// bundle was captured in.
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
fn capture_args(capture: &Capture) -> Vec<String> {
    let mut args = vec![
        // Exactly one path: the patchmap's own `path` field.
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
        args.push("--max-files".into());
        args.push("1".into());
    }
    args
}

/// Drive one capture end to end and assert `docs/06` §6's four per-capture
/// facts plus the §4.3 degraded-but-green guards.
///
/// **One run per capture**, always: a `download_bytes` total is only
/// attributable to a single capture.
async fn drive_capture(bridge: &Bridge, capture: &Capture) {
    let bundle = bridge.bundle().clone();
    let scratch = Scratch::new(&bundle);
    let before = snapshot(&scratch.game());

    // ---- the free pre-pass (docs/06 5.1) --------------------------------
    //
    // `--print-plan` returns before `create_dir_all`, so it costs no disk and
    // separates the two failure modes that otherwise arrive tangled in one red
    // run: when `files=1` fails the glob is wrong; when the real run's
    // `download_bytes` fails, reuse is wrong.
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

    match capture.download_bytes {
        // A repair capture: the estimate IS the patchmap's total. Equal by
        // construction (A6 asserts `len(blob) == compressed_size`), and it is
        // the assertion that catches a run whose local manifest did not load:
        // with an empty reuse map the estimate becomes the whole file.
        Some(total) => {
            assert_eq!(
                plan.num("plan", "est_download_bytes"),
                total,
                "the pre-pass estimate must equal the patchmap total for {}",
                capture.id
            );
            assert_eq!(plan.num("plan", "purge_files"), 0, "{}", capture.id);
            assert_eq!(plan.num("plan", "purge_bytes"), 0, "{}", capture.id);
        }
        // `removed`: the rule of 5.1 is false for this capture. A `removed`
        // patchmap carries `totals: null` because there is nothing to patch, so
        // the *purge* counters are the assertion.
        None => {
            assert_eq!(
                plan.num("plan", "purge_files"),
                1,
                "{}: the cleanup must find exactly the removed path",
                capture.id
            );
            assert_eq!(
                plan.num("plan", "purge_bytes"),
                capture.old_size,
                "{}: priced from the old file's size",
                capture.id
            );
            assert_eq!(plan.num("plan", "est_download_bytes"), 0, "{}", capture.id);
        }
    }

    // ---- the real run ---------------------------------------------------
    let mut args = base_args(
        &scratch.game(),
        &bundle,
        bridge.origins(),
        &scratch.log_file(),
    );
    args.extend(capture_args(capture));
    let run = girpr(&args).await;

    // ---- 4: exit 0, config_bumped=0, nothing else touched --------------
    assert_eq!(
        run.code, 0,
        "{}: exit must be 0\n{}\n{}",
        capture.id, run.stdout, run.stderr
    );
    assert_eq!(run.num("summary", "exit"), 0, "{}", capture.id);
    assert_eq!(
        run.num("summary", "config_bumped"),
        0,
        "{}: a scoped run deliberately did not put the bytes in place",
        capture.id
    );
    assert_eq!(run.num("summary", "scoped"), 1, "{}", capture.id);
    // §4.3: the one degradation that is otherwise invisible. A tag the shim does
    // not serve degrades to the local-build fallback: an empty reuse map, every
    // chunk downloaded, and still exit 0. This key and the byte comparison
    // below are the two things that catch it.
    assert_eq!(
        run.word("summary", "local_manifest"),
        "ok",
        "{}: the local build's manifests must have loaded",
        capture.id
    );

    // ---- 1: the byte total, exactly ------------------------------------
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

    // ---- 2: the patched file's MD5 -------------------------------------
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
            "{}: new size",
            capture.id
        );
    }

    // ---- 3: no forbidden chunk id was ever requested --------------------
    let asked: BTreeSet<String> = bridge.chunk_ids().into_iter().collect();
    let forbidden: Vec<&String> = capture
        .must_not_fetch
        .iter()
        .filter(|id| asked.contains(*id))
        .collect();
    assert!(
        forbidden.is_empty(),
        "{}: these ids are in must_not_fetch, so they must not have been requested \
         (reuse was broken): {:?}\nserver saw {} chunk request(s)",
        capture.id,
        forbidden,
        asked.len()
    );

    // The same fact from the **documented last resort** (`docs/06` §4.7), which
    // is what makes that rule worth writing down at all. The server's log is
    // the primary source; this is the cross-check that `--log-file` parsing
    // would actually work, so it has to agree.
    //
    // Note what is matched: the two *download records*, never the bare id. A
    // reused chunk is logged too — `reuse slice md5 ok -> copy chunk=<id>` —
    // so a substring search for an id finds it there and false-positives. That
    // is not hypothetical: the first attempt at this check failed on 19 chunks
    // that were never downloaded, for exactly that reason.
    let log = std::fs::read_to_string(scratch.log_file()).unwrap_or_default();
    let downloaded = downloaded_chunks(&log);
    assert_eq!(
        downloaded, asked,
        "{}: the trace log's download records must agree with the server's request log",
        capture.id
    );
    let logged_forbidden: Vec<&String> = capture
        .must_not_fetch
        .iter()
        .filter(|id| downloaded.contains(*id))
        .collect();
    assert!(
        logged_forbidden.is_empty(),
        "{}: must_not_fetch ids reported as downloaded: {logged_forbidden:?}",
        capture.id
    );

    // ---- 4, continued: the before/after tree diff ----------------------
    //
    // The only check that the scope held across the whole filesystem rather
    // than just the repair plan. Per run, because §6.3 gives every capture its
    // own copy: a shared baseline would compare two different trees.
    let after = snapshot(&scratch.game());
    let (added, removed) = tree_delta(&before, &after);
    let changed = tree_changed(&before, &after);
    let selected = capture.path.as_str();
    let outside = added
        .iter()
        .chain(removed.iter())
        .chain(changed.iter())
        .filter(|p| p.as_str() != selected)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert!(
        outside.is_empty(),
        "{}: every path outside the scope must be untouched on disk. \
         changed outside the scope: {outside:?}\nadded: {added:?}\nremoved: {removed:?}",
        capture.id
    );

    // The `build_id` invariant of `docs/06` §4.1, measured through a real run
    // rather than probed: every blob request's `<build_id>` must equal the build
    // whose manifest declared that id. Only chunks are checked here because only
    // chunk downloads carry the prefix `girpr` builds from the latest build's
    // body (manifests were pinned directly in M1).
    let wrong_build: Vec<(String, String)> = bridge
        .chunk_requests()
        .into_iter()
        .filter(|(b, _)| *b != bundle.new_build)
        .collect();
    assert!(
        wrong_build.is_empty(),
        "{}: every chunk request must carry the new build's id ({}); these did not: \
         {wrong_build:?}",
        capture.id,
        bundle.new_build
    );

    // ---- per-class expectations (docs/06 6.2) ---------------------------
    //
    // The four assertions above are the same for every capture; these are the
    // places a class is *different in kind* rather than in number.
    for class in &capture.classes {
        match class.as_str() {
            // Planned, then skipped on size+MD5: not downloaded, not touched.
            // `must_not_fetch` is the only thing that proves it, since
            // `download_bytes=0` is also what a broken run reports.
            "identical" => {
                assert_eq!(
                    run.num("summary", "total"),
                    1,
                    "{}: it is planned",
                    capture.id
                );
                assert_eq!(run.num("summary", "skipped"), 1, "{}", capture.id);
                assert_eq!(run.num("summary", "repaired"), 0, "{}", capture.id);
                assert_eq!(run.num("summary", "download_bytes"), 0, "{}", capture.id);
                assert!(
                    asked.is_empty(),
                    "{}: nothing may be downloaded",
                    capture.id
                );
                assert!(
                    added.is_empty() && removed.is_empty() && changed.is_empty(),
                    "{}: an intact file must not be touched at all; \
                     added {added:?} removed {removed:?} changed {changed:?}",
                    capture.id
                );
            }
            // The path is absent from `game/` and present in the new manifest, so
            // it is planned and downloaded in full — which makes it the one
            // class where the tree diff must show an *addition*.
            "added" => {
                assert_eq!(run.num("summary", "repaired"), 1, "{}", capture.id);
                assert!(
                    capture.must_not_fetch.is_empty(),
                    "{}: nothing to reuse",
                    capture.id
                );
                assert!(
                    added.contains(selected),
                    "{}: the file is new on disk",
                    capture.id
                );
                assert!(changed.is_empty() && removed.is_empty(), "{}", capture.id);
            }
            // No repair, so no byte total: the cleanup deletes it and the
            // counters are the assertion.
            "removed" => {
                assert_eq!(run.num("summary", "total"), 0, "{}", capture.id);
                assert_eq!(run.num("summary", "repaired"), 0, "{}", capture.id);
                assert_eq!(run.num("summary", "skipped"), 0, "{}", capture.id);
                assert_eq!(
                    run.num("summary", "deleted_extra_bytes"),
                    capture.old_size,
                    "{}: the cleanup deleted the file the patchmap priced",
                    capture.id
                );
                assert!(
                    removed.contains(selected),
                    "{}: it must be gone",
                    capture.id
                );
                assert!(added.is_empty() && changed.is_empty(), "{}", capture.id);
                assert!(asked.is_empty(), "{}: nothing is downloaded", capture.id);
            }
            // The three patchable classes and the two zero-reuse classes are
            // the load-bearing ones, and assertions 1-3 above already bite on
            // them: the byte total is the patchmap's, the result MD5 is the
            // ground truth, and `must_not_fetch` is non-empty for every partial.
            "partial_same_offset"
            | "partial_moved_offset"
            | "partial_size_changed"
            | "zero_reuse_single"
            | "zero_reuse_multi" => {
                assert_eq!(
                    run.num("summary", "repaired"),
                    1,
                    "{}: a patchable file is repaired, not skipped",
                    capture.id
                );
                assert_eq!(run.num("summary", "skipped"), 0, "{}", capture.id);
                assert!(
                    changed.contains(selected),
                    "{}: the bytes on disk must have moved",
                    capture.id
                );
            }
            other => println!(
                "note: {} is class {other:?}, which docs/06 6.2 does not describe; \
                 the four common assertions still applied",
                capture.id
            ),
        }
    }

    // The report says what the tool believes; the diff is the proof.
    assert_eq!(
        run.word("report", "local_version"),
        bundle.old_tag,
        "{}: the fixture is materialized at the old tag",
        capture.id
    );
    assert_eq!(
        run.word("report", "latest_version"),
        bundle.new_tag,
        "{}",
        capture.id
    );

    println!(
        "bridge {:<24} plan files={} est={} | run total={} skipped={} repaired={} failed={} \
         dl={} deleted={} | chunks requested={} | tree +{} -{} ~{}",
        capture.id,
        plan.num("plan", "files"),
        plan.num("plan", "est_download_bytes"),
        run.num("summary", "total"),
        run.num("summary", "skipped"),
        run.num("summary", "repaired"),
        run.num("summary", "failed"),
        run.num("summary", "download_bytes"),
        run.num("summary", "deleted_extra_bytes"),
        asked.len(),
        added.len(),
        removed.len(),
        changed.len(),
    );
}

/// One capture, end to end, against real bundle bytes.
///
/// `partial_same_offset` is the load-bearing first case: it is the one
/// `docs/06` quotes a figure for (`download_bytes=274426`), and it is a
/// three-chunk file where two chunks must come from the local reuse map and one
/// pair must come off the network — so assertions 1, 2 and 3 all bite at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn one_capture_patches_to_the_new_version() {
    let Some(bridge) = bridge_or_skip().await else {
        return;
    };
    let capture = bridge
        .capture("partial_same_offset")
        .unwrap_or_else(|| {
            panic!(
                "this bundle has no partial_same_offset capture: {:?}",
                bridge
                    .bundle()
                    .captures
                    .iter()
                    .map(|c| &c.id)
                    .collect::<Vec<_>>()
            )
        })
        .clone();
    drive_capture(&bridge, &capture).await;
}

// ---------------------------------------------------------------------------
// M3 — every capture, each in its own world
// ---------------------------------------------------------------------------
//
// Rust runs `#[test]`s in parallel threads, so one test per capture is not a
// style choice: it is what gives every capture its own scratch `game/` copy,
// its own listeners, its own request log and its own before/after diff. A
// shared tree and a shared log would mean two runs fighting over one path, one
// run's `must_not_fetch` seeing the other's requests, and a diff comparing a
// tree some other run had already changed — intermittent, and it would look
// like a product bug (`docs/06` §6.3).
//
// One run per capture is kept regardless, because it is what makes
// `download_bytes` attributable to a single capture.

/// Drive one named capture, skipping cleanly when the bundle does not carry it.
///
/// A bundle may legitimately lack a class — `MANIFEST.json`'s matrix records
/// `status: unavailable` with a reason for those. Skipping with the reason is
/// right; failing is not, for the same reason Gate 2 is not.
async fn drive_named(bridge: &Bridge, id: &str) {
    let Some(capture) = bridge.capture(id).cloned() else {
        println!(
            "SKIP {id}: this bundle's matrix does not carry that class (it has: {:?})",
            bridge
                .bundle()
                .captures
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>()
        );
        return;
    };
    drive_capture(bridge, &capture).await;
}

macro_rules! capture_test {
    ($name:ident, $id:literal, $doc:literal) => {
        #[doc = $doc]
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
        async fn $name() {
            let Some(bridge) = bridge_or_skip().await else {
                return;
            };
            drive_named(&bridge, $id).await;
        }
    };
}

capture_test!(
    capture_identical_is_planned_and_skipped,
    "identical",
    "`identical`: planned, then skipped on size+MD5. `download_bytes=0` is also\n\
     what a *broken* run reports, so the load-bearing assertion here is the\n\
     server log: not one chunk requested. The file must not be touched at all."
);
capture_test!(
    capture_zero_reuse_single_downloads_its_whole_file,
    "zero_reuse_single",
    "`zero_reuse_single`: one chunk in the new manifest with no match on the old\n\
     side, so the whole (tiny) file is downloaded. Assertions 1-3 all bite."
);
capture_test!(
    capture_zero_reuse_multi_downloads_its_whole_file,
    "zero_reuse_multi",
    "`zero_reuse_multi`: the same, over a file with several chunks. A path with a\n\
     space in it, which the `--include` glob has to carry verbatim."
);
capture_test!(
    capture_partial_moved_offset_reuses_the_nine_it_can,
    "partial_moved_offset",
    "`partial_moved_offset`: the load-bearing reuse case at scale — 10 chunks,\n\
     8 reused locally and 2 fetched. `must_not_fetch` names all 8."
);
capture_test!(
    capture_partial_size_changed_reuses_the_nine_it_can,
    "partial_size_changed",
    "`partial_size_changed`: 10 chunks, 9 reused, 1 fetched (197192 bytes). The\n\
     file also changes size, so the result MD5 is not reachable by reusing the\n\
     old file whole."
);
capture_test!(
    capture_added_is_downloaded_in_full,
    "added",
    "`added`: the path is absent from `game/` and present in the new manifest, so\n\
     it is planned and downloaded in full. `must_not_fetch` is empty, and the\n\
     tree diff must show an *addition* rather than a change."
);
capture_test!(
    capture_removed_is_purged_by_the_cleanup,
    "removed",
    "`removed`: no repair, so no byte total — `totals: null`. It gets the\n\
     `--purge-only` run of `docs/06` §5 and the purge-counter pre-pass of §5.1,\n\
     where `files=0` and `purge_files=1 purge_bytes=1731133` are the assertion."
);

/// `full_reuse_reordered` is the ninth matrix class, and `MANIFEST.json`
/// records it as **unavailable** for 7.0.0 -> 7.1.0 — no path in any selected
/// field satisfies it. A class that never materialised must be skipped with its
/// reason, never invented, and the two records must agree in both directions.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn the_matrix_and_the_patchmaps_agree_in_both_directions() {
    let Some(bridge) = bridge_or_skip().await else {
        return;
    };
    let bundle = bridge.bundle();
    let matrix = read_json(&bundle.root.join("MANIFEST.json"));
    let entries = matrix["matrix"].as_array().expect("MANIFEST.json matrix[]");

    for entry in entries {
        let status = entry["status"].as_str().unwrap_or_default();
        // `class` is on every entry; `capture_id` is only on the captured ones —
        // an entry that never materialised has no capture id to give.
        let id = entry["class"].as_str().expect("matrix[].class");
        let present = bridge.capture(id).is_some();
        if status == "captured" {
            assert!(
                present,
                "{id}: recorded captured, but there is no patchmaps/{id}.json"
            );
            assert_eq!(
                entry["capture_id"].as_str(),
                Some(id),
                "{id}: a captured entry's capture_id is the patchmap stem, i.e. its class"
            );
        } else {
            assert!(
                entry["reason"].as_str().is_some_and(|r| !r.is_empty()),
                "{id}: an unavailable class must carry its reason"
            );
            assert!(
                !present,
                "{id}: recorded unavailable, yet the bundle carries a patchmap for it"
            );
        }
    }
    // ...and nothing on disk that the matrix does not mention.
    for c in &bundle.captures {
        assert!(
            entries
                .iter()
                .any(|e| e["class"].as_str() == Some(c.id.as_str())),
            "{}: a patchmap with no matrix entry",
            c.id
        );
    }
    println!(
        "matrix: {} entr(ies), {} capture(s) on disk, {} recorded captured",
        entries.len(),
        bundle.captures.len(),
        entries
            .iter()
            .filter(|e| e["status"].as_str() == Some("captured"))
            .count()
    );
}

/// Read a JSON file from the bundle. Kept local so the driver never grows a
/// second bundle reader beside the server's.
fn read_json(path: &Path) -> serde_json::Value {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_slice(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// M4 — not-found is replayed, not invented (`docs/06` §4.4)
// ---------------------------------------------------------------------------

/// Blank out the fields `docs/06` §4.4.1 marks volatile, so two bodies can be
/// compared for everything else.
fn without_volatile(body: &str) -> String {
    let mut out = body.to_string();
    for tag in ["Key", "RequestId"] {
        if let (Some(a), Some(b)) = (
            out.find(&format!("<{tag}>")),
            out.find(&format!("</{tag}>")),
        ) && b > a
        {
            out.replace_range(a..b + tag.len() + 3, &format!("<{tag}/>"));
        }
    }
    out
}

/// The length of `<Key>`'s contents, which is the only part of a recorded
/// shape whose length legitimately changes on replay.
fn key_len(body: &str) -> usize {
    body.split("<Key>")
        .nth(1)
        .and_then(|s| s.split("</Key>").next())
        .map(str::len)
        .unwrap_or(0)
}

/// Assert one 404 against the shape the bundle recorded for it.
async fn assert_replays(bridge: &Bridge, shape_id: &str, url: &str) {
    let (status, ctype, body) = get(url).await;
    let Some((rec_status, rec_ctype, rec_body)) = bridge.recorded_shape(shape_id) else {
        panic!(
            "{shape_id}: this bundle has no api/error_shapes.json, so there is no \
             evidence to replay. That is the `shapes_synthesized` case, not this one."
        )
    };
    // `girpr` calls `.error_for_status()` and never reads the body, so status
    // and content-type are the contract and the body is fidelity.
    assert_eq!(status, rec_status, "{shape_id}: replayed status");
    assert_eq!(ctype, rec_ctype, "{shape_id}: replayed content-type");

    let replayed = String::from_utf8_lossy(&body).into_owned();
    assert_eq!(
        without_volatile(&replayed),
        without_volatile(&rec_body),
        "{shape_id}: only RequestId / Key (and the length that follows from Key) \
         may differ from the recorded body"
    );
    // The body must name the key that was actually requested, and its length
    // must follow from that substitution rather than being copied verbatim.
    let requested = url
        .split_once("://")
        .map(|(_, rest)| rest.split_once('/').map(|(_, p)| p).unwrap_or_default())
        .unwrap_or_default();
    assert!(
        replayed.contains(&format!("<Key>{requested}</Key>")),
        "{shape_id}: the replayed body must name the requested key"
    );
    assert_eq!(
        replayed.len(),
        rec_body.len() - key_len(&rec_body) + requested.len(),
        "{shape_id}: the body length must follow from the substituted Key alone"
    );
    assert!(
        !replayed.contains("6AD16DF4"),
        "{shape_id}: the recorded RequestId is marked volatile and must be synthesized"
    );
}

/// The two route families answer failure **differently**, and both answers are
/// replayed from what the bundle recorded rather than chosen here. Measured on
/// 2026-10-16 and tabulated in `docs/06` §4.4:
///
/// | family | request | answer |
/// |---|---|---|
/// | blob (`AliyunOSS`) | bogus id, unknown build, a valid id under the other build | `404` + `application/xml` `NoSuchKey` |
/// | JSON (`sg-downloader-api`) | `getBuild?tag=<unknown>` | `200` + `retcode -202` |
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn not_found_is_replayed_from_the_recorded_shapes() {
    let Some(bridge) = bridge_or_skip().await else {
        return;
    };
    let bundle = bridge.bundle().clone();
    let o = bridge.origins().clone();
    assert!(
        !bridge.shapes_synthesized(),
        "this bundle carries api/error_shapes.json, so these are replayed"
    );

    let missing = "0000000000000000_00000000000000000000000000000000";
    let manifest_ns = format!(
        "{}/client_app/sophon/manifests/cxhpq4g4rgg0",
        o.manifest_origin
    );
    let chunk_ns = format!("{}/client_app/sophon/chunks/cxhpq4g4rgg0", o.chunk_origin);

    // The blob family: 404 + application/xml, for all three recorded reasons.
    assert_replays(
        &bridge,
        "blob_missing",
        &format!("{manifest_ns}/{}/{missing}", bundle.new_build),
    )
    .await;
    assert_replays(
        &bridge,
        "chunk_missing",
        &format!("{chunk_ns}/{}/{missing}", bundle.new_build),
    )
    .await;
    assert_replays(
        &bridge,
        "blob_wrong_build",
        &format!("{chunk_ns}/{}/{missing}", bundle.old_build),
    )
    .await;

    // The JSON family: 200 + retcode -202, which is the call that populates the
    // chunk-reuse map.
    let (status, ctype, body) = get(&format!(
        "{}/getBuild?branch=main&package_id=x&password=&tag=0.0.0-nope",
        o.sophon_base
    ))
    .await;
    let (rec_status, rec_ctype, rec_body) = bridge
        .recorded_shape("getbuild_unknown_tag")
        .expect("recorded");
    assert_eq!((status, ctype), (rec_status, rec_ctype));
    assert_eq!(String::from_utf8_lossy(&body), rec_body);

    // The document's other three shapes have no route here, because the bridge
    // never issues such a request: a bad `package_id` or a bad `game_ids[]` is
    // something only `gircap capture` sends. They are still checked for
    // presence and shape, so the bundle's record is read rather than assumed.
    for (id, status, retcode) in [
        ("getbuild_bad_package", 200u16, -201i64),
        ("getgameconfigs_bad_game_id", 200, -1001),
        ("getgamebranches_bad_game_id", 200, 0),
    ] {
        let (s, ct, b) = bridge.recorded_shape(id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!((s, ct.as_str()), (status, "application/json"), "{id}");
        let v: serde_json::Value = serde_json::from_str(&b).expect("json body");
        assert_eq!(v["retcode"].as_i64(), Some(retcode), "{id}");
    }
    println!(
        "not-found: 4 shapes replayed on their routes, 3 more present and unmodified \
         (no route: the bridge never issues those requests)"
    );
}

/// A bundle that **predates** `api/error_shapes.json` has no evidence to
/// replay. The shim then answers with the measured live shape — `404` +
/// `application/xml` — and **says so**, rather than passing the not-found
/// assertions silently. Inventing the statuses would make `docs/06`'s claim
/// about the API unfounded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn a_bundle_without_recorded_shapes_says_so_instead_of_inventing() {
    let Some(bundle) = bundle_or_skip() else {
        return;
    };
    let shadow = ShadowBundle::new(&bundle);
    assert!(
        !shadow.dir().join("api").join("error_shapes.json").exists(),
        "the shadow bundle must not carry the file it is here to test the absence of"
    );
    let bridge = Bridge::start(Bundle::open(shadow.dir())).await;
    assert!(
        bridge.shapes_synthesized(),
        "with no api/error_shapes.json the shapes are synthesized, and the bridge \
         must be able to say so"
    );
    assert!(
        bridge.recorded_shape("chunk_missing").is_none(),
        "there is nothing recorded to replay"
    );

    let b = bridge.bundle().clone();
    let o = bridge.origins().clone();
    let missing = "0000000000000000_00000000000000000000000000000000";
    for url in [
        format!(
            "{}/client_app/sophon/chunks/cxhpq4g4rgg0/{}/{missing}",
            o.chunk_origin, b.new_build
        ),
        format!(
            "{}/client_app/sophon/manifests/cxhpq4g4rgg0/{}/{missing}",
            o.manifest_origin, b.new_build
        ),
    ] {
        let (status, ctype, body) = get(&url).await;
        assert_eq!(status, 404);
        assert_eq!(
            ctype, "application/xml",
            "the measured blob 404 content-type"
        );
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("NoSuchKey"), "{text}");
        assert!(text.contains(&format!(
            "<Key>{}</Key>",
            url.split_once("://").unwrap().1.split_once('/').unwrap().1
        )));
    }
    println!(
        "SKIP the replayed-shape assertions for {}: the bundle carries no \
         api/error_shapes.json, so the 404 shape was synthesized from the measured \
         live behaviour rather than replayed from evidence",
        b.root.display()
    );
}

/// The bundle's small files, without `api/error_shapes.json`.
///
/// Only the files the server reads at `load()` time are copied — `MANIFEST.json`,
/// `patchmaps/` and the four `api/` bodies, ~120 KB in total. `manifests/` and
/// `chunks/` are deliberately **not** present: nothing in this test asks for a
/// real id, and inventing placeholder archives would be exactly the "serves
/// something the bundle does not vouch for" the spec forbids.
struct ShadowBundle {
    dir: PathBuf,
}

impl ShadowBundle {
    fn new(bundle: &Bundle) -> Self {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "girpr_artifact_bridge_noshapes_{}_{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("api")).expect("create shadow api/");
        std::fs::create_dir_all(dir.join("patchmaps")).expect("create shadow patchmaps/");
        std::fs::copy(bundle.root.join("MANIFEST.json"), dir.join("MANIFEST.json")).expect("copy");
        copy_tree(&bundle.root.join("patchmaps"), &dir.join("patchmaps"));
        for name in [
            "getGameConfigs.json",
            "getGameBranches.json",
            "getGameDeprecatedFileConfigs.json",
            &format!("getBuild.{}.json", bundle.old_tag),
            &format!("getBuild.{}.json", bundle.new_tag),
        ] {
            std::fs::copy(
                bundle.root.join("api").join(name),
                dir.join("api").join(name),
            )
            .unwrap_or_else(|e| panic!("copy api/{name}: {e}"));
        }
        Self { dir }
    }

    fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for ShadowBundle {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The rewrite must be surgical. `sophon::fetch_manifest` MD5-verifies the
/// **decompressed** manifest against `manifest.checksum`, so a checksum that
/// moved kills the run at metadata with exit `2` — a failure that looks like a
/// product bug and is not one. `manifest.id` is the URL path segment.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn the_rewrite_changes_only_the_two_url_prefixes() {
    let Some(bridge) = bridge_or_skip().await else {
        return;
    };
    let bundle = bridge.bundle();

    for tag in [&bundle.old_tag, &bundle.new_tag] {
        let recorded: serde_json::Value =
            serde_json::from_slice(&bundle.recorded_build_body(tag)).expect("recorded body");
        let served: serde_json::Value =
            serde_json::from_slice(&bridge.served_build(tag).expect("tag is served"))
                .expect("served body");

        assert_eq!(
            recorded["retcode"], served["retcode"],
            "{tag}: the envelope must be untouched"
        );
        assert_eq!(recorded["message"], served["message"], "{tag}");
        let rec = recorded["data"]["manifests"]
            .as_array()
            .expect("recorded[]")
            .clone();
        let srv = served["data"]["manifests"]
            .as_array()
            .expect("served[]")
            .clone();
        assert_eq!(rec.len(), srv.len(), "{tag}: manifest count changed");
        for (r, s) in rec.iter().zip(srv.iter()) {
            // The claim, pinned by comparing everything that is not one of the
            // two rewritten fields. `manifest` is compared whole, so checksum
            // and id are covered; they are then pinned by name below because
            // *why* they must not move is not obvious from an equality.
            for key in [
                "category_id",
                "category_name",
                "matching_field",
                "stats",
                "deduplicated_stats",
            ] {
                assert_eq!(r[key], s[key], "{tag}: {key} must be identical");
            }
            assert_eq!(
                r["manifest"], s["manifest"],
                "{tag}: manifest{{}} identical"
            );
            assert_eq!(
                s["manifest"]["checksum"], r["manifest"]["checksum"],
                "manifest.checksum is MD5 of the DECOMPRESSED manifest, verified before the \
                 protobuf is parsed; moving it is a metadata failure that looks like a product bug"
            );
            assert_eq!(
                s["manifest"]["id"], r["manifest"]["id"],
                "manifest.id is the URL path segment"
            );
            // `url_suffix` is irrelevant — `sophon::join_url` builds
            // `{prefix}/{id}` and never reads it — but it is still recorded,
            // so it must still be the recorded one.
            for field in ["manifest_download", "chunk_download"] {
                for key in ["url_suffix", "encryption", "password", "compression"] {
                    assert_eq!(
                        r[field][key], s[field][key],
                        "{tag}: {field}.{key} must be identical"
                    );
                }
            }
        }
    }
}

/// Each namespace gets its **own** origin, and the path is kept byte-for-byte
/// so a blob route can be "take the last two segments and ignore the rest" —
/// which is what makes the route indistinguishable from production's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn each_namespace_keeps_its_path_and_gets_its_own_origin() {
    let Some(bridge) = bridge_or_skip().await else {
        return;
    };
    let bundle = bridge.bundle();
    let o = bridge.origins().clone();
    assert_ne!(
        o.manifest_origin, o.chunk_origin,
        "the two ports must differ"
    );

    for tag in [&bundle.old_tag, &bundle.new_tag] {
        let recorded: serde_json::Value =
            serde_json::from_slice(&bundle.recorded_build_body(tag)).expect("recorded");
        let served: serde_json::Value =
            serde_json::from_slice(&bridge.served_build(tag).expect("body")).expect("json");
        let rec = recorded["data"]["manifests"]
            .as_array()
            .expect("[]")
            .clone();
        let srv = served["data"]["manifests"].as_array().expect("[]").clone();
        for (r, s) in rec.iter().zip(srv.iter()) {
            for (field, origin) in [
                ("manifest_download", &o.manifest_origin),
                ("chunk_download", &o.chunk_origin),
            ] {
                let before = r[field]["url_prefix"].as_str().expect("prefix");
                let after = s[field]["url_prefix"].as_str().expect("prefix");
                assert!(
                    after.starts_with(origin.as_str()),
                    "{field} must name the listener that serves it: {after}"
                );
                assert_eq!(
                    &after[origin.len()..],
                    recorded_path(before),
                    "{field}: only the scheme and authority may change, never the path"
                );
                assert!(
                    after.starts_with("http://127.0.0.1:"),
                    "loopback only: {after}"
                );
            }
        }
    }
}

/// The path of a recorded `url_prefix` — everything after `scheme://authority`.
fn recorded_path(prefix: &str) -> &str {
    let rest = prefix.split_once("://").expect("scheme://").1;
    match rest.find('/') {
        Some(i) => &rest[i..],
        None => "",
    }
}

/// The live ids are `manifest_<16hex>_<32hex>`; the bundle stores archives
/// under *field* names. The translation is built while rewriting the body, so
/// an id resolves only if **this** build declared it and the bundle carries
/// that field's archive.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn a_manifest_id_resolves_to_the_field_archive_of_its_own_build() {
    let Some(bridge) = bridge_or_skip().await else {
        return;
    };
    let bundle = bridge.bundle().clone();

    let mut resolvable = 0;
    let mut uncarried = 0;
    for tag in [&bundle.old_tag, &bundle.new_tag] {
        let body: serde_json::Value =
            serde_json::from_slice(&bundle.recorded_build_body(tag)).expect("body");
        for m in body["data"]["manifests"].as_array().expect("[]") {
            let id = m["manifest"]["id"].as_str().expect("id");
            let (got_tag, field, build, carried) = bridge
                .manifest_resolution(id)
                .unwrap_or_else(|| panic!("id {id} is not indexed"));
            assert_eq!(got_tag, *tag, "the tag decides which archive");
            assert_eq!(field, m["matching_field"].as_str().expect("field"));
            assert_eq!(
                build,
                body["data"]["build_id"].as_str().expect("build_id"),
                "the declaring build is the one whose body declared it"
            );
            assert_eq!(
                carried,
                bundle.manifest_archive(tag, &field).is_some(),
                "carried == the archive exists"
            );
            if carried {
                resolvable += 1;
                // Serve it over loopback: real bytes, real route.
                let url = format!(
                    "{}/client_app/sophon/manifests/cxhpq4g4rgg0/{build}/{id}",
                    bridge.origins().manifest_origin
                );
                let (status, ctype, body) = get(&url).await;
                assert_eq!(status, 200, "{id}");
                assert_eq!(ctype, "application/octet-stream");
                assert!(!body.is_empty(), "{id} served no bytes");
            } else {
                uncarried += 1;
            }
        }
    }
    assert!(
        resolvable > 0,
        "the bundle must carry at least one manifest"
    );
    assert!(
        uncarried > 0,
        "a getBuild body declares five manifests and the bundle stores the fields it \
         captured; if every one resolved, docs/06 4.5's --audio warning has no teeth"
    );

    // An id no build declared is a 404, not a best-effort guess.
    let bogus = "manifest_0000000000000000_00000000000000000000000000000000";
    let url = format!(
        "{}/client_app/sophon/manifests/cxhpq4g4rgg0/{}/{bogus}",
        bridge.origins().manifest_origin,
        bundle.new_build
    );
    let (status, ctype, _) = get(&url).await;
    assert_eq!(status, 404, "an undeclared id must not be served");
    assert_eq!(
        ctype, "application/xml",
        "the measured blob 404 content-type"
    );
}

/// The invariant `docs/06` §4.1 buys: **every blob request's `<build_id>` must
/// equal the build whose manifest declared that id.** The same id under the
/// other build's prefix is a 404 — which is what makes a shim with the two blob
/// prefixes crossed impossible to mistake for a working bridge.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn a_blob_under_the_other_builds_prefix_is_refused() {
    let Some(bridge) = bridge_or_skip().await else {
        return;
    };
    let bundle = bridge.bundle().clone();
    let o = bridge.origins().clone();

    // A manifest the new build really declares.
    let body: serde_json::Value =
        serde_json::from_slice(&bundle.recorded_build_body(&bundle.new_tag)).expect("body");
    let m = &body["data"]["manifests"].as_array().expect("[]")[0];
    let id = m["manifest"]["id"].as_str().expect("id");
    let field = m["matching_field"].as_str().expect("field");
    if bundle.manifest_archive(&bundle.new_tag, field).is_none() {
        println!("SKIP: the bundle carries no {field} manifest for the new build");
        return;
    }

    // Under its own build: served.
    let right = format!(
        "{}/client_app/sophon/manifests/cxhpq4g4rgg0/{}/{id}",
        o.manifest_origin, bundle.new_build
    );
    assert_eq!(get(&right).await.0, 200);

    // Under the other build's: 404, replayed from `blob_wrong_build`.
    let wrong = format!(
        "{}/client_app/sophon/manifests/cxhpq4g4rgg0/{}/{id}",
        o.manifest_origin, bundle.old_build
    );
    let (status, ctype, body) = get(&wrong).await;
    assert_eq!(
        status, 404,
        "the same id under the other build must be a 404"
    );
    assert_eq!(ctype, "application/xml");
    if let Some((recorded_status, recorded_ctype, _)) = bridge.recorded_shape("blob_wrong_build") {
        assert_eq!(status, recorded_status, "replayed status");
        assert_eq!(ctype, recorded_ctype, "replayed content-type");
    }
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("NoSuchKey"), "{text}");
    assert!(
        text.contains(&bundle.old_build),
        "the replayed <Key> must name the build actually requested: {text}"
    );

    // And the chunk namespace: only the latest build's prefix is ever a
    // download target, so the old build's is refused for any id at all.
    let chunk = format!(
        "{}/client_app/sophon/chunks/cxhpq4g4rgg0/{}/whatever",
        o.chunk_origin, bundle.old_build
    );
    assert_eq!(get(&chunk).await.0, 404, "old-build chunk prefix must 404");
}

/// A tag the bundle does not carry answers with the recorded `retcode -202`
/// body — **not** one the shim made up. This is the call that populates the
/// chunk-reuse map, so getting it wrong degrades to an empty reuse map, every
/// chunk downloading, and a run that still exits `0` (`docs/06` §4.3).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn an_unserved_getbuild_tag_replays_the_recorded_not_found() {
    let Some(bridge) = bridge_or_skip().await else {
        return;
    };
    let bundle = bridge.bundle().clone();

    // Both served tags are real bodies, not error shapes.
    for tag in [&bundle.old_tag, &bundle.new_tag] {
        let (status, _, body) = get(&format!(
            "{}/getBuild?branch=main&package_id=x&password=&tag={tag}",
            bridge.origins().sophon_base
        ))
        .await;
        assert_eq!(status, 200, "{tag}");
        let v: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(v["retcode"], 0, "{tag} must be a real build body");
    }

    // A tag nobody serves: 200 + retcode -202, replayed.
    let (status, ctype, body) = get(&format!(
        "{}/getBuild?branch=main&package_id=x&password=&tag=0.0.0-not-a-real-tag",
        bridge.origins().sophon_base
    ))
    .await;
    assert_eq!(
        status, 200,
        "the JSON family answers 200 and puts it in retcode"
    );
    assert_eq!(ctype, "application/json");
    let v: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(v["retcode"], -202, "{}", body.len());
    assert!(v["data"].is_null(), "data is null on a not-found");

    if let Some((recorded_status, recorded_ctype, recorded_body)) =
        bridge.recorded_shape("getbuild_unknown_tag")
    {
        assert_eq!(status, recorded_status);
        assert_eq!(ctype, recorded_ctype);
        assert_eq!(
            String::from_utf8_lossy(&body),
            recorded_body,
            "no volatile fields are marked on this shape, so the body is verbatim"
        );
    } else {
        assert!(
            bridge.shapes_synthesized(),
            "a bundle with no error_shapes.json must say so rather than silently \
             passing the not-found assertions"
        );
    }

    // The log distinguishes the latest tag from an explicit one, because the
    // driver asserts the local tag was the one served.
    let hits = bridge.hits();
    assert!(
        hits.iter()
            .any(|h| h.kind == server::HitKind::Build && h.id == bundle.new_tag),
        "{hits:?}"
    );
    assert!(
        hits.iter()
            .any(|h| h.kind == server::HitKind::Build && h.id == "0.0.0-not-a-real-tag"),
        "{hits:?}"
    );
}

/// The capture matrix loaded off disk. `docs/06` assertion 5 lives here too:
/// a repeated path would make the second run silently skip and report
/// `download_bytes=0` against a non-zero expectation, which is a baffling
/// failure to diagnose from the number that is wrong.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a gircap bundle: cargo test -p girpr --test artifact_bridge -- --ignored --nocapture"]
async fn the_capture_matrix_loads_with_distinct_paths() {
    let Some(bridge) = bridge_or_skip().await else {
        return;
    };
    let b = bridge.bundle();

    assert!(
        !b.biz.is_empty(),
        "--biz comes from MANIFEST.json -> biz.id"
    );
    assert_ne!(b.old_build, b.new_build, "two builds, two namespaces");
    assert!(b.captures.len() >= 2, "the matrix is the point");

    let mut paths: Vec<&str> = b.captures.iter().map(|c| c.path.as_str()).collect();
    let before = paths.len();
    paths.sort_unstable();
    paths.dedup();
    assert_eq!(
        paths.len(),
        before,
        "capture paths must be pairwise distinct"
    );

    for c in &b.captures {
        assert!(!c.classes.is_empty(), "{}: classes[]", c.id);
        // `added` is the one class whose path is *absent* from `game/` by
        // construction — it is the file the new version introduces — so the
        // fixture must not carry it, or the run would find nothing to plan.
        let on_disk = b.root.join("game").join(&c.path).is_file();
        assert_eq!(
            on_disk,
            !c.classes.iter().any(|k| k == "added"),
            "{}: game/ presence must follow the class",
            c.id
        );
        match c.download_bytes {
            // `removed` carries `totals: null` and is driven by the purge
            // counters instead (`docs/06` §5.1).
            None => {
                assert!(c.absent_after, "{}: no byte total implies a deletion", c.id);
                assert!(
                    c.must_not_fetch.is_empty(),
                    "{}: nothing is downloaded",
                    c.id
                );
                assert!(
                    c.old_size > 0,
                    "{}: purge_bytes comes from the old size",
                    c.id
                );
            }
            Some(n) => {
                assert!(!c.absent_after, "{}: repairs to a file, not away", c.id);
                assert!(!c.result_md5.is_empty(), "{}: ground truth", c.id);
                assert!(
                    b.root.join(&c.result_path).is_file(),
                    "{}: expected/ ground truth",
                    c.id
                );
                assert!(c.new_size > 0, "{}: new size", c.id);
                if n == 0 {
                    assert!(
                        !c.must_not_fetch.is_empty(),
                        "{}: a zero-byte run is only provable by what was NOT fetched",
                        c.id
                    );
                }
            }
        }
    }
    let _ = b.capture_id;
}
