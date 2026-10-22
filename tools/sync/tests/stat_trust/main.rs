//! `--no-trust-size` / `--no-trust-mtime`: switching the stat short circuit off.
//!
//! The short circuit is an optimisation, so it has to be switchable. These two flags
//! name the stat fields it consults, and **both together is "disable it"** — no third
//! spelling is needed to reach the whole thing off.
//!
//! What each one does is stated in the planner's tests; what is left here is what can
//! only be seen from outside:
//!
//! - **the counters.** A flag that widens the undecided set has to actually cost the
//!   reads, and the counts are the only honest observable. Every case states the
//!   `all-of`-style default as a control, because a test asserting only "the flag
//!   reads more" would pass against a planner that always read everything.
//! - **the verdict.** `--no-trust-mtime` can *change* one — a touched file with
//!   identical bytes comes back equal. That is the flag's whole point, so it is
//!   asserted end to end rather than left to the planner's arithmetic.
//! - **the cache.** This is what `--no-trust-size` is really for, and it is invisible
//!   in both counters and verdicts.
//! - **the flags themselves**, through the real binary.
//!
//! ## Layout
//!
//! This file holds the options builder and the fixture every case measures
//! against. The cases are grouped by what they can observe:
//!
//! | module | what it covers |
//! | --- | --- |
//! | [`counters`] | reads: a widened undecided set has to cost them |
//! | [`verdict`] | verdicts: the one flag whose answer can change |
//! | [`cache_repair`] | the stuck cache row, invisible in both of the above |
//! | [`other_commands`] | `sync` and `compare-self`, where the flags stop being an optimisation |
//! | [`flag_surface`] | `update`, and the flags as the CLI accepts them |
//! | [`no_fallback`] | what a pair is called when nothing is left to settle it |
//! | [`refusal`] | the combination refused at the flag layer |

// This target lives at `tests/stat_trust/main.rs`, so the shared fixtures one
// level up need an explicit path rather than the sibling `mod common;` every
// other test target gets.
#[path = "../common/mod.rs"]
mod common;

mod cache_repair;
mod counters;
mod flag_surface;
mod no_fallback;
mod other_commands;
mod refusal;
mod verdict;

use common::{TempDir, age, log, opts, recs_of, serial_rw, sync_mtime, temp_root, update, wfile};
use girsync::planner::{CasePairing, StatTrust};
use girsync::{CommonOpts, cmd_compare, cmd_compare_self, cmd_sync, cmd_update};

/// One invocation of the real binary: exit code and stderr.
///
/// Here rather than in whichever section first needed it: the flag surface is
/// checked in three places — `update`'s warning, the parser's independence, and
/// the refusal at the end — and all three are the same invocation.
fn bin(args: &[String]) -> (i32, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args(args)
        .output()
        .expect("spawn girsync");
    (
        out.status.code().expect("girsync exits with a code"),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Restore a file's mtime to a value captured earlier.
///
/// Load-bearing wherever a *rewrite* is part of the fixture: rewriting moves the mtime,
/// and a pair whose mtime also differs is settled by the field this run still trusts —
/// so the flag under test would widen nothing and the fixture would appear broken.
fn restore_mtime(p: &std::path::Path, t: std::time::SystemTime) {
    std::fs::OpenOptions::new()
        .write(true)
        .open(p)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

/// src and dst differing in length, with the mtime pinned so size is the only signal.
///
/// The shape both `--no-trust-size`'s cases and the "no digest to fall back on"
/// cases need: a pair whose *only* evidence of difference is size, so switching
/// that field off leaves the pair undecidable.
fn size_differs(t: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"short");
    wfile(
        &dst,
        "a.txt",
        b"a considerably longer body than the other side",
    );
    sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));
    (src, dst)
}

/// `opts()` asking for `algos`, with stat trust set.
///
/// The flags are on `CommonOpts` rather than on a command's own opts because they are a
/// property of the *comparison*, not of any one side — the short circuit compares the
/// two sides' stats against each other, so there is no per-side version to have. This
/// is unlike `--no-trust-cached-hashes`, which distrusts one side's digest and so does
/// take a side.
fn distrusting(algos: &[&str], stat: StatTrust) -> CommonOpts {
    CommonOpts {
        algos: algos.iter().map(|a| a.parse().unwrap()).collect(),
        stat,
        ..opts()
    }
}

/// Two folders, four files, one per interesting pair state.
///
/// ```text
///   equal.txt    identical bytes, mtime pinned   -> undecided, hashed either way
///   sized.txt    dst is longer, mtime pinned     -> free by size
///   mtime'd.txt  identical bytes, dst's mtime moved -> free by mtime
///   gone.txt     dst only                        -> EXTRA, free
/// ```
///
/// `equal.txt` is the control: it is already undecided, so it hashes in every
/// configuration and any change in the other three is attributable.
fn four_states(t: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    for rel in ["equal.txt", "sized.txt", "mtime'd.txt"] {
        let body = format!("body of {rel}");
        wfile(&src, rel, body.as_bytes());
        wfile(&dst, rel, body.as_bytes());
    }
    // `sized.txt` differs in length on dst, mtime pinned so size is the only signal.
    wfile(
        &dst,
        "sized.txt",
        b"a much longer body than src has for this path",
    );
    for rel in ["equal.txt", "sized.txt", "mtime'd.txt"] {
        sync_mtime(&src.join(rel), &dst.join(rel));
    }
    // `mtime'd.txt` then gets its mtime pushed forward: identical bytes, same size,
    // so mtime is the only thing that differs.
    age(&dst, "mtime'd.txt", 60);
    wfile(&dst, "gone.txt", b"dst only");
    (src, dst)
}

/// Files opened by a two-sided `compare`, summed over both sides.
///
/// Resolving a folder side **writes to its cache**, so this is not a pure function of
/// its inputs and a case must not measure two configurations against one fixture — the
/// first run backfills whatever the second would then find cached. Every case here
/// builds a fresh fixture per configuration.
fn hashed(src: &std::path::Path, dst: &std::path::Path, common: &CommonOpts) -> usize {
    use girsync::config::ScanMode;
    use girsync::effective::{classify, ensure_distinct_sides, open_side, resolve_side};
    use girsync::planner::{SideRequest, plan_pairs};
    use girsync::rw::RwSide;

    let (s_side, d_side) = (classify(src), classify(dst));
    ensure_distinct_sides(&s_side, &d_side).unwrap();
    let mode = ScanMode {
        no_trust_cached_hashes: false,
        dry_run: false,
    };
    let mut s = open_side(&s_side, common, mode).unwrap();
    let mut d = open_side(&d_side, common, mode).unwrap();
    let s_label = format!("src {}", s_side.cache_path().display());
    let d_label = format!("dst {}", d_side.cache_path().display());
    let plans = plan_pairs(
        SideRequest {
            entries: &s.phase_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: s.cap,
            label: &s_label,
        },
        SideRequest {
            entries: &d.phase_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: d.cap,
            label: &d_label,
        },
        CasePairing::from(common.case_sensitive),
        common.hash_mode,
        common.stat,
    )
    .unwrap();
    let sm = resolve_side(&mut s, mode, &plans.src, RwSide::Src, &serial_rw()).unwrap();
    let dm = resolve_side(&mut d, mode, &plans.dst, RwSide::Dst, &serial_rw()).unwrap();
    sm.stats.hashed + dm.stats.hashed
}
