//! `--require-class`: the only thing that turns an absent class into a failed
//! run.
//!
//! **Lived inline in `run::capture`,** as ~40 lines between the selection and
//! the chunk plan. Without the flag an over-budget class is recorded and the run
//! still exits 0 — the caller asked for a best effort and got one. With it, an
//! unmet class is exit 4 and the run says which, and why.
//!
//! # Why the detail is built in one pass
//!
//! The unmet set is built by finding each required class's matrix row and
//! testing it. Building the message then did a **second** `.find()` per class
//! plus `.expect("every required class has a matrix row")` — provably
//! unreachable, since the message is built from the identical predicate over the
//! identical slice, but it was a panic in a function that takes a
//! user-supplied `--require-class`.
//!
//! So [`unmet`] returns the rows, not the classes: the rows *are* the evidence
//! for the message, and there is nothing left to look up. A `filter_map` with a
//! `?` would be the same fix wearing a disguise — it would still silently drop a
//! class from the message if the two lookups ever disagreed.

use crate::exit::ExitCode;
use crate::failure::Failure;

use super::{Class, MatrixRow, Selection};

/// The matrix rows for required classes that were not satisfied.
///
/// Empty means the requirement holds. A required class with **no** row is not
/// reported: the matrix carries one row per required class by construction, so
/// there is nothing to complain about, and inventing a complaint for a class
/// the run never considered would be a message about the matrix rather than
/// about the capture.
pub fn unmet<'a>(required: &[Class], selection: &'a Selection) -> Vec<&'a MatrixRow> {
    required
        .iter()
        .filter_map(|c| {
            selection
                .matrix
                .iter()
                .find(|r| r.class == *c)
                .filter(|r| !r.is_satisfied())
        })
        .collect()
}

/// One class's line of the failure message: `reuse_moved: over_budget (the …)`.
///
/// The status is rendered through its **artifact spelling** (`over_budget`),
/// not its Rust `Debug` spelling (`OverBudget`): the operator greps the same
/// vocabulary that `MANIFEST.json` and the schema use.
fn line(row: &MatrixRow) -> String {
    format!(
        "{}{}",
        row.status,
        row.reason
            .as_deref()
            .map(|r| format!(" ({r})"))
            .unwrap_or_default()
    )
}

/// Fail the run unless every required class was answered.
///
/// The message names each unmet class with its status **and** its reason, so the
/// operator does not have to go and find out which of four unmet classes was the
/// one the budget could not afford.
///
/// **Empty is the correct default rather than a missing one.** `--require-class`
/// is opt-in, so a class is only fatal when the caller named it. Requiring all
/// six by default would make *every* run of a pair that lacks one of them exit
/// `4` — and a class the manifests do not have is a fact about the world,
/// recorded as `status: unavailable`, not a failure of the tool.
///
/// The distinction the exit code turns on:
///
/// - **unavailable** — no path in the pool satisfies the class. A statement
///   about the manifests. Recorded, exit `0`.
/// - **over budget** — a candidate existed and `--max-download-bytes` would not
///   pay for it. A statement about *this run's* configuration. Recorded,
///   exit `0` unless the caller named the class.
///
/// The list arrives already parsed and validated: by the time it reaches here the
/// misspelt-class case is a usage error, and it is raised before any socket opens
/// rather than after a complete capture has been paid for.
pub fn check(required: &[Class], selection: &Selection) -> Result<(), Failure> {
    if required.is_empty() {
        return Ok(());
    }
    let missing = unmet(required, selection);
    if missing.is_empty() {
        return Ok(());
    }
    let classes: Vec<Class> = missing.iter().map(|r| r.class).collect();
    let detail = missing
        .iter()
        .map(|r| format!("{}: {}", r.class, line(r)))
        .collect::<Vec<_>>()
        .join("; ");
    tracing::error!(classes = ?classes, "--require-class was not met");
    Err(Failure {
        code: ExitCode::RequirementUnmet,
        detail: format!("--require-class not met: {detail}"),
        preceded_by: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::select::MatrixStatus;

    fn row(class: Class, status: MatrixStatus, reason: Option<&str>) -> MatrixRow {
        MatrixRow {
            class,
            status,
            capture_id: None,
            covered_by: None,
            path: None,
            reason: reason.map(str::to_string),
            candidates_considered: 3,
        }
    }

    fn selection_with(rows: Vec<MatrixRow>) -> Selection {
        Selection {
            admitted: Vec::new(),
            rejected: Vec::new(),
            forced_skipped: Vec::new(),
            forced_unscored: Vec::new(),
            matrix: rows,
            bytes_admitted: 0,
            budget: super::super::Budget::unlimited(),
        }
    }

    fn all_classes() -> Vec<Class> {
        Class::ALL.to_vec()
    }

    #[test]
    fn no_required_classes_is_always_satisfied() {
        // The flag's default. `unmet` over an empty requirement must be empty
        // even when the matrix is full of unmet rows, or every run without the
        // flag would fail.
        let sel = selection_with(
            all_classes()
                .into_iter()
                .map(|c| row(c, MatrixStatus::Unavailable, Some("no candidates")))
                .collect(),
        );
        assert!(unmet(&[], &sel).is_empty());
        check(&[], &sel).expect("no requirement, no failure");
    }

    #[test]
    fn a_captured_class_is_satisfied_and_a_covered_one_too() {
        // Both statuses answer the class. A run that required `identical` and
        // got it by reuse has met the requirement even though nothing was
        // downloaded for it directly.
        for status in [MatrixStatus::Captured, MatrixStatus::CoveredBy] {
            let sel = selection_with(vec![row(Class::Identical, status, None)]);
            assert!(
                check(&[Class::Identical], &sel).is_ok(),
                "{status:?} answers the class"
            );
        }
    }

    #[test]
    fn an_unmet_class_fails_with_its_status_and_its_reason() {
        // The message is the product here. A run that says "requirement not met"
        // without saying *which* class or *why* has told the operator to go and
        // look it up.
        let sel = selection_with(vec![row(
            Class::ReuseMoved,
            MatrixStatus::Unavailable,
            Some("over budget"),
        )]);
        let e = check(&[Class::ReuseMoved], &sel).expect_err("unmet");
        assert_eq!(e.code, ExitCode::RequirementUnmet);
        assert!(
            e.detail.contains("reuse_moved"),
            "the class must be named: {}",
            e.detail
        );
        assert!(
            e.detail.contains("over budget"),
            "and the reason: {}",
            e.detail
        );
    }

    #[test]
    fn the_failure_text_uses_the_artifact_spelling_not_debug() {
        // **O3.** `line` formatted the status with `{:?}`, so the operator saw
        // `OverBudget` while `MANIFEST.json`, the schema and every log line say
        // `over_budget`. The status has a `Display` impl whose only job is to
        // delegate to `id()`; the message must use it.
        let sel = selection_with(vec![row(
            Class::ReuseMoved,
            MatrixStatus::OverBudget,
            Some("the cheapest candidate cost 9000000 B"),
        )]);
        let e = check(&[Class::ReuseMoved], &sel).expect_err("over budget is unmet");
        assert!(
            e.detail.contains("reuse_moved: over_budget"),
            "the artifact spelling must appear: {}",
            e.detail
        );
        assert!(
            !e.detail.contains("OverBudget"),
            "the Rust Debug spelling must not: {}",
            e.detail
        );
    }

    #[test]
    fn several_unmet_classes_are_all_named() {
        // One pass, every class. A message that named only the first would send
        // the operator round the loop once per class, and the classes are known
        // together before the run starts.
        let sel = selection_with(vec![
            row(Class::Identical, MatrixStatus::Unavailable, Some("a")),
            row(Class::Added, MatrixStatus::Unavailable, Some("b")),
            row(Class::Removed, MatrixStatus::Captured, None),
        ]);
        let e =
            check(&[Class::Identical, Class::Added, Class::Removed], &sel).expect_err("two unmet");
        for want in ["identical", "added", "a", "b"] {
            assert!(e.detail.contains(want), "{want} missing from: {}", e.detail);
        }
        assert!(
            !e.detail.contains("removed"),
            "a satisfied class must not appear in the failure: {}",
            e.detail
        );
    }

    #[test]
    fn a_required_class_with_no_matrix_row_does_not_fail_the_run() {
        // The matrix carries one row per required class, so this cannot happen —
        // but if it did, the honest report is silence, not a complaint about the
        // matrix. A panic here would be a user-supplied flag reaching an
        // `expect`.
        let sel = selection_with(Vec::new());
        assert!(unmet(&[Class::Identical], &sel).is_empty());
        check(&[Class::Identical], &sel).expect("a class with no row is not a failure");
    }
}
