//! M10, end to end: a real `gircap capture` against a local mock, and then the
//! whole offline half on what it wrote.
//!
//! **No test here can reach the network**, and that is enforced by construction
//! rather than by remembering: the mock is the only listener, it binds
//! `127.0.0.1:0`, and every base the capture uses — including the manifest and
//! chunk CDN bases, which come from the `getBuild` body the mock itself wrote —
//! points back at it. A capture against this mock has no route to anything.
//!
//! The capture is driven as a **process**, not as a `CapturePlan`. A plan built by
//! hand can drift from the flags a user types without anything noticing, and the
//! whole point of this suite is that the documented command line works.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Mock, World};

/// The binary under test, beside this test's own executable.
fn bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("the test binary has a path");
    p.pop(); // deps/
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("gircap.exe")
}

/// A scratch root that is removed when the test ends, whatever happens.
///
/// `Drop` rather than a teardown call: the interesting tests are the ones that
/// fail partway, and a fixture that only cleans up on the happy path leaks into
/// the next one.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Scratch {
        let path = std::env::temp_dir().join(format!(
            "gircap-m10-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        Scratch { path }
    }

    fn out(&self) -> PathBuf {
        self.path.join("artifacts")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// One finished command.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    /// The `SUMMARY` line, parsed into key=value pairs.
    fn summary(&self) -> BTreeMap<&str, String> {
        let line = self
            .stdout
            .lines()
            .find(|l| l.starts_with("SUMMARY "))
            .unwrap_or_else(|| {
                panic!(
                    "no SUMMARY on stdout:\n{}\n---\n{}",
                    self.stdout, self.stderr
                )
            });
        line.strip_prefix("SUMMARY ")
            .expect("starts with SUMMARY")
            .split(' ')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k, percent_decode(v)))
            .collect()
    }
}

type BTreeMap<K, V> = std::collections::BTreeMap<K, V>;

/// The summary percent-encodes whitespace and `%`, so decode it back.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b'%'));
                i += 3;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn run(args: &[&str]) -> Run {
    let out = Command::new(bin())
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("running {args:?}: {e}"));
    Run {
        code: out.status.code().expect("the process was not killed"),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// `gircap capture` against `mock`, writing into `scratch.out()`.
fn capture(mock: &Mock, scratch: &Scratch, extra: &[&str]) -> Run {
    let mut args: Vec<String> = vec![
        "capture".into(),
        "--out".into(),
        scratch.out().to_string_lossy().into_owned(),
        "--hyp-base".into(),
        mock.hyp_base(),
        "--sophon-base".into(),
        mock.sophon_base(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    let borrowed: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run(&borrowed)
}

/// The one bundle under a scratch root, whichever way the layout nests it.
fn bundle_in(out: &Path) -> PathBuf {
    let mut found = Vec::new();
    walk(out, &mut found);
    found
        .into_iter()
        .find(|p| p.join("MANIFEST.json").is_file())
        .unwrap_or_else(|| panic!("no bundle under {}", out.display()))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.push(p.clone());
            walk(&p, out);
        }
    }
}

/// A capture against a healthy mock, reused by several tests.
fn capture_ok(name: &str) -> (Mock, Scratch, Run, PathBuf) {
    let mock = Mock::start(World::five_classes());
    let scratch = Scratch::new(name);
    let r = capture(&mock, &scratch, &[]);
    assert_eq!(r.code, 0, "capture failed:\n{}\n{}", r.stdout, r.stderr);
    let bundle = bundle_in(&scratch.out());
    (mock, scratch, r, bundle)
}

// ------------------------------------------------------- M10's acceptance

/// M10: a captured artifact has **zero failed selfcheck rows**.
///
/// The one sentence the milestone's acceptance is written as, and the only
/// assertion here that would be false about a real regression.
#[test]
fn a_capture_leaves_no_failed_selfcheck_rows() {
    let (_m, _s, _r, bundle) = capture_ok("green");
    let raw = std::fs::read(bundle.join("verify/selfcheck.json")).expect("selfcheck written");
    let doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(doc["result"], "pass");
    assert_eq!(doc["counts"]["failed"], 0);
    let failed: Vec<&serde_json::Value> = doc["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["ok"] != serde_json::Value::Bool(true))
        .collect();
    assert!(failed.is_empty(), "failed rows left in place: {failed:#?}");
    assert!(
        doc["counts"]["total"].as_u64().unwrap() > 0,
        "and rows were produced"
    );
}

/// M10: `cargo test -p gircap` green and no network. The "no network" half is
/// the structural one: every request the capture made went to `127.0.0.1`.
#[test]
fn every_request_the_capture_made_went_to_the_mock() {
    let (mock, _s, _r, _b) = capture_ok("requests");
    let reqs = mock.requests();
    assert!(reqs.len() > 10, "the capture made {} requests", reqs.len());
    for r in &reqs {
        assert!(
            r.path.starts_with("/hyp/")
                || r.path.starts_with("/sophon/")
                || r.path.starts_with("/cdn/"),
            "a request the mock does not own: {r:?}"
        );
    }
    // **Only the ordering that still exists.** `getGameConfigs` and
    // `getGameBranches` used to be asserted to arrive in that order, and that
    // was always an accident: only the *second* carries the `package_id` that
    // `getBuild` needs, so only `branches < builds` is a real invariant. The
    // four HoYoPlay endpoints are now issued together, because none of them
    // depends on another, and an assertion about which of two concurrent
    // requests the mock happened to accept first is a test of the mock.
    let order: Vec<&str> = reqs.iter().map(|r| r.path.as_str()).collect();
    let branches = order
        .iter()
        .position(|p| p.ends_with("/getGameBranches"))
        .expect("getGameBranches");
    let builds = order
        .iter()
        .position(|p| p.ends_with("/getBuild"))
        .expect("getBuild");
    assert!(
        branches < builds,
        "the branch that carries package_id is fetched before getBuild"
    );
}

/// `api/error_shapes.json` records what the API does with requests it will not
/// serve. This is the acceptance that needs no network: against the mock, which
/// answers the measured way, the document must carry the measured statuses —
/// `404` from the blob routes and `200` from the JSON ones — and the one case
/// that is **not** a failure must say so.
#[test]
fn the_error_shapes_document_reproduces_the_measured_failure_answers() {
    let (_m, _s, _r, bundle) = capture_ok("shapes");
    let bytes = std::fs::read(bundle.join("api/error_shapes.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSON");
    let rows = doc["shapes"].as_array().unwrap();
    let raw = String::from_utf8_lossy(&bytes);

    let find = |id: &str| -> serde_json::Value {
        rows.iter()
            .find(|r| r["id"] == id)
            .unwrap_or_else(|| panic!("no row for {id}: {raw}"))
            .clone()
    };

    // The blob routes: 404, never a 200 carrying a code.
    for (id, ctype) in [
        ("blob_missing", "application/xml"),
        ("chunk_missing", "application/xml"),
        ("blob_wrong_build", "application/xml"),
    ] {
        let r = find(id);
        assert_eq!(r["family"], "blob", "{id}");
        assert_eq!(r["status"], 404, "{id}: {r}");
        assert_eq!(r["content_type"], ctype, "{id}: {r}");
        assert_eq!(
            r["retcode"],
            serde_json::Value::Null,
            "{id}: a blob has no retcode"
        );
        assert_eq!(r["error"], true, "{id}");
        // The volatile fields are named, because the body cannot be replayed
        // byte-for-byte and a replayer has to know that.
        let volatile: Vec<&str> = r["volatile_fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        for want in ["RequestId", "HostId", "Key"] {
            assert!(
                volatile.contains(&want),
                "{id} must mark {want}: {volatile:?}"
            );
        }
    }

    // The JSON routes: always 200, failure in retcode.
    for (id, retcode) in [
        ("getbuild_unknown_tag", -202),
        ("getbuild_bad_package", -201),
        ("getgameconfigs_bad_game_id", -1001),
    ] {
        let r = find(id);
        assert_eq!(r["family"], "json", "{id}: {r}");
        assert_eq!(r["status"], 200, "{id}: {r}");
        assert_eq!(r["retcode"], retcode, "{id}: {r}");
        assert_eq!(r["error"], true, "{id}");
        assert_eq!(
            r["volatile_fields"].as_array().unwrap().len(),
            0,
            "{id}: the JSON envelope has no per-request ids, so it replays exactly"
        );
    }

    // The case that is NOT a failure. Recording it is the whole point: without
    // it a reader infers "bad game id == error" from the two rows above.
    let branches = find("getgamebranches_bad_game_id");
    assert_eq!(branches["status"], 200);
    assert_eq!(branches["retcode"], 0, "{branches}");
    assert_eq!(
        branches["error"], false,
        "an empty game_branches at retcode 0 is a success, not a failure: {branches}"
    );
    assert!(
        branches["body"].as_str().unwrap().contains("game_branches"),
        "the empty node is recorded, so the claim is checkable: {branches}"
    );

    // The retry policy is stated in the artifact, not only in the fetcher — and
    // "stated" now means *derived*. This used to assert `retryable == false` for
    // every row, which passed against a hardcoded constant and would have kept
    // passing if the policy changed. Asserting the policy *relation* is the
    // claim the field's doc makes: if the fetcher would retry this status, the
    // document says so, whatever the answer is today.
    for r in rows {
        let status = r["status"].as_u64().expect("every shape records a status");
        let expected =
            gircap::http::RetryPolicy::should_retry_status(u16::try_from(status).unwrap());
        assert_eq!(
            r["retryable"], expected,
            "{} (status {status}) must record what the retry policy says: {r}",
            r["id"]
        );
    }
}

/// The `blob_wrong_build` probe asks for a **real** chunk id under the other
/// build's prefix. If it asked for a bogus id the row would prove nothing — a
/// prefix that 404s everything would satisfy it.
#[test]
fn the_wrong_build_probe_asks_for_a_real_id_under_the_other_prefix() {
    let (_m, _s, _r, bundle) = capture_ok("wrongbuild");
    let raw = std::fs::read(bundle.join("api/error_shapes.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    let rows = doc["shapes"].as_array().unwrap();
    let wrong = rows
        .iter()
        .find(|r| r["id"] == "blob_wrong_build")
        .expect("a blob_wrong_build row");

    let url = wrong["url"].as_str().unwrap();
    let missing = rows
        .iter()
        .find(|r| r["id"] == "chunk_missing")
        .expect("a chunk_missing row");
    assert_ne!(
        url,
        missing["url"].as_str().unwrap(),
        "the two probes must not ask for the same thing"
    );
    assert!(
        !url.ends_with("0000000000000000_00000000000000000000000000000000"),
        "the wrong-build probe asks for a real id, not the bogus one: {url}"
    );
    // And the id it asks for really is one the new build declares *and the old one
    // does not*. A shared chunk is one id under two addresses and the old prefix
    // does serve it, so a probe that picked one would record a `200` and claim
    // the opposite of what it is there to show.
    let real_id = url.rsplit('/').next().unwrap();
    let new_declares = manifest_chunk_ids(&bundle, "7.1.0");
    let old_declares = manifest_chunk_ids(&bundle, "7.0.0");
    assert!(
        new_declares.iter().any(|c| c == real_id),
        "{real_id} is not a chunk id the new build declares, so the probe proves nothing"
    );
    assert!(
        !old_declares.iter().any(|c| c == real_id),
        "{real_id} is shared with the old build, which does serve it under its own \
         prefix -- the probe would record a 200 and prove the opposite: {old_declares:?}"
    );
}

/// A bundle without `api/error_shapes.json` is still a valid bundle: the file
/// is additive, and `verify` must not have started requiring it.
#[test]
fn a_bundle_without_the_error_shapes_document_still_verifies() {
    let (_m, _s, r, bundle) = capture_ok("noshapes");
    assert_eq!(r.code, 0);
    let file = bundle.join("api/error_shapes.json");
    assert!(file.is_file(), "the capture wrote it");
    std::fs::remove_file(&file).unwrap();

    // The index still lists it, which is what a real pre-change bundle's does not.
    let v = run(&["verify", "--artifact", &bundle.to_string_lossy()]);
    assert_ne!(
        v.code, 2,
        "verify must not demand the document: it is not in the required set\n{}{}",
        v.stdout, v.stderr
    );
}

/// Every chunk id the negative probes asked for, read from the document that
/// records them.
fn probe_chunk_ids(bundle: &std::path::Path) -> Vec<String> {
    let bytes = std::fs::read(bundle.join("api/error_shapes.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    doc["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["family"] == "blob" && s["url"].as_str().unwrap().contains("/chunk/"))
        .map(|s| {
            s["url"]
                .as_str()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .to_string()
        })
        .collect()
}

/// How many chunk blobs the bundle stored.
fn count_blobs(bundle: &std::path::Path) -> usize {
    std::fs::read_dir(bundle.join("chunks/blobs"))
        .map(|d| d.count())
        .unwrap_or(0)
}

/// Inflate a `.gz` document the bundle wrote.
fn gunzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .expect("a .gz document inflates");
    out
}

/// Every chunk id one build's `game` manifest declares, read from the full
/// decode the bundle wrote beside it.
///
/// Both encodings: `files[].encoding` decides whether the decode on disk is
/// `game.manifest.json` or `game.manifest.json.gz`, and which one it is depends
/// on the gzip threshold and the decode's size, not on anything this test sets.
fn manifest_chunk_ids(bundle: &std::path::Path, tag: &str) -> Vec<String> {
    let plain_path = bundle.join(format!("manifests/{tag}/game.manifest.json"));
    let gz_path = bundle.join(format!("manifests/{tag}/game.manifest.json.gz"));
    let plain = if plain_path.is_file() {
        std::fs::read(&plain_path).expect("read the decode")
    } else {
        assert!(
            gz_path.is_file(),
            "no full decode for {tag}: looked at {} and {}",
            plain_path.display(),
            gz_path.display()
        );
        gunzip(&std::fs::read(&gz_path).unwrap())
    };
    let doc: serde_json::Value = serde_json::from_slice(&plain).expect("the decode is JSON");
    let files = doc["entries"].as_array().expect("the decode's entries");
    let mut ids = Vec::new();
    for f in files {
        for c in f["chunks"].as_array().expect("a file's chunks") {
            ids.push(c["id"].as_str().expect("a chunk id").to_string());
        }
    }
    assert!(!ids.is_empty(), "{tag} declares no chunks");
    ids
}

/// `list` reports each bundle on stdout, not only on stderr at INFO. Every
/// per-bundle fact a caller needs to decide *which* bundle to verify is on the
/// record — `selfcheck` and `complete` in particular, neither of which the
/// summary carries per-bundle.
#[test]
fn list_emits_one_bundle_record_per_bundle_and_a_summary_last() {
    let (mock, scratch, r, bundle) = capture_ok("listrec");
    let root = scratch.out();
    let v = run(&[
        "list",
        "--root",
        &root.to_string_lossy(),
        "--output",
        "json",
    ]);
    assert_eq!(v.code, 0, "{}\n{}", v.stdout, v.stderr);

    let lines: Vec<&str> = v.stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines.len(),
        2,
        "one BUNDLE for the one bundle, then one SUMMARY: {lines:?}"
    );

    let b: serde_json::Value = serde_json::from_str(lines[0]).expect("NDJSON");
    assert_eq!(b["event"], "bundle", "{lines:?}");
    assert_eq!(b["tool"], "gircap");
    assert_eq!(
        b["id"].as_str().unwrap(),
        bundle.file_name().unwrap().to_string_lossy(),
        "the record names the bundle directory it found"
    );
    assert!(
        b["path"]
            .as_str()
            .unwrap()
            .ends_with(bundle.file_name().unwrap().to_string_lossy().as_ref()),
        "and the path ends with it: {b}"
    );
    assert_eq!(b["tags"], "7.0.0 -> 7.1.0");
    assert!(b["patchmaps"].as_u64().unwrap() > 0, "{b}");
    assert!(b["blobs"].as_u64().unwrap() > 0, "{b}");
    assert_eq!(b["selfcheck"], "pass");
    assert_eq!(b["complete"], true);

    // The terminal record is last, and it is the one the summary is. Its totals
    // must equal the one bundle's record — two views of the same walk.
    let s: serde_json::Value = serde_json::from_str(lines[1]).expect("NDJSON");
    assert_eq!(s["event"], "list");
    assert_eq!(s["bundles"], 1);
    assert_eq!(
        s["patchmaps"], b["patchmaps"],
        "the summary totals the records"
    );
    assert_eq!(s["blobs"], b["blobs"], "the summary totals the records");
    assert_eq!(s["exit_code"], 0);

    // The text plane says the same thing.
    let t = run(&["list", "--root", &root.to_string_lossy()]);
    assert_eq!(t.code, 0, "{}", t.stderr);
    let tlines: Vec<&str> = t.stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(tlines.len(), 2, "{tlines:?}");
    assert!(tlines[0].starts_with("BUNDLE "), "{tlines:?}");
    assert!(tlines[1].starts_with("SUMMARY "), "{tlines:?}");
    assert!(tlines[0].contains("selfcheck=pass"), "{tlines:?}");
    assert!(tlines[0].contains("complete=true"), "{tlines:?}");

    drop(mock);
    drop(r);
}

/// The artifact has the shape the schema doc promises.
#[test]
fn the_bundle_has_the_shape_the_schema_documents() {
    let (_m, _s, r, bundle) = capture_ok("shape");
    for rel in [
        "MANIFEST.json",
        "README.md",
        "api/getGameConfigs.json",
        "api/getGameBranches.json",
        "api/getGameDeprecatedFileConfigs.json",
        "api/getGamePackages.json",
        "api/getBuild.7.0.0.json",
        "api/getBuild.7.1.0.json",
        "api/requests.ndjson",
        "api/tag_probe.json",
        "api/error_shapes.json",
        "chunks/index.json",
        "diff/paths.diff.json",
        "diff/candidates.json",
        "game/config.ini",
        "verify/checksums.json",
        "verify/selfcheck.json",
    ] {
        assert!(
            bundle.join(rel).is_file(),
            "missing {rel} in {}",
            bundle.display()
        );
    }
    for rel in [
        "manifests/7.0.0/game.manifest.meta.json",
        "manifests/7.1.0/game.manifest.meta.json",
        "patchmaps",
        "expected",
        "game",
    ] {
        assert!(bundle.join(rel).exists(), "missing {rel}");
    }
    let s = r.summary();
    assert_eq!(s["old_tag"], "7.0.0");
    assert_eq!(s["new_tag"], "7.1.0");
    assert_eq!(s["biz"], "hk4e_global");
    assert_eq!(s["result"], "ok");
    assert!(
        s["artifact"].ends_with(bundle.to_string_lossy().as_ref()),
        "the summary names the bundle it wrote: {}{}",
        s["artifact"],
        bundle.to_string_lossy()
    );
}

/// The request log's rows, pinned.
///
/// **§5.5 rows 1 and 16 landed here.** `api/requests.ndjson` is one of the six
/// documents that always differ run to run — `ts`, `elapsed_ms`, CDN headers,
/// `etag` — so `tests/bundle_diff.rs` compares it with those blanked and nothing
/// else compares it at all. That is sound for what it was built to do: two
/// captures of the *same* binary cannot disagree about a change they both carry,
/// so the gate catches nondeterminism and nothing else. A refactor that changes
/// what a row *says* passes it, and both bundles under `artifacts/` drifted that
/// way without anything failing.
///
/// So the row content is pinned here instead. Each line is
///
///     seq kind method path status bytes_in attempts stored_as range
///
/// with the loopback host and query-free URL prefix dropped, `ts`/`elapsed_ms`
/// and both header maps left out, and the rows in `seq` order.
///
/// **The expectation is the specification.** It was dumped from a real capture
/// and read back, not written from what the code appears to intend — which is
/// the only way a pin like this catches a mistake rather than restating one.
///
/// What it catches, each confirmed by mutation rather than by argument:
///
/// - `bytes_in` on the seven `probe` rows. §5.5 row 1 found these were **0 for
///   every non-2xx**, because the body was dropped unread. Zeroing them again
///   fails here and nowhere else.
/// - `stored_as` on the six `api` rows. §6.1 filled those in; the two bundles
///   under `artifacts/` still carry 42 marked rows where the tool writes 48.
/// - the `seq` column: dense, unique, from 1. Making `reserve()` return a
///   constant fails here.
///
/// What it does **not** catch, stated because a pin that only lists wins is
/// misleading: `attempts` is 1 on every row, because this fixture never
/// retries. Forcing it to 1 survives. So a **spurious** retry is caught and a
/// **hidden** one is not — the pin says "nothing was retried", not "retries are
/// recorded". Closing that would need a route that fails once and then answers,
/// which the mock does not have.
///
/// `--jobs 1` is pinned for determinism, the same reason `bundle_diff`'s module
/// doc gives: `seq` is assigned at reserve time, so which request gets which
/// `seq` is a function of the network unless there is one worker.
/// Which part of a path inventory a row belongs to.
///
/// The pin below lists every **document** by name and counts the rest, because
/// the documents are the bundle's fixed skeleton — a new one appearing or an old
/// one going missing is a real change — while `chunks/blobs/` and the assembled
/// trees are named after chunk ids and manifest paths that legitimately move.
fn kind_of(path: &str) -> &'static str {
    if path.starts_with("chunks/blobs/") {
        "blob"
    } else if path.starts_with("game/") {
        "game"
    } else if path.starts_with("expected/") {
        "expected"
    } else {
        "document"
    }
}

/// Split a path inventory into (documents sorted, counts by kind).
fn split_inventory<'a>(paths: &[&'a str]) -> (Vec<&'a str>, BTreeMap<&'a str, usize>) {
    let mut counts: BTreeMap<&'a str, usize> = BTreeMap::new();
    let mut docs = Vec::new();
    for p in paths {
        let k = kind_of(p);
        *counts.entry(k).or_default() += 1;
        if k == "document" {
            docs.push(*p);
        }
    }
    docs.sort_unstable();
    (docs, counts)
}

/// `MANIFEST.json`, `verify/checksums.json` and `README.md` say the same
/// inventory, and `d745680` showed that pinning one document is not enough —
/// these three are still only compared between two runs of the same binary.
///
/// What is pinned, and why each part:
///
/// - every **document** path, by name. This is the bundle's skeleton, and the
///   realistic regressions are a document appearing, disappearing, or being
///   renamed — all of which a count would hide.
/// - the **count** of blobs, `game/` and `expected/` rows, which move with the
///   world rather than with the tool, and are pinned as counts deliberately.
/// - the scalar header fields of `MANIFEST.json`: `schema_version`, `tool`, the
///   non-URL half of `biz`, `field`, `audio`, `gzip_threshold`,
///   `selection_pool`, `versions`' key set, `capture.complete`, and the two
///   `totals` that count things rather than bytes.
/// - `README.md` with **every digit run blanked**, so the prose is pinned and
///   the sizes are not.
///
/// Deliberately **not** pinned, because they are the volatile cascade:
/// `capture.id`, `capture.captured_at`, `capture.tool_argv`, `capture.host_os`,
/// `biz.hyp_base`, `biz.sophon_base`, `totals.bytes_downloaded`,
/// `totals.bytes_on_disk`, and every `files[]`/`entries[]` digest. A16 covers
/// those digests, and `bundle_diff` covers the ones that move.
#[test]
fn the_inventory_documents_describe_the_same_bundle() {
    let (_m, _s, _r, bundle) = capture_ok("inv");
    let read = |p: &str| -> String { std::fs::read_to_string(bundle.join(p)).expect(p) };
    let json = |p: &str| -> serde_json::Value { serde_json::from_str(&read(p)).expect(p) };

    const MANIFEST_DOCUMENTS: &[&str] = &[
        "api/error_shapes.json",
        "api/getBuild.7.0.0.json",
        "api/getBuild.7.1.0.json",
        "api/getGameBranches.json",
        "api/getGameConfigs.json",
        "api/getGameDeprecatedFileConfigs.json",
        "api/getGamePackages.json",
        "api/requests.ndjson",
        "api/tag_probe.json",
        "chunks/index.json",
        "diff/candidates.json",
        "diff/matrix.json",
        "diff/paths.diff.json",
        "diff/paths.tsv",
        "diff/reuse_pool.json",
        "manifests/7.0.0/en-us.manifest.json",
        "manifests/7.0.0/en-us.manifest.meta.json",
        "manifests/7.0.0/en-us.manifest.zst",
        "manifests/7.0.0/en-us.paths.tsv",
        "manifests/7.0.0/game.manifest.json",
        "manifests/7.0.0/game.manifest.meta.json",
        "manifests/7.0.0/game.manifest.zst",
        "manifests/7.0.0/game.paths.tsv",
        "manifests/7.1.0/en-us.manifest.json",
        "manifests/7.1.0/en-us.manifest.meta.json",
        "manifests/7.1.0/en-us.manifest.zst",
        "manifests/7.1.0/en-us.paths.tsv",
        "manifests/7.1.0/game.manifest.json",
        "manifests/7.1.0/game.manifest.meta.json",
        "manifests/7.1.0/game.manifest.zst",
        "manifests/7.1.0/game.paths.tsv",
        "patchmaps/added.json",
        "patchmaps/identical.json",
        "patchmaps/partial_same_offset.json",
        "patchmaps/removed.json",
        "patchmaps/zero_reuse_multi.json",
    ];
    const CHECKSUM_DOCUMENTS: &[&str] = &[
        "MANIFEST.json",
        "README.md",
        "api/error_shapes.json",
        "api/getBuild.7.0.0.json",
        "api/getBuild.7.1.0.json",
        "api/getGameBranches.json",
        "api/getGameConfigs.json",
        "api/getGameDeprecatedFileConfigs.json",
        "api/getGamePackages.json",
        "api/requests.ndjson",
        "api/tag_probe.json",
        "chunks/index.json",
        "diff/candidates.json",
        "diff/matrix.json",
        "diff/paths.diff.json",
        "diff/paths.tsv",
        "diff/reuse_pool.json",
        "manifests/7.0.0/en-us.manifest.json",
        "manifests/7.0.0/en-us.manifest.meta.json",
        "manifests/7.0.0/en-us.manifest.zst",
        "manifests/7.0.0/en-us.paths.tsv",
        "manifests/7.0.0/game.manifest.json",
        "manifests/7.0.0/game.manifest.meta.json",
        "manifests/7.0.0/game.manifest.zst",
        "manifests/7.0.0/game.paths.tsv",
        "manifests/7.1.0/en-us.manifest.json",
        "manifests/7.1.0/en-us.manifest.meta.json",
        "manifests/7.1.0/en-us.manifest.zst",
        "manifests/7.1.0/en-us.paths.tsv",
        "manifests/7.1.0/game.manifest.json",
        "manifests/7.1.0/game.manifest.meta.json",
        "manifests/7.1.0/game.manifest.zst",
        "manifests/7.1.0/game.paths.tsv",
        "patchmaps/added.json",
        "patchmaps/identical.json",
        "patchmaps/partial_same_offset.json",
        "patchmaps/removed.json",
        "patchmaps/zero_reuse_multi.json",
        "verify/selfcheck.json",
    ];

    let m = json("MANIFEST.json");
    assert_eq!(m["schema_version"], 1);
    assert_eq!(m["tool"]["name"], "gircap");
    // The bases name this run's ephemeral loopback port; the identity does not.
    assert_eq!(m["biz"]["id"], "hk4e_global");
    assert_eq!(m["biz"]["channel"], "1");
    assert_eq!(m["biz"]["sub_channel"], "0");
    assert_eq!(m["biz"]["game_id"], "gopR6Cufr3");
    assert_eq!(m["biz"]["launcher_id"], "VYTpXlbWo8");
    assert_eq!(m["biz"]["cps"], "hyp_hoyoverse");
    assert_eq!(m["field"], "game");
    assert_eq!(m["audio"], serde_json::json!(["en-us"]));
    assert_eq!(m["gzip_threshold"], 1_048_576);
    assert_eq!(m["selection_pool"]["fields"], serde_json::json!(["game", "en-us"]));
    assert_eq!(m["selection_pool"]["manifest_entries"], serde_json::json!({"en-us": 1, "game": 4}));
    assert_eq!(m["capture"]["complete"], true);
    let vkeys: Vec<&String> = m["versions"].as_object().unwrap().keys().collect();
    assert_eq!(vkeys, vec!["diff_tags", "new", "old"]);
    assert_eq!(m["totals"]["files_captured"], 5);
    assert_eq!(m["totals"]["chunks_blobs"], 10);

    let mpaths: Vec<&str> = m["files"].as_array().unwrap().iter()
        .map(|f| f["path"].as_str().expect("a path")).collect();
    let (mdocs, mcounts) = split_inventory(&mpaths);
    assert_eq!(mdocs, MANIFEST_DOCUMENTS, "MANIFEST.json documents[] changed");
    assert_eq!(mcounts.get("blob"), Some(&10), "MANIFEST.json blob rows");
    assert_eq!(mcounts.get("game"), Some(&5), "MANIFEST.json game rows");
    assert_eq!(mcounts.get("expected"), Some(&3), "MANIFEST.json expected rows");

    let c = json("verify/checksums.json");
    assert_eq!(c["algorithm"], "sha256");
    let epaths: Vec<&str> = c["entries"].as_array().unwrap().iter()
        .map(|f| f["path"].as_str().expect("a path")).collect();
    assert_eq!(c["file_count"], epaths.len());
    let (cdocs, ccounts) = split_inventory(&epaths);
    assert_eq!(cdocs, CHECKSUM_DOCUMENTS, "checksums.json documents[] changed");
    assert_eq!(ccounts.get("blob"), Some(&10), "checksums.json blob rows");

    // A16's claim, restated against the projection: every file `MANIFEST.json`
    // lists is also checksummed, plus the three documents that cannot list
    // themselves.
    for p in &mpaths {
        assert!(epaths.contains(p), "checksums.json does not cover {p}");
    }

    // README, with every digit run collapsed to `#`. The prose is the pin; the
    // sizes are the cascade.
    let mut skel = String::new();
    let mut prev = false;
    for ch in read("README.md").chars() {
        let d = ch.is_ascii_digit();
        skel.push(if d { if prev { ' ' } else { '#' } } else { ch });
        prev = d;
    }
    const README_SKELETON: &[&str] = &[
        "# gircap artifact",
        "- **tool** gircap #.#.#",
        "- **region** `hk#e_global`",
        "- **captured** #   -# -# T# :# :# Z (`#       T#     Z`, windows)",
        "**complete**: `#.#.#` (`sMOCKold# `) → `#.#.#` (`sMOCKnew# `)",
        "selfcheck: **Pass** — #  of #  assertions passed",
        "## Matrix",
        "| class | status | capture | path | field | bytes |",
        "|---|---|---|---|---|---|",
        "| `identical` | captured | identical | `GenshinImpact_Data/StreamingAssets/Audio/en-us/vo_all` | en-us | - |",
        "| `zero_reuse_single` | unavailable | - | `-` | - | - |",
        "| `zero_reuse_multi` | captured | zero_reuse_multi | `GenshinImpact_Data/StreamingAssets/blocks/zero.blk` | game | #    |",
        "| `partial_same_offset` | captured | partial_same_offset | `GenshinImpact_Data/Plugins/HoYoChannel.dll` | game | #    |",
        "| `partial_moved_offset` | unavailable | - | `-` | - | - |",
        "| `partial_size_changed` | unavailable | - | `-` | - | - |",
        "| `added` | captured | added | `GenshinImpact_Data/Plugins/libNew.dll` | game | - |",
        "| `removed` | captured | removed | `GenshinImpact_Data/StreamingAssets/blocks/old.blk` | game | #    |",
        "| `full_reuse_reordered` | unavailable | - | `-` | - | - |",
        "## Totals",
        "- # files captured, #  chunk blobs stored",
        "- #     B downloaded, #     B on disk",
        "- selection pool: `game`, `en-us`",
        "- `materialize_new`: # chunks",
        "- `materialize_old`: # chunks",
        "- `patch_fetch`: # chunks",
        "## Re-verifying",
        "```",
        "gircap verify <this directory>",
        "```",
        "Offline: no network, no writes. Exit # if every assertion still holds.",
    ];
    let lines: Vec<&str> = skel.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    assert_eq!(lines, README_SKELETON, "README.md prose changed");
}

/// `verify/selfcheck.json` is the tool's own verdict on itself, and it is the one
/// document a reader is most likely to trust, so its shape is pinned: the
/// result, the counts, and the exact wording of every assertion it makes.
///
/// The wording is the point. `id` alone would pass if an assertion were
/// silently narrowed — "every reused chunk's old-file slice MD5 matches" and
/// "some reused chunk's MD5 matches" are both `A10`, and only the text tells
/// them apart. §5.5 row 16 (`is_fatal`) is the same idea from the other side.
#[test]
fn the_selfcheck_document_asserts_what_it_always_asserted() {
    let (_m, _s, _r, bundle) = capture_ok("sc");
    let s: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(bundle.join("verify/selfcheck.json")).unwrap())
            .expect("verify/selfcheck.json");

    assert_eq!(s["result"], "pass");
    assert_eq!(s["counts"]["failed"], 0);
    assert_eq!(s["counts"]["total"], s["counts"]["passed"]);

    let rows = s["assertions"].as_array().expect("assertions[]");
    let mut ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().expect("id")).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(
        ids,
        vec!["A1", "A10", "A11", "A12", "A13", "A14", "A15", "A16", "A17", "A19",
             "A2", "A3", "A4", "A6", "A7", "A8", "A9"],
        "the set of assertions a capture makes changed; A5, A18 and A20 are \
         verify-only or absent by design and must stay that way"
    );

    let mut worded: Vec<String> = rows
        .iter()
        .map(|r| format!("{} {}", r["id"].as_str().unwrap(), r["assertion"].as_str().unwrap()))
        .collect();
    worded.sort();
    worded.dedup();
    const ASSERTIONS: &[&str] = &[
        "A1 every API response captured has retcode == 0",
        "A10 every reused chunk's old-file slice MD5 == uncompressed_md5",
        "A11 chunk offsets gapless from 0, ascending, sum(uncompressed_size) == size",
        "A12 patch_fetch disjoint from the reuse set, and a subset of materialize_new",
        "A13 game/config.ini re-parses with game_version == --old-tag",
        "A14 game/ contains no path outside expected/'s manifest set + the tool-read config files",
        "A15 diff/ counts equal the recomputed set differences",
        "A16 checksums.json covers every file written under the artifact root, and re-hashing matches",
        "A17 each required matrix class is captured | covered_by | unavailable - never absent",
        "A19 each two-sided captured file differs between game/ and expected/ exactly when its class says it should",
        "A2 getBuild(new).data.tag == --new-tag; getBuild(old).data.tag == --old-tag",
        "A3 manifest blob md5(decompressed) == manifest.checksum",
        "A4 manifest.checksum == id 32-hex suffix; id 16-hex prefix recorded as opaque",
        "A6 len(raw) == compressed_size; md5(raw) == compressed_md5; md5(zstd(raw)) == uncompressed_md5; len(decoded) == uncompressed_size",
        "A7 assemble(materialize_old) has md5 == old entry.md5",
        "A8 assemble(materialize_new) has md5 == new entry.md5",
        "A9 assemble(REUSE-from-old union patch_fetch) byte-equals assemble(materialize_new)",
    ];
    let got: Vec<&str> = worded.iter().map(String::as_str).collect();
    assert_eq!(got, ASSERTIONS, "an assertion's wording changed");
}
/// The captured `api/` bodies are byte-exact, which is the whole reason they are
/// stored rather than re-rendered.
#[test]
fn the_request_log_rows_are_what_the_spec_says_they_are() {
    let mock = Mock::start(World::five_classes());
    let scratch = Scratch::new("rows");
    let r = capture(&mock, &scratch, &["--jobs", "1"]);
    assert_eq!(r.code, 0, "capture failed:\n{}\n{}", r.stdout, r.stderr);
    let bundle = bundle_in(&scratch.out());
    let log = std::fs::read_to_string(bundle.join("api").join("requests.ndjson"))
        .expect("api/requests.ndjson");

    const EXPECTED: &[&str] = &[
"1 api GET /hyp/getGameConfigs?launcher_id=VYTpXlbWo8&language=en-us&game_ids[]=gopR6Cufr3 200 435 1 api/getGameConfigs.json -",
"2 api GET /hyp/getGameBranches?launcher_id=VYTpXlbWo8&language=en-us&game_ids[]=gopR6Cufr3 200 476 1 api/getGameBranches.json -",
"3 api GET /hyp/getGameDeprecatedFileConfigs?launcher_id=VYTpXlbWo8&language=en-us&game_ids[]=gopR6Cufr3&channel=1&sub_channel=0 200 183 1 api/getGameDeprecatedFileConfigs.json -",
"4 api GET /hyp/getGamePackages?launcher_id=VYTpXlbWo8&language=en-us&game_ids[]=gopR6Cufr3 200 103 1 api/getGamePackages.json -",
"5 api GET /sophon/getBuild?branch=main&package_id=ScSYQBFhu9&password=sMOCKpassword&tag=7.1.0 200 1344 1 api/getBuild.7.1.0.json -",
"6 api GET /sophon/getBuild?branch=main&package_id=ScSYQBFhu9&password=sMOCKpassword&tag=7.0.0 200 1344 1 api/getBuild.7.0.0.json -",
"7 probe GET /sophon/getBuild?branch=main&package_id=ScSYQBFhu9&password=sMOCKpassword&tag=6.7.0 200 50 1 - -",
"8 manifest GET /cdn/sMOCKold00/manifest/manifest_2771a8e0cdb7b707_abf47822fe1d2cab22e470e6a2b43a09 200 706 1 manifests/7.0.0/game.manifest.zst -",
"9 manifest GET /cdn/sMOCKnew00/manifest/manifest_3fc907e1b5db4352_412a9dbff152a4f482b470a8c466a1e8 200 719 1 manifests/7.1.0/game.manifest.zst -",
"10 manifest GET /cdn/sMOCKold00/manifest/manifest_f56ab1254eaa50c8_5018981248fba1a947670fa7daffc756 200 252 1 manifests/7.0.0/en-us.manifest.zst -",
"11 manifest GET /cdn/sMOCKnew00/manifest/manifest_82e9a75baad12ba8_5018981248fba1a947670fa7daffc756 200 252 1 manifests/7.1.0/en-us.manifest.zst -",
"12 probe GET /cdn/sMOCKnew00/manifest/0000000000000000_00000000000000000000000000000000 404 276 1 - -",
"13 probe GET /cdn/sMOCKnew00/chunk/0000000000000000_00000000000000000000000000000000 404 273 1 - -",
"14 probe GET /cdn/sMOCKold00/chunk/ba3ffe0a4f82d1b5_a003157e0deaac5832ecfc30f3e62982 404 273 1 - -",
"15 probe GET /sophon/getBuild?branch=main&package_id=ScSYQBFhu9&password=sMOCKpassword&tag=null 200 50 1 - -",
"16 probe GET /sophon/getBuild?branch=main&package_id=undefined&password=sMOCKpassword 200 55 1 - -",
"17 probe GET /hyp/getGameConfigs?launcher_id=VYTpXlbWo8&language=en-us&game_ids[]=undefined 200 56 1 - -",
"18 probe GET /hyp/getGameBranches?launcher_id=VYTpXlbWo8&language=en-us&game_ids[]=undefined 200 56 1 - -",
"19 chunk GET /cdn/sMOCKnew00/chunk/059f931481c3a4bf_90d769cb1f8fb7037073188f9399952e 200 1033 1 chunks/blobs/059f931481c3a4bf_90d769cb1f8fb7037073188f9399952e.zst -",
"20 chunk GET /cdn/sMOCKold00/chunk/4411d85567f739ce_6940ec80459b4352adaffa992aacdab3 200 521 1 chunks/blobs/4411d85567f739ce_6940ec80459b4352adaffa992aacdab3.zst -",
"21 chunk GET /cdn/sMOCKnew00/chunk/69edd52759c53740_12283b0a8bbcfc40b5e00175c1d7dfea 200 1033 1 chunks/blobs/69edd52759c53740_12283b0a8bbcfc40b5e00175c1d7dfea.zst -",
"22 chunk GET /cdn/sMOCKnew00/chunk/6a2c2034429981aa_1fbd0959e726da54e75cd528a6c15469 200 521 1 chunks/blobs/6a2c2034429981aa_1fbd0959e726da54e75cd528a6c15469.zst -",
"23 chunk GET /cdn/sMOCKnew00/chunk/78389a4e7cc1b62b_4a84467752ea9c122de6b7288b0f7e73 200 521 1 chunks/blobs/78389a4e7cc1b62b_4a84467752ea9c122de6b7288b0f7e73.zst -",
"24 chunk GET /cdn/sMOCKold00/chunk/a80d7329a2bf0f6c_3b5a7d035458664fcdd3e13fb1b193cf 200 521 1 chunks/blobs/a80d7329a2bf0f6c_3b5a7d035458664fcdd3e13fb1b193cf.zst -",
"25 chunk GET /cdn/sMOCKnew00/chunk/ba3ffe0a4f82d1b5_a003157e0deaac5832ecfc30f3e62982 200 265 1 chunks/blobs/ba3ffe0a4f82d1b5_a003157e0deaac5832ecfc30f3e62982.zst -",
"26 chunk GET /cdn/sMOCKold00/chunk/d3263f518c944d35_5f37fc15a53060db835c98ae5f3af747 200 1033 1 chunks/blobs/d3263f518c944d35_5f37fc15a53060db835c98ae5f3af747.zst -",
"27 chunk GET /cdn/sMOCKold00/chunk/dc4c4277b58c6559_9c389ad675e81e5be30dc2aea4770636 200 1033 1 chunks/blobs/dc4c4277b58c6559_9c389ad675e81e5be30dc2aea4770636.zst -",
"28 chunk GET /cdn/sMOCKnew00/chunk/dced37b2dd2b2118_558db22ff96610039fa29418b658bbaa 200 1033 1 chunks/blobs/dced37b2dd2b2118_558db22ff96610039fa29418b658bbaa.zst -",
    ];

    let mut got: Vec<(u64, String)> = log
        .lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).expect("one row per line");
            (v["seq"].as_u64().expect("seq is a number"), row_key(&v))
        })
        .collect();
    got.sort_by_key(|(s, _)| *s);
    let got_keys: Vec<&str> = got.iter().map(|(_, k)| k.as_str()).collect();

    assert_eq!(
        got_keys,
        EXPECTED,
        "api/requests.ndjson changed. Left is the run, right is the pin.\n\
         A changed `bytes_in`, `stored_as` or `attempts` is a real behaviour \
         change and needs a band-3 note; a changed `seq` means the log is no \
         longer dense, unique, or sorted."
    );
}

/// One request row, reduced to what does not move between runs.
///
/// The host is dropped because it is an ephemeral loopback port, `range` and
/// `stored_as` collapse to `-` when null rather than printing `None`, and the
/// two header maps and both timestamps are left out entirely — they carry the
/// CDN's per-response ids and the clock, and no assertion should depend on
/// either.
fn row_key(v: &serde_json::Value) -> String {
    let url = v["url"].as_str().unwrap_or("-");
    let path = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.find('/').map(|i| &rest[i..]))
        .unwrap_or(url);
    format!(
        "{} {} {} {} {} {} {} {} {}",
        v["seq"],
        v["kind"].as_str().unwrap(),
        v["method"].as_str().unwrap(),
        path,
        v["status"],
        v["bytes_in"],
        v["attempts"],
        v["stored_as"].as_str().unwrap_or("-"),
        v["range"].as_str().unwrap_or("-"),
    )
}

/// The captured `api/` bodies are byte-exact, which is the whole reason they are
/// stored rather than re-rendered.
#[test]
fn the_captured_api_bodies_are_the_bytes_the_server_sent() {
    let mock = Mock::start(World::five_classes());
    let scratch = Scratch::new("byteexact");
    let r = capture(&mock, &scratch, &[]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let bundle = bundle_in(&scratch.out());

    // Ask the mock again for the same endpoint and compare. The query has to be
    // the one the tool sends: `game_ids[]` is not optional, and the mock answers
    // a request without it the measured way (`retcode -1001`) rather than the
    // real body.
    let url = format!("{}/getGameConfigs?game_ids[]=gopR6Cufr3", mock.hyp_base());
    let again = ureq_get(&url);
    let stored = std::fs::read(bundle.join("api/getGameConfigs.json")).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&stored).trim(),
        String::from_utf8_lossy(&again).trim(),
        "the stored body is not what the server served"
    );
}

/// The minimum blocking HTTP client, so the test can re-ask the mock without
/// pulling in the client under test — asking `gircap` whether `gircap` stored the
/// right bytes would be circular.
fn ureq_get(url: &str) -> Vec<u8> {
    use std::io::{Read, Write};
    let rest = url.strip_prefix("http://").expect("an http url");
    let (host_port, path) = rest.split_once('/').expect("a path");
    let mut s = std::net::TcpStream::connect(host_port).expect("connect to the mock");
    s.write_all(
        format!("GET /{path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    )
    .unwrap();
    let mut all = Vec::new();
    s.read_to_end(&mut all).unwrap();
    let split = all
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a header/body split");
    all[split + 4..].to_vec()
}

/// M10: **no test touches the network.** Proven, not asserted.
///
/// The claim is checkable from the artifact itself: `api/requests.ndjson` records
/// the URL of every request the run made, including the manifest and chunk CDN
/// requests whose bases come from the `getBuild` body — and that body was written
/// by the mock. So if every recorded URL is a loopback address, there was no other
/// address to reach, whatever the flags said.
#[test]
fn every_request_url_in_the_bundle_is_a_loopback_address() {
    let (mock, _s, _r, bundle) = capture_ok("loopback");
    let text =
        std::fs::read_to_string(bundle.join("api/requests.ndjson")).expect("the request log");
    let mut checked = 0;
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).expect("a JSON line");
        for field in ["url", "final_url"] {
            let Some(url) = v[field].as_str() else {
                continue;
            };
            assert!(
                url.starts_with("http://127.0.0.1:"),
                "{field} is not loopback: {url}"
            );
            assert!(
                url.starts_with(mock.base()) || url.starts_with(&mock.hyp_base()),
                "{field} points somewhere the mock does not own: {url}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 20, "only {checked} URLs were checked");
}

/// The bases the mock advertises are the bases the capture used, which is the
/// other half of the same claim: a capture cannot have reached a real CDN even if
/// it had wanted to, because nothing told it one.
#[test]
fn the_chunk_cdn_base_comes_from_the_mock_not_from_the_region() {
    let mock = Mock::start(World::five_classes());
    let scratch = Scratch::new("cdn");
    let r = capture(&mock, &scratch, &[]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let bundle = bundle_in(&scratch.out());

    // The `getBuild` body the bundle stored names the mock's own CDN.
    let body = std::fs::read_to_string(bundle.join("api/getBuild.7.1.0.json")).unwrap();
    assert!(
        body.contains(&format!("{}/cdn/", mock.base())),
        "the stored getBuild advertises the mock's CDN:\n{body}"
    );
    assert!(
        !body.contains("hoyoverse.com"),
        "and nothing points at a production host"
    );
    // `MANIFEST.json` records which bases were used, so a reader can tell from
    // the bundle alone.
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(bundle.join("MANIFEST.json")).unwrap())
            .unwrap();
    assert!(
        manifest["biz"]["hyp_base"]
            .as_str()
            .unwrap()
            .contains("127.0.0.1")
    );
    assert!(
        manifest["biz"]["sophon_base"]
            .as_str()
            .unwrap()
            .contains("127.0.0.1")
    );
}

// ------------------------------------------- the offline half, on this bundle

/// M10's real payload: what the capture wrote is good enough to verify and to
/// rebuild, with no network involved in either step.
#[test]
fn the_captured_bundle_verifies_offline() {
    let (_m, _s, _r, bundle) = capture_ok("verify");
    let r = run(&["verify", "--artifact", &bundle.to_string_lossy()]);
    assert_eq!(r.code, 0, "verify failed:\n{}\n{}", r.stdout, r.stderr);
    let s = r.summary();
    assert_eq!(s["result"], "pass");
    assert_eq!(s["checks_failed"], "0");
    assert!(
        s["captures_rebuilt"].parse::<u64>().unwrap() >= 4,
        "a bundle whose captures cannot be rebuilt from its own chunks proves nothing"
    );
    // Nine ids, not seven. A18 and A20 were always re-derived - A18 was simply
    // never named, which made the summary understate what `verify` had looked
    // at. A reader who cannot tell what was checked from what was merely
    // recorded would take this list at face value.
    assert_eq!(
        s["checks_derived"], "A16,A6,A11,A7,A8,A13,A17,A18,A20",
        "verify names exactly what it re-derived"
    );
    // And the two sets stay apart: the summary names nine, but only seven can be
    // cross-checked against a recording, because A18 and A20 are statements
    // about the bundle that no capture writes down.
    assert!(!gircap::verify::REDERIVED.contains(&"A20"));
    assert!(gircap::verify::DERIVED_UNRECORDED.contains(&"A20"));
}

#[test]
fn the_captured_bundle_materializes_to_the_same_bytes() {
    let (_m, _s, _r, bundle) = capture_ok("materialize");
    let out = bundle.with_file_name("rebuilt");
    let r = run(&[
        "materialize",
        "--artifact",
        &bundle.to_string_lossy(),
        "--out",
        &out.to_string_lossy(),
    ]);
    assert_eq!(r.code, 0, "materialize failed:\n{}\n{}", r.stdout, r.stderr);

    // Every manifest-derived file the bundle shipped is reproduced byte-for-byte.
    let mut compared = 0;
    for tree in ["game", "expected"] {
        let shipped_root = bundle.join(tree);
        let walk_from = shipped_root.clone();
        let mut files = Vec::new();
        collect(&walk_from, &mut files);
        for f in files {
            let rel = f.strip_prefix(&shipped_root).unwrap();
            let rebuilt = out.join(tree).join(rel);
            if !rebuilt.is_file() {
                // `config.ini` and the audio scan file are tool-written, not
                // chunk-derived, and are not part of the closure set.
                continue;
            }
            assert_eq!(
                gircap::hash::sha256_hex(&std::fs::read(&f).unwrap()),
                gircap::hash::sha256_hex(&std::fs::read(&rebuilt).unwrap()),
                "{tree}/{rel:?} differs"
            );
            compared += 1;
        }
    }
    assert!(compared >= 5, "only {compared} files were compared");
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[test]
fn the_captured_bundle_is_found_by_list() {
    let (_m, scratch, _r, bundle) = capture_ok("list");
    let r = run(&["list", "--root", &scratch.out().to_string_lossy()]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let s = r.summary();
    assert_eq!(s["bundles"], "1");
    assert_eq!(s["bundles_passing"], "1");
    assert!(bundle.ends_with(s["first"].as_str()), "{}", s["first"]);
}

// ------------------------------------------------------------ the matrix

/// The five classes the synthetic world publishes are found, and the four it
/// does not are `unavailable` rather than absent.
#[test]
fn the_matrix_names_every_class_and_says_why_the_rest_are_unavailable() {
    let (_m, _s, _r, bundle) = capture_ok("matrix");
    let raw = std::fs::read(bundle.join("MANIFEST.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    let rows = doc["matrix"].as_array().expect("a matrix");
    assert_eq!(rows.len(), 9, "one row per required class");
    let by = |c: &str| {
        rows.iter()
            .find(|r| r["class"] == c)
            .unwrap_or_else(|| panic!("no row for {c}"))
            .clone()
    };
    for c in [
        "identical",
        "added",
        "removed",
        "zero_reuse_multi",
        "partial_same_offset",
    ] {
        assert_eq!(by(c)["status"], "captured", "{c}: {}", by(c));
    }
    for c in [
        "zero_reuse_single",
        "partial_moved_offset",
        "partial_size_changed",
        "full_reuse_reordered",
    ] {
        assert_eq!(by(c)["status"], "unavailable", "{c}: {}", by(c));
        assert!(
            !by(c)["reason"].as_str().unwrap_or_default().is_empty(),
            "{c} is unavailable without a reason"
        );
    }
}

/// `--audio` widens the pool, which is M5's whole claim and the only reason the
/// flag exists.
///
/// The default is `en-us`, so the widening is asserted as a *difference*: with
/// the flag the pool carries two fields, with `--audio none` it carries one, and
/// the audio manifest is fetched from **both** builds either way it appears.
#[test]
fn audio_widens_the_selection_pool() {
    let mock = Mock::start(World::five_classes());
    let scratch = Scratch::new("audio");
    let with = capture(&mock, &scratch, &[]);
    assert_eq!(with.code, 0, "{}\n{}", with.stdout, with.stderr);
    assert_eq!(with.summary()["selection_pool"], "game,en-us");

    // Same world, audio switched off: one field, two manifests.
    let mock2 = Mock::start(World::game_only());
    let scratch2 = Scratch::new("audio-none");
    let without = capture(&mock2, &scratch2, &["--audio", "none"]);
    assert_eq!(without.code, 0, "{}\n{}", without.stdout, without.stderr);
    assert_eq!(without.summary()["selection_pool"], "game");
    let a = with.summary()["manifests_captured"].parse::<u64>().unwrap();
    let b = without.summary()["manifests_captured"]
        .parse::<u64>()
        .unwrap();
    assert_eq!(
        a,
        b + 2,
        "the audio field contributes one manifest per build"
    );
}

/// A pool naming a field no build publishes is refused, naming the field.
#[test]
fn a_pool_naming_an_unpublished_field_is_refused() {
    let mock = Mock::start(World::game_only());
    let scratch = Scratch::new("audiomissing");
    let r = capture(&mock, &scratch, &[]);
    assert_eq!(r.code, 2, "{}\n{}", r.stdout, r.stderr);
    let detail = percent_decode(&r.summary()["detail"]);
    assert!(detail.contains("en-us"), "{detail}");
    assert!(
        detail.contains("published"),
        "and what is published: {detail}"
    );
}

/// `--require-class` on a class the world has no candidate for is exit `4`,
/// and the artifact is still written.
#[test]
fn requiring_an_unavailable_class_is_exit_four() {
    let mock = Mock::start(World::five_classes());
    let scratch = Scratch::new("require");
    let r = capture(
        &mock,
        &scratch,
        &["--require-class", "full_reuse_reordered"],
    );
    assert_eq!(r.code, 4, "{}\n{}", r.stdout, r.stderr);
    assert_eq!(r.summary()["exit_code"], "4");
    // And it says which class, rather than only that something was unmet.
    let detail = percent_decode(&r.summary()["detail"]);
    assert!(detail.contains("full_reuse_reordered"), "{detail}");
}

/// The six `api` rows in `requests.ndjson` carry the path their body was
/// written to, and nothing else is marked.
///
/// **§6.1, verified defect #2.** All six `api` rows recorded a null `stored_as`
/// for a body that was sitting on disk, while `chunk` was 38/38 and `manifest`
/// 4/4. The cause was `CapturedBody::seq` hardcoded to `0` with the comment
/// "filled in by the caller", and the only thing that could fill it in —
/// `hyp::attach_seq` — had no production caller. `set_stored_as(0, ..)` found no
/// row and recorded nothing.
///
/// Asserted as a set equality rather than by sequence number or URL shape,
/// because both of those are incidental: the property that matters is that the
/// marked rows are **exactly** the bodies on disk. The tag probe and the error
/// shapes are written to `api/` too and are correctly left unmarked — they come
/// from different requests, and their answer is a summary rather than a stored
/// body — so counting files would be counting the wrong thing.
#[test]
fn the_six_api_rows_record_where_their_body_was_written() {
    let mock = Mock::start(World::five_classes());
    let scratch = Scratch::new("stored-as");
    let r = capture(&mock, &scratch, &[]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let bundle = bundle_in(&scratch.out());

    let log = std::fs::read_to_string(bundle.join("api").join("requests.ndjson"))
        .expect("requests.ndjson");
    let rows: Vec<serde_json::Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).expect("one json per line"))
        .collect();

    let mut marked: Vec<&str> = rows
        .iter()
        .filter_map(|r| r["stored_as"].as_str())
        .filter(|p| p.starts_with("api/"))
        .collect();
    marked.sort_unstable();

    // The bodies on disk, minus the two documents that are not bodies.
    let mut on_disk: Vec<String> = std::fs::read_dir(bundle.join("api"))
        .expect("api/")
        .flatten()
        .map(|e| format!("api/{}", e.file_name().to_string_lossy()))
        .filter(|p| p.ends_with(".json"))
        .filter(|p| {
            !p.ends_with("tag_probe.json") && !p.ends_with("error_shapes.json")
        })
        .collect();
    on_disk.sort();

    assert_eq!(
        marked,
        on_disk,
        "the marked api rows must be exactly the bodies on disk, and each must exist"
    );
    assert_eq!(
        marked.len(),
        6,
        "six metadata bodies are fetched; the count is the claim: {marked:?}"
    );

    // And the two summary documents stayed unmarked, which is the half that
    // stops a later edit from marking everything under `api/`.
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    for row in &rows {
        if let Some(p) = row["stored_as"].as_str() {
            *counts.entry(p).or_default() += 1;
        }
    }
    for (p, n) in &counts {
        assert_eq!(*n, 1, "each stored path is recorded once, not per request: {p}");
    }
    assert!(
        !marked.iter().any(|p| p.contains("tag_probe") || p.contains("error_shapes")),
        "a probe row is not a stored body: {marked:?}"
    );
}

/// The launcher metadata is not a chunk, so `--no-chunk-blobs` still writes it.
///
/// `emit::fixture::build`'s prelude call carries a long comment about a bug
/// this pins: putting `config.ini` and the scan file behind the flag left a
/// metadata-only bundle with no `game/` at all, and A13 - "the config re-parses
/// with `game_version == --old-tag`" - failed on a tree that simply had no
/// config in it. A missing file reported as a broken assertion is the worst of
/// the two: it says the tool is wrong when the tool did exactly what it was
/// asked.
///
/// The neighbouring test proves nothing goes *red*; this proves the row went
/// *green for the right reason* - because there was a file to read.
#[test]
fn no_chunk_blobs_still_writes_the_launcher_metadata() {
    let mock = Mock::start(World::five_classes());
    let scratch = Scratch::new("nobloobs-prelude");
    let r = capture(&mock, &scratch, &["--no-chunk-blobs"]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let bundle = bundle_in(&scratch.out());

    let config = bundle.join("game").join("config.ini");
    assert!(
        config.is_file(),
        "the flag suppresses chunk blobs, not launcher metadata: no {}",
        config.display()
    );
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(
        text.contains("game_version="),
        "a config with no version cannot satisfy A13: {text}"
    );

    // The scan file lands under the path the *launcher* named, which is the
    // point of reading `audio_pkg_scan_dir` rather than hardcoding one. The
    // path is the file: the launcher gives a directory and the tool writes the
    // code list to it, which is why this is not `Persistent/AudioScan.txt`.
    let scan = bundle
        .join("game")
        .join("GenshinImpact_Data")
        .join("Persistent");
    assert!(
        scan.is_file(),
        "the audio scan file goes where getGameConfigs said, under that exact \
         name: no {}",
        scan.display()
    );

    // And A13 evaluated that file rather than reporting it missing.
    let doc: serde_json::Value = serde_json::from_slice(
        &std::fs::read(bundle.join("verify").join("selfcheck.json")).unwrap(),
    )
    .unwrap();
    let a13 = doc["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "A13")
        .expect("an A13 row");
    assert_eq!(
        a13["ok"],
        serde_json::Value::Bool(true),
        "A13 must pass on a tree that has a config to read: {a13}"
    );
}

/// `--no-chunk-blobs` stores no blobs, and the result is `incomplete` rather
/// than `pass`: a skip is not a success.
#[test]
fn no_chunk_blobs_is_incomplete_not_pass() {    let mock = Mock::start(World::five_classes());
    let scratch = Scratch::new("noblobs");
    let r = capture(&mock, &scratch, &["--no-chunk-blobs"]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let bundle = bundle_in(&scratch.out());
    let raw = std::fs::read(bundle.join("verify/selfcheck.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    let failed: Vec<&serde_json::Value> = doc["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["ok"] != serde_json::Value::Bool(true))
        .collect();
    assert!(
        failed.is_empty(),
        "a skip is not a failure, so nothing may be red: {failed:#?}"
    );
    assert_eq!(doc["result"], "incomplete");
    assert_eq!(doc["counts"]["failed"], 0, "a skip is not a failure");
    // The flag governs what is *stored*, not what is asked for: the negative
    // probes in `api/error_shapes.json` need to see the origin's answer for a
    // bogus chunk id and for a real one under the wrong build's prefix, and that
    // is a property of the API rather than of this run's storage mode. So two
    // chunk requests happen and both are 404s; nothing else touches a chunk.
    let chunk_reqs = mock.chunk_requests();
    assert_eq!(
        chunk_reqs.len(),
        2,
        "only the two negative chunk probes may request a chunk: {chunk_reqs:?}"
    );
    let bogus = chunk_reqs
        .iter()
        .filter(|r| r.starts_with("0000000000000000_"))
        .count();
    assert_eq!(
        bogus, 1,
        "the chunk_missing probe asks for the bogus id: {chunk_reqs:?}"
    );
    assert_eq!(
        chunk_reqs.len() - bogus,
        1,
        "and the wrong-build probe asks for one real id: {chunk_reqs:?}"
    );
    assert!(count_blobs(&bundle) == 0, "and still nothing was stored");
    // And such a bundle cannot be materialized, which is what `verify` says.
    let v = run(&["verify", "--artifact", &bundle.to_string_lossy()]);
    assert_eq!(v.code, 3, "{}\n{}", v.stdout, v.stderr);
    assert_eq!(v.summary()["result"], "fail");
}

/// An old tag the API does not serve is refused, not patched to itself.
///
/// Exit `2`, not `1` and not `0`: the request was well-formed, the server
/// answered, and what it said was `retcode -202`. The tool reads that as *no such
/// tag* rather than as a build with no manifests, and says so in the detail.
#[test]
fn an_old_tag_the_api_does_not_serve_is_refused() {
    let mock = Mock::start(World::five_classes());
    let scratch = Scratch::new("nosuchtag");
    let r = capture(&mock, &scratch, &["--old-tag", "9.9.9"]);
    assert_eq!(r.code, 2, "{}\n{}", r.stdout, r.stderr);
    let detail = percent_decode(&r.summary()["detail"]);
    assert!(detail.contains("-202"), "the retcode is named: {detail}");
    assert!(detail.contains("9.9.9"), "and the tag: {detail}");
}

/// `diff_tags[0]` is the default for `--old-tag`, so a branch that does not list
/// the old build resolves the pair to something else - and the tool then refuses
/// *that*, rather than silently falling back.
#[test]
fn the_default_old_tag_is_diff_tags_zero_and_whatever_it_names_must_exist() {
    let mut world = World::five_classes();
    // Put the unservable tag first, as a branch whose old build was retired does.
    world.old_in_diff_tags = false;
    let mock = Mock::start(world);
    let scratch = Scratch::new("nodiff");
    let r = capture(&mock, &scratch, &[]);
    assert_eq!(r.code, 2, "{}\n{}", r.stdout, r.stderr);
    let detail = percent_decode(&r.summary()["detail"]);
    assert!(
        detail.contains("6.7.0"),
        "it resolved to what diff_tags[0] names: {detail}"
    );
    assert!(detail.contains("-202"), "{detail}");
}

/// `getBuild` for a tag that does not exist answers HTTP 200 with `retcode
/// -202`, and the tool reads that as "no such tag" rather than as a build.
#[test]
fn the_probe_records_an_unservable_tag_rather_than_pretending() {
    let (_m, _s, _r, bundle) = capture_ok("probe");
    let raw = std::fs::read(bundle.join("api/tag_probe.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    let tags = doc["results"].as_array().expect("one row per tag probed");
    let unservable: Vec<&str> = tags
        .iter()
        .filter(|t| t["served"].as_bool() == Some(false))
        .map(|t| t["tag"].as_str().unwrap_or_default())
        .collect();
    assert!(
        unservable.contains(&"6.7.0"),
        "the world's unservable tag should be recorded as unservable: {tags:#?}"
    );
    let servable: Vec<&str> = tags
        .iter()
        .filter(|t| t["served"].as_bool() == Some(true))
        .map(|t| t["tag"].as_str().unwrap_or_default())
        .collect();
    assert!(
        servable.contains(&"7.0.0") && servable.contains(&"7.1.0"),
        "{servable:?}"
    );
}

/// A shared chunk is fetched once, not once per build.
#[test]
fn a_chunk_shared_by_both_builds_is_fetched_once() {
    let (mock, _s, r, bundle) = capture_ok("reuse");
    // The negative probes ask for two chunk ids on purpose — a bogus one and a
    // new-build-only one under the old prefix — and those are the store's
    // business in no sense. Take them off by name from the document that records
    // them, rather than by a heuristic.
    let probed = probe_chunk_ids(&bundle);
    let ids: Vec<String> = mock
        .chunk_requests()
        .into_iter()
        .filter(|id| !probed.iter().any(|p| p == id))
        .collect();
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert!(!ids.is_empty(), "chunks were fetched: {ids:?}");
    assert_eq!(
        ids.len(),
        unique.len(),
        "every chunk was requested more than once: {ids:?}"
    );
    // And the tool reports it reused rather than re-downloading.
    let s = r.summary();
    let patch_fetch = s["patch_fetch_chunks"].parse::<u64>().unwrap();
    let stored = s["chunk_blobs_stored"].parse::<u64>().unwrap();
    assert!(
        stored >= patch_fetch,
        "{stored} stored vs {patch_fetch} patch fetches"
    );
}
