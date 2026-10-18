//! The pick, at the planner: which algorithm settles a pair, and why.
//!
//! Tier 1 first — an algorithm cached on both sides costs no read — then the user
//!'s flag order. These are plan_pairs tests: they read the plan, not a run.

use super::*;

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
        StatTrust::default(),
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
        StatTrust::default(),
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
        StatTrust::default(),
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
        StatTrust::default(),
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
        StatTrust::default(),
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
/// is the remedy the all-of error message could only promise in prose.
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
        StatTrust::default(),
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
        StatTrust::default(),
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
        StatTrust::default(),
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
/// — the rule that holds for all-of holds for any-of too.
#[test]
fn a_stat_differing_pair_needs_no_coverage_in_either_mode() {
    let s = map(vec![("d.txt", file(10, 100, &[]))]);
    let d = map(vec![("d.txt", file(11, 100, &[]))]);
    for mode in [HashMode::AllOf, HashMode::AnyOf] {
        assert!(
            plan(
                &s,
                &d,
                record(),
                folder(),
                &["md5", "sha256"],
                mode,
                StatTrust::default()
            )
            .is_ok(),
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
        let p = plan(&s, &d, record(), folder(), &[], mode, StatTrust::default()).unwrap();
        assert!(p.src.pending().is_empty() && p.dst.pending().is_empty());
    }
}
