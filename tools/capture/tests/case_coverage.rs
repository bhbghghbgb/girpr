//! The instrument, gated against the artifact it measures.
//!
//! `tools/case_coverage.py` is the instrument behind `docs/00-spec.md` §4.1's
//! published populations, and it is supposed to **reproduce the crate's rules
//! exactly**. Nothing checked that, so it drifted: it counted a *kept* chunk
//! toward the two salvage counts — the artifact counts only the damaged ones,
//! exactly as `repair::reuse::salvage_order` does — and it always named a
//! write direction, where the artifact names `n/a` for the tie. On the
//! reference bundle it disagreed with the artifact on six of the nine
//! captures.
//!
//! This is the gate that would have caught it: the script is run against a
//! bundle the **crate** wrote, and its numbers are compared with the numbers
//! the crate wrote into that same bundle. Two comparisons:
//!
//! | what | against what |
//! |---|---|
//! | the script's outcome table | the patchmaps' `repair` blocks |
//! | the script's population | the matrix's `candidates_considered` |
//!
//! The second one is the class drift: the crate's class set, the instrument's
//! and the published table are three statements of the same fact, and this is
//! what says they agree.
//!
//! **Python is optional.** The gate needs it, and a developer without it gets
//! a note rather than a failure — but the note says what was *not* checked, so
//! a skipped gate is never mistaken for a green one.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Mock, World, tempdir::TempDir};

/// The instrument, beside the crate it measures.
fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tools")
        .join("case_coverage.py")
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

/// A python that runs.
fn python() -> Option<&'static str> {
    for name in ["python", "python3"] {
        if Command::new(name)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return Some(if name == "python" { "python" } else { "python3" });
        }
    }
    None
}

/// A scratch root that is removed when the test ends, whatever happens.
struct Scratch {
    root: TempDir,
}

impl Scratch {
    fn new(name: &str) -> Scratch {
        Scratch {
            root: TempDir::new("gircap-case-coverage", name),
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

fn run(args: &[&str]) -> Run {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("the capture runs");
    Run {
        code: out.status.code().expect("the capture exits normally"),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// The one bundle under a scratch root, whichever way the layout nests it.
fn bundle_in(out: &Path, r: &Run) -> PathBuf {
    let mut found = Vec::new();
    walk(out, &mut found);
    found
        .into_iter()
        .find(|p| p.join("MANIFEST.json").is_file())
        .unwrap_or_else(|| {
            panic!(
                "no bundle under {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
                out.display(),
                r.stdout,
                r.stderr
            )
        })
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

/// The script's own JSON, for one section.
fn instrument_json(py: &str, bundle: &Path, section: &str) -> serde_json::Value {
    let out = Command::new(py)
        .arg(script())
        .arg(bundle)
        .arg("--section")
        .arg(section)
        .arg("--json")
        .output()
        .expect("the instrument runs");
    assert!(
        out.status.success(),
        "the instrument failed on {section}:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("the instrument's JSON parses")
}

/// The artifact's `repair` block for every capture that has one.
fn repair_blocks(bundle: &Path) -> Vec<(String, serde_json::Value)> {
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
            let v: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&p).expect("the patchmap")).expect("json");
            let id = v["capture_id"].as_str().expect("a capture id").to_string();
            (id, v["repair"].clone())
        })
        .collect()
}

/// **The instrument reproduces the artifact's `repair` block.**
///
/// The block is what the crate says the in-place repair would do with the
/// file: how many chunks are kept, copied and fetched, how much RAM the
/// staged tier needs, how many sources each write order salvages, which order
/// it picks, when the dead tail is released, and how much the file grows.
///
/// The instrument answers the same question by hashing the fixture's bytes,
/// and it has to answer it the same way — otherwise the published populations
/// and the artifact are describing two different repairs.
#[test]
fn the_instrument_reproduces_the_artifacts_repair_block() {
    let Some(py) = python() else {
        println!(
            "note: no python on this machine, so the instrument was not checked against \
             the artifact; the gate is skipped, not passed"
        );
        return;
    };
    let mock = Mock::start(World::five_classes());
    let scratch = Scratch::new("outcome");
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
    let bundle = bundle_in(&scratch.out(), &r);

    let doc = instrument_json(py, &bundle, "outcome");
    let rows = doc["outcome"].as_array().expect("an outcome array");

    let mut checked = 0;
    for (id, repair) in repair_blocks(&bundle) {
        if repair.is_null() {
            // `removed`: there is nothing to repair, so there is no block.
            continue;
        }
        let row = rows
            .iter()
            .find(|r| r["capture_id"] == id.as_str())
            .unwrap_or_else(|| panic!("{id}: the instrument has no row for it"));
        for (field, ours, theirs) in [
            ("kept", row["kept"].as_u64(), repair["kept"].as_u64()),
            ("moved", row["moved"].as_u64(), repair["moved"].as_u64()),
            ("fetched", row["fetched"].as_u64(), repair["fetch"].as_u64()),
            (
                "payload_bytes",
                row["payload"].as_u64(),
                repair["payload_bytes"].as_u64(),
            ),
            (
                "keeps_ascending",
                row["keeps_ascending"].as_u64(),
                repair["keeps_ascending"].as_u64(),
            ),
            (
                "keeps_descending",
                row["keeps_descending"].as_u64(),
                repair["keeps_descending"].as_u64(),
            ),
            (
                "growth_bytes",
                row["growth"].as_u64(),
                repair["growth_bytes"].as_u64(),
            ),
        ] {
            assert_eq!(
                ours, theirs,
                "{id}: the instrument's {field} and the artifact's disagree"
            );
        }
        assert_eq!(
            row["direction"].as_str(),
            repair["direction"].as_str(),
            "{id}: the instrument's direction and the artifact's disagree"
        );
        assert_eq!(
            row["trim"].as_str(),
            repair["trim"].as_str(),
            "{id}: the instrument's trim and the artifact's disagree"
        );
        checked += 1;
    }
    assert!(checked > 0, "no capture carried a repair block, so nothing was checked");
}

/// **The instrument reproduces the crate's class populations.**
///
/// The matrix's `candidates_considered` is the number of paths the crate
/// scored for each class, and the instrument's population is the number of
/// paths that satisfy it. They are the same fact, stated twice, and the
/// published table in `docs/00-spec.md` §4.1 is a third statement of it.
#[test]
fn the_instrument_reproduces_the_crates_class_populations() {
    let Some(py) = python() else {
        println!(
            "note: no python on this machine, so the instrument was not checked against \
             the crate; the gate is skipped, not passed"
        );
        return;
    };
    let mock = Mock::start(World::five_classes());
    let scratch = Scratch::new("population");
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
    let bundle = bundle_in(&scratch.out(), &r);

    let doc = instrument_json(py, &bundle, "population");
    let proposed = &doc["population"]["proposed"];
    let raw = std::fs::read(bundle.join("diff/matrix.json")).expect("the matrix");
    let matrix: serde_json::Value = serde_json::from_slice(&raw).expect("json");

    let mut checked = 0;
    for row in matrix["matrix"].as_array().expect("a matrix array") {
        let class = row["class"].as_str().expect("a class");
        let considered = row["candidates_considered"].as_u64().expect("a count");
        let ours = proposed[class].as_u64().expect("the instrument counts it");
        assert_eq!(
            ours, considered,
            "{class}: the instrument counts {ours} paths and the crate considered \
             {considered}\n--- the instrument says ---\n{}\n--- the bundle's manifests ---\n{:?}",
            doc["population"],
            std::fs::read_dir(bundle.join("manifests").join("7.0.0")).map(|d| {
                d.filter_map(|e| e.ok().map(|e| e.path().display().to_string())).collect::<Vec<_>>()
            })
        );
        checked += 1;
    }
    assert!(checked > 0, "the matrix had no rows, so nothing was checked");
}
