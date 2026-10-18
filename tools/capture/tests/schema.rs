//! The artifact's own documents - schema stability, the gzip rule, and
//! `checksums.json` coverage.
//!
//! Everything here reads a bundle a real `capture` wrote against the mock, so
//! each assertion is about the bytes that ship rather than about a struct in the
//! writer's memory. A test that asserted the writer produced what the writer
//! produces would catch nothing; this one re-reads from disk.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::{Mock, World};

fn bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("the test binary has a path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("gircap.exe")
}

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin())
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("running {args:?}: {e}"));
    (
        out.status.code().expect("not killed"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// One capture, written once and shared by the tests below.
///
/// `OnceLock` rather than one capture per test: each capture is a process
/// against a listener, and five tests paying for five captures makes the suite
/// slow for no extra coverage. The bundle is read-only to every test that uses
/// it.
fn bundle() -> &'static PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let scratch = std::env::temp_dir().join(format!("gircap-schema-{}", std::process::id()));
    DIR.get_or_init(|| {
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch");
        let mock = Mock::start(World::five_classes());
        let out = scratch.join("artifacts");
        let (code, stdout, stderr) = run(&[
            "capture",
            "--out",
            &out.to_string_lossy(),
            "--hyp-base",
            &mock.hyp_base(),
            "--sophon-base",
            &mock.sophon_base(),
        ]);
        assert_eq!(code, 0, "{stdout}\n{stderr}");
        let bundle = find_bundle(&out);
        // The listener and the scratch root go away with this function; the
        // bundle does not, and is what the assertions read.
        std::mem::forget(mock);
        bundle
    })
}

/// A capture with a threshold low enough that the gzip rule both fires and
/// declines.
///
/// Separate from [`bundle`] because a lower threshold is a *different tree*,
/// and a test that reused the default bundle could never see the rule fire.
fn gzipped() -> &'static PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let scratch = std::env::temp_dir().join(format!("gircap-gzip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch");
        let mock = Mock::start(World::five_classes());
        let out = scratch.join("artifacts");
        let (code, stdout, stderr) = run(&[
            "capture",
            "--out",
            &out.to_string_lossy(),
            "--hyp-base",
            &mock.hyp_base(),
            "--sophon-base",
            &mock.sophon_base(),
            // Small enough to split a 1.5 KB manifest from a 400 B meta file.
            "--gzip-threshold",
            "1000",
        ]);
        assert_eq!(code, 0, "{stdout}\n{stderr}");
        let b = find_bundle(&out);
        std::mem::forget(mock);
        b
    })
}

fn find_bundle(dir: &std::path::Path) -> PathBuf {
    let mut dirs = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if d.join("MANIFEST.json").is_file() {
            dirs.push(d);
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(&d) {
            let mut subs: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            subs.sort();
            stack.extend(subs.into_iter().rev());
        }
    }
    dirs.into_iter()
        .next()
        .unwrap_or_else(|| panic!("no bundle under {}", dir.display()))
}

fn json(path: &std::path::Path) -> serde_json::Value {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_slice(&raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn keys(v: &serde_json::Value) -> Vec<&str> {
    let mut k: Vec<&str> = v
        .as_object()
        .expect("an object")
        .keys()
        .map(|s| s.as_str())
        .collect();
    k.sort_unstable();
    k
}

// ------------------------------------------------------------------ the schema

/// Every generated document declares the same schema version, and it is the
/// crate's.
///
/// A reader that finds two versions in one bundle cannot tell whether the
/// difference is a version or a bug, and the only cheap defence is that there is
/// one number to compare.
#[test]
fn every_document_carries_the_crate_schema_version() {
    let b = bundle();
    let want = gircap::SCHEMA_VERSION;
    let mut checked = 0;
    for rel in [
        "MANIFEST.json",
        "chunks/index.json",
        "verify/checksums.json",
        "verify/selfcheck.json",
        "api/tag_probe.json",
        "manifests/7.0.0/game.manifest.meta.json",
    ] {
        let doc = json(&b.join(rel));
        assert_eq!(
            doc["schema_version"], want,
            "{rel} declares a different schema version"
        );
        checked += 1;
    }
    assert!(checked >= 6);
    // And the patchmaps, which are one document per capture.
    for name in std::fs::read_dir(b.join("patchmaps")).unwrap().flatten() {
        let doc = json(&name.path());
        assert_eq!(doc["schema_version"], want, "{}", name.path().display());
    }
}

/// `MANIFEST.json`'s top-level key set, pinned.
///
/// This is the document a downstream tool reads first, and a rename here is a
/// silent breaking change to everything that reads it.
#[test]
fn the_manifest_index_key_set_is_the_documented_one() {
    let doc = json(&bundle().join("MANIFEST.json"));
    assert_eq!(
        keys(&doc),
        vec![
            "audio",
            "biz",
            "capture",
            "field",
            "files",
            "gzip_threshold",
            "matrix",
            "schema_version",
            "selection_pool",
            "tool",
            "totals",
            "totals_by_set",
            "versions",
        ],
        "MANIFEST.json's top-level keys"
    );
    assert_eq!(keys(&doc["versions"]), vec!["diff_tags", "new", "old"]);
    assert_eq!(
        keys(&doc["capture"]),
        vec!["captured_at", "complete", "host_os", "id", "tool_argv"]
    );
}

/// The chunk index's key set, pinned. A blob entry is what `materialize` reads
/// back, so a rename here is a rename of the tool's own reconstruction input.
#[test]
fn the_chunk_index_key_set_is_the_documented_one() {
    let doc = json(&bundle().join("chunks/index.json"));
    assert_eq!(keys(&doc), vec!["blobs", "schema_version", "sets"]);
    let blob = &doc["blobs"][0];
    assert_eq!(
        keys(blob),
        vec![
            "bytes",
            "chunk_id",
            "container",
            "decode_error",
            "md5_compressed",
            "md5_decompressed",
            "referenced_by",
            "sets",
            "sha256_compressed",
            "stored_path",
            "uncompressed_size",
            "unknown_field6",
            "verified",
        ],
        "a blob entry's keys"
    );
    assert_eq!(
        keys(&blob["referenced_by"][0]),
        vec!["build_id", "capture_id", "offset", "path", "role", "tag"],
        "`build_id` is part of a chunk's address, so a reference records it"
    );
    assert_eq!(
        keys(&blob["verified"]),
        vec![
            "decoded_size_matches",
            "md5_compressed_matches",
            "md5_decompressed_matches",
            "raw_size_matches",
        ],
        "the four A6 checks, as one object per blob"
    );
}

// ------------------------------------------------------------------- the gzip rule

/// **The gzip rule fires on a `>threshold` body and not below.**
///
/// The interesting half is "not below": a rule that gzipped everything would pass
/// the first case and make every bundle 40% larger than it needs to be, and a
/// reader could not tell the difference without the document saying which
/// decision was made. So the rule is checked in both directions, and the
/// threshold is checked at its own boundary.
#[test]
fn the_gzip_rule_fires_above_the_threshold_and_not_below() {
    // The rule itself, at and around the boundary.
    for (len, want) in [
        (0_usize, false),
        (1, false),
        (999, false),
        (1000, false),
        (1001, true),
        (2000, true),
        (1_000_000, true),
    ] {
        assert_eq!(
            gircap::hash::should_gzip(len, 1000),
            want,
            "{len} bytes against a 1000-byte threshold"
        );
    }
}

/// And the decision the capture actually recorded, for a document on both sides.
///
/// The mock's documents are all small, so this needs a threshold low enough to
/// split them. That is the point of the flag: it exists precisely so a reader -
/// or a test - can choose where the boundary is, and a rule that can only ever
/// fire at 1 MiB is untestable without a 1 MiB fixture.
#[test]
fn a_capture_records_which_documents_it_gzipped_and_why() {
    let b = gzipped();
    // The small documents are stored plain and the large ones are `.gz`, and
    // `checksums.json` - the complete list - says which is which.
    let checksums = json(&b.join("verify/checksums.json"));
    let entries = checksums["entries"].as_array().unwrap();
    let encodings: Vec<&str> = entries
        .iter()
        .map(|e| e["encoding"].as_str().unwrap())
        .collect();
    assert!(
        encodings.contains(&"gzip"),
        "nothing was gzipped, so the rule never fired: {encodings:?}"
    );
    assert!(
        encodings.contains(&"identity"),
        "everything was gzipped, so the rule never declined either: {encodings:?}"
    );

    // Every decision agrees with the recorded threshold. The rule governs the
    // **full manifest decode** documents, and only those: `MANIFEST.json` and
    // `checksums.json` are small, are read by tooling that expects plain JSON,
    // and are named without a `.gz`. Scoping the assertion to the paths the rule
    // actually applies to is the difference between testing the rule and testing
    // an assumption about it.
    let threshold = json(&b.join("MANIFEST.json"))["gzip_threshold"]
        .as_u64()
        .expect("the threshold is recorded");
    let governed: Vec<&serde_json::Value> = entries
        .iter()
        .filter(|e| {
            e["path"].as_str().unwrap().ends_with(".manifest.json")
                || e["path"].as_str().unwrap().ends_with(".manifest.json.gz")
        })
        .collect();
    assert!(
        governed.len() >= 2,
        "the full decodes are what the rule governs, and there should be at least two: {}",
        governed.len()
    );
    for e in &governed {
        let path = e["path"].as_str().unwrap();
        let stored = std::fs::read(b.join(path)).expect("the stored body");
        let plain = if e["encoding"] == "gzip" {
            gircap::hash::gunzip(&stored).unwrap_or_else(|_| panic!("{path} is not gzip"))
        } else {
            stored
        };
        let over = plain.len() as u64 > threshold;
        assert_eq!(
            over,
            e["encoding"] == "gzip",
            "{path}: {len} bytes against a {threshold} threshold is recorded as {enc}",
            len = plain.len(),
            enc = e["encoding"]
        );
        // And a gzipped entry's digest is of the *decompressed* body, which is
        // the only way a reader can check it with a stock tool.
        if e["encoding"] == "gzip" {
            assert!(path.ends_with(".gz"), "{path} is gzipped but not named .gz");
            assert_eq!(
                gircap::hash::md5_hex(&plain),
                e["md5"].as_str().unwrap(),
                "the recorded digest is of the decompressed body, not the stored one"
            );
        }
    }
    // And a document the rule does not govern is stored plain whatever its size.
    let manifest = entries
        .iter()
        .find(|e| e["path"] == "MANIFEST.json")
        .expect("MANIFEST.json is listed");
    assert_eq!(manifest["encoding"], "identity");
}

/// The threshold itself is recorded, so a reader knows the rule the run used.
#[test]
fn the_gzip_threshold_is_recorded_in_the_manifest() {
    let doc = json(&bundle().join("MANIFEST.json"));
    assert_eq!(
        doc["gzip_threshold"].as_u64(),
        Some(1_048_576),
        "the threshold the run used, so a reader can tell why one document was gzipped \
         and the next was not"
    );
}

// ----------------------------------------------------------------- the checksums

/// `checksums.json` covers every file under the root and re-hashes to what it
/// recorded, which is A16 read back off disk.
#[test]
fn checksums_cover_every_file_and_re_hash_to_what_they_record() {
    let b = bundle();
    let raw = std::fs::read(b.join("verify/checksums.json")).unwrap();
    let c: gircap::emit::index::Checksums = serde_json::from_slice(&raw).unwrap();
    let (ok, detail) = c.a16(b);
    assert!(ok, "A16: {detail}");

    // And independently: the list is exactly the set of files on disk, minus
    // the list itself.
    let listed: Vec<String> = {
        let raw = std::fs::read(b.join("verify/checksums.json")).unwrap();
        let c: gircap::emit::index::Checksums = serde_json::from_slice(&raw).unwrap();
        c.entries.iter().map(|e| e.path.clone()).collect()
    };
    let mut on_disk = Vec::new();
    collect(b, &mut on_disk);
    let mut actual: Vec<String> = on_disk
        .iter()
        .map(|p| {
            p.strip_prefix(b)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .filter(|r| r != "verify/checksums.json")
        .collect();
    actual.sort();
    let mut expected = listed.clone();
    expected.sort();
    assert_eq!(actual, expected, "the list is not the set of files on disk");
}

/// The counts are self-consistent, so a reader can use either number.
#[test]
fn the_checksum_totals_agree_with_the_entries() {
    let c = json(&bundle().join("verify/checksums.json"));
    let entries = c["entries"].as_array().unwrap();
    assert_eq!(c["file_count"].as_u64().unwrap(), entries.len() as u64);
    let bytes: u64 = entries.iter().map(|e| e["bytes"].as_u64().unwrap()).sum();
    assert_eq!(c["total_bytes"].as_u64().unwrap(), bytes);
    assert_eq!(c["algorithm"], "sha256");
    // Sorted by path, so a diff between two bundles is readable.
    let paths: Vec<&str> = entries
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    let mut sorted = paths.clone();
    sorted.sort_unstable();
    assert_eq!(paths, sorted, "entries are not in path order");
}

fn collect(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
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

/// `MANIFEST.json → files[]` is *not* the complete list, and says so in the
/// document's own shape.
///
/// A document that hashes itself has no fixed point, so the file that cannot
/// list itself is the one file not in it. A reader who assumes otherwise will
/// report `README.md` as missing.
#[test]
fn the_manifest_file_list_is_a_prefix_of_the_complete_one() {
    let b = bundle();
    let manifest = json(&b.join("MANIFEST.json"));
    let raw = std::fs::read(b.join("verify/checksums.json")).unwrap();
    let checksums: gircap::emit::index::Checksums = serde_json::from_slice(&raw).unwrap();
    let complete: std::collections::BTreeSet<&str> =
        checksums.entries.iter().map(|e| e.path.as_str()).collect();
    let prefix: std::collections::BTreeSet<&str> = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(!prefix.is_empty(), "files[] is empty");
    for p in &prefix {
        assert!(
            complete.contains(*p),
            "{p} is in files[] but not in checksums"
        );
    }
    // And it is genuinely incomplete: it cannot contain itself, the README or
    // the checksums, because none of those existed when it was written.
    for absent in ["MANIFEST.json", "README.md", "verify/checksums.json"] {
        assert!(
            !prefix.contains(absent),
            "{absent} cannot be in files[]: it is written after the manifest"
        );
    }
    assert!(
        complete.len() > prefix.len(),
        "the complete list is longer than the prefix: {} vs {}",
        complete.len(),
        prefix.len()
    );
}

/// The `api/` bodies are stored exactly as served.
///
/// The mock renders them compactly, so "byte-exact" here means the file is the
/// server's own JSON - one line, no re-indenting. A capture that re-serialized
/// through a typed view would come out pretty-printed and this would catch it.
#[test]
fn the_api_bodies_are_byte_exact_and_the_generated_one_is_marked() {
    let b = bundle();
    let configs = std::fs::read_to_string(b.join("api/getGameConfigs.json")).unwrap();
    assert_eq!(
        configs.lines().count(),
        1,
        "the body was re-formatted; it should be exactly what the server sent:\n{configs}"
    );
    assert!(
        configs.starts_with(r#"{"retcode":0"#),
        "and it is the server's own key order:\n{configs}"
    );
    // The mock's JSON is compact with no spaces after separators, which is the
    // tell that nothing was re-rendered on the way to disk.
    assert!(
        !configs.contains(": "),
        "spaces after separators mean a re-render"
    );
    // `tag_probe.json` is this tool's measurement, not the API's words, and it
    // is the one document there that is re-rendered.
    let probe = json(&b.join("api/tag_probe.json"));
    assert_eq!(
        keys(&probe),
        vec!["requested", "results", "schema_version", "summary"],
        "tag_probe is the generated document"
    );
    assert_eq!(
        keys(&probe["results"][0]),
        vec![
            "already_fetched",
            "build_id",
            "http",
            "in_diff_tags",
            "retcode",
            "served",
            "tag",
            "tag_echoed",
        ]
    );
}

/// `api/requests.ndjson` records every request, one JSON object per line, in
/// sequence order.
#[test]
fn the_request_log_is_one_json_object_per_line_in_sequence_order() {
    let text = std::fs::read_to_string(bundle().join("api/requests.ndjson")).unwrap();
    let mut last = 0;
    let mut n = 0;
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("line {n} is not JSON: {e}\n{line}"));
        let seq = v["seq"].as_u64().expect("a seq");
        assert!(seq > last, "line {n}: seq {seq} does not follow {last}");
        last = seq;
        n += 1;
    }
    assert!(n >= 10, "only {n} requests recorded");
    // Every body the capture stored is reachable from the log, which is what
    // makes `stored_as` worth having.
    assert!(
        text.contains("stored_as"),
        "no request records where its bytes ended up"
    );
}
/// The one-sided captures carry a plan, totals and expectations - the shapes
/// `01-artifact-schema.md` §13 promises.
///
/// This exists because three documents state the shape and the writer is a
/// fourth: nothing checked that they agreed. `added` in particular must carry a
/// real `download_bytes` — not "unrecorded" but *false* — or a consumer comparing
/// against it loses to a run behaving perfectly.
///
/// Asserted on the emitted JSON rather than on the struct: the struct cannot
/// represent a missing plan, so a struct-level assertion would pass whatever the
/// writer wrote.
#[test]
fn a_one_sided_capture_carries_the_plan_and_totals_the_schema_promises() {
    let b = bundle();
    let added = json(&b.join("patchmaps/added.json"));

    // §13: "for `added` there is no `old`, and every `plan` entry is `FETCH`
    // with `source_range: null`".
    assert!(added["old"].is_null(), "`added` has no old side");
    let plan = added["plan"]
        .as_array()
        .expect("`added` has a plan: every chunk is fetched whole");
    assert!(!plan.is_empty(), "a plan with no entries is not a plan");
    let nchunks = added["new"]["nchunks"]
        .as_u64()
        .expect("declared chunk count");
    assert_eq!(
        plan.len() as u64,
        nchunks,
        "one plan entry per new-side chunk, or the totals are not the plan's"
    );
    let mut fetched = 0_u64;
    let mut bytes = 0_i64;
    for e in plan {
        assert_eq!(e["decision"], "FETCH", "an `added` chunk cannot be reused");
        assert!(
            e["source_range"].is_null(),
            "there is no old side to slice from"
        );
        assert!(!e["chunk_id"].as_str().unwrap_or_default().is_empty());
        assert!(
            e["note"].is_string(),
            "a reader seeing no `source_range` should be told why, not left guessing"
        );
        fetched += 1;
        bytes += e["compressed_size"]
            .as_i64()
            .expect("declared compressed size");
    }

    // The arithmetic, re-done by this test from the plan rather than trusted.
    let t = added["totals"].as_object().expect("`added` has totals");
    assert_eq!(t["fetch"].as_u64(), Some(fetched));
    assert_eq!(t["reuse"].as_u64(), Some(0));
    assert_eq!(t["moved"].as_u64(), Some(0));
    assert_eq!(t["chunks"].as_u64(), Some(fetched));
    assert_eq!(t["download_bytes"].as_i64(), Some(bytes));
    assert!(
        bytes > 0,
        "a file that must be downloaded whole is not free; a zero here is the defect"
    );

    // And the expectation points at that same real number, not a default.
    assert_eq!(
        added["expectations"]["download_bytes"].as_i64(),
        Some(bytes)
    );
    assert_eq!(added["expectations"]["absent_after"], false);
    assert!(added["expectations"]["result_path"].is_string());
}

#[test]
fn a_removed_capture_carries_an_expectation_a_consumer_can_assert() {
    let b = bundle();
    let removed = json(&b.join("patchmaps/removed.json"));

    assert!(removed["new"].is_null(), "`removed` has no new side");
    assert!(removed["plan"].is_null(), "and so no target to plan for");
    assert!(removed["totals"].is_null(), "and so nothing to cost");

    // `removed` carries `expectations` too: a consumer needs something to assert
    // for the one capture whose whole point is a deletion.
    let e = removed["expectations"]
        .as_object()
        .expect("`removed` has expectations");
    assert_eq!(e["absent_after"], true, "the outcome is deletion");
    assert!(e["result_path"].is_null(), "there is nothing to rebuild");
    assert!(e["result_md5"].is_null());
    assert_eq!(
        e["download_bytes"].as_i64(),
        Some(0),
        "a true zero: a removal costs nothing to perform"
    );
    assert_eq!(
        e["must_not_fetch"].as_array().map(Vec::len),
        Some(0),
        "nothing to fetch, so nothing is forbidden"
    );
}

#[test]
fn every_capture_carries_expectations_so_none_is_assert_against_nothing() {
    let b = bundle();
    let mut checked = 0;
    for entry in std::fs::read_dir(b.join("patchmaps")).expect("patchmaps/") {
        let p = entry.expect("entry").path();
        let m = json(&p);
        assert!(
            m["expectations"].is_object(),
            "{}: a patchmap a consumer cannot assert anything about is the defect",
            p.display()
        );
        // `download_bytes` is always a number, never null and never missing.
        // An absent figure has to be absent; `0` is a value a consumer will
        // compare against.
        assert!(
            m["expectations"]["download_bytes"].is_i64(),
            "{}: `download_bytes` must be stated, not left implicit",
            p.display()
        );
        // Whichever way the outcome goes, it is stated rather than inferred.
        let e = &m["expectations"];
        assert!(
            e["absent_after"].is_boolean(),
            "{}: the outcome is not stated",
            p.display()
        );
        if e["absent_after"] == serde_json::Value::Bool(false) {
            assert!(
                e["result_path"].is_string(),
                "{}: a file that survives the run needs a result path",
                p.display()
            );
        } else {
            assert!(
                e["result_path"].is_null(),
                "{}: a file that does not survive has no result path",
                p.display()
            );
        }
        checked += 1;
    }
    assert!(
        checked >= 5,
        "expected the mock's five classes, saw {checked}"
    );
}
