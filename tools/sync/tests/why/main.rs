//! What the diff concluded about each pair, and the tag that says so.
//!
//! `--why` prints a tag per record, so a tag is a **contract**: a parser splits it
//! on the first `:` and switches on what is left. That makes the vocabulary worth
//! pinning per pair state, which is what the table below does — each row names a
//! path, the pair state it is put in, and the one classification and tag that state
//! must produce.
//!
//! ## Why a tag per *decision* and not per check
//!
//! `diff` is a short circuit: a trusted stat field settles the pair and no digest is
//! read at all. So the honest reason is whatever *settled* it. That makes "never
//! report previous checks' efforts" fall out for free — there is no earlier evidence
//! to accumulate — instead of needing enforcement.
//!
//! ## Why the table, and not a transcript
//!
//! Each row is a claim about intent that survives a change of implementation, fails
//! naming the path, and adds behaviour as a row rather than as an edit to a
//! recorded vector.
//!
//! ## The rows that earn their place
//!
//! - `digest-differs:md5+sha256` is the one that pins **which** algorithms a tag
//!   names. `--hash-all-of md5 sha256` asks for both, so a reason built from the
//!   *request* would look identical to one built from the *disagreement* — and the
//!   difference is the whole usefulness of the field.
//! - The tag's **order** is pinned separately, and is the disagreement's rather
//!   than the flag order's: `--hash-all-of sha256 md5` produces the same
//!   `digest-differs:md5+sha256`, so a parser can compare tags without first
//!   normalising them.
//! - The `any-of` row is the same claim against a different mechanism. There the run
//!   asked for two algorithms and the planner picked one; naming the request would
//!   say `md5+sha256` for a pair only `md5` ever touched, and `pick_one`'s tier
//!   logic makes the wrong answer entirely plausible.
//! - `unverifiable:*` is pinned because it means **"cannot tell"**, not "differs".
//!   Folding it into a stat tag would report a difference the run explicitly refused
//!   to accept as evidence.
//!
//! ## Layout
//!
//! This file holds the shared fixture builders, the configurations and the
//! harness; the cases are grouped by what they are pinning:
//!
//! | module | what it covers |
//! | --- | --- |
//! | [`table`] | the pair-state table itself, and the cases that read it |
//! | [`identical`] | `--show-identical`: the second bucket and how it is reported |
//! | [`flags`] | both flags' surface, through the reporter and through the real binary |
//! | [`disagreement`] | "what disagreed" versus "what was asked for" |

// This target lives at `tests/why/main.rs`, so the shared fixtures one level up
// need an explicit path rather than the sibling `mod common;` every other test
// target gets.
#[path = "../common/mod.rs"]
mod common;

mod disagreement;
mod flags;
mod identical;
mod table;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use common::{TempDir, age, serial_rw, sync_mtime, temp_root, wfile};
use girsync::cache::TestCacheBackend;
use girsync::config::ScanMode;
use girsync::diff::{Diff, StatField, Verdict, diff_maps, hashes_differ, pair_verdict};
use girsync::effective::{classify, ensure_distinct_sides, open_side, resolve_side};
use girsync::hash::{AlgoSet, HashAlg};
use girsync::planner::{
    CasePairing, HashMode, PairPlan, Required, SideRequest, StatTrust, plan_pairs,
};
use girsync::rw::RwSide;
use girsync::{CommonOpts, EffRec, cmd_update};

// -- configurations ------------------------------------------------------------

fn md5() -> CommonOpts {
    common::opts()
}

fn md5_and_sha256() -> CommonOpts {
    common::with_algos(&["md5", "sha256"])
}

fn none() -> CommonOpts {
    common::with_algos(&[])
}

fn none_without_trusting_size() -> CommonOpts {
    CommonOpts {
        algos: vec![],
        stat: StatTrust::default().without_size(),
        ..common::opts()
    }
}

fn none_without_trusting_anything() -> CommonOpts {
    CommonOpts {
        algos: vec![],
        stat: StatTrust::default().without_size().without_mtime(),
        ..common::opts()
    }
}

fn any_of_md5_sha256() -> CommonOpts {
    CommonOpts {
        // The order is meaningful: `pick_one` breaks a tie in the user's flag order.
        algos: vec![HashAlg::Md5, HashAlg::Sha256],
        hash_mode: HashMode::AnyOf,
        ..common::opts()
    }
}

// -- fixture -------------------------------------------------------------------

/// Two folders holding `rel` with the given bytes on each side, dst's mtime pinned
/// to src's.
///
/// The pin is load-bearing wherever a *rewrite* is the fixture: writing moves the
/// mtime, and a pair whose mtime also differs is settled by a stat field instead of
/// by the digest this row is about — so the row would appear not to work, for a
/// reason that has nothing to do with the code.
fn pinned(t: &TempDir, rel: &str, src_bytes: &[u8], dst_bytes: &[u8]) -> (PathBuf, PathBuf) {
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, rel, src_bytes);
    wfile(&dst, rel, dst_bytes);
    sync_mtime(&src.join(rel), &dst.join(rel));
    (src, dst)
}

// -- harness -------------------------------------------------------------------

/// The two effective maps and the planner's answer, from the exact phase sequence
/// `cmd_compare` runs.
///
/// Driven here rather than delegated to so a row can read the *pair's* classification
/// rather than only the buckets: `--why` prints what the diff concluded, and a row
/// that could only see `CHANGED` could not tell a stat verdict from a digest one.
/// `temp_tag` names the fixture so a panic points at a directory that still exists.
fn resolved(
    temp_tag: &str,
    src: &Path,
    dst: &Path,
    common: &CommonOpts,
) -> (HashMap<String, EffRec>, HashMap<String, EffRec>, PairPlan) {
    let (s_side, d_side) = (classify(src), classify(dst));
    ensure_distinct_sides(&s_side, &d_side)
        .unwrap_or_else(|e| panic!("{temp_tag}: the two sides name one cache: {e:#}"));
    // Semantic suite: assert verdicts/tags, not the cache file, so serve folder
    // caches from the in-memory backend and skip the per-open create + fsync.
    let common = CommonOpts {
        test_cache: TestCacheBackend::Memory,
        ..common.clone()
    };
    let mode = ScanMode {
        no_trust_cached_hashes: false,
        dry_run: false,
    };
    let mut s =
        open_side(&s_side, &common, mode).unwrap_or_else(|e| panic!("{temp_tag}: src side: {e:#}"));
    let mut d =
        open_side(&d_side, &common, mode).unwrap_or_else(|e| panic!("{temp_tag}: dst side: {e:#}"));
    let plans = plan_pairs(
        SideRequest {
            entries: &s.phase_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: s.cap,
            label: &format!("src {}", s_side.cache_path().display()),
        },
        SideRequest {
            entries: &d.phase_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: d.cap,
            label: &format!("dst {}", d_side.cache_path().display()),
        },
        CasePairing::from(common.case_sensitive),
        common.hash_mode,
        common.stat,
    )
    .unwrap_or_else(|e| panic!("{temp_tag}: this pair is answerable: {e:#}"));
    let sm = resolve_side(&mut s, mode, &plans.src, RwSide::Src, &serial_rw())
        .unwrap_or_else(|e| panic!("{temp_tag}: resolving src: {e:#}"));
    let dm = resolve_side(&mut d, mode, &plans.dst, RwSide::Dst, &serial_rw())
        .unwrap_or_else(|e| panic!("{temp_tag}: resolving dst: {e:#}"));
    (sm.map, dm.map, plans)
}
