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
use girsync::cache::CACHE_PREFIX;
use girsync::commands::side_label;
use girsync::config::ScanMode;
use girsync::diff::{Diff, diff_maps};
use girsync::effective::{
    Side, SideCapability, classify, open_folder_cache, resolve_folder, scan_stat_only, side_folder,
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

/// Resolve both sides and diff them through the production computation.
///
/// `cmd_compare` renders a verdict and an exit code, so the per-side counters a
/// laziness case needs to assert are not reachable from it. `compare_detailed`
/// is the very same pipeline handed back as data: the phases, the side labels and
/// the per-side `ScanMode`/`RwSide` threading all live in the command, not in a
/// second copy here. The wiring of the command to that computation is pinned
/// end-to-end in `tests/compare.rs`.
fn run_pair(src: &Path, dst: &Path, common: &CommonOpts, trust: TrustOpts) -> Run {
    let opts = girsync::CompareOpts {
        src: src.to_path_buf(),
        dst: dst.to_path_buf(),
        trust,
        common: common.clone(),
    };
    let outcome = girsync::commands::compare_detailed(opts, &log())
        .expect("two folders cannot fail coverage");
    Run {
        diff: outcome.diff,
        hashed: outcome.src_stats.hashed + outcome.dst_stats.hashed,
        src_hashed: outcome.src_stats.hashed,
        dst_hashed: outcome.dst_stats.hashed,
    }
}

/// Four file pairs, each dst content a different length from src's, so every
/// pair differs in size and therefore in stat.
fn four_differing(t: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
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
    let mut common = opts_mem();
    common.set_case_sensitive(true);
    girsync::SyncOpts {
        src,
        dst,
        trust: TrustOpts::default(),
        missing_only: false,
        keep_extra: false,
        common,
    }
}
