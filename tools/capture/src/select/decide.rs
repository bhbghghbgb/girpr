//! Deciding what to capture, and saying what was decided.
//!
//! **The narration moved here with the decision it narrates.** In `run::capture`
//! this was five lines of arithmetic followed by fifty of `tracing::` — the
//! admitted list, the refused paths, the unmet classes, the over-budget
//! rejections, and a one-line summary. Read together they are one thing: what
//! the run decided and how the operator learns about it. Split across a
//! function boundary they were a decision with a separate audit trail that
//! could fall out of step with it.
//!
//! Keeping them together is what makes a new field narrate itself: there is one
//! place where a [`Selection`] is produced and one place where it is reported,
//! so "we captured something and never said so" is not a state the code can be
//! in for long.

use super::{Budget, Selection, candidates, select};
use crate::manifest::diff::FieldDiff;

/// What the selection decided, plus the one number about the pool it came from.
pub struct Decided {
    /// The selection itself.
    pub selection: Selection,
    /// How many candidates the diffs offered.
    ///
    /// Returned rather than recomputed: `candidates()` walks every path of every
    /// field, so a caller that re-derived it would walk the manifests a second
    /// time to produce a number the decision already had.
    pub pool_len: u64,
}

/// Score every candidate, apply the budget, and report the outcome.
///
/// `max_download_bytes` becomes the budget here as well as the ceiling
/// `blobs::plan` checks later. That is deliberate and they are not redundant:
/// M5 charges `total_cost` — what admitting a capture *commits* the run to —
/// while `blobs::plan` measures `Σ compressed_size` — what the chunk store will
/// actually put on the wire. Two numbers, two ceilings, both holding.
///
/// `per_class` is floored at one: asking for zero captures of a class is not a
/// request for an empty bundle, it is a request to capture nothing while still
/// charging admission for it. Clamping here means every caller gets the same
/// reading, and `select` never has to defend against a zero.
pub fn decide(
    diffs: &[FieldDiff],
    per_class: u32,
    max_download_bytes: Option<u64>,
    files: &[String],
) -> Decided {
    let pool = candidates(diffs);
    let budget = match max_download_bytes {
        Some(n) => Budget::capped(n),
        None => Budget::unlimited(),
    };
    let selection = select(&pool, per_class.max(1) as usize, budget, files);

    for a in &selection.admitted {
        tracing::info!(
            capture_id = %a.capture_id,
            class = %a.primary_class,
            matching_field = %a.matching_field,
            cost = a.budget_cost,
            path = %a.path,
            "admitted"
        );
    }
    for p in &selection.forced_skipped {
        // Named individually: the count says something was missed and the path
        // says what, and a caller who asked for five files needs both.
        tracing::warn!(
            path = %p,
            "--files named a path neither manifest carries; nothing to capture for it"
        );
    }
    for row in &selection.matrix {
        if row.is_satisfied() {
            continue;
        }
        tracing::warn!(
            class = %row.class,
            considered = row.candidates_considered,
            reason = row.reason.as_deref().unwrap_or(""),
            "class not satisfied"
        );
    }
    for r in &selection.rejected {
        tracing::info!(
            capture_id = %r.capture_id,
            class = %r.primary_class,
            cost = r.budget_cost,
            reason = %r.reason,
            path = %r.path,
            "wanted but not funded"
        );
    }
    tracing::info!(
        pool = pool.len(),
        admitted = selection.admitted.len(),
        rejected = selection.rejected.len(),
        bytes = selection.bytes_admitted,
        budget = %selection.budget.describe(),
        "selection decided"
    );

    Decided {
        selection,
        pool_len: pool.len() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A diff whose pool holds exactly one candidate, big enough that only the
    /// budget can refuse it.
    ///
    /// **Not a stub.** The first version of this fixture had `rows: vec![]`, so
    /// `candidates()` returned an empty pool and every assertion about what got
    /// admitted passed whether or not the code under test did anything — the
    /// mutant that removed the `per_class` floor survived it. A fixture with
    /// nothing in it cannot test what happens to something.
    fn one_changed() -> Vec<FieldDiff> {
        vec![FieldDiff {
            field: "game".to_string(),
            counts: crate::manifest::diff::FieldCounts {
                common: 1,
                identical: 0,
                added: 0,
                removed: 0,
            },
            scores: vec![score("a/one.bin")],
            rows: vec![crate::manifest::diff::PathRow {
                path: "a/one.bin".to_string(),
                state: crate::manifest::diff::State::Changed,
                size: 2_000,
                score: Some(score("a/one.bin")),
            }],
            pool: Default::default(),
        }]
    }

    /// One changed path's score, spelled out in full.
    ///
    /// Old file 1 000 B, new file 2 000 B, nothing reusable - so the ranking key
    /// (`old_bytes + new_bytes`, the cost `select` charges) is 3 000. A path with
    /// a real size is what makes the budget tests able to refuse it.
    fn score(path: &str) -> crate::manifest::diff::PathScore {
        crate::manifest::diff::PathScore {
            path: path.to_string(),
            old_size: 1_000,
            new_size: 2_000,
            nchunks: 4,
            reused: 0,
            moved: 0,
            moved_up: 0,
            moved_down: 0,
            old_max_end: 0,
            same_md5: false,
            old_bytes: 1_000,
            new_bytes: 2_000,
            patch_bytes: 2_000,
            total_cost: 3_000,
        }
    }

    #[test]
    fn the_pool_is_not_empty_before_the_selection_is_asserted_about() {
        // A guard on the fixture, not on the code. Every other test here reasons
        // about what was admitted, which is only interesting if something could
        // have been — and the first version of this file had a fixture with no
        // rows, so `pool_len` was 0 and the assertions below were true of a
        // function that did nothing.
        let d = decide(&one_changed(), 1, None, &[]);
        assert_eq!(
            d.pool_len, 1,
            "the fixture must offer exactly one candidate"
        );
        assert_eq!(
            d.selection.admitted.len(),
            1,
            "and an unlimited budget must admit it"
        );
    }

    #[test]
    fn an_empty_diff_set_produces_an_empty_selection_and_still_reports() {
        // The degenerate run is still a run: it must return a usable Selection,
        // not panic, and must reach the summary line - otherwise the operator
        // sees no `selection decided` at all and cannot tell "nothing to
        // capture" from "stopped before deciding".
        let d = decide(&[], 1, None, &[]);
        let s = d.selection;
        assert!(s.admitted.is_empty());
        assert!(s.rejected.is_empty());
        assert_eq!(s.bytes_admitted, 0);
        assert!(s.budget.limit().is_none(), "no cap means no cap");
    }

    #[test]
    fn a_cap_becomes_the_budget_and_no_cap_stays_unlimited() {
        // The budget is charged against `total_cost`; `blobs::plan` later
        // measures the store's real bytes against the same flag. Both hold, and
        // this is the one that decides admission.
        assert_eq!(
            decide(&one_changed(), 1, Some(5_000), &[])
                .selection
                .budget
                .limit(),
            Some(5_000)
        );
        assert_eq!(
            decide(&one_changed(), 1, None, &[])
                .selection
                .budget
                .limit(),
            None
        );
    }

    #[test]
    fn per_class_is_floored_at_one() {
        // `--per-class 0` must not become "capture nothing". Clamping in one
        // place means `select` never has to defend against a zero, and every
        // caller gets the same reading of the flag.
        let zero = decide(&one_changed(), 0, None, &[]).selection;
        let one = decide(&one_changed(), 1, None, &[]).selection;
        assert_eq!(
            zero.admitted.len(),
            one.admitted.len(),
            "per-class 0 and per-class 1 must decide the same thing"
        );
    }
}
