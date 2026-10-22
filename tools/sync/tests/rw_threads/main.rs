//! The rw limiter: one gate over every file read and write, with a per-side mode.
//!
//! The limiter is wired into phase C and the two sides of a pair resolve
//! concurrently, so the invariant has to be pinned somewhere that is not the CLI:
//! a scheduling bug does not necessarily change what the run prints. It is
//! exercised here through the limiter's own public API, at every shape the
//! `--rw-threads*` family can be given.
//!
//! What is asserted throughout is the crate's standing rule: the limiter changes
//! **scheduling** and nothing else. Every setting must resolve the same trees to
//! the same effective maps — digests included — and report the same `ScanStats`,
//! compared whole rather than field by field, so a counter added later cannot
//! diverge between settings without this noticing.
//!
//! What is *not* here, and why: a `sync` driven at a **split** limit through the
//! CLI. `--rw-threads-src 1 --rw-threads-dst 1` is spelled in `--rw-dual-drive`,
//! so the flags in [`flag_surface`] do reach that shape — but only for a shape a
//! user asked for. The single-copy shape is asserted directly against the
//! limiter, where the two-permit path is contended rather than incidentally
//! exercised.
//!
//! ## Layout
//!
//! This file holds the settings table, the fixture and the binary helpers that
//! more than one section needs. The cases are grouped by how deep the claim goes:
//!
//! | module | what it covers |
//! | --- | --- |
//! | [`flag_surface`] | what the family parses into, checked against `clap` |
//! | [`limiter`] | the gate itself, contended, and the invariant that scheduling changes nothing |
//! | [`through_the_binary`] | the same answer end to end, at every limit |

// This target lives at `tests/rw_threads/main.rs`, so the shared fixtures one
// level up need an explicit path rather than the sibling `mod common;` every
// other test target gets.
#[path = "../common/mod.rs"]
mod common;

mod flag_surface;
mod limiter;
mod through_the_binary;

use std::collections::{BTreeSet, HashMap};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Barrier;
use std::sync::atomic::{AtomicUsize, Ordering};

use clap::Parser;
use common::{Spec, TempDir, pair, recs_of, serial_rw, temp_root, wfile};
use girsync::cache::CACHE_PREFIX;
use girsync::cli::{Cli, RwArgs};
use girsync::config::{CommonOpts, ScanMode};
use girsync::effective::{
    EffRec, OpenSide, ScanStats, classify, ensure_distinct_sides, open_side, resolve_side,
    resolve_sides_concurrently,
};
use girsync::planner::{CasePairing, SideRequest, plan_pairs};
use girsync::rw::{RwLimits, RwRuntime, RwSide};

/// Resolve a flag combination the way the CLI does, without going through clap.
fn limits_of(a: RwArgs) -> RwLimits {
    RwLimits::try_from(a).unwrap_or_else(|e| panic!("{e:#}"))
}

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).unwrap()
}

/// One resolved side: the map it produced and the counters that describe how.
type Resolved = (HashMap<String, EffRec>, ScanStats);

/// Every shape the CLI resolves to, plus what a command builds when the user
/// names nothing.
fn settings() -> Vec<(&'static str, RwLimits)> {
    vec![
        ("serial", RwLimits::Shared(nz(1))),
        ("shared(2)", RwLimits::Shared(nz(2))),
        ("shared(4)", RwLimits::Shared(nz(4))),
        (
            "split(1,1)",
            RwLimits::Split {
                src: nz(1),
                dst: nz(1),
            },
        ),
        (
            "split(4,4)",
            RwLimits::Split {
                src: nz(4),
                dst: nz(4),
            },
        ),
        (
            "split(4,1)",
            RwLimits::Split {
                src: nz(4),
                dst: nz(1),
            },
        ),
    ]
}

/// Padding content for the bulk of the fixture. A `const` rather than a repeat
/// expression so the slices in the spec are genuinely `'static`, which the
/// fixture's `Spec` type requires.
const PAD: [u8; 512] = [b'x'; 512];

/// A tree with every kind of work in the plan: stat-equal pairs that only a
/// digest can settle, a pair settled by size, a src-only and a dst-only path, a
/// directory and an empty file.
///
/// Deliberately **more files than one resolve window**, so the windowed loop runs
/// several times. A fixture that fit in a single window would pass even if the
/// window boundary dropped a file, duplicated one, or lost the write order
/// between windows.
fn fixture(t: &TempDir) -> (PathBuf, PathBuf) {
    let mut spec: Vec<Spec> = vec![
        ("changed.txt", Some(b"src version"), Some(b"dst")),
        ("sized.txt", Some(b"src is longer"), Some(b"s")),
        ("src_only.txt", Some(b"only here"), None),
        ("dst_only.txt", None, Some(b"only there")),
        ("sub/nested.txt", Some(b"nested"), Some(b"nest")),
        ("emptydir", Some(&[]), None),
    ];
    // 96 files. With the smallest window the limiter computes (32) that is three
    // windows, so the boundary is exercised three times rather than trivially
    // satisfied once.
    for i in 0..96usize {
        let rel: &'static str = Box::leak(format!("pad{i:02}.bin").into_boxed_str());
        spec.push((rel, Some(&PAD[..]), Some(&PAD[..])));
    }
    pair(t, &spec, &["md5"], false)
}

/// Action events `sync` emits, as opposed to the records that describe why.
///
/// Copied rather than imported: `sync_plan/` owns this vocabulary and duplicating
/// one match arm here would be a second place for it to be wrong. It is a *filter*
/// either way -- the same events `plan::dry_run_records` and the applier phases
/// announce -- so a new action event has to be added in two places, and that is
/// visible in review.
fn is_action(event: &str) -> bool {
    matches!(event, "mkdir" | "fix-dir" | "delete" | "rmdir" | "copy")
}

/// An argv built from a subcommand, a fixture's two paths, and extra flags.
///
/// One helper rather than three inline `vec!`s, because the repeated part is exactly
/// the part that is easy to get wrong in a way the assertions would not notice: a
/// `--src`/`--dst` that does not parse leaves the run failing for a reason unrelated
/// to concurrency.
fn argv<'a>(sub: &'a str, src: &'a Path, dst: &'a Path, extra: &[&'a str]) -> Vec<&'a str> {
    let s = src.to_str().unwrap();
    let d = dst.to_str().unwrap();
    let mut v = vec![sub, "--src", s, "--dst", d];
    v.extend_from_slice(extra);
    v
}

/// Run `girsync` with these arguments and return its exit code plus its
/// `--output json` records.
///
/// Generic over the argument type so a caller can pass the `Vec<String>` the
/// builder produces or a literal `&str` array without a conversion dance at every
/// call site.
fn run(args: &[&str]) -> (i32, Vec<serde_json::Value>) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args(args)
        .arg("--output")
        .arg("json")
        .output()
        .expect("spawn girsync");
    (
        out.status.code().unwrap_or(-1),
        common::parse_ndjson(&out.stdout),
    )
}

/// Every rw spelling the CLI accepts, paired with what it should resolve to.
fn flag_spellings() -> Vec<Vec<&'static str>> {
    vec![
        vec![],
        vec!["--rw-threads", "4"],
        vec!["--rw-dual-drive"],
        vec!["--rw-threads-src", "3"],
        vec!["--rw-threads-dst", "3"],
        vec!["--rw-threads-src", "3", "--rw-threads-dst", "2"],
    ]
}
