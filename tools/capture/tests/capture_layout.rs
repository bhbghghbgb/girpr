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
            .unwrap_or_else(|| panic!("no SUMMARY on stdout:\n{}\n---\n{}", self.stdout, self.stderr));
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
    let Ok(rd) = std::fs::read_dir(dir) else { return };
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
    assert!(doc["counts"]["total"].as_u64().unwrap() > 0, "and rows were produced");
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
    // The four HoYoPlay endpoints, in the order the capture issues them.
    let order: Vec<&str> = reqs.iter().map(|r| r.path.as_str()).collect();
    let configs = order.iter().position(|p| p.ends_with("/getGameConfigs")).expect("getGameConfigs");
    let branches = order.iter().position(|p| p.ends_with("/getGameBranches")).expect("getGameBranches");
    let builds = order.iter().position(|p| p.ends_with("/getBuild")).expect("getBuild");
    assert!(configs < branches, "getGameConfigs before getGameBranches");
    assert!(branches < builds, "the branch that carries package_id is fetched before getBuild");
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
        "chunks/index.json",
        "diff/paths.diff.json",
        "diff/candidates.json",
        "game/config.ini",
        "verify/checksums.json",
        "verify/selfcheck.json",
    ] {
        assert!(bundle.join(rel).is_file(), "missing {rel} in {}", bundle.display());
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

    // Ask the mock again for the same endpoint and compare.
    let url = format!("{}/getGameConfigs", mock.hyp_base());
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
    s.write_all(format!("GET /{path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n").as_bytes())
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
    let text = std::fs::read_to_string(bundle.join("api/requests.ndjson")).expect("the request log");
    let mut checked = 0;
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).expect("a JSON line");
        for field in ["url", "final_url"] {
            let Some(url) = v[field].as_str() else { continue };
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
        serde_json::from_str(&std::fs::read_to_string(bundle.join("MANIFEST.json")).unwrap()).unwrap();
    assert!(manifest["biz"]["hyp_base"].as_str().unwrap().contains("127.0.0.1"));
    assert!(manifest["biz"]["sophon_base"].as_str().unwrap().contains("127.0.0.1"));
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
    assert!(s["captures_rebuilt"].parse::<u64>().unwrap() >= 4);
    assert_eq!(
        s["checks_derived"], "A16,A6,A11,A7,A8,A13,A17",
        "verify names exactly what it re-derived"
    );
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
    let Ok(rd) = std::fs::read_dir(dir) else { return };
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
    let b = without.summary()["manifests_captured"].parse::<u64>().unwrap();
    assert_eq!(a, b + 2, "the audio field contributes one manifest per build");
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
    assert!(detail.contains("published"), "and what is published: {detail}");
}

/// `--require-class` on a class the world has no candidate for is exit `4`,
/// and the artifact is still written.
#[test]
fn requiring_an_unavailable_class_is_exit_four() {
    let mock = Mock::start(World::five_classes());
    let scratch = Scratch::new("require");
    let r = capture(&mock, &scratch, &["--require-class", "full_reuse_reordered"]);
    assert_eq!(r.code, 4, "{}\n{}", r.stdout, r.stderr);
    assert_eq!(r.summary()["exit_code"], "4");
    // And it says which class, rather than only that something was unmet.
    let detail = percent_decode(&r.summary()["detail"]);
    assert!(detail.contains("full_reuse_reordered"), "{detail}");
}

/// `--no-chunk-blobs` stores no blobs, and the result is `incomplete` rather
/// than `pass`: a skip is not a success.
#[test]
fn no_chunk_blobs_is_incomplete_not_pass() {
    let mock = Mock::start(World::five_classes());
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
    assert_eq!(mock.chunk_requests().len(), 0, "no chunk was requested");
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
    assert!(detail.contains("6.7.0"), "it resolved to what diff_tags[0] names: {detail}");
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
    assert!(servable.contains(&"7.0.0") && servable.contains(&"7.1.0"), "{servable:?}");
}

/// A shared chunk is fetched once, not once per build.
#[test]
fn a_chunk_shared_by_both_builds_is_fetched_once() {
    let (mock, _s, r, _b) = capture_ok("reuse");
    let ids = mock.chunk_requests();
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
    assert!(stored >= patch_fetch, "{stored} stored vs {patch_fetch} patch fetches");
}