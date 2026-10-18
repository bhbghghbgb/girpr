//! Cross-commit probe: capture `World::five_classes()` against the loopback
//! mock and leave the raw bundles on disk for an external normaliser/differ.
//!
//! Lives outside version control on purpose: it is a measurement instrument, not
//! part of the crate. It is copied into each worktree under test and must stay
//! byte-identical across all of them so the bundles it produces are comparable.
//!
//! Environment:
//!   GIRCAP_PROBE_OUT  directory to write `a/` and `b/` into
//!   GIRCAP_PROBE_N    number of captures (default 2)

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Mock, World};

fn bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("the test binary has a path");
    p.pop(); // deps/
    if p.ends_with("deps") {
        p.pop();
    }
    p.join(format!("gircap{}", std::env::consts::EXE_SUFFIX))
}

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin())
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("running {args:?}: {e}"));
    (
        out.status.code().expect("the process was not killed"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The bundle directory, tolerating a capture that stopped early.
///
/// A degraded-input profile can legitimately end the run before the close-out,
/// in which case there is no `MANIFEST.json` — and the evidence is precisely
/// what *was* written. So the bundle is the deepest directory holding any of
/// the bundle's own subtrees.
fn bundle_in(out: &Path) -> PathBuf {
    let mut found = Vec::new();
    walk(out, &mut found);
    let dirs: Vec<PathBuf> = found
        .into_iter()
        .filter(|p| p.is_dir())
        .collect();
    if let Some(b) = dirs
        .iter()
        .find(|p| p.join("MANIFEST.json").is_file())
    {
        return b.clone();
    }
    dirs.iter()
        .filter(|p| {
            ["chunks", "api", "manifests", "diff", "patchmaps", "verify", "game", "expected"]
                .iter()
                .any(|d| p.join(d).is_dir())
        })
        .max_by_key(|p| p.components().count())
        .cloned()
        .unwrap_or_else(|| panic!("no bundle under {}", out.display()))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        out.push(p.clone());
        if p.is_dir() {
            walk(&p, out);
        }
    }
}

#[test]
fn probe_captures() {
    let root = std::env::var("GIRCAP_PROBE_OUT").expect("GIRCAP_PROBE_OUT");
    let n: usize = std::env::var("GIRCAP_PROBE_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let root = PathBuf::from(root);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("probe root");

    let mock = Mock::start(World::five_classes());
    // Armed profiles may legitimately end the run early.
    let allow_fail = std::env::var("GIRCAP_PROBE_ALLOW_FAIL").is_ok();
    // Extra capture flags, space-separated, so one probe binary can drive more
    // than one capture profile (e.g. a low --gzip-threshold to force the .gz path).
    let extra: Vec<String> = std::env::var("GIRCAP_PROBE_ARGS")
        .unwrap_or_default()
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    for slot in 0..n {
        let out = root.join(slot.to_string());
        let mut args: Vec<String> = vec![
            "capture".into(),
            "--out".into(),
            out.to_string_lossy().into_owned(),
            "--hyp-base".into(),
            mock.hyp_base(),
            "--sophon-base".into(),
            mock.sophon_base(),
            "--jobs".into(),
            "1".into(),
        ];
        args.extend(extra.iter().cloned());
        let args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let (code, stdout, stderr) = run(&args);
        // An armed profile is *expected* to end the run early - that is what a
        // mis-served chunk or a throttled origin does. The exit code is
        // recorded, not asserted; only a healthy profile must reach 0.
        println!("PROBE_EXIT {slot} {code}");
        assert!(
            code == 0 || allow_fail,
            "capture into slot {slot} failed ({code})\nARGS: {args:?}\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}"
        );
        let bundle = bundle_in(&out);
        println!("PROBE_BUNDLE {slot} {}", bundle.display());
    }
}