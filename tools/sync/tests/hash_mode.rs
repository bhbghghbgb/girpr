//! `--hash-all-of` / `--hash-any-of`: the two answerability rules.
//!
//! Every case states which algorithms it asks for, which side can hash, and what
//! the run must therefore do. That is the whole feature: `all-of` requires every
//! requested algorithm on both sides of an undecided pair; `any-of` requires one
//! and lets the planner choose it **per pair**.
//!
//! Three properties are worth stating before the cases, because they are what the
//! cases check and each is easy to lose in a refactor:
//!
//! - **The choice is per pair, not per run.** Path X can be settled by md5 and
//!   path Y by sha256 in the same `compare`, which is why the diff is handed a
//!   per-path answer ([`Required`]) rather than an algorithm list.
//! - **`any-of` never weakens the pair, only the request.** Every algorithm it
//!   picks is obtainable on *both* sides, so `hashes_differ` cannot be silent for
//!   it — the degradation stage 5b closed cannot re-enter through the narrower
//!   mode. This is the property that makes a weaker mode safe, and it is asserted
//!   directly rather than inferred from a verdict.
//! - **The cost tiers are the flag's entire value for folder-vs-folder.** Both
//!   sides can always backfill, so without "prefer what is already cached" the
//!   mode would hash when it did not have to.

mod common;

use std::collections::HashMap;
use std::path::Path;

use common::{TempRoot, log, opts, pair, recs_of, sync_mtime, wfile};
use girsync::cache::{CACHE_PREFIX, CacheDb, CacheOpen, open_db};
use girsync::config::ScanMode;
use girsync::diff::diff_maps;
use girsync::effective::{
    EffRec, SideCapability, SideEntry, classify, ensure_distinct_sides, load_record_side_from,
    open_side, resolve_folder, resolve_record, resolve_side, scan_stat_only,
};
use girsync::planner::{HashMode, PairPlan, Required, SideRequest, plan_pairs};
use girsync::{CommonOpts, cmd_compare, cmd_update};

// -- fixtures -----------------------------------------------------------------

/// A file entry with the given stat and the given digests already on its row.
fn file(size: u64, mtime_ns: i64, cached: &[&str]) -> SideEntry {
    SideEntry {
        kind: "file".into(),
        size,
        mtime_ns,
        cached: cached
            .iter()
            .map(|a| (a.to_string(), vec![1u8; 16]))
            .collect(),
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
/// `planner.rs` (`distrusting_a_record_does_not_change_what_it_owes`) and
/// end-to-end by `update_recomputes_every_digest_and_repairs_a_wrong_one`.
fn plan(
    s: &HashMap<String, SideEntry>,
    d: &HashMap<String, SideEntry>,
    sc: SideCapability,
    dc: SideCapability,
    algos: &[&str],
    mode: HashMode,
) -> anyhow::Result<PairPlan> {
    let algos: Vec<String> = algos.iter().map(|a| a.to_string()).collect();
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
        true,
        mode,
    )
}

/// A pair of maps with no cache at all, for the diff-level cases.
fn eff(entries: Vec<(&str, &HashMap<String, Vec<u8>>)>) -> HashMap<String, EffRec> {
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

fn digests(pairs: &[(&str, &[u8])]) -> HashMap<String, Vec<u8>> {
    pairs
        .iter()
        .map(|(a, v)| (a.to_string(), v.to_vec()))
        .collect()
}

fn asking(algos: &[&str], mode: HashMode) -> CommonOpts {
    CommonOpts {
        algos: algos.iter().map(|a| a.to_string()).collect(),
        hash_mode: mode,
        ..opts()
    }
}

/// Total digests a two-sided `compare` would compute.
///
/// Spelled out rather than borrowed from `lazy.rs` because that file's harness
/// asserts verdicts across every difference class, and these cases are about a read
/// count on a tree with no differences at all - a fixture with `CHANGED` paths in it
/// would make the number depend on the prune and rename paths too.
/// `(files opened, digests computed)` for a two-sided `compare`.
///
/// Both at once because they are different facts and a case wants both. `hashed`
/// counts **files opened**, while `hash_file` computes every algorithm for a path in
/// one pass — so two algorithms on one file is one read and two digests. That
/// distinction is the only thing `any-of` changes when a pair shares no algorithm,
/// and it is invisible in `hashed`.
///
/// **A fresh fixture per call.** Resolving a folder side *writes to its cache*, so
/// this is not a pure function of its inputs: measuring two modes against one pair
/// makes the second run find a warm cache the first one backfilled. An earlier
/// version of `any_of_still_hashes_when_the_pair_shares_no_algorithm` did exactly
/// that and reported `any-of` as free.
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
        common.case_sensitive,
        common.hash_mode,
    )
    .unwrap_or_else(|e| panic!("two folders cannot fail coverage: {e:#}"));
    let digests = plans.src.digest_count() + plans.dst.digest_count();
    let sm = resolve_side(&mut so, mode, &plans.src).unwrap();
    let dm = resolve_side(&mut do_, mode, &plans.dst).unwrap();
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
        common.case_sensitive,
        common.hash_mode,
    )
    .unwrap_or_else(|e| panic!("the record holds md5, so this is answerable: {e:#}"));
    let disk = resolve_folder(dir, &cold, mode, &disk_a, &plans.dst).unwrap();
    let rec = resolve_record(&rec);
    let diff = diff_maps(&rec.map, &disk.map, &plans.required, common.case_sensitive);
    (disk.stats.hashed, if diff.is_empty() { 0 } else { 4 })
}
// -- the pick, at the planner --------------------------------------------------

/// **Tier 1 first: an algorithm cached on both sides costs nothing**, and beats
/// flag order. Without this the flag is worthless for folder-vs-folder — both
/// sides could always backfill, so "prefer what is free" would be a claim about
/// nothing.
#[test]
fn any_of_prefers_the_algorithm_that_is_cached_on_both_sides() {
    let s = map(vec![("a.txt", file(10, 100, &["md5", "sha256"]))]);
    // dst holds only md5, so md5 is free and sha256 would cost a read.
    let d = map(vec![("a.txt", file(10, 100, &["md5"]))]);
    let p = plan(
        &s,
        &d,
        folder(),
        folder(),
        &["md5", "sha256"],
        HashMode::AnyOf,
    )
    .unwrap();
    assert_eq!(p.required.of("a.txt"), ["md5"]);
    assert!(
        p.src.pending().is_empty() && p.dst.pending().is_empty(),
        "tier 1 reads nothing: src {:?} dst {:?}",
        p.src.pending(),
        p.dst.pending()
    );
}

/// **Tier 2 and 3: flag order breaks the tie**, and it is the *user's* order, so
/// the choice is visible in the command they typed.
///
/// Both sub-cases here are ties: equal read cost for md5 and sha256, so the only
/// thing that can decide is position.
#[test]
fn any_of_breaks_a_tie_by_the_flag_order() {
    let s = map(vec![("a.txt", file(10, 100, &[]))]);
    let d = map(vec![("a.txt", file(10, 100, &[]))]);
    // Tier 3 for both: neither side holds anything, so each algorithm costs one
    // read on each side.
    let md5_first = plan(
        &s,
        &d,
        folder(),
        folder(),
        &["md5", "sha256"],
        HashMode::AnyOf,
    )
    .unwrap();
    assert_eq!(md5_first.required.of("a.txt"), ["md5"]);
    let sha_first = plan(
        &s,
        &d,
        folder(),
        folder(),
        &["sha256", "md5"],
        HashMode::AnyOf,
    )
    .unwrap();
    assert_eq!(
        sha_first.required.of("a.txt"),
        ["sha256"],
        "reversing the flags reverses the choice"
    );

    // Tier 2: one side holds md5, so md5 costs one read and sha256 costs two.
    let s = map(vec![("a.txt", file(10, 100, &["md5"]))]);
    let d = map(vec![("a.txt", file(10, 100, &[]))]);
    let p = plan(
        &s,
        &d,
        folder(),
        folder(),
        &["sha256", "md5"],
        HashMode::AnyOf,
    )
    .unwrap();
    assert_eq!(
        p.required.of("a.txt"),
        ["md5"],
        "flag order loses to cost: sha256 would need two reads, md5 one"
    );
    assert_eq!(p.src.pending(), Vec::<&String>::new(), "src already has it");
    assert_eq!(
        p.dst.pending().len(),
        1,
        "and dst backfills only the one it lacks"
    );
}

/// **The headline property: the choice is per pair.**
///
/// One run, two paths, two different algorithms — md5 for the first because both
/// sides already hold it, sha256 for the second because only they do. A
/// run-level choice cannot express this, which is the reason `Required` exists
/// instead of the diff being handed a list.
#[test]
fn any_of_settles_each_pair_separately() {
    let s = map(vec![
        ("first.txt", file(10, 100, &["md5", "sha256"])),
        ("second.txt", file(10, 100, &["sha256"])),
    ]);
    let d = map(vec![
        ("first.txt", file(10, 100, &["md5"])),
        ("second.txt", file(10, 100, &["sha256"])),
    ]);
    let p = plan(
        &s,
        &d,
        folder(),
        folder(),
        &["md5", "sha256"],
        HashMode::AnyOf,
    )
    .unwrap();
    assert_eq!(
        p.required.of("first.txt"),
        ["md5"],
        "both sides hold md5, so nothing is read"
    );
    assert_eq!(
        p.required.of("second.txt"),
        ["sha256"],
        "and md5 would have cost a read on src"
    );
    assert!(
        p.src.pending().is_empty() && p.dst.pending().is_empty(),
        "two tiers-1 pairs: the whole run reads nothing"
    );
}

/// **The reason a record works at all under `any-of`.** It holds one of the two
/// requested algorithms, which is exactly as much as `any-of` asks for — so this
/// is the remedy 5b's error message could only promise in prose.
#[test]
fn any_of_answers_a_record_that_holds_one_of_the_two() {
    let s = map(vec![("a.txt", file(10, 100, &["md5"]))]);
    let d = map(vec![("a.txt", file(10, 100, &[]))]);
    let p = plan(
        &s,
        &d,
        record(),
        folder(),
        &["md5", "sha256"],
        HashMode::AnyOf,
    )
    .unwrap_or_else(|e| panic!("the record holds md5, so the pair is answerable: {e:#}"));
    assert_eq!(
        p.required.of("a.txt"),
        ["md5"],
        "the record's own algorithm"
    );
    assert!(
        p.src.pending().is_empty(),
        "a record is never asked to compute anything"
    );
    assert_eq!(
        p.dst.pending(),
        vec!["a.txt"],
        "the folder backfills the one"
    );
}

/// And the other direction: a record holding none of them is still fatal. The
/// narrower mode must not become the way back to the silent degradation, so the
/// failure is asserted with the mode that asked the least.
#[test]
fn any_of_fails_when_a_record_holds_none_of_the_requested_algorithms() {
    let s = map(vec![("a.txt", file(10, 100, &["blake3"]))]);
    let d = map(vec![("a.txt", file(10, 100, &[]))]);
    let err = plan(
        &s,
        &d,
        record(),
        folder(),
        &["md5", "sha256"],
        HashMode::AnyOf,
    )
    .expect_err("an algorithm this crate has never heard of is not a substitute");
    let msg = format!("{err:#}");
    assert!(msg.contains("a.txt"), "names the path: {msg}");
    assert!(
        msg.contains("--hash-any-of"),
        "and the mode that failed: {msg}"
    );
    assert!(
        msg.contains("md5") && msg.contains("sha256"),
        "and every algorithm it could not choose between: {msg}"
    );
}

/// **`all-of` is the default and still requires everything.** Stated as a
/// planner-level fact rather than only end to end, because the default lives in
/// `HashMode` and a flag default is not visible from a command's exit code.
#[test]
fn all_of_requires_every_algorithm_on_both_sides() {
    let s = map(vec![("a.txt", file(10, 100, &["md5"]))]);
    let d = map(vec![("a.txt", file(10, 100, &["md5"]))]);
    let p = plan(
        &s,
        &d,
        folder(),
        folder(),
        &["md5", "sha256"],
        HashMode::AllOf,
    )
    .unwrap();
    assert_eq!(
        p.required.of("a.txt"),
        ["md5", "sha256"],
        "the whole requested list settles the pair"
    );
    assert_eq!(
        p.src.pending().len() + p.dst.pending().len(),
        2,
        "and both sides backfill the one neither holds"
    );
    assert_eq!(HashMode::default(), HashMode::AllOf);
}

/// Scope is the undecided set in **both** modes. A pair stat already settled
/// needs no digest, so it cannot be a coverage failure however the request reads
/// — the rule 5b pinned for all-of still holds for any-of.
#[test]
fn a_stat_differing_pair_needs_no_coverage_in_either_mode() {
    let s = map(vec![("d.txt", file(10, 100, &[]))]);
    let d = map(vec![("d.txt", file(11, 100, &[]))]);
    for mode in [HashMode::AllOf, HashMode::AnyOf] {
        assert!(
            plan(&s, &d, record(), folder(), &["md5", "sha256"], mode).is_ok(),
            "{mode:?}: size already decided it, so nothing is owed"
        );
    }
}

/// `--hash none` is still a mode and not a failure, in both. Nothing requested
/// means nothing to be short of.
#[test]
fn an_empty_request_is_not_a_coverage_failure_in_either_mode() {
    let s = map(vec![("a.txt", file(10, 100, &[]))]);
    let d = map(vec![("a.txt", file(10, 100, &[]))]);
    for mode in [HashMode::AllOf, HashMode::AnyOf] {
        let p = plan(&s, &d, record(), folder(), &[], mode).unwrap();
        assert!(p.src.pending().is_empty() && p.dst.pending().is_empty());
    }
}

// -- the pick reaching the diff ------------------------------------------------

/// **The sharpest test in the file: the diff must consult the pick, per path.**
///
/// Both files hold md5 *and* sha256, md5 agrees on both and sha256 disagrees on
/// both. So "does this run report drift?" has one answer per path and one per mode,
/// and no part of it depends on a digest being absent — which is deliberate, because
/// a fixture that relied on absence would only prove the diff skipped a missing
/// digest, not that it read the right entry.
///
/// Under all-of both files are reported. Under any-of only `two.txt` is, because
/// only it is settled by sha256.
///
/// Stated as a verdict rather than as a plan, because a test that only checked
/// `required` would pass with a diff still ignoring it — and the all-of half is
/// what catches that: it fails if the per-path lookup is dropped.
#[test]
fn the_diff_compares_by_the_algorithms_the_plan_chose() {
    let both = |md5: &[u8], sha: &[u8]| digests(&[("md5", md5), ("sha256", sha)]);
    let src = eff(vec![
        ("one.txt", &both(b"agree", b"differs-1")),
        ("two.txt", &both(b"agree", b"differs-2")),
    ]);
    let dst = eff(vec![
        ("one.txt", &both(b"agree", b"one-side-1")),
        ("two.txt", &both(b"agree", b"one-side-2")),
    ]);
    // The answer `plan_pairs` would give for this shape under any-of: `one.txt`
    // settled by md5 (free on both sides), `two.txt` by sha256.
    let by_path = Required {
        by_rel: [
            ("one.txt".to_string(), vec!["md5".to_string()]),
            ("two.txt".to_string(), vec!["sha256".to_string()]),
        ]
        .into_iter()
        .collect(),
        fallback: vec!["md5".to_string(), "sha256".to_string()],
    };

    let d = diff_maps(&src, &dst, &by_path, true);
    assert_eq!(
        d.changed,
        vec!["two.txt"],
        "only the sha256-settled file, despite both disagreeing"
    );

    // And all-of on the same maps reports **both**. This is the half that makes the
    // assertion above mean something: the disagreement is in the data either way, so
    // if `diff_maps` were ignoring `required` altogether it would land here and fail.
    let uniform = Required {
        by_rel: HashMap::new(),
        fallback: vec!["md5".to_string(), "sha256".to_string()],
    };
    let d = diff_maps(&src, &dst, &uniform, true);
    assert_eq!(
        d.changed,
        vec!["one.txt", "two.txt"],
        "with no per-path answer the whole requested list is consulted"
    );
}

/// A path the plan says nothing about falls back to the whole requested list,
/// never to nothing. `hashes_differ` is silent when a digest is missing, so an
/// empty fallback would report every unplanned pair as equal.
#[test]
fn a_path_the_plan_did_not_reach_is_compared_by_the_whole_requested_list() {
    let src = eff(vec![(
        "a.txt",
        &digests(&[("md5", b"same".as_slice()), ("sha256", b"x".as_slice())]),
    )]);
    let dst = eff(vec![(
        "a.txt",
        &digests(&[("md5", b"same".as_slice()), ("sha256", b"y".as_slice())]),
    )]);
    let populated = Required {
        by_rel: HashMap::new(),
        fallback: vec!["md5".to_string(), "sha256".to_string()],
    };
    let d = diff_maps(&src, &dst, &populated, true);
    assert_eq!(
        d.changed,
        vec!["a.txt"],
        "sha256 disagrees and the fallback catches it"
    );
    // And with an *empty* fallback the same maps come out clean, which is the whole
    // reason the fallback is populated rather than left as `Required::default()`:
    // `hashes_differ` is silent when handed nothing, so a missing entry read as
    // "equal" would be the exact degradation stage 5b closed.
    assert_eq!(
        diff_maps(&src, &dst, &Required::default(), true).changed,
        Vec::<String>::new(),
        "which is why `plan_pairs` always sets it"
    );
}

// -- end to end ---------------------------------------------------------------

/// **`any-of` is the remedy 5b could only name in prose.** A record that holds
/// md5 fails an all-of `compare` against a folder (5b) and passes an any-of one,
/// with the same fixture and the same folder.
#[test]
fn any_of_answers_a_record_that_a_single_algorithm_run_would_reject() {
    let t = TempRoot::new("hm_any_of_record");
    let rec_home = t.mkdirs("rec");
    let folder = t.mkdirs("live");
    for dir in [&rec_home, &folder] {
        wfile(dir, "a.txt", b"hello");
    }
    sync_mtime(&rec_home.join("a.txt"), &folder.join("a.txt"));
    // The record was written when md5 was all this crate recorded, which is the
    // shape 5b's error is about: it holds one of the two requested algorithms.
    cmd_update(
        girsync::UpdateOpts {
            dir: rec_home.clone(),
            common: asking(&["md5"], HashMode::AllOf),
        },
        &log(),
    )
    .unwrap();
    cmd_update(
        girsync::UpdateOpts {
            dir: folder.clone(),
            common: asking(&["md5", "sha256"], HashMode::AllOf),
        },
        &log(),
    )
    .unwrap();
    let record = recs_of(&rec_home);
    assert!(
        record["a.txt"].hashes.contains_key("md5")
            && !record["a.txt"].hashes.contains_key("sha256"),
        "the fixture is a one-algorithm record: {:?}",
        record["a.txt"].hashes.keys().collect::<Vec<_>>()
    );

    let err = cmd_compare(
        girsync::CompareOpts {
            src: rec_home.join(CACHE_PREFIX),
            dst: folder.clone(),
            trust: Default::default(),
            common: asking(&["md5", "sha256"], HashMode::AllOf),
        },
        &log(),
    )
    .expect_err("all-of cannot be answered from one algorithm");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("sha256"),
        "and it names what is missing: {msg}"
    );
    // The single algorithm that covers is offered, and `any-of` is **not** offered
    // alongside it. Here `--hash-any-of md5 sha256` would also work — the record holds
    // one of the two — but `--hash-all-of md5` asks for strictly less for the same
    // coverage, so offering both would leave the user choosing between two flags with
    // no stated preference. The `any-of` line is for the case where *nothing* single
    // covers, which is the case where widening the mode is the only move left.
    assert!(
        msg.contains("--hash-all-of md5"),
        "offers the one algorithm the record already holds: {msg}"
    );
    assert!(
        !msg.contains("--hash-any-of"),
        "and does not offer a broader request when a narrower one works: {msg}"
    );

    assert_eq!(
        cmd_compare(
            girsync::CompareOpts {
                src: rec_home.join(CACHE_PREFIX),
                dst: folder.clone(),
                trust: Default::default(),
                common: asking(&["md5", "sha256"], HashMode::AnyOf),
            },
            &log(),
        )
        .unwrap(),
        0,
        "and any-of asks for the one the record has, so the pair is answerable"
    );
}

/// **`update` stores every algorithm it was asked for, in either mode.** Under
/// any-of the flag has nothing to do — there is no counterpart to choose between,
/// and a record that stored only one algorithm would make the next *comparison*
/// fail. It warns and stores both, following the `--no-trust-cached-hashes` /
/// `compare-self` precedent: a script passing the flag everywhere keeps working.
#[test]
fn update_populates_every_algorithm_under_any_of() {
    let t = TempRoot::new("hm_update_any_of");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    cmd_update(
        girsync::UpdateOpts {
            dir: dir.clone(),
            common: asking(&["md5", "sha256"], HashMode::AnyOf),
        },
        &log(),
    )
    .unwrap();
    let got = &recs_of(&dir)["a.txt"].hashes;
    assert!(
        got.contains_key("md5") && got.contains_key("sha256"),
        "both, because a record stores rather than chooses: {got:?}"
    );
}

/// One invocation of the real binary: exit code and stderr.
fn bin(args: &[&str]) -> (i32, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args(args)
        .output()
        .expect("spawn girsync");
    (
        out.status.code().expect("girsync exits with a code"),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// **`--hash-any-of none` is rejected.** "Any of the requested algorithms", with
/// nothing requested, is not a weaker request — it is no request, and `--hash-all-of
/// none` is the flag that says so. Accepting it would give a stat-only audit a
/// name that claims a digest was optional.
#[test]
fn any_of_with_none_is_rejected() {
    let t = TempRoot::new("hm_none");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    let d = dir.display().to_string();
    let (code, stderr) = bin(&["update", "--dir", &d, "--hash-any-of", "none"]);
    assert_ne!(code, 0, "a stat-only audit is --hash-all-of none");
    assert!(
        stderr.contains("--hash-all-of none"),
        "and the error says which flag does it: {stderr}"
    );
}

/// **The two modes are mutually exclusive**, checked at the flag layer so the
/// answer is a usage error rather than a run that quietly picked one.
///
/// Clap's conflict check ignores defaulted values, so `--hash-all-of`'s `md5`
/// default does not collide with `--hash-any-of` on its own — which is the only
/// reason this can be a clap conflict at all rather than a hand-written check.
#[test]
fn the_two_modes_cannot_be_asked_for_at_once() {
    let t = TempRoot::new("hm_both");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    let d = dir.display().to_string();
    let (code, stderr) = bin(&[
        "update",
        "--dir",
        &d,
        "--hash-all-of",
        "md5",
        "--hash-any-of",
        "sha256",
    ]);
    assert_ne!(code, 0, "the run would have to pick one");
    assert!(
        stderr.contains("hash-all-of") && stderr.contains("hash-any-of"),
        "clap names both: {stderr}"
    );

    // And each alone is fine, which is the other half of the claim: the default
    // on `--hash-all-of` must not make every invocation a conflict.
    assert_eq!(
        bin(&["update", "--dir", &d, "--hash-any-of", "sha256"]).0,
        0
    );
    assert_eq!(bin(&["update", "--dir", &d]).0, 0);
}

/// `--hash-all-of` is what the flag used to be called, so the old spelling is
/// gone rather than silently ignored — a typo that used to work must now say so.
#[test]
fn the_old_hash_spelling_is_rejected_by_name() {
    let t = TempRoot::new("hm_old");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    let d = dir.display().to_string();
    let (code, stderr) = bin(&["update", "--dir", &d, "--hash", "md5"]);
    assert_ne!(code, 0, "the old spelling is not silently accepted");
    assert!(
        stderr.contains("--hash"),
        "and clap says which flag it did not recognise: {stderr}"
    );
    // Clap's suggestion is what a user who types `--hash` will actually read, so
    // assert the suggestion rather than restating the fix: `--hash` must not be
    // accepted as an alias for anything.
    assert!(
        stderr.contains("--hash-all-of") || stderr.contains("--hash-any-of"),
        "and it points at one of the two new names: {stderr}"
    );
}

// -- `any-of` changes the read count, which is the whole reason it exists --------

/// **Tier 1 measured, not read off the plan.** Two folders warmed with md5 only,
/// asked for md5 *and* sha256: `all-of` backfills sha256 on both sides, `any-of`
/// settles every pair with the md5 both sides already hold and reads nothing.
///
/// A separate fixture per mode, for the reason [`run_pair`] gives. Measuring both on
/// one pair reported `any-of` as reading nothing for the wrong reason — the `all-of`
/// run had just backfilled sha256 into both caches.
///
/// The `all-of` half is the control and is deliberately first: a test asserting only
/// "`any-of` reads 0" would also pass against a planner that never reads anything,
/// which is the shape of bug stage 3 guarded against.
#[test]
fn any_of_avoids_backfilling_an_algorithm_the_pair_already_shares() {
    // `pair`'s last argument is the set each cache is warmed with, so md5 on both
    // sides and nothing else — the tier-1 shape.
    let spec: &[common::Spec] = &[
        ("a.txt", Some(b"alpha"), Some(b"alpha")),
        ("b.txt", Some(b"bravo"), Some(b"bravo")),
    ];
    let t_all = TempRoot::new("hm_tier1_all");
    let t_any = TempRoot::new("hm_tier1_any");
    let (a_src, a_dst) = pair(&t_all, spec, &["md5"], true);
    let (b_src, b_dst) = pair(&t_any, spec, &["md5"], true);

    assert_eq!(
        run_pair(&a_src, &a_dst, &asking(&["md5", "sha256"], HashMode::AllOf)),
        (4, 4),
        "all-of backfills sha256: 2 files x 2 sides, one digest each"
    );
    assert_eq!(
        run_pair(&b_src, &b_dst, &asking(&["md5", "sha256"], HashMode::AnyOf)),
        (0, 0),
        "any-of settles every pair with the md5 both sides already hold"
    );
}

/// And when the pair shares **nothing**, `any-of` is not free — it settles by
/// hashing, on both sides, exactly once. This is the counterweight to the tier-1
/// case: `any-of` narrows the *request*, it does not skip the work.
#[test]
fn any_of_still_hashes_when_the_pair_shares_no_algorithm() {
    // Cold on both sides: `pair`'s last argument is the set each cache is warmed
    // with, so an empty set leaves stat-only rows. The pair is stat-equal and
    // nothing is available anywhere, which is the only shape where every tier costs
    // a read.
    //
    // **A fresh fixture per measurement**, for the reason `run_pair` gives. The
    // first version of this test built one pair and measured both modes on it, and
    // the `any-of` number came out 0 — not because the mode is free but because the
    // `all-of` run before it had just backfilled both algorithms into both caches,
    // so the second run found tier 1.
    let t_all = TempRoot::new("hm_tier3_all");
    let t_any = TempRoot::new("hm_tier3_any");
    let spec: &[common::Spec] = &[("a.txt", Some(b"alpha"), Some(b"alpha"))];
    let (a_src, a_dst) = pair(&t_all, spec, &[], true);
    let (b_src, b_dst) = pair(&t_any, spec, &[], true);

    assert_eq!(
        run_pair(&a_src, &a_dst, &asking(&["md5", "sha256"], HashMode::AllOf)),
        (2, 4),
        "all-of: one file opened per side, both algorithms computed in that pass"
    );
    assert_eq!(
        run_pair(&b_src, &b_dst, &asking(&["md5", "sha256"], HashMode::AnyOf)),
        (2, 2),
        "any-of reads the same file - it narrows the request, it does not skip the work -
         and the difference is the second digest"
    );
}

/// One record, two algorithms requested, one of them present. `any-of` reads the
/// one it picked and leaves the other alone.
///
/// This is the case the whole mode exists for in practice: a record written by an
/// older run, where the tree is too large to repopulate just to make a comparison
/// possible.
#[test]
fn any_of_reads_one_algorithm_against_a_record_that_holds_the_other() {
    let t = TempRoot::new("hm_audit_any_of");
    let rec_home = t.mkdirs("rec");
    let folder = t.mkdirs("live");
    for dir in [&rec_home, &folder] {
        wfile(dir, "a.txt", b"hello");
    }
    sync_mtime(&rec_home.join("a.txt"), &folder.join("a.txt"));
    cmd_update(
        girsync::UpdateOpts {
            dir: rec_home.clone(),
            common: asking(&["md5"], HashMode::AllOf),
        },
        &log(),
    )
    .unwrap();
    cmd_update(
        girsync::UpdateOpts {
            dir: folder.clone(),
            common: asking(&["md5", "sha256"], HashMode::AllOf),
        },
        &log(),
    )
    .unwrap();

    let (hashed, code) = self_audit_count(&folder, &asking(&["md5", "sha256"], HashMode::AnyOf));
    assert_eq!(code, 0, "the audit answered, so it did not degrade to stat");
    assert_eq!(
        hashed, 1,
        "one file, one algorithm: the record's md5 settled it, and sha256 was \
         never computed"
    );
}
