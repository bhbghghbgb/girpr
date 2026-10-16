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
    fn log_file(&self) -> PathBuf {
        self.root.join("girpr.log")
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
fn base_args(game: &Path, bundle: &Bundle, o: &Origins, scratch: &Scratch) -> Vec<String> {
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
        // Keep the trace log in the scratch dir rather than under target/.
        "--log-file".into(),
        scratch.log_file().display().to_string(),
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
    let mut pre = base_args(&scratch.game(), &bundle, bridge.origins(), &scratch);
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
    let mut args = base_args(&scratch.game(), &bundle, bridge.origins(), &scratch);
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
