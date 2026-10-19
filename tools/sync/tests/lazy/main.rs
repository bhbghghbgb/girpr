//! Lazy digest resolution: how much a run reads, and what it concludes.
//!
//! Each case states its own expected verdict and its own expected `hashed`
//! count. There is no differential oracle and no generator: for a fixture built
//! by hand the answer is known before the run, and writing it down pins *more*
//! than comparing two runs would — the counters as well as the verdict.
//!
//! Verdicts are asserted as **JSON records**, the same objects `--output json`
//! writes to stdout, rather than as rendered text lines. A record has names and
//! values, so `{"event":"changed","path":"diff1.txt"}` states what was reported
//! without restating how it is spelled, and a change to the prose rendering
//! cannot fail a case whose point is the verdict.
//!
//! The `hashed` expectation in each test's doc comment is the count the run
//! actually produces. Stating it is the point: an eager run over the same fixture
//! would read every file on both sides, so the gap between the two numbers is
//! exactly what laziness bought.
//!
//! ## Layout
//!
//! This file is the harness and the shared fixtures; the cases are grouped by
//! the command whose laziness they pin, because each group drives a different
//! phase sequence:
//!
//! | module | what it covers |
//! | --- | --- |
//! | [`compare_self`] | the audit: the disk side hashed against nothing |
//! | [`compare`] | two-sided laziness, verdict **and** read count per case |
//! | [`sync`] | `sync`'s scan, plus what a folder side does to its own cache |

// This target lives at `tests/lazy/main.rs`, so the shared fixtures one level
// up need an explicit path rather than the sibling `mod common;` every other
// test target gets.
#[path = "../common/mod.rs"]
mod common;

mod compare;
mod compare_self;
mod sync;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use common::*;
use girsync::cache::{CACHE_PREFIX, CacheDb, CacheOpen, open_db};
use girsync::config::ScanMode;
use girsync::diff::{Diff, diff_maps};
use girsync::effective::{
    Side, SideCapability, classify, ensure_distinct_sides, load_record_side_from,
    open_folder_cache, open_side, resolve_folder, resolve_record, resolve_side, scan_stat_only,
};
use girsync::hash::{AlgoSet, HashAlg};
use girsync::planner::{CasePairing, Required, SideRequest, StatTrust, plan_pairs};
use girsync::report::verdict;
use girsync::rw::RwSide;
use girsync::{CommonOpts, TrustOpts, UpdateOpts, cmd_sync};
use serde_json::{Value, json};

/// What one two-sided run produced: the verdict, and how much it read.
struct Run {
    diff: Diff,
    /// Summed over both sides. Read per-side where a case needs to say which
    /// side paid.
    hashed: usize,
    src_hashed: usize,
    dst_hashed: usize,
}

impl Run {
    /// The records a run would report, in report order, as the JSON objects
    /// `--output json` writes. Asserting on these rather than on text keeps a
    /// case about the verdict independent of how the verdict is spelled.
    fn records(&self) -> Vec<Value> {
        // `why: false` throughout this file: laziness is asserted on *which* pairs
        // were read, and a `why` field would be a reporting difference on records
        // whose content is not what's under test. `tests/why/` covers the flag.
        verdict(&self.diff, false, false)
            .iter()
            .map(|r| r.json())
            .collect()
    }

    fn exit(&self) -> i32 {
        if self.diff.is_empty() { 0 } else { 4 }
    }
}

/// Resolve both sides and diff them, the way `cmd_compare` does.
///
/// The phases are driven here rather than through `cmd_compare` because
/// `cmd_compare` returns only an exit code, and the counter is the thing under
/// test. `cmd_compare_reports_the_same_verdict_and_its_exit_code` covers the
/// wiring.
fn run_pair(src: &Path, dst: &Path, common: &CommonOpts, trust: TrustOpts) -> Run {
    let s_side = classify(src);
    let d_side = classify(dst);
    ensure_distinct_sides(&s_side, &d_side).unwrap();
    let mode = |no_trust: bool| ScanMode {
        no_trust_cached_hashes: no_trust,
        dry_run: false,
    };
    // Both handles stay live across plan and resolve.
    let mut s = open_side(&s_side, common, mode(trust.no_trust_src)).unwrap();
    let mut d = open_side(&d_side, common, mode(trust.no_trust_dst)).unwrap();

    // Labels only so a coverage error can name a side. Every case here is two
    // folders, so coverage cannot fail and the labels are never read.
    let s_label = format!("src {}", s_side.cache_path().display());
    let d_label = format!("dst {}", d_side.cache_path().display());
    let plans = plan_pairs(
        SideRequest {
            entries: &s.phase_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_src,
            cap: s.cap,
            label: &s_label,
        },
        SideRequest {
            entries: &d.phase_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_dst,
            cap: d.cap,
            label: &d_label,
        },
        CasePairing::from(common.case_sensitive),
        common.hash_mode,
        common.stat,
    )
    .unwrap();
    let sm = resolve_side(
        &mut s,
        mode(trust.no_trust_src),
        &plans.src,
        RwSide::Src,
        &serial_rw(),
    )
    .unwrap();
    let dm = resolve_side(
        &mut d,
        mode(trust.no_trust_dst),
        &plans.dst,
        RwSide::Dst,
        &serial_rw(),
    )
    .unwrap();
    Run {
        diff: diff_maps(
            &sm.map,
            &dm.map,
            &plans.required,
            plans.stat,
            CasePairing::from(common.case_sensitive),
            false,
        ),
        hashed: sm.stats.hashed + dm.stats.hashed,
        src_hashed: sm.stats.hashed,
        dst_hashed: dm.stats.hashed,
    }
}

/// Four file pairs, each dst content a different length from src's, so every
/// pair differs in size and therefore in stat.
fn four_differing(t: &TempRoot) -> (std::path::PathBuf, std::path::PathBuf) {
    pair(
        t,
        &[
            ("a.txt", Some(b"alpha"), Some(b"a")),
            ("b.txt", Some(b"bravo-longer"), Some(b"bb")),
            ("sub/c.txt", Some(b"charlie-longer"), Some(b"ccc")),
            ("sub/deep/d.txt", Some(b"delta-longer"), Some(b"dddd")),
        ],
        &["md5"],
        false,
    )
}

/// A `SyncOpts` for the `sync` cases. Case-sensitive by default, so the counts are
/// not perturbed by a rename pass; a case that wants insensitive sets it.
fn sync_opts(src: std::path::PathBuf, dst: std::path::PathBuf) -> girsync::SyncOpts {
    let mut common = opts();
    common.case_sensitive = true;
    girsync::SyncOpts {
        src,
        dst,
        trust: TrustOpts::default(),
        missing_only: false,
        keep_extra: false,
        common,
    }
}
