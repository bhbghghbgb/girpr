//! The bundle's own consistency: the matrix, the patchmaps, and the discovery
//! that finds them.
//!
//! **These are `gircap`'s contract, not the repair tool's.** The matrix says
//! which patch shapes the bundle covers and the patchmaps say what each of
//! them is; a disagreement in either direction means the bundle's own records
//! do not describe each other, and every consumer of it is standing on sand.
//!
//! They live here rather than in the repair tool's suite because the bundle
//! is `gircap`'s output and the checks need nothing but the bundle — which is
//! why they cost nothing: the bundle is built by the tool's own writers, so
//! there is no fixture to copy.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Mock, World};
use girpr_tempdir::TempDir;

/// A scratch root that is removed when the test ends, whatever happens.
struct Scratch {
    root: TempDir,
}

impl Scratch {
    fn new(name: &str) -> Scratch {
        Scratch {
            root: TempDir::new("gircap-consistency", name),
        }
    }

    fn out(&self) -> PathBuf {
        self.root.join("artifacts")
    }
}

/// The binary under test, beside this test's own executable.
fn bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("the test binary has a path");
    p.pop(); // deps/
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("gircap.exe")
}

/// One finished command.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
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
fn capture_ok(name: &str) -> (Scratch, PathBuf) {
    let mock = Mock::start(World::all_shapes());
    let scratch = Scratch::new(name);
    let r = run(&[
        "capture",
        "--out",
        &scratch.out().to_string_lossy(),
        "--hyp-base",
        &mock.hyp_base(),
        "--sophon-base",
        &mock.sophon_base(),
    ]);
    assert_eq!(r.code, 0, "capture failed:\n{}\n{}", r.stdout, r.stderr);
    let bundle = bundle_in(&scratch.out());
    (scratch, bundle)
}

fn json(path: &Path) -> serde_json::Value {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_slice(&raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The matrix rows of a bundle.
fn matrix(bundle: &Path) -> Vec<serde_json::Value> {
    json(&bundle.join("MANIFEST.json"))["matrix"]
        .as_array()
        .expect("a matrix")
        .clone()
}

/// The patchmaps of a bundle, keyed by the capture id each declares.
fn patchmaps(bundle: &Path) -> Vec<(String, serde_json::Value)> {
    let dir = bundle.join("patchmaps");
    let mut names: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the patchmaps directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|p| {
            let v = json(&p);
            let id = v["capture_id"].as_str().expect("a capture id").to_string();
            (id, v)
        })
        .collect()
}

/// **The matrix and the patchmaps agree, in both directions.**
///
/// `MANIFEST.json` records one row per class: `captured`, `covered_by`,
/// `over_budget` or `unavailable`. The patchmap set is the same fact on disk.
/// A disagreement in either direction means the bundle's own records do not
/// describe each other.
///
/// **The `covered_by` row is the one that needs care.** A class the matrix
/// records as `covered_by` has **no capture of its own**: the file that shows
/// it is another capture's, and the row names that capture's id. So the row's
/// `capture_id` must be a capture that *does* have a patchmap, and the class
/// itself must not have one.
#[test]
fn the_matrix_and_the_patchmaps_agree_in_both_directions() {
    let (_scratch, bundle) = capture_ok("matrix-agrees");
    let rows = matrix(&bundle);
    let maps = patchmaps(&bundle);

    for m in &rows {
        match m["status"].as_str() {
            Some("captured") => {
                // The row names its **own** capture, and that capture is on disk.
                assert_eq!(
                    m["capture_id"].as_str(),
                    m["class"].as_str(),
                    "a captured row names its own capture: {m}"
                );
                assert!(
                    maps.iter()
                        .any(|(id, _)| *id == m["class"].as_str().unwrap()),
                    "recorded captured, but there is no patchmaps/{}.json: {m}",
                    m["class"].as_str().unwrap()
                );
            }
            Some("covered_by") => {
                // The row names **another** capture's id, that capture is on
                // disk, and the class has no capture of its own.
                let id = m["capture_id"]
                    .as_str()
                    .unwrap_or_else(|| panic!("a covered_by row names no capture: {m}"));
                assert_ne!(
                    id,
                    m["class"].as_str().unwrap_or_default(),
                    "a covered_by row must not name its own class: {m}"
                );
                assert!(
                    maps.iter().any(|(have, _)| have == id),
                    "covered by {id}, which has no patchmap: {m}"
                );
                assert!(
                    !maps
                        .iter()
                        .any(|(have, _)| have == m["class"].as_str().unwrap()),
                    "recorded covered by another capture, yet the bundle carries a \
                     patchmap for it: {m}"
                );
            }
            Some(other) => {
                // Not answered: no capture, and a reason.
                assert!(
                    !m["reason"].as_str().unwrap_or_default().is_empty(),
                    "recorded {other} but gives no reason: {m}"
                );
                assert!(
                    m["capture_id"].is_null(),
                    "recorded {other} yet names a capture: {m}"
                );
                assert!(
                    !maps
                        .iter()
                        .any(|(have, _)| have == m["class"].as_str().unwrap()),
                    "recorded {other}, yet the bundle carries a patchmap for it: {m}"
                );
            }
            None => panic!("a matrix row has no status: {m}"),
        }
    }
    // ...and nothing on disk that the matrix does not mention.
    for (id, _) in &maps {
        assert!(
            rows.iter().any(|m| m["class"] == id.as_str()),
            "a patchmap with no matrix entry: {id}"
        );
    }
    let captured = rows.iter().filter(|m| m["status"] == "captured").count();
    assert_eq!(
        captured,
        maps.len(),
        "the two counts are the same fact stated twice"
    );
}

/// **The matrix loads with distinct paths**, and the fixture follows it.
///
/// A repeated path would make a second capture silently skip and report
/// `download_bytes=0` against a non-zero expectation, which is a baffling
/// failure to diagnose from the number that is wrong.
#[test]
fn the_capture_matrix_loads_with_distinct_paths() {
    let (_scratch, bundle) = capture_ok("matrix-paths");
    let rows = matrix(&bundle);
    let maps = patchmaps(&bundle);

    let mut paths: Vec<&str> = rows.iter().filter_map(|m| m["path"].as_str()).collect();
    let before = paths.len();
    paths.sort_unstable();
    paths.dedup();
    assert_eq!(
        paths.len(),
        before,
        "capture paths must be pairwise distinct: {paths:?}"
    );

    for (id, m) in &maps {
        let classes = m["classes"].as_array().expect("classes[]");
        assert!(!classes.is_empty(), "{id}: classes[]");
        // `added` is the one shape whose file is *absent* from `game/` by
        // construction — it is the file the new version introduces — so the
        // fixture must not carry it, or the run would find nothing to plan.
        //
        // Only the **game** field is checked: the audio field's files are not
        // part of the `game/` fixture at all, so their absence says nothing.
        let on_disk = bundle
            .join("game")
            .join(m["path"].as_str().unwrap())
            .is_file();
        let is_added = classes.iter().any(|c| c == "added");
        if m["matching_field"] == "game" {
            assert_eq!(
                on_disk, !is_added,
                "{id}: game/ presence must follow the class"
            );
        }
        // And the patchmap's own arithmetic must be readable: a capture with
        // no plan has nothing to compare against, and a capture with a plan
        // and no totals has an unrecordable cost.
        let has_new = m["new"].is_object();
        assert_eq!(
            has_new,
            m["plan"].is_array(),
            "{id}: a plan and a new side are the same fact"
        );
        assert_eq!(
            has_new,
            m["totals"].is_object(),
            "{id}: totals and a new side are the same fact"
        );
    }
}

/// **Discovery agrees with itself**: `gircap list` and a plain directory scan
/// must name the same bundles, in the same order.
///
/// `gircap list` is the primitive that exists so a consumer need not be
/// ad-hoc about finding a bundle. If the two ever disagreed, a bundle could
/// be found by one and missed by the other, and which one ran would depend on
/// whether a sibling workspace member happened to be compiled.
#[test]
fn discovery_agrees_whether_it_asks_gircap_or_scans() {
    let (scratch, bundle) = capture_ok("discovery");
    let root = scratch.out();

    let v = run(&[
        "list",
        "--root",
        &root.to_string_lossy(),
        "--output",
        "json",
    ]);
    assert_eq!(v.code, 0, "{}\n{}", v.stdout, v.stderr);

    let listed: Vec<String> = v
        .stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|r| r["event"] == "bundle")
        .map(|r| r["path"].as_str().unwrap_or_default().to_string())
        .collect();

    let scanned = scan_for_bundles(&root);

    assert_eq!(
        listed.len(),
        scanned.len(),
        "`gircap list` and the scan must name the same number of bundles"
    );
    for (l, s) in listed.iter().zip(scanned.iter()) {
        assert_eq!(
            Path::new(l).file_name(),
            s.file_name(),
            "`gircap list` and the scan must name the same bundle"
        );
    }
    assert_eq!(
        Path::new(&listed[0]).file_name(),
        bundle.file_name(),
        "and it is the bundle this test captured"
    );
}

/// Every directory under `root` that holds a `MANIFEST.json`, sorted.
fn scan_for_bundles(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            if p.join("MANIFEST.json").is_file() {
                out.push(p);
            } else {
                stack.push(p);
            }
        }
    }
    out.sort();
    out
}
