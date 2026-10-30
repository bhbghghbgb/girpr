//! End to end: a real `gircap capture` against a local mock, and then the
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

use common::{File, Mock, World};
use girpr_tempdir::TempDir;

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
/// The guard rather than a teardown call, because the interesting tests are the
/// ones that fail partway, and a fixture that only cleans up on the happy path
/// leaks into the next one.
struct Scratch {
    root: TempDir,
}

impl Scratch {
    fn new(name: &str) -> Scratch {
        Scratch {
            root: TempDir::new("gircap-layout", name),
        }
    }

    fn out(&self) -> PathBuf {
        self.root.join("artifacts")
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

/// The directory a refused capture left its rejection document in.
///
/// A run that refuses at the manifest boundary writes no `MANIFEST.json`, so
/// [`bundle_in`] cannot find it. The diagnostics document is the one thing such
/// a run does write, and it is written into the directory the bundle would have
/// occupied.
fn rejections_in(out: &Path) -> PathBuf {
    let mut found = Vec::new();
    walk(out, &mut found);
    found
        .into_iter()
        .find(|p| p.join("diagnostics/rejections.json").is_file())
        .unwrap_or_else(|| panic!("no rejection document under {}", out.display()))
}

/// A capture against a healthy mock, reused by several tests.
fn capture_ok(name: &str) -> (Mock, Scratch, Run, PathBuf) {
    let mock = Mock::start(World::all_shapes());
    let scratch = Scratch::new(name);
    let r = capture(&mock, &scratch, &[]);
    assert_eq!(r.code, 0, "capture failed:\n{}\n{}", r.stdout, r.stderr);
    let bundle = bundle_in(&scratch.out());
    (mock, scratch, r, bundle)
}

// ----------------------------------------------------------------- acceptance

/// A captured artifact has **zero failed selfcheck rows**.
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

/// `cargo nextest run -p gircap` green and no network. The "no network" half
/// is the structural one: every request the capture made went to `127.0.0.1`.
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
    // **Only the ordering that is real.** Only `getGameBranches` carries the
    // `package_id` that `getBuild` needs, so `branches < builds` is the invariant;
    // anything asserting which of two *concurrent* requests the mock happened to
    // accept first would be a test of the mock. The four HoYoPlay endpoints are
    // issued together, because none of them depends on another.
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
    // "stated" means *derived*. Asserting `retryable == false` for every row would
    // pass against a hardcoded constant and would keep passing if the policy
    // changed. Asserting the policy *relation* is the claim the field's doc
    // makes: if the fetcher would retry this status, the document says so,
    // whatever the answer is today.
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

/// A bundle without `api/error_shapes.json` does not open.
///
/// An additive document — "a bundle captured before it existed has no
/// `api/error_shapes.json`, and that is still a valid bundle" — lets a bundle
/// drop the tool's only measurement of what the API does with a request it
/// will not serve, and still verify clean: "this bundle can tell you how the
/// API fails" is then true of some bundles and not others, with nothing in
/// the bundle to tell a reader which is which.
///
/// The exit code is `1`, not `3`. "This directory is not a readable bundle" is
/// already `1` for every other reason — no `MANIFEST.json`, a truncated index —
/// and that mapping is pinned by `cli_contract`; a missing required document is
/// the same complaint. What matters here is that it is refused at all, and that
/// the refusal names the file rather than leaving the operator to guess between
/// five candidates.
#[test]
fn a_bundle_without_the_error_shapes_document_does_not_verify() {
    let (_m, _s, r, bundle) = capture_ok("noshapes");
    assert_eq!(r.code, 0);
    let file = bundle.join("api/error_shapes.json");
    assert!(file.is_file(), "the capture wrote it");
    std::fs::remove_file(&file).unwrap();

    let v = run(&["verify", "--artifact", &bundle.to_string_lossy()]);
    assert_ne!(
        v.code, 0,
        "a bundle without its error shapes must not verify clean:\n{}{}",
        v.stdout, v.stderr
    );
    assert!(
        format!("{}{}", v.stdout, v.stderr).contains("error_shapes.json"),
        "the refusal must name the document:\n{}{}",
        v.stdout,
        v.stderr
    );
}

/// And the same for `materialize`, which reads the bundle through the same door.
///
/// Without this the requirement would be `verify`-only, and the other two
/// commands would happily rebuild and re-list a bundle that `verify` refuses —
/// which is the shape of an inconsistency a reader meets as a bug in `verify`.
#[test]
fn every_command_that_reads_a_bundle_agrees_that_the_error_shapes_are_required() {
    let (_m, _s, r, bundle) = capture_ok("noshapes2");
    assert_eq!(r.code, 0);
    std::fs::remove_file(bundle.join("api/error_shapes.json")).unwrap();
    let dir = bundle.to_string_lossy().to_string();

    for cmd in [
        vec!["verify", "--artifact", &dir],
        vec!["materialize", "--artifact", &dir],
    ] {
        let out = run(&cmd);
        assert_ne!(
            out.code, 0,
            "{} must refuse a bundle with no error shapes:\n{}{}",
            cmd[0], out.stdout, out.stderr
        );
        assert!(
            format!("{}{}", out.stdout, out.stderr).contains("error_shapes.json"),
            "{} must name the document:\n{}{}",
            cmd[0],
            out.stdout,
            out.stderr
        );
    }
}

/// And the same for `api/error_responses.json`, the version-3 addition.
///
/// A capture that saw no non-2xx still writes the document, with an empty
/// `responses` list: absence would be indistinguishable from "this tool never
/// looked", which is exactly the ambiguity `error_shapes.json` was made required
/// to remove.
#[test]
fn a_bundle_without_the_error_responses_document_does_not_verify() {
    let (_m, _s, r, bundle) = capture_ok("noresps");
    assert_eq!(r.code, 0);
    let file = bundle.join("api/error_responses.json");
    assert!(file.is_file(), "the capture wrote it even with no failures");
    std::fs::remove_file(&file).unwrap();

    let v = run(&["verify", "--artifact", &bundle.to_string_lossy()]);
    assert_ne!(
        v.code, 0,
        "a bundle without its error responses must not verify clean:\n{}{}",
        v.stdout, v.stderr
    );
    assert!(
        format!("{}{}", v.stdout, v.stderr).contains("error_responses.json"),
        "the refusal must name the document:\n{}{}",
        v.stdout,
        v.stderr
    );
}

/// And `materialize` agrees, through the same door.
#[test]
fn every_command_that_reads_a_bundle_agrees_that_the_error_responses_are_required() {
    let (_m, _s, r, bundle) = capture_ok("noresps2");
    assert_eq!(r.code, 0);
    std::fs::remove_file(bundle.join("api/error_responses.json")).unwrap();
    let dir = bundle.to_string_lossy().to_string();

    for cmd in [
        vec!["verify", "--artifact", &dir],
        vec!["materialize", "--artifact", &dir],
    ] {
        let out = run(&cmd);
        assert_ne!(
            out.code, 0,
            "{} must refuse a bundle with no error responses:\n{}{}",
            cmd[0], out.stdout, out.stderr
        );
        assert!(
            format!("{}{}", out.stdout, out.stderr).contains("error_responses.json"),
            "{} must name the document:\n{}{}",
            cmd[0],
            out.stdout,
            out.stderr
        );
    }
}

/// And the same for `diagnostics/rejections.json`.
#[test]
fn a_bundle_without_the_rejection_document_does_not_verify() {
    let (_m, _s, r, bundle) = capture_ok("norej");
    assert_eq!(r.code, 0);
    let file = bundle.join("diagnostics/rejections.json");
    assert!(file.is_file(), "the capture wrote it even with no refusals");
    std::fs::remove_file(&file).unwrap();

    let v = run(&["verify", "--artifact", &bundle.to_string_lossy()]);
    assert_ne!(
        v.code, 0,
        "a bundle without its rejection document must not verify clean:\n{}{}",
        v.stdout, v.stderr
    );
    assert!(
        format!("{}{}", v.stdout, v.stderr).contains("rejections.json"),
        "the refusal must name the document:\n{}{}",
        v.stdout,
        v.stderr
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
        "api/error_responses.json",
        "diagnostics/path_map.json",
        "diagnostics/rejections.json",
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
/// `api/requests.ndjson` is one of the six documents that always differ run to
/// run — `ts`, `elapsed_ms`, CDN headers, `etag` — so `tests/bundle_diff.rs`
/// compares it with those blanked and nothing else compares it at all. That is
/// sound for what it was built to do: two captures of the *same* binary cannot
/// disagree about a change they both carry, so the gate catches nondeterminism
/// and nothing else. A change to what a row *says* passes it, so the row content
/// is pinned here instead.
///
/// Each line is
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
/// - `bytes_in` on the seven `probe` rows, which are the only non-2xx rows this
///   fixture produces. They must carry the real body length: zeroing them fails
///   here and nowhere else.
/// - `stored_as` on the six `api` rows. Zeroing them again fails here, and
///   nothing in `verify` would, because `stored_as` is a claim about where a body
///   was written rather than a digest it could re-derive.
/// - the `seq` column: dense, unique, from 1. Making `reserve()` return a
///   constant fails here.
///
/// What it does **not** catch, stated because a pin that only lists wins is
/// misleading: `attempts` is 1 on every row, because this fixture never
/// retries. Forcing it to 1 survives. So a **spurious** retry is caught and a
/// **hidden** one is not — the pin says "nothing was retried", not "retries are
/// recorded". `a_retry_is_recorded_as_retried` covers the other half, with a mock
/// route that answers `503` exactly once.
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
///   `max_decompressed_bytes`, `selection_pool`, `versions`' key set,
///   `capture.complete`, and the two
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
        "api/error_responses.json",
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
        "diagnostics/path_map.json",
        "diagnostics/rejections.json",
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
        "patchmaps/no_reuse.json",
        "patchmaps/removed.json",
        "patchmaps/reuse_in_place.json",
        "patchmaps/reuse_moved.json",
    ];
    const CHECKSUM_DOCUMENTS: &[&str] = &[
        "MANIFEST.json",
        "README.md",
        "api/error_responses.json",
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
        "diagnostics/path_map.json",
        "diagnostics/rejections.json",
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
        "patchmaps/no_reuse.json",
        "patchmaps/removed.json",
        "patchmaps/reuse_in_place.json",
        "patchmaps/reuse_moved.json",
        "verify/selfcheck.json",
    ];

    let m = json("MANIFEST.json");
    assert_eq!(m["schema_version"], gircap::SCHEMA_VERSION);
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
    assert_eq!(
        m["max_decompressed_bytes"],
        gircap::hash::DEFAULT_MAX_DECOMPRESSED_BYTES
    );
    assert_eq!(
        m["selection_pool"]["fields"],
        serde_json::json!(["game", "en-us"])
    );
    assert_eq!(
        m["selection_pool"]["manifest_entries"],
        serde_json::json!({"en-us": 1, "game": 7})
    );
    assert_eq!(m["capture"]["complete"], true);
    let vkeys: Vec<&String> = m["versions"].as_object().unwrap().keys().collect();
    assert_eq!(vkeys, vec!["diff_tags", "new", "old"]);
    assert_eq!(m["totals"]["files_captured"], 6);
    assert_eq!(m["totals"]["chunks_blobs"], 16);

    let mpaths: Vec<&str> = m["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().expect("a path"))
        .collect();
    let (mdocs, mcounts) = split_inventory(&mpaths);
    assert_eq!(
        mdocs, MANIFEST_DOCUMENTS,
        "MANIFEST.json documents[] changed"
    );
    assert_eq!(mcounts.get("blob"), Some(&16), "MANIFEST.json blob rows");
    assert_eq!(mcounts.get("game"), Some(&7), "MANIFEST.json game rows");
    assert_eq!(
        mcounts.get("expected"),
        Some(&5),
        "MANIFEST.json expected rows"
    );

    let c = json("verify/checksums.json");
    assert_eq!(c["algorithm"], "sha256");
    let epaths: Vec<&str> = c["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().expect("a path"))
        .collect();
    assert_eq!(c["file_count"], epaths.len());
    let (cdocs, ccounts) = split_inventory(&epaths);
    assert_eq!(
        cdocs, CHECKSUM_DOCUMENTS,
        "checksums.json documents[] changed"
    );
    assert_eq!(ccounts.get("blob"), Some(&16), "checksums.json blob rows");

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
        "- **captured** #   -# -# T# :# :# Z (`#       T#     .#     Z`, windows)",
        "**complete**: `#.#.#` (`sMOCKold# `) → `#.#.#` (`sMOCKnew# `)",
        "selfcheck: **Pass** — #  of #  assertions passed",
        "## Matrix",
        "| class | status | capture | path | field | bytes |",
        "|---|---|---|---|---|---|",
        "| `identical` | captured | identical | `GenshinImpact_Data/StreamingAssets/Audio/en-us/vo_all` | en-us | #   |",
        "| `no_reuse` | captured | no_reuse | `GenshinImpact_Data/StreamingAssets/blocks/zero.blk` | game | #    |",
        "| `reuse_in_place` | captured | reuse_in_place | `GenshinImpact_Data/Plugins/HoYoChannel.dll` | game | #    |",
        "| `reuse_moved` | captured | reuse_moved | `GenshinImpact_Data/StreamingAssets/blocks/deferred.blk` | game | #    |",
        "| `added` | captured | added | `GenshinImpact_Data/Plugins/libNew.dll` | game | - |",
        "| `removed` | captured | removed | `GenshinImpact_Data/StreamingAssets/blocks/old.blk` | game | #     |",
        "## Totals",
        "- # files captured, #  chunk blobs stored",
        "- #     B downloaded, #      B on disk",
        "- selection pool: `game`, `en-us`",
        "- `materialize_new`: #  chunks",
        "- `materialize_old`: #  chunks",
        "- `patch_fetch`: # chunks",
        "## Re-verifying",
        "```",
        "gircap verify --artifact <this directory>",
        "```",
        "Offline: no network, no writes. Exit # if every assertion still holds.",
    ];
    let lines: Vec<&str> = skel
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(lines, README_SKELETON, "README.md prose changed");

    // The skeleton's counts are blanked, so the *agreement* between the two
    // documents has to be checked on the raw bytes. A generated file that
    // disagrees with the document it renders is worse than no file: it is the
    // one a reader opens first, and it was wrong by exactly the assertion the
    // close-out adds after rendering.
    let sc = json("verify/selfcheck.json");
    let (total, passed) = (
        sc["counts"]["total"].as_u64().expect("counts.total"),
        sc["counts"]["passed"].as_u64().expect("counts.passed"),
    );
    assert!(
        read("README.md").contains(&format!("{passed} of {total} assertions passed")),
        "README.md's counts disagree with verify/selfcheck.json's"
    );
}

/// `verify/selfcheck.json` is the tool's own verdict on itself, and it is the one
/// document a reader is most likely to trust, so its shape is pinned: the
/// result, the counts, and the exact wording of every assertion it makes.
///
/// The wording is the point. `id` alone would pass if an assertion were
/// silently narrowed — "every reused chunk's old-file slice MD5 matches" and
/// "some reused chunk's MD5 matches" are both `A10`, and only the text tells
/// them apart.
#[test]
fn the_selfcheck_document_asserts_what_it_always_asserted() {
    let (_m, _s, _r, bundle) = capture_ok("sc");
    let s: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(bundle.join("verify/selfcheck.json")).unwrap(),
    )
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
        vec![
            "A1", "A10", "A11", "A12", "A13", "A14", "A15", "A16", "A17", "A19", "A2", "A21",
            "A22", "A23", "A3", "A4", "A6", "A7", "A8", "A9"
        ],
        "the set of assertions a capture makes changed; A5, A18 and A20 are \
         verify-only or absent by design and must stay that way"
    );

    let mut worded: Vec<String> = rows
        .iter()
        .map(|r| {
            format!(
                "{} {}",
                r["id"].as_str().unwrap(),
                r["assertion"].as_str().unwrap()
            )
        })
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
        "A17 each required matrix class is captured | covered_by | over_budget | unavailable - never absent",
        "A19 each two-sided captured file differs between game/ and expected/ exactly when its class says it should",
        "A2 getBuild(new).data.tag == --new-tag; getBuild(old).data.tag == --old-tag",
        "A21 the bundle covers both sides of the reuse boundary: at least one capture with nothing reusable and at least one with something",
        "A22 the bundle covers both sides of the moved boundary: at least one capture whose reused chunks are in place and at least one whose reused chunks moved",
        "A23 the bundle covers both sides of the size boundary: at least one capture whose file grows and at least one whose file shrinks",
        "A3 manifest blob md5(decompressed) == manifest.checksum",
        "A4 manifest.checksum == id 32-hex suffix; id 16-hex prefix recorded as opaque",
        "A6 per chunk: len(raw blob) == compressed_size; md5(raw) == compressed_md5; md5(zstd(raw)) == uncompressed_md5; len(decoded) == uncompressed_size",
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
    let mock = Mock::start(World::all_shapes());
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
        "5 api GET /sophon/getBuild?branch=main&package_id=ScSYQBFhu9&password=sMOCKpassword&tag=7.1.0 200 1348 1 api/getBuild.7.1.0.json -",
        "6 api GET /sophon/getBuild?branch=main&package_id=ScSYQBFhu9&password=sMOCKpassword&tag=7.0.0 200 1348 1 api/getBuild.7.0.0.json -",
        "7 probe GET /sophon/getBuild?branch=main&package_id=ScSYQBFhu9&password=sMOCKpassword&tag=6.7.0 200 50 1 - -",
        "8 manifest GET /cdn/sMOCKold00/manifest/manifest_2771a8e0cdb7b707_70ef994bc1c8a1a3f8bae061f3173a59 200 1302 1 manifests/7.0.0/game.manifest.zst -",
        "9 manifest GET /cdn/sMOCKnew00/manifest/manifest_3fc907e1b5db4352_3004baafaed168985353c643b2daa6f4 200 1249 1 manifests/7.1.0/game.manifest.zst -",
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
        "21 chunk GET /cdn/sMOCKold00/chunk/4e03e3d397f031f6_bce567638abdc69b80c7bde1f50677f6 200 1033 1 chunks/blobs/4e03e3d397f031f6_bce567638abdc69b80c7bde1f50677f6.zst -",
        "22 chunk GET /cdn/sMOCKnew00/chunk/5323e19e9e03d8c5_ee3f3f3e27aa76ff65c4c3a1c1aa1d7b 200 265 1 chunks/blobs/5323e19e9e03d8c5_ee3f3f3e27aa76ff65c4c3a1c1aa1d7b.zst -",
        "23 chunk GET /cdn/sMOCKnew00/chunk/69edd52759c53740_12283b0a8bbcfc40b5e00175c1d7dfea 200 1033 1 chunks/blobs/69edd52759c53740_12283b0a8bbcfc40b5e00175c1d7dfea.zst -",
        "24 chunk GET /cdn/sMOCKnew00/chunk/6a2c2034429981aa_1fbd0959e726da54e75cd528a6c15469 200 521 1 chunks/blobs/6a2c2034429981aa_1fbd0959e726da54e75cd528a6c15469.zst -",
        "25 chunk GET /cdn/sMOCKnew00/chunk/734acba1f3db65a7_759d96ceaf6d243146547304a953ad3c 200 1033 1 chunks/blobs/734acba1f3db65a7_759d96ceaf6d243146547304a953ad3c.zst -",
        "26 chunk GET /cdn/sMOCKnew00/chunk/78389a4e7cc1b62b_4a84467752ea9c122de6b7288b0f7e73 200 521 1 chunks/blobs/78389a4e7cc1b62b_4a84467752ea9c122de6b7288b0f7e73.zst -",
        "27 chunk GET /cdn/sMOCKold00/chunk/7bc80346da24dc10_36bed2b08e4df3d4ba63710fcc4e7389 200 1033 1 chunks/blobs/7bc80346da24dc10_36bed2b08e4df3d4ba63710fcc4e7389.zst -",
        "28 chunk GET /cdn/sMOCKnew00/chunk/8cdfadf64dbc1235_c67e30034329d2ba4419ca814f691bde 200 1033 1 chunks/blobs/8cdfadf64dbc1235_c67e30034329d2ba4419ca814f691bde.zst -",
        "29 chunk GET /cdn/sMOCKnew00/chunk/91ea6819c901e42b_8b31ee982e6de0878e3b4515b4a09ad4 200 265 1 chunks/blobs/91ea6819c901e42b_8b31ee982e6de0878e3b4515b4a09ad4.zst -",
        "30 chunk GET /cdn/sMOCKold00/chunk/a80d7329a2bf0f6c_3b5a7d035458664fcdd3e13fb1b193cf 200 521 1 chunks/blobs/a80d7329a2bf0f6c_3b5a7d035458664fcdd3e13fb1b193cf.zst -",
        "31 chunk GET /cdn/sMOCKnew00/chunk/ba3ffe0a4f82d1b5_a003157e0deaac5832ecfc30f3e62982 200 265 1 chunks/blobs/ba3ffe0a4f82d1b5_a003157e0deaac5832ecfc30f3e62982.zst -",
        "32 chunk GET /cdn/sMOCKold00/chunk/d3263f518c944d35_f639a004b4ab92e314c88a73b9c0aa36 200 32777 1 chunks/blobs/d3263f518c944d35_f639a004b4ab92e314c88a73b9c0aa36.zst -",
        "33 chunk GET /cdn/sMOCKold00/chunk/dc4c4277b58c6559_9c389ad675e81e5be30dc2aea4770636 200 1033 1 chunks/blobs/dc4c4277b58c6559_9c389ad675e81e5be30dc2aea4770636.zst -",
        "34 chunk GET /cdn/sMOCKnew00/chunk/dced37b2dd2b2118_558db22ff96610039fa29418b658bbaa 200 1033 1 chunks/blobs/dced37b2dd2b2118_558db22ff96610039fa29418b658bbaa.zst -",
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
        got_keys, EXPECTED,
        "api/requests.ndjson changed. Left is the run, right is the pin.\n\
         A changed `bytes_in`, `stored_as` or `attempts` is a real behaviour \
         change and needs a band-3 note; a changed `seq` means the log is no \
         longer dense, unique, or sorted."
    );
}

/// A retried request is recorded as retried.
///
/// `d745680`'s pin has `attempts` at 1 on all 28 rows, because the mock answers
/// consistently — which meant the field said "nothing was retried" and could not
/// tell a retry that was recorded from one that was dropped. Forcing `attempts`
/// to 1 survived that mutation, and this is the test that would kill it.
///
/// `Mock::fail_once_for` arms one path to answer `503` exactly once. A `5xx` is
/// what `RetryPolicy` retries, so the first attempt fails, the second succeeds,
/// and the request log must say `2` — with the blob still landing, because a retry
/// that recorded itself but lost the body would be the worse bug.
#[test]
fn a_retried_request_is_recorded_as_retried() {
    // Armed on **one chunk id**, not on `/chunk/`. The negative probes request
    // `/cdn/<build>/chunk/0000000000000000_...`, so a bare `/chunk/` needle hits
    // a *probe* first — and `observe` does not retry, which is how the first
    // version of this test passed with 28 rows and nothing retried. The id below
    // is the same blob the row pin already names, so it is the same construction
    // either way.
    let armed = "/chunk/059f931481c3a4bf_90d769cb1f8fb7037073188f9399952e";
    let mock = Mock::start(World::all_shapes()).fail_once_for(armed);
    let scratch = Scratch::new("retry");
    let r = capture(&mock, &scratch, &["--jobs", "1"]);
    assert_eq!(r.code, 0, "capture failed:\n{}\n{}", r.stdout, r.stderr);
    let bundle = bundle_in(&scratch.out());

    let log = std::fs::read_to_string(bundle.join("api").join("requests.ndjson"))
        .expect("api/requests.ndjson");
    let rows: Vec<serde_json::Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).expect("one row per line"))
        .collect();

    let retried: Vec<&serde_json::Value> = rows
        .iter()
        .filter(|r| r["attempts"].as_u64().unwrap_or(0) > 1)
        .collect();
    let statuses: std::collections::BTreeSet<String> =
        rows.iter().map(|r| r["status"].to_string()).collect();
    assert_eq!(
        retried.len(),
        1,
        "exactly one request should have been retried. rows={} statuses={:?} \
         chunk_requests={}",
        rows.len(),
        statuses,
        mock.requests()
            .iter()
            .filter(|r| r.path.contains("/chunk/"))
            .count()
    );

    let row = retried[0];
    assert_eq!(
        row["attempts"], 2,
        "one failure then one success is two attempts"
    );
    assert_eq!(
        row["status"], 200,
        "the retried request succeeded in the end"
    );
    assert_eq!(row["kind"], "chunk", "the armed route was a chunk route");

    // And the body it retried for is actually in the bundle. A retry that
    // recorded itself but dropped the bytes would pass every assertion above.
    let stored = row["stored_as"]
        .as_str()
        .expect("a retried chunk is stored");
    assert!(
        bundle.join(stored).is_file(),
        "the retried chunk's body is missing: {stored}"
    );

    // Every other row is still a single attempt, so the retry was not smeared
    // across the log by an off-by-one in the counter.
    let singles = rows
        .iter()
        .filter(|r| r["attempts"].as_u64() == Some(1))
        .count();
    assert_eq!(singles, rows.len() - 1);
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
    let mock = Mock::start(World::all_shapes());
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

/// **No test touches the network.** Proven, not asserted.
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
    let mock = Mock::start(World::all_shapes());
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

/// The real payload: what the capture wrote is good enough to verify and to
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

/// The matrix names **every** class, and the synthetic world publishes a
/// candidate for each of them — so every row is `captured`.
///
/// A class with no candidate is `unavailable` **with a reason** rather than
/// absent from the matrix, and that is what the selection's own tests pin.
#[test]
fn the_matrix_names_every_class_and_says_what_happened_to_it() {
    let (_m, _s, _r, bundle) = capture_ok("matrix");
    let raw = std::fs::read(bundle.join("MANIFEST.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    let rows = doc["matrix"].as_array().expect("a matrix");
    assert_eq!(rows.len(), 6, "one row per patch shape");
    let by = |c: &str| {
        rows.iter()
            .find(|r| r["class"] == c)
            .unwrap_or_else(|| panic!("no row for {c}"))
            .clone()
    };
    for c in [
        "identical",
        "no_reuse",
        "reuse_in_place",
        "reuse_moved",
        "added",
        "removed",
    ] {
        assert_eq!(by(c)["status"], "captured", "{c}: {}", by(c));
    }
}

/// `--audio` widens the pool, which is its whole claim and the only reason the
/// flag exists.
///
/// The default is `en-us`, so the widening is asserted as a *difference*: with
/// the flag the pool carries two fields, with `--audio none` it carries one, and
/// the audio manifest is fetched from **both** builds either way it appears.
#[test]
fn audio_widens_the_selection_pool() {
    let mock = Mock::start(World::all_shapes());
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

/// A manifest path is document-supplied. It becomes
/// `<artifact>/game/<file>` and `expected/<file>`, so an escaping value must be
/// refused at decode rather than written four directories above `--out`.
///
/// The hostile file is the only game file in this world, so an ordinary
/// selection reaches it: without the decode check the capture would assemble it
/// and write `game/../../../../../gircap-escape`.
#[test]
fn a_manifest_file_path_that_escapes_the_bundle_is_refused() {
    let hostile = "../../../../../gircap-escape";
    let mut world = World::game_only();
    for (build, seed) in [(&mut world.old, 501_u64), (&mut world.new, 502_u64)] {
        let game = &mut build
            .fields
            .iter_mut()
            .find(|(field, _)| field == "game")
            .expect("the world publishes game")
            .1;
        *game = vec![File::new(hostile, &[seed], 32)];
    }
    let mock = Mock::start(world);
    let scratch = Scratch::new("manifest-escape");
    let r = capture(&mock, &scratch, &["--audio", "none"]);

    assert_eq!(r.code, 2, "{}\n{}", r.stdout, r.stderr);
    let text = format!("{}{}", r.stdout, r.stderr);
    assert!(text.contains("refused name"), "{text}");
    assert!(text.contains("escapes the bundle root"), "{text}");
    assert!(
        !scratch.root.join("gircap-escape").exists(),
        "nothing may be written outside --out"
    );
    // **The refusal is evidence.** The value is not written as a path — the
    // escape target does not exist — but it is kept in the diagnostics document
    // inside the directory the bundle would have occupied, with the rule it
    // failed.
    let dir = rejections_in(&scratch.out());
    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("diagnostics/rejections.json")).unwrap())
            .unwrap();
    assert_eq!(doc["rejections"][0]["value"], hostile);
    assert_eq!(doc["rejections"][0]["rule"], "path.parent");
    assert_eq!(
        doc["rejections"][0]["source"],
        "manifest 7.0.0/game entry 0 file"
    );
}

/// **L1: a device name is kept, not discarded.** A manifest path that names
/// `CON` is a fact about the API. The documents keep the game's spelling, the
/// tree is written under `%CON`, and `diagnostics/path_map.json` records the
/// pair. `verify` and `materialize` follow the same escape, so the bundle is
/// readable on a host where the unescaped name would open a device.
#[test]
fn a_device_name_manifest_path_is_kept_and_escaped_on_disk() {
    const LOGICAL: &str = "GenshinImpact_Data/Resources/CON";
    let mut world = World::game_only();
    for build in [&mut world.old, &mut world.new] {
        let game = &mut build
            .fields
            .iter_mut()
            .find(|(field, _)| field == "game")
            .expect("the world publishes game")
            .1;
        let file = game
            .iter_mut()
            .find(|f| f.path == "GenshinImpact_Data/Resources/browser_assets")
            .expect("the all-shapes world has the identical file");
        file.path = LOGICAL.to_string();
    }
    let mock = Mock::start(world);
    let scratch = Scratch::new("device-path");
    let r = capture(&mock, &scratch, &["--audio", "none"]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let bundle = bundle_in(&scratch.out());

    // The tree holds the escaped name, and not the one that would open a device.
    assert!(
        bundle
            .join("game/GenshinImpact_Data/Resources/%CON")
            .is_file(),
        "the old side is written escaped"
    );
    assert!(
        bundle
            .join("expected/GenshinImpact_Data/Resources/%CON")
            .is_file(),
        "the new side is written escaped"
    );
    assert!(
        !bundle
            .join("game/GenshinImpact_Data/Resources/CON")
            .exists(),
        "the unescaped device name must never be created"
    );

    // The documents keep the game's own spelling.
    let patchmap: serde_json::Value =
        serde_json::from_slice(&std::fs::read(bundle.join("patchmaps/identical.json")).unwrap())
            .unwrap();
    assert_eq!(patchmap["path"], LOGICAL);
    let index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(bundle.join("chunks/index.json")).unwrap()).unwrap();
    let referenced: Vec<String> = index["blobs"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|b| b["referenced_by"].as_array().cloned().unwrap_or_default())
        .filter_map(|r| r["path"].as_str().map(str::to_string))
        .collect();
    assert!(
        referenced.iter().any(|p| p == LOGICAL),
        "the index keeps the logical path: {referenced:?}"
    );

    // The mapping is recorded, and `files[]` lists what was written.
    let map: serde_json::Value =
        serde_json::from_slice(&std::fs::read(bundle.join("diagnostics/path_map.json")).unwrap())
            .unwrap();
    let game_map = map["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["logical"] == format!("game/{LOGICAL}"))
        .expect("the old side is mapped");
    assert_eq!(
        game_map["on_disk"],
        "game/GenshinImpact_Data/Resources/%CON".to_string()
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(bundle.join("MANIFEST.json")).unwrap()).unwrap();
    let files: Vec<&str> = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();
    assert!(
        files.contains(&"game/GenshinImpact_Data/Resources/%CON"),
        "files[] lists the on-disk name"
    );

    // `verify` reads the escaped tree and passes; `materialize` reproduces it.
    let dir = bundle.to_string_lossy().to_string();
    let v = run(&["verify", "--artifact", &dir]);
    assert_eq!(v.code, 0, "{}\n{}", v.stdout, v.stderr);

    let out = scratch.root.join("rebuilt");
    let m = run(&[
        "materialize",
        "--artifact",
        &dir,
        "--out",
        &out.to_string_lossy(),
    ]);
    assert_eq!(m.code, 0, "{}\n{}", m.stdout, m.stderr);
    assert!(out.join("game/GenshinImpact_Data/Resources/%CON").is_file());
    assert!(!out.join("game/GenshinImpact_Data/Resources/CON").exists());
}

/// A device name is only escaped where it is an **observation**. At argument
/// resolution there is no bundle, so `--files` keeps refusing it.
#[test]
fn a_device_name_from_a_flag_is_still_refused() {
    let mock = Mock::start(World::game_only());
    let scratch = Scratch::new("device-flag");
    let r = capture(
        &mock,
        &scratch,
        &[
            "--audio",
            "none",
            "--files",
            "GenshinImpact_Data/Resources/CON",
        ],
    );
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    assert!(
        format!("{}{}", r.stdout, r.stderr).contains("device"),
        "{}{}",
        r.stdout,
        r.stderr
    );
}

/// A chunk id is both a CDN path segment and `chunks/blobs/<id>.zst`. The
/// documented `<16 hex>_<32 hex>` shape is checked where the manifest is
/// decoded, before the id can reach either sink.
#[test]
fn a_manifest_chunk_id_outside_the_documented_shape_is_refused() {
    let hostile = "../../../../gircap-chunk";
    let mut world = World::game_only();
    for build in [&mut world.old, &mut world.new] {
        let game = &mut build
            .fields
            .iter_mut()
            .find(|(field, _)| field == "game")
            .expect("the world publishes game")
            .1;
        game[0].chunks[0].id = hostile.to_string();
    }
    let mock = Mock::start(world);
    let scratch = Scratch::new("chunk-id-escape");
    let r = capture(&mock, &scratch, &["--audio", "none"]);

    assert_eq!(r.code, 2, "{}\n{}", r.stdout, r.stderr);
    let text = format!("{}{}", r.stdout, r.stderr);
    assert!(text.contains("not a chunk id"), "{text}");
    assert!(
        !scratch.root.join("gircap-chunk").exists(),
        "nothing may be written outside --out"
    );
    let dir = rejections_in(&scratch.out());
    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("diagnostics/rejections.json")).unwrap())
            .unwrap();
    assert_eq!(doc["rejections"][0]["value"], hostile);
    assert_eq!(doc["rejections"][0]["rule"], "chunk-id.shape");
    assert_eq!(
        doc["rejections"][0]["source"],
        "manifest 7.0.0/game entry 0 chunk 0 id"
    );
}

/// `getGameConfigs.audio_pkg_scan_dir` is joined under `game/`. A proxy or a
/// compromised API can supply it, so it is checked before the tree is created.
#[test]
fn an_audio_scan_dir_that_escapes_the_bundle_is_refused() {
    let mut world = World::all_shapes();
    world.audio_pkg_scan_dir = "../../../../../gircap-scan".into();
    let mock = Mock::start(world);
    let scratch = Scratch::new("scan-dir-escape");
    let r = capture(&mock, &scratch, &["--audio", "none"]);

    assert_eq!(r.code, 3, "{}\n{}", r.stdout, r.stderr);
    let text = format!("{}{}", r.stdout, r.stderr);
    assert!(text.contains("audio_pkg_scan_dir"), "{text}");
    assert!(text.contains("escapes the bundle root"), "{text}");
    assert!(
        !scratch.root.join("gircap-scan").exists(),
        "nothing may be written outside --out"
    );
    assert!(
        !scratch.out().exists(),
        "the refusal happens before the artifact directory exists"
    );
}

/// `--require-class` on a class the run cannot satisfy is exit `4`, and the
/// artifact is still written.
///
/// A ceiling of zero makes every class `over_budget`, which is the honest way
/// to ask for a class the run cannot have: the matrix says so, and the
/// requirement is not met.
#[test]
fn requiring_an_unmet_class_is_exit_four() {
    let mock = Mock::start(World::all_shapes());
    let scratch = Scratch::new("require");
    let r = capture(
        &mock,
        &scratch,
        &["--max-download-bytes", "0", "--require-class", "identical"],
    );
    assert_eq!(r.code, 4, "{}\n{}", r.stdout, r.stderr);
    assert_eq!(r.summary()["exit_code"], "4");
    // And it says which class, rather than only that something was unmet.
    let detail = percent_decode(&r.summary()["detail"]);
    assert!(detail.contains("identical"), "{detail}");
}

/// The six `api` rows in `requests.ndjson` carry the path their body was
/// written to, and nothing else is marked.
///
/// **The property that is easy to lose.** A null `stored_as` here means the row
/// is a claim about where a body went, and a null claim on a body that is
/// sitting on disk under `api/` is a falsehood in the one field a reader would
/// use the log for. Nothing in `verify` re-derives it — it is not a digest — so
/// only a field-level check catches it.
///
/// Asserted as a set equality rather than by sequence number or URL shape,
/// because both of those are incidental: the property that matters is that the
/// marked rows are **exactly** the bodies on disk. The tag probe and the error
/// shapes are written to `api/` too and are correctly left unmarked — they come
/// from different requests, and their answer is a summary rather than a stored
/// body — so counting files would be counting the wrong thing.
#[test]
fn the_six_api_rows_record_where_their_body_was_written() {
    let mock = Mock::start(World::all_shapes());
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
            !p.ends_with("tag_probe.json")
                && !p.ends_with("error_shapes.json")
                && !p.ends_with("error_responses.json")
        })
        .collect();
    on_disk.sort();

    assert_eq!(
        marked, on_disk,
        "the marked api rows must be exactly the bodies on disk, and each must exist"
    );
    assert_eq!(
        marked.len(),
        6,
        "six metadata bodies are fetched; the count is the claim: {marked:?}"
    );

    // And the summary documents stayed unmarked, which is the half that
    // stops a later edit from marking everything under `api/`.
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    for row in &rows {
        if let Some(p) = row["stored_as"].as_str() {
            *counts.entry(p).or_default() += 1;
        }
    }
    for (p, n) in &counts {
        assert_eq!(
            *n, 1,
            "each stored path is recorded once, not per request: {p}"
        );
    }
    assert!(
        !marked.iter().any(|p| p.contains("tag_probe")
            || p.contains("error_shapes")
            || p.contains("error_responses")),
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
    let mock = Mock::start(World::all_shapes());
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
fn no_chunk_blobs_is_incomplete_not_pass() {
    let mock = Mock::start(World::all_shapes());
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
    let mock = Mock::start(World::all_shapes());
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
    let mut world = World::all_shapes();
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
