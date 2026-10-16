//! The A1–A17 assertion log.
//!
//! **M8.** One row per assertion *instance*, not per assertion kind — 71 chunks
//! produce 71 A6 rows — and a failing row is never dropped. A capture that failed
//! an assertion leaves the failure in place to be read, because the useful
//! artifact in that case is the one that says exactly which byte disagreed.
//!
//! ## Which failures end the run
//!
//! The spec says `capture` exits non-zero for A1–A14. A15–A17 are about the
//! *documents this tool just wrote* — the diff counts, the checksum coverage and
//! the matrix's completeness — and a failure in any of them means the bundle is
//! internally inconsistent, which is exactly as untrustworthy as a wrong digest.
//! So all seventeen are fatal here, and [`is_fatal`] records why the spec's split
//! is kept in the table rather than quietly widened.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One assertion, checked once against one subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assertion {
    /// `A1` … `A17`.
    pub id: String,
    /// What was checked, in the spec's words.
    pub assertion: String,
    /// What it was checked against: a blob path, a chunk id, a file name.
    pub subject: String,
    /// What the manifest or the plan said it should be.
    pub expected: String,
    /// What it actually is.
    pub actual: String,
    /// Whether they agree.
    pub ok: bool,
}

impl Assertion {
    /// A green row.
    pub fn pass(
        id: &'static str,
        assertion: &'static str,
        subject: &str,
        value: &str,
    ) -> Assertion {
        Assertion {
            id: id.to_string(),
            assertion: assertion.to_string(),
            subject: subject.to_string(),
            expected: value.to_string(),
            actual: value.to_string(),
            ok: true,
        }
    }

    /// A red row.
    pub fn fail(
        id: &'static str,
        assertion: &'static str,
        subject: &str,
        expected: &str,
        actual: &str,
    ) -> Assertion {
        Assertion {
            id: id.to_string(),
            assertion: assertion.to_string(),
            subject: subject.to_string(),
            expected: expected.to_string(),
            actual: actual.to_string(),
            ok: false,
        }
    }

    /// From a checked pair, which is the shape almost every assertion has.
    pub fn check(
        id: &'static str,
        assertion: &'static str,
        subject: &str,
        expected: &str,
        actual: &str,
    ) -> Assertion {
        Assertion {
            id: id.to_string(),
            assertion: assertion.to_string(),
            subject: subject.to_string(),
            expected: expected.to_string(),
            actual: actual.to_string(),
            ok: expected == actual,
        }
    }
}

/// The spec's wording for each assertion, so the log reads without the docs.
///
/// A reader looking at a failed bundle should not have to open another file to
/// find out what A11 means, and a one-line table is cheap.
///
/// `A19` is **M11's**, added after running the whole thing against the live API:
/// it is the converse of A14, and without it a capture whose two variant files
/// came out identical would pass every other assertion while holding no evidence
/// of the patch it was taken for.
pub const WORDING: [(&str, &str); 18] = [
    ("A1", "every API response captured has retcode == 0"),
    (
        "A2",
        "getBuild(new).data.tag == --new-tag; getBuild(old).data.tag == --old-tag",
    ),
    ("A3", "manifest blob md5(decompressed) == manifest.checksum"),
    (
        "A4",
        "manifest.checksum == id 32-hex suffix; id 16-hex prefix recorded as opaque",
    ),
    (
        "A5",
        "decoded manifest file_count/chunk_count/uncompressed_size reconcile with getBuild.stats",
    ),
    (
        "A6",
        "len(raw) == compressed_size; md5(raw) == compressed_md5; md5(zstd(raw)) == uncompressed_md5; len(decoded) == uncompressed_size",
    ),
    ("A7", "assemble(materialize_old) has md5 == old entry.md5"),
    ("A8", "assemble(materialize_new) has md5 == new entry.md5"),
    (
        "A9",
        "assemble(REUSE-from-old union patch_fetch) byte-equals assemble(materialize_new)",
    ),
    (
        "A10",
        "every reused chunk's old-file slice MD5 == uncompressed_md5",
    ),
    (
        "A11",
        "chunk offsets gapless from 0, ascending, sum(uncompressed_size) == size",
    ),
    (
        "A12",
        "patch_fetch disjoint from the reuse set, and a subset of materialize_new",
    ),
    (
        "A13",
        "game/config.ini re-parses with game_version == --old-tag",
    ),
    (
        "A14",
        "game/ contains no path outside expected/'s manifest set + the tool-read config files",
    ),
    ("A15", "diff/ counts equal the recomputed set differences"),
    (
        "A16",
        "checksums.json covers every file written under the artifact root, and re-hashing matches",
    ),
    (
        "A17",
        "each required matrix class is captured | covered_by | unavailable - never absent",
    ),
    (
        "A19",
        "each two-sided captured file differs between game/ and expected/ exactly when its class says it should",
    ),
];

/// The wording for an id, or a placeholder for one this build does not know.
pub fn wording(id: &str) -> &'static str {
    WORDING
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, w)| *w)
        .unwrap_or("an assertion this build does not describe")
}

/// Whether a failure of this assertion ends the run.
///
/// All seventeen, and the split in the spec is kept as a table rather than a
/// branch so that widening it later is a one-line change with the reasoning
/// still attached.
pub fn is_fatal(id: &str) -> bool {
    matches!(
        id,
        "A1" | "A2"
            | "A3"
            | "A4"
            | "A5"
            | "A6"
            | "A7"
            | "A8"
            | "A9"
            | "A10"
            | "A11"
            | "A12"
            | "A13"
            | "A14"
            | "A15"
            | "A16"
            | "A17"
            | "A19"
    )
}

/// How a run ended, as the selfcheck says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Result_ {
    /// Every assertion held.
    Pass,
    /// At least one did not.
    Fail,
    /// Assertions that could not be run at all — `--no-chunk-blobs` skips A7–A12,
    /// which is a known and declared gap rather than a failure.
    Incomplete,
}

/// `verify/selfcheck.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Selfcheck {
    /// The artifact schema version.
    pub schema_version: u32,
    /// `pass` | `fail` | `incomplete`.
    pub result: Result_,
    /// How many rows, and how they came out.
    pub counts: Counts,
    /// One row per assertion instance.
    pub assertions: Vec<Assertion>,
    /// The matrix, so the two documents cannot disagree.
    pub matrix: Vec<MatrixRow>,
}

/// The three numbers, kept consistent with the rows by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    /// How many rows.
    pub total: u64,
    /// How many held.
    pub passed: u64,
    /// How many did not.
    pub failed: u64,
}

/// A matrix row as the selfcheck repeats it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixRow {
    /// The class.
    pub class: String,
    /// What happened to it.
    pub status: String,
    /// Whether that is one of the three acceptable answers.
    pub ok: bool,
}

impl MatrixRow {
    /// Whether a `status` string is one A17 accepts.
    ///
    /// Takes the string rather than a `&MatrixRow` because M9's `verify` has a
    /// status it read out of `MANIFEST.json` and no row to hang the answer on.
    /// `over_budget` is excluded for the reason [`crate::select::MatrixRow`]
    /// gives: the caller was told to take it and the tool could not afford to.
    pub fn ok_status(status: &str) -> bool {
        matches!(status, "captured" | "covered_by" | "unavailable")
    }
}

impl Selfcheck {
    /// Build the document, deriving `counts` and `result` from the rows.
    ///
    /// Derived rather than passed in, because a selfcheck whose counts disagree
    /// with its own rows is the one document nobody can use.
    pub fn build(
        assertions: Vec<Assertion>,
        matrix: Vec<MatrixRow>,
        skipped: &[&str],
    ) -> Selfcheck {
        let failed = assertions.iter().filter(|a| !a.ok).count() as u64;
        let total = assertions.len() as u64;
        let result = if failed > 0 {
            Result_::Fail
        } else if !skipped.is_empty() {
            Result_::Incomplete
        } else {
            Result_::Pass
        };
        Selfcheck {
            schema_version: crate::SCHEMA_VERSION,
            result,
            counts: Counts {
                total,
                passed: total - failed,
                failed,
            },
            assertions,
            matrix,
        }
    }

    /// Whether every assertion held and none was skipped.
    pub fn is_pass(&self) -> bool {
        self.result == Result_::Pass
    }

    /// The failing rows, for the run log and the refusal message.
    pub fn failures(&self) -> Vec<&Assertion> {
        self.assertions.iter().filter(|a| !a.ok).collect()
    }

    /// How many rows each assertion id contributed.
    ///
    /// This is the number that makes "one row per *instance*" checkable: a
    /// capture with 38 blobs must report 38 A6 rows, not one.
    pub fn by_id(&self) -> BTreeMap<&str, u64> {
        let mut m: BTreeMap<&str, u64> = BTreeMap::new();
        for a in &self.assertions {
            *m.entry(a.id.as_str()).or_default() += 1;
        }
        m
    }

    /// The ids that could not be run at all, and why they were skipped.
    pub fn skipped(&self) -> Vec<&'static str> {
        let present: std::collections::BTreeSet<&str> =
            self.assertions.iter().map(|a| a.id.as_str()).collect();
        WORDING
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| !present.contains(id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &'static str, ok: bool) -> Assertion {
        if ok {
            Assertion::pass(id, wording(id), "s", "v")
        } else {
            Assertion::fail(id, wording(id), "s", "v", "other")
        }
    }

    #[test]
    fn counts_and_result_are_derived_from_the_rows_and_cannot_disagree() {
        let s = Selfcheck::build(
            vec![row("A6", true), row("A6", false), row("A7", true)],
            Vec::new(),
            &[],
        );
        assert_eq!(
            s.counts,
            Counts {
                total: 3,
                passed: 2,
                failed: 1
            }
        );
        assert_eq!(s.result, Result_::Fail);
        assert!(!s.is_pass());
        assert_eq!(s.failures().len(), 1);
    }

    #[test]
    fn an_all_green_run_passes_and_a_skipped_assertion_is_incomplete_not_failed() {
        let s = Selfcheck::build(vec![row("A6", true), row("A7", true)], Vec::new(), &[]);
        assert_eq!(s.result, Result_::Pass);
        assert!(s.is_pass());

        // `--no-chunk-blobs` skips A7-A12: declared, not failed.
        let s = Selfcheck::build(vec![row("A1", true)], Vec::new(), &["A7", "A8"]);
        assert_eq!(s.result, Result_::Incomplete);
        assert_eq!(s.counts.failed, 0, "a skip is not a failure");
    }

    /// One row per *instance*, which is the property that makes "38 blobs, 38 A6
    /// rows" checkable rather than aspirational.
    #[test]
    fn a_row_per_instance_is_what_by_id_reports() {
        let rows: Vec<Assertion> = (0..38).map(|_| row("A6", true)).collect();
        let s = Selfcheck::build(rows, Vec::new(), &[]);
        assert_eq!(s.by_id().get("A6"), Some(&38));
        assert_eq!(s.counts.total, 38);
    }

    /// A `skipped` list derived from the rows rather than from what the caller
    /// passed in, so the document cannot claim coverage it does not have.
    #[test]
    fn skipped_is_derived_from_the_rows_that_are_present() {
        let s = Selfcheck::build(vec![row("A1", true), row("A13", true)], Vec::new(), &[]);
        let sk = s.skipped();
        // Everything in `WORDING` that produced no row. Derived, so the count is
        // `WORDING.len() - 2` rather than a second literal to keep in step.
        assert_eq!(sk.len(), WORDING.len() - 2);
        assert!(!sk.contains(&"A1") && !sk.contains(&"A13"));
        assert!(sk.contains(&"A9"));
        // The ids are also unique, so a count is not hiding a duplicate.
        let mut dedup = sk.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(dedup.len(), sk.len(), "a duplicated id in the skipped list");
    }

    /// The eighteen ids (seventeen from the spec plus M11's A19) each have wording
    /// and each is fatal.
    ///
    /// Eighteen rather than seventeen is the point of asserting the length: a new
    /// assertion added without wording would otherwise pass, and a wording entry
    /// added without a producer would be a promise nothing keeps.
    #[test]
    fn every_assertion_id_has_wording_and_is_fatal() {
        for (id, w) in WORDING {
            assert!(!w.is_empty(), "{id} has no wording");
            assert_eq!(wording(id), w);
            assert!(is_fatal(id), "{id}");
        }
        assert_eq!(WORDING.len(), 18);
        assert_eq!(wording("A99"), "an assertion this build does not describe");
    }

    /// A19 and A18 are the two that are not the spec's, and both are named.
    #[test]
    fn the_two_added_assertions_are_present_and_distinct() {
        assert!(WORDING.iter().any(|(id, _)| *id == "A19"));
        // A18 is verify-only and deliberately has no capture-side wording: a
        // capture cannot assert it, because it is a statement about the bundle.
        assert!(
            !WORDING.iter().any(|(id, _)| *id == "A18"),
            "A18 belongs to `verify`, not to a capture's selfcheck"
        );
    }

    /// A red row must say both numbers, or a reader cannot tell which side
    /// disagreed — and that is the whole question a failed capture poses.
    #[test]
    fn a_failed_row_carries_both_sides_of_the_disagreement() {
        let a = Assertion::check("A7", wording("A7"), "game/x.dll", "d992a789", "1e210ff1");
        assert!(!a.ok);
        assert_eq!(a.expected, "d992a789");
        assert_eq!(a.actual, "1e210ff1");
        assert_eq!(a.subject, "game/x.dll");
        let j = serde_json::to_value(&a).unwrap();
        for k in ["id", "assertion", "subject", "expected", "actual", "ok"] {
            assert!(j.get(k).is_some(), "{k} is missing");
        }
    }

    #[test]
    fn the_result_serialises_lowercase_the_schema_doc_writes() {
        assert_eq!(serde_json::to_string(&Result_::Pass).unwrap(), "\"pass\"");
        assert_eq!(serde_json::to_string(&Result_::Fail).unwrap(), "\"fail\"");
        assert_eq!(
            serde_json::to_string(&Result_::Incomplete).unwrap(),
            "\"incomplete\""
        );
    }
}
