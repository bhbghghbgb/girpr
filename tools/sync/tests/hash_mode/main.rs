//! `--hash-all-of` / `--hash-any-of`: the two answerability rules.
//!
//! Every case states which algorithms it asks for, which side can hash, and what
//! the run must therefore do. That is the whole feature: `all-of` requires every
//! requested algorithm on both sides of an undecided pair; `any-of` requires one
//! and lets the planner choose it **per pair**.
//!
//! Three properties are worth stating before the cases, because they are what the
//! cases check and each is easy to lose:
//!
//! - **The choice is per pair, not per run.** Path X can be settled by md5 and
//!   path Y by sha256 in the same `compare`, which is why the diff is handed a
//!   per-path answer ([`Required`]) rather than an algorithm list.
//! - **`any-of` never weakens the pair, only the request.** Every algorithm it
//!   picks is obtainable on *both* sides, so `hashes_differ` cannot be silent for
//!   it — a narrower mode cannot admit a pair whose content was never read. This is
//!   the property that makes a weaker mode safe, and it is asserted directly
//!   rather than inferred from a verdict.
//! - **The cost tiers are the flag's entire value for folder-vs-folder.** Both
//!   sides can always backfill, so without "prefer what is already cached" the
//!   mode would hash when it did not have to.
//!
//! ## Layout
//!
//! This file holds the entry fixtures and the harness. The cases are grouped by
//! how far the claim reaches:
//!
//! | module | what it covers |
//! | --- | --- |
//! | [`planner`] | the pick itself, read off `plan_pairs` |
//! | [`reaching_diff`] | that the diff consults the pick, per path |
//! | [`end_to_end`] | the mode as the remedy a stricter mode could only name |
//! | [`read_count`] | the tiers, measured as reads rather than asserted off the plan |

// This target lives at `tests/hash_mode/main.rs`, so the shared fixtures one
// level up need an explicit path rather than the sibling `mod common;` every
// other test target gets.
#[path = "../common/mod.rs"]
mod common;

mod end_to_end;
mod planner;
mod reaching_diff;
mod read_count;

use std::collections::HashMap;
use std::path::Path;

use common::{digests_of, log, opts, pair_mem, recs_of, serial_rw, sync_mtime, temp_root, wfile};
use girsync::cache::{CACHE_PREFIX, CacheDb, CacheOpen, TestCacheBackend, open_db};
use girsync::config::ScanMode;
use girsync::diff::diff_maps;
use girsync::effective::{
    EffRec, SideCapability, SideEntry, classify, ensure_distinct_sides, load_record_side_from,
    open_side, resolve_folder, resolve_record, resolve_side, scan_stat_only,
};
use girsync::hash::{AlgoSet, DigestSet, HashAlg};
use girsync::planner::{
    CasePairing, HashMode, PairPlan, Required, SideRequest, StatTrust, plan_pairs,
};
use girsync::rw::RwSide;
use girsync::{CommonOpts, cmd_compare, cmd_update};

// -- fixtures -----------------------------------------------------------------

/// A file entry with the given stat and the given digests already on its row.
fn file(size: u64, mtime_ns: i64, cached: &[&str]) -> SideEntry {
    SideEntry {
        kind: "file".into(),
        size,
        mtime_ns,
        cached: {
            let mut d = DigestSet::default();
            for a in cached {
                d.insert_named(a, &[1u8; 16]);
            }
            d
        },
        fresh: true,
    }
}

fn map(pairs: Vec<(&str, SideEntry)>) -> HashMap<String, SideEntry> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// A side with a filesystem: it can compute anything it is asked for.
fn folder() -> SideCapability {
    SideCapability {
        can_hash_from_disk: true,
        can_write_cache: true,
    }
}

/// A side without one: only what its rows hold.
fn record() -> SideCapability {
    SideCapability::record()
}

/// Plan a two-sided run, with each side **trusting** its cache.
///
/// Trust is on deliberately, because the cases here are about which algorithms a
/// side is *short of*, and distrusting makes every side short of everything — which
/// would hide the very tiers being tested. Trust's own rule is pinned separately in
/// `src/planner/coverage.rs`
/// (`distrusting_a_record_does_not_change_what_it_owes`) and end-to-end by
/// `update_recomputes_every_digest_and_repairs_a_wrong_one` in `tests/update.rs`.
fn plan(
    s: &HashMap<String, SideEntry>,
    d: &HashMap<String, SideEntry>,
    sc: SideCapability,
    dc: SideCapability,
    algos: &[&str],
    mode: HashMode,
    stat: StatTrust,
) -> anyhow::Result<PairPlan> {
    let algos: Vec<HashAlg> = algos.iter().map(|a| a.parse().unwrap()).collect();
    plan_pairs(
        SideRequest {
            entries: s,
            algos: &algos,
            no_trust: false,
            cap: sc,
            label: "src",
        },
        SideRequest {
            entries: d,
            algos: &algos,
            no_trust: false,
            cap: dc,
            label: "dst",
        },
        CasePairing::Exact,
        mode,
        stat,
    )
}

/// A pair of maps with no cache at all, for the diff-level cases.
fn eff(entries: Vec<(&str, &DigestSet)>) -> HashMap<String, EffRec> {
    entries
        .into_iter()
        .map(|(rel, hashes)| {
            (
                rel.to_string(),
                EffRec {
                    kind: "file".into(),
                    size: 10,
                    mtime_ns: 100,
                    hashes: hashes.clone(),
                },
            )
        })
        .collect()
}

fn digests(pairs: &[(&str, &[u8])]) -> DigestSet {
    let mut set = DigestSet::default();
    for (a, v) in pairs {
        set.insert_named(a, v);
    }
    set
}

fn asking(algos: &[&str], mode: HashMode) -> CommonOpts {
    CommonOpts {
        algos: algos.iter().map(|a| a.parse().unwrap()).collect(),
        hash_mode: mode,
        ..opts()
    }
}

/// [`asking`] served from the shared in-memory cache, for the cases that measure
/// reads rather than inspect the cache file. The warmed fixture must have been
/// built with [`pair_mem`](common::pair_mem) so the rows are in the same store.
fn asking_mem(algos: &[&str], mode: HashMode) -> CommonOpts {
    CommonOpts {
        test_cache: TestCacheBackend::Memory,
        ..asking(algos, mode)
    }
}

/// `(files opened, digests computed)` for a two-sided `compare`.
///
/// Spelled out rather than borrowed from `lazy/`: that harness asserts verdicts
/// across every difference class, and these cases are about a read count on a tree
/// with no differences at all — a fixture with `CHANGED` paths in it would make
/// the number depend on the prune and rename paths too.
///
/// Both at once because they are different facts and a case wants both. `hashed`
/// counts **files opened**, while `hash_file` computes every algorithm for a path in
/// one pass — so two algorithms on one file is one read and two digests. That
/// distinction is the only thing `any-of` changes when a pair shares no algorithm,
/// and it is invisible in `hashed`.
///
/// **A fresh fixture per call.** Resolving a folder side *writes to its cache*, so
/// this is not a pure function of its inputs: measuring two modes against one pair
/// makes the second run find a warm cache the first one backfilled, and reports the
/// second mode as free.
fn run_pair(src: &Path, dst: &Path, common: &CommonOpts) -> (usize, usize) {
    let s = classify(src);
    let d = classify(dst);
    ensure_distinct_sides(&s, &d).unwrap();
    let mode = ScanMode {
        no_trust_cached_hashes: false,
        dry_run: false,
    };
    let mut so = open_side(&s, common, mode).unwrap();
    let mut do_ = open_side(&d, common, mode).unwrap();
    let s_label = format!("src {}", s.cache_path().display());
    let d_label = format!("dst {}", d.cache_path().display());
    let plans = plan_pairs(
        SideRequest {
            entries: &so.phase_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: so.cap,
            label: &s_label,
        },
        SideRequest {
            entries: &do_.phase_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: do_.cap,
            label: &d_label,
        },
        CasePairing::from(common.case_sensitive),
        common.hash_mode,
        common.stat,
    )
    .unwrap_or_else(|e| panic!("two folders cannot fail coverage: {e:#}"));
    let digests = plans.src.digest_count() + plans.dst.digest_count();
    let sm = resolve_side(&mut so, mode, &plans.src, RwSide::Src, &serial_rw()).unwrap();
    let dm = resolve_side(&mut do_, mode, &plans.dst, RwSide::Dst, &serial_rw()).unwrap();
    (sm.stats.hashed + dm.stats.hashed, digests)
}

/// A `compare-self` audit's read count and exit code: `(hashed, code)`.
///
/// The disk side is scanned against an **empty** handle, so an undecided pair is
/// always rehashed - which is what makes the count a measure of the plan rather than
/// of whatever the record happened to hold.
fn self_audit_count(dir: &Path, common: &CommonOpts) -> (usize, i32) {
    let mode = ScanMode {
        no_trust_cached_hashes: false,
        dry_run: true,
    };
    let record = open_db(&dir.join(CACHE_PREFIX), true, CacheOpen::ReadOnly).unwrap();
    let rec = load_record_side_from(&record, common, "record").unwrap();
    let cold = CacheDb::open_temp(common.case_sensitive).unwrap();
    let disk_a = scan_stat_only(dir, &cold, common, mode).unwrap();
    let rec_label = format!("record {}", dir.join(CACHE_PREFIX).display());
    let disk_label = format!("folder {}", dir.display());
    let plans = plan_pairs(
        SideRequest {
            entries: &rec.map,
            algos: &common.algos,
            no_trust: false,
            cap: SideCapability::record(),
            label: &rec_label,
        },
        SideRequest {
            entries: &disk_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: SideCapability::for_folder(&cold, mode),
            label: &disk_label,
        },
        CasePairing::from(common.case_sensitive),
        common.hash_mode,
        common.stat,
    )
    .unwrap_or_else(|e| panic!("the record holds md5, so this is answerable: {e:#}"));
    let disk = resolve_folder(
        dir,
        &cold,
        mode,
        &disk_a,
        &plans.dst,
        RwSide::Dst,
        &serial_rw(),
    )
    .unwrap();
    let rec = resolve_record(&rec);
    let diff = diff_maps(
        &rec.map,
        &disk.map,
        &plans.required,
        plans.stat,
        CasePairing::from(common.case_sensitive),
        false,
    );
    (disk.stats.hashed, if diff.is_empty() { 0 } else { 4 })
}
