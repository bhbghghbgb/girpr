//! The table as a specification: one row per path, asserted per mode.
//!
//! The assertions below are all *derived* from the plan rather than written beside
//! it - the summary counts, the case-sensitive expectation, the record order - so
//! a new row is a new claim rather than a new number to keep in step.

use super::*;

/// Assert the plan reports what the table requires for every row, and nothing else.
///
/// Two directions, because they fail differently: a missing record means the plan
/// under-reports, an extra one means it over-reports. Both name the path.
///
/// Checked twice — once for the plan print, once for the actions — because they are
/// two claims. Doing it as one set would pass for a run that printed the plan and no
/// actions, or the actions and no plan, which is precisely the split the two-segment
/// report is about.
fn assert_plan_matches(cases: &[Case], recs: &[Value], mode: &str) {
    for (label, reported, required) in [
        (
            "rename",
            segment_pairs(recs, is_rename),
            required_renames(cases),
        ),
        (
            "plan",
            reported_plan_pairs(recs),
            required_plan_pairs(cases),
        ),
        (
            "actions",
            reported_pairs(recs),
            required_pairs(cases)
                .into_iter()
                .filter(|(e, _)| !is_rename(e))
                .collect(),
        ),
    ] {
        for (event, subject) in &required {
            assert!(
                reported.contains(&(event.clone(), subject.clone())),
                "{mode}: the {label} segment expected a `{event}` record for {subject}\n  \
                 reported: {}",
                shown(&reported)
            );
        }
        let extra: Vec<_> = reported.difference(&required).collect();
        assert!(
            extra.is_empty(),
            "{mode}: the {label} segment reported but required by no row: {extra:?}\n  \
             required: {:?}",
            required
        );
    }
}

fn shown(pairs: &BTreeSet<(String, String)>) -> String {
    pairs
        .iter()
        .map(|(e, s)| format!("{e} {s}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Assert the summary counts follow from the plan, so it cannot disagree with it.
///
/// The counts are tallied from the records rather than written out, which is what
/// lets a new row be added without a new number — and what makes a summary that
/// lies about the plan a failure here rather than something a reader has to notice.
/// The flags are literals because they are inputs echoed back, not deductions.
fn assert_summary_agrees_with_the_plan(recs: &[Value], mode: &str) {
    let s = summary_of(recs);
    let count = |event: &str| recs.iter().filter(|r| r["event"] == event).count();
    for (field, event) in [
        ("renamed", "rename"),
        ("mkdir", "mkdir"),
        ("copied", "copy"),
        ("deleted", "delete"),
        ("rmdir", "rmdir"),
    ] {
        let n = count(event);
        assert_eq!(
            s[field], n,
            "{mode}: summary {field} disagrees with the {n} `{event}` records"
        );
    }
    assert_eq!(s["missing_only"], false, "{mode}");
    assert_eq!(s["keep_extra"], false, "{mode}");
}

/// Assert the record stream is **plan, then actions in apply order, then the
/// summary**.
///
/// One check covering three properties, all of which are about *position* rather than
/// content:
///
/// - the plan segment precedes every action — the plan is printed up-front, and an
///   action before a plan record would mean the tool acted before it had said what
///   it was doing;
/// - action kinds appear in apply order and contiguously (indices never decrease);
/// - the summary is last, because a run emits exactly one and a consumer reads
///   "the last line is the summary".
///
/// This is what a golden cannot do — it cannot ask a new record kind where it
/// applies, it can only notice afterwards that the transcript changed.
fn assert_apply_order(recs: &[Value], mode: &str) {
    // Each record's segment rank, as one ordered sequence. Monotonicity of that
    // sequence is the whole property: it says the renames come first, then the
    // plan, then the actions in apply order — and, because indices never decrease,
    // that no segment is interleaved with another.
    let rank = |r: &Value| -> usize {
        let e = r["event"].as_str().unwrap_or("");
        if is_rename(e) {
            0
        } else if is_plan(e) {
            1
        } else {
            // The apply phases, in the order `APPLY_ORDER` states (which does not
            // include `rename`).
            APPLY_ORDER
                .iter()
                .position(|k| *k == e)
                .unwrap_or_else(|| panic!("{mode}: `{e}` is in no known segment"))
                + 2
        }
    };
    assert_eq!(
        recs.last().map(|r| r["event"].as_str().unwrap_or("")),
        Some("summary"),
        "{mode}: the summary is last"
    );
    let ranks: Vec<usize> = recs[..recs.len() - 1].iter().map(rank).collect();
    assert!(
        ranks.windows(2).all(|w| w[0] <= w[1]),
        "{mode}: records are out of order — renames, then the plan, then the actions \
         in apply order {APPLY_ORDER:?}\n  got: {}",
        recs[..recs.len() - 1]
            .iter()
            .map(|r| r["event"].as_str().unwrap_or("?"))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

#[test]
fn the_dry_run_does_exactly_what_the_table_says() {
    let t = temp_root("plan");
    let (src, dst) = build(&t);
    let recs = run(&src, &dst, &["--dry-run"]);

    assert_plan_matches(&cases(), &recs, "insensitive");
    assert_summary_agrees_with_the_plan(&recs, "insensitive");
    assert_apply_order(&recs, "insensitive");
    assert_eq!(
        summary_of(&recs)["dry_run"],
        true,
        "the dry run marks itself"
    );
}

/// The same table under `--case-sensitive`, with the case-only rows resolved by
/// [`sensitive_expect`] rather than by a second hand-written plan. Two rows differ
/// between the modes and both are asserted, so the rename pass cannot quietly
/// become unconditional.
#[test]
fn case_sensitive_is_the_same_table_without_the_rename_pass() {
    let t = temp_root("plan_cs");
    let (src, dst) = build(&t);
    let recs = run(&src, &dst, &["--dry-run", "--case-sensitive"]);

    let cases = cases();
    for (label, reported, required) in [
        (
            "plan",
            reported_plan_pairs(&recs),
            cases
                .iter()
                .flat_map(sensitive_plan)
                .map(|(e, s)| (e.to_string(), s))
                .collect::<BTreeSet<_>>(),
        ),
        (
            "actions",
            reported_pairs(&recs),
            cases
                .iter()
                .flat_map(sensitive_expect)
                .map(|(e, s)| (e.to_string(), s))
                .collect::<BTreeSet<_>>(),
        ),
    ] {
        assert_eq!(
            reported, required,
            "case-sensitive {label} differs from the derived expectation"
        );
    }
    assert_summary_agrees_with_the_plan(&recs, "case-sensitive");
    assert_apply_order(&recs, "case-sensitive");
    assert_eq!(summary_of(&recs)["dry_run"], true);
    assert!(
        !recs.iter().any(|r| r["event"] == "rename"),
        "there is no rename pass in this mode"
    );
}
