//! The bundle shapes that `capture_layout` cannot state without a separate
//! world: what a metadata-only run actually writes (H4), what a manifest that
//! lies about its arithmetic produces (K1), and what happens to a chunk the CDN
//! does not serve as a zstd frame (K2).
//!
//! These are integration tests because the claims are about files. An
//! in-process `CapturePlan` would not prove that `chunks/index.json` still names
//! the declared bytes after `--no-chunk-blobs` skipped every fetch, and a unit
//! test of `BlobProps` would prove the row shape without proving that a capture
//! wrote it to disk.
//!
//! **Every test here is ignored.** Each spawns `gircap capture` against the full
//! mock world; K2 also fetches the complete chunk store before the assembly
//! failure. That is seconds per test, not milliseconds, and the default suite
//! should not pay for it. Run them with:
//!
//! `cargo nextest run -p gircap --test bundle_shapes --run-ignored only`
//!
//! **H4 asks for two things the implementation does not do.** A
//! `--no-chunk-blobs` run skips fixture assembly, so it writes no `expected/`
//! tree at all; and `verify` deliberately treats every row with no
//! `stored_path` as a red A6 row, so it exits `3` rather than succeeding. Those
//! are the actual, pinned shapes below; the divergence is reported with the
//! fixture rather than hidden by asserting the requirement.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{File, Mock, World, blob};
use gircap::report::percent_decode;
use girpr_tempdir::TempDir;
use serde_json::Value;

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
struct Scratch {
    root: TempDir,
}

impl Scratch {
    fn new(name: &str) -> Scratch {
        Scratch {
            root: TempDir::new("gircap-bundle-shapes", name),
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
    /// The `SUMMARY` line, parsed into key=value pairs. The summary percent-encodes
    /// whitespace and `%`, so the values are decoded back.
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

/// The one directory under `out` holding `marker`, whichever way the layout
/// nests it.
///
/// K2 fails before `MANIFEST.json` is written, so the caller names the document
/// that proves a bundle started (`chunks/index.json`) rather than assuming the
/// closing document exists.
fn bundle_with(out: &Path, marker: &str) -> PathBuf {
    let mut found = Vec::new();
    walk(out, &mut found);
    found
        .into_iter()
        .find(|p| p.join(marker).is_file())
        .unwrap_or_else(|| panic!("no bundle with {marker} under {}", out.display()))
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

fn read_json(path: &Path) -> Value {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_slice(&raw)
        .unwrap_or_else(|e| panic!("parsing {} as JSON: {e}", path.display()))
}

fn index_rows(bundle: &Path) -> Vec<Value> {
    read_json(&bundle.join("chunks/index.json"))["blobs"]
        .as_array()
        .expect("chunks/index.json has a blobs array")
        .clone()
}

/// Blob files under `chunks/blobs/`, or zero when the directory does not exist.
fn blob_files(bundle: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(bundle.join("chunks/blobs")) else {
        return 0;
    };
    rd.flatten().filter(|e| e.path().is_file()).count()
}

/// The on-disk predicate `BlobProps::is_usable` applies to an index row.
///
/// Mirrors `src/blobs/props.rs`: all four A6 checks green, no decode, fetch or
/// measurement error, not partial, and a stored path. A test that only checked
/// one of those would pass on a row the tool itself considers unusable.
fn row_is_usable(row: &Value) -> bool {
    let verified = &row["verified"];
    let all_green = [
        "raw_size_matches",
        "md5_compressed_matches",
        "md5_decompressed_matches",
        "decoded_size_matches",
    ]
    .iter()
    .all(|k| verified[*k] == Value::Bool(true));
    all_green
        && row["decode_error"].is_null()
        && row["fetch_error"].is_null()
        && row["partial"] == Value::Bool(false)
        && row["measurement_error"].is_null()
        && row["stored_path"].is_string()
}

// ----------------------------------------------------------------------- H4

/// **H4.** A `--no-chunk-blobs` bundle is an index of declarations, not a
/// partial fixture: no bytes, no blob files, and every row says "not measured".
///
/// The cost is a full capture against the mock. The point is what is on disk
/// afterwards, not what the plan predicted: the index is written by the same
/// code that writes a measured one, and `md5_decompressed`/`uncompressed_size`
/// must still name the manifest's declaration so a reader can see exactly which
/// bytes are missing.
///
/// Two H4 claims are contradicted by the implementation and pinned as it is:
///
/// * `expected/` is written per assembled capture, and `--no-chunk-blobs` skips
///   assembly, so there is no `expected/` tree at all.
/// * `verify` reports every pathless row as a failing A6 row (`verify::check_blobs`),
///   so it exits `3` and prints `result=fail`; it does not succeed. That is
///   deliberate - a bundle without blobs cannot be materialized - but it is the
///   opposite of H4's wording.
#[ignore = "runs a full capture process and builds its complete chunk index"]
#[test]
fn no_chunk_blobs_bundle_has_no_bytes_and_records_the_declarations() {
    let world = World::all_shapes();
    let declared = world.declared();
    let mock = Mock::start(world);
    let scratch = Scratch::new("h4");
    let r = capture(&mock, &scratch, &["--no-chunk-blobs"]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let bundle = bundle_with(&scratch.out(), "MANIFEST.json");

    // A skip is not a pass, and a skip is not a failure.
    let doc = read_json(&bundle.join("verify/selfcheck.json"));
    assert_eq!(doc["result"], "incomplete");
    assert_eq!(doc["counts"]["failed"], 0, "a skip is not a failure");

    // The index survives the flag; it is how a reader tells "nothing was
    // asked for" from "the capture had nothing to fetch".
    let rows = index_rows(&bundle);
    assert!(
        !rows.is_empty(),
        "the captures were admitted, so their chunks have index rows"
    );
    for row in &rows {
        let id = row["chunk_id"].as_str().expect("a chunk id");
        assert!(
            row["stored_path"].is_null(),
            "{id}: --no-chunk-blobs stores no path"
        );
        assert!(
            row["verified"].is_null(),
            "{id}: nothing was measured, so `verified` is null, not all-false"
        );
        assert_eq!(row["bytes"], 0, "{id}: no bytes arrived");
        assert_eq!(
            row["md5_compressed"], "",
            "{id}: an absent digest is empty, not zeroed"
        );
        assert_eq!(row["sha256_compressed"], "", "{id}: same for SHA-256");
        assert_eq!(row["container"], "zstd");

        let (md5, size) = declared
            .get(id)
            .unwrap_or_else(|| panic!("{id} is not declared by the world's manifests"));
        assert_eq!(
            row["md5_decompressed"].as_str(),
            Some(md5.as_str()),
            "{id}: the row carries the manifest's declared digest"
        );
        assert_eq!(
            row["uncompressed_size"].as_u64(),
            Some(*size),
            "{id}: and its declared length, clamped exactly as the decoder does"
        );
    }

    // No blob directory, or an empty one: either way no file was published.
    assert_eq!(blob_files(&bundle), 0, "no blob file may exist");
    assert!(
        !bundle.join("chunks/blobs").is_dir(),
        "the flag skips the fetch, so the blob directory is never created"
    );

    // The prelude and the closing documents are still there. `expected/` is
    // not, because assembly never ran; see the doc comment above.
    assert!(bundle.join("game").is_dir(), "launcher metadata is written");
    assert!(bundle.join("MANIFEST.json").is_file());
    assert!(bundle.join("diagnostics/path_map.json").is_file());
    assert!(
        bundle.join("diagnostics/rejections.json").is_file(),
        "an empty rejection document is still written"
    );
    assert!(
        !bundle.join("expected").exists(),
        "H4 asks for expected/; --no-chunk-blobs skips assembly, which is the \
         only writer of that tree. Pinning the actual shape."
    );

    // And the bundle is not materializable. H4 asks for a passing verify; the
    // implementation's answer is the one a reader needs (exit 3, result fail).
    let v = run(&["verify", "--artifact", &bundle.to_string_lossy()]);
    assert_eq!(
        v.code, 3,
        "H4 asks for success, but a pathless index row is a red A6 row:\n{}\n{}",
        v.stdout, v.stderr
    );
    assert_eq!(v.summary()["result"], "fail");
}

// ----------------------------------------------------------------------- K1

/// The K1 world: ordinary shapes plus files with hostile declarations.
///
/// Three files declare a chunk at `i64::MAX`, so the aggregate saturates at
/// `u64::MAX`. A fourth keeps an ordinary 128-byte chunk but declares a
/// **negative file size**, which is the one hostile value the manifest decoder
/// accepts: `SophonChunk::validate_arithmetic` refuses a negative chunk size
/// before the capture can run (see `proto.rs`), while `decode_counts` is the
/// function that clamps a negative file size out of the totals. The bytes stay
/// ordinary; only the manifest's arithmetic is hostile.
struct HostileK1 {
    huge: Vec<common::Blob>,
    negative_file: common::Blob,
}

fn k1_world() -> (World, HostileK1) {
    let mut world = World::all_shapes();
    let mut huge = Vec::new();
    let game = &mut world
        .new
        .fields
        .iter_mut()
        .find(|(field, _)| field == "game")
        .expect("the all-shapes world publishes game")
        .1;
    for i in 0..3 {
        let b = blob(900 + i as u64, 128).declaring_uncompressed_size(i64::MAX);
        huge.push(b.clone());
        game.push(File {
            path: format!("GenshinImpact_Data/StreamingAssets/blocks/k1_huge_{i}.blk"),
            chunks: vec![b],
            declared_size: None,
        });
    }
    let negative_file = blob(903, 128);
    game.push(
        File {
            path: "GenshinImpact_Data/StreamingAssets/blocks/k1_negative_file.blk".into(),
            chunks: vec![negative_file.clone()],
            declared_size: None,
        }
        .declaring_size(-1),
    );
    (
        world,
        HostileK1 {
            huge,
            negative_file,
        },
    )
}

/// **K1.** A manifest may declare a chunk or file size that is negative or
/// larger than any real file. The capture must still run, and the bundle must
/// record the value the decoder actually uses - not the raw signed number, and
/// not a wrapping cast.
///
/// The decoder refuses a negative **chunk** size at its arithmetic boundary, so
/// the negative case here is the file's `size`: 128 real bytes declared as `-1`.
/// `decode_counts` clamps that file's contribution with `.max(0)` and still
/// records the chunk/size disagreement as a structural violation; the totals
/// saturate at `u64::MAX`. Three `i64::MAX` chunks make the saturation visible.
/// With `--no-chunk-blobs` no hostile-size body is fetched, so this is about
/// arithmetic rather than about zstd.
///
/// `--per-class 5` admits every forced added candidate; the default of one per
/// class would keep the expensive ones out of the index and make the test pass
/// without looking at them. The cost is a full capture, hence `#[ignore]`.
#[ignore = "runs a full capture process and builds its complete chunk index"]
#[test]
fn hostile_declared_sizes_are_recorded_clamped_in_the_index() {
    let (world, hostile) = k1_world();
    let declared = world.declared();
    let mock = Mock::start(world);
    let scratch = Scratch::new("k1");
    let r = capture(&mock, &scratch, &["--no-chunk-blobs", "--per-class", "5"]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let bundle = bundle_with(&scratch.out(), "MANIFEST.json");

    // A5 passed, or the capture would have stopped before writing a bundle.
    // This is the evidence that the declared total and the decoded total are
    // the same saturated number.
    let doc = read_json(&bundle.join("verify/selfcheck.json"));
    assert_eq!(doc["counts"]["failed"], 0);
    assert_eq!(doc["result"], "incomplete");

    let rows = index_rows(&bundle);
    let row = |id: &str| {
        rows.iter()
            .find(|r| r["chunk_id"] == id)
            .unwrap_or_else(|| panic!("{id} was not admitted into the index"))
    };
    for b in &hostile.huge {
        let r = row(&b.id);
        let (md5, _) = declared
            .get(&b.id)
            .unwrap_or_else(|| panic!("{} is not declared", b.id));
        assert_eq!(
            r["md5_decompressed"].as_str(),
            Some(md5.as_str()),
            "{}: the declaration survives untouched",
            b.id
        );
        assert_eq!(
            r["uncompressed_size"],
            i64::MAX as u64,
            "{}: a positive extreme is recorded as the i64 it decoded to",
            b.id
        );
        assert!(r["stored_path"].is_null());
        assert!(r["verified"].is_null());
        assert_eq!(r["bytes"], 0);
    }

    // The negative declaration is a *file* size, not a chunk size. The chunk
    // row still names its real 128 bytes; the clamp lives in the totals.
    let (md5, chunk_size) = declared
        .get(&hostile.negative_file.id)
        .unwrap_or_else(|| panic!("{} is not declared", hostile.negative_file.id));
    let negative = row(&hostile.negative_file.id);
    assert_eq!(negative["md5_decompressed"].as_str(), Some(md5.as_str()));
    assert_eq!(negative["uncompressed_size"].as_u64(), Some(*chunk_size));
    assert_eq!(
        *chunk_size, 128,
        "the chunk is ordinary: only the file lies"
    );

    // The manifest metadata is where the totals live. The declared `stats`
    // saturated, the decode saturated to the same number, and the negative file
    // became zero bytes rather than 2^64-1.
    let meta = read_json(&bundle.join("manifests/7.1.0/game.manifest.meta.json"));
    let saturated = u64::MAX.to_string();
    assert_eq!(
        meta["declared"]["stats"]["uncompressed_size"], saturated,
        "the mock's declared total saturated"
    );
    assert_eq!(
        meta["decoded"]["total_file_bytes"],
        u64::MAX,
        "and the decode's total is the same saturated u64"
    );
    assert_eq!(
        meta["decoded"]["structure"]["size_mismatch_files"], 1,
        "the negative file is clamped out of the totals, not silently rewritten"
    );
}

// ----------------------------------------------------------------------- K2

/// **K2.** A chunk the CDN serves as something that is not a zstd frame is kept,
/// not turned into a failed fetch: the row names the file, says why it will not
/// decode, and the raw bytes stay on disk.
///
/// The mock changes only the served bytes; the manifest's compressed size and
/// MD5 are still the ones derived from the real frame. That is exactly a
/// mis-served response, and `BlobProps::measure_file` records it as `decode_error`
/// with the two decode-dependent A6 checks false.
///
/// Capture exits **3**, not 0: the bytes are indexed first (that is the evidence
/// K2 asks to keep), then `emit::fixture::build` tries to assemble the admitted
/// capture and aborts on the unusable chunk. The bundle therefore has no
/// `MANIFEST.json`, which is why the locator looks for `chunks/index.json`.
#[ignore = "runs a full capture, fetches every chunk, then fails assembly"]
#[test]
fn a_mis_served_chunk_keeps_its_bytes_and_fails_assembly() {
    let xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error>\n  \
                <Code>NoSuchKey</Code>\n  <Message>The specified key does not \
                exist.</Message>\n</Error>\n"
        .to_vec();

    // The one `added` file is the class the default selection must capture, so
    // this chunk is certain to be fetched.
    let mut world = World::all_shapes();
    let chunk = world
        .new
        .field("game")
        .expect("the world publishes game")
        .iter()
        .find(|f| f.path.ends_with("/libNew.dll"))
        .expect("the all-shapes world has the new-only file")
        .chunks[0]
        .clone();
    assert!(
        world.mis_serve(&chunk.id, xml.clone()),
        "the chunk id must exist in a build"
    );

    let mock = Mock::start(world);
    let scratch = Scratch::new("k2");
    let r = capture(&mock, &scratch, &[]);
    assert_eq!(
        r.code, 3,
        "assembly fails on the unusable chunk; the indexed bytes are still kept:\n{}\n{}",
        r.stdout, r.stderr
    );
    let bundle = bundle_with(&scratch.out(), "chunks/index.json");
    assert!(
        !bundle.join("MANIFEST.json").exists(),
        "the failure happens before close-out, so there is no closing document"
    );

    let rows = index_rows(&bundle);
    let row = rows
        .iter()
        .find(|r| r["chunk_id"] == chunk.id)
        .unwrap_or_else(|| panic!("{} was fetched but not indexed", chunk.id));

    assert_eq!(
        row["stored_path"].as_str(),
        Some(gircap::blobs::props::BlobProps::stored_path_for(&chunk.id).as_str()),
        "the row keeps the path of the bytes it published"
    );
    assert_eq!(
        row["bytes"].as_u64(),
        Some(xml.len() as u64),
        "the row counts the bytes that arrived, not zero"
    );
    let why = row["decode_error"]
        .as_str()
        .expect("an unframed body is a recorded decode error");
    assert!(
        why.contains("not a zstd frame"),
        "the row says why it will not decode: {why}"
    );
    assert_eq!(row["fetch_error"], Value::Null);
    assert_eq!(row["partial"], Value::Bool(false));
    assert_eq!(row["measurement_error"], Value::Null);
    assert!(
        !row_is_usable(row),
        "bytes that will not decode are evidence, never usable input"
    );
    assert_eq!(
        row["verified"]["md5_decompressed_matches"],
        Value::Bool(false)
    );
    assert_eq!(row["verified"]["decoded_size_matches"], Value::Bool(false));

    let path = row["stored_path"].as_str().expect("a path");
    let on_disk = std::fs::read(bundle.join(path)).expect("the indexed bytes exist on disk");
    assert_eq!(
        on_disk, xml,
        "the file holds exactly what the CDN served, mis-served or not"
    );
}
