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

    // The retry policy is stated in the artifact, not only in the fetcher.
    for r in rows {
        assert_eq!(r["retryable"], false, "{} is not retryable: {r}", r["id"]);
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
