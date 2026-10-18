//! [`Coverage`]: what a run says when it cannot answer its own question.
//!
//! The failure half of the planner, and deliberately separate from the decision
//! half. [`crate::planner::plan_pairs`] *raises* this; everything here exists to
//! tell a user which side owed what, how much of it is missing, and what would let
//! the run proceed.
//!
//! A shortfall is an error rather than a degraded verdict because the alternative is
//! the worst available: [`crate::diff::hashes_differ`] skips any algorithm either
//! side lacks, so an uncovered pair falls back to a size+mtime that had already
//! agreed and the run reports `CHANGED = 0` about content nobody read.

use std::collections::BTreeMap;
use std::fmt;

use crate::hash::HashAlg;

use super::HashMode;

/// A side that could not supply every digest the run required.
///
/// The run cannot answer its own question, so this is an error rather than a
/// degraded verdict. [`fmt::Display`] carries the message because the planner is
/// the only place that holds all four facts a user needs to act on it: the
/// shortfall, its size, where it is, and which algorithms are involved.
#[derive(Debug)]
pub struct Coverage {
    /// `src` or `dst`, as the two-sided commands name them.
    pub side: &'static str,
    /// The caller's own description of that side — a record or folder path.
    ///
    /// Owned rather than borrowed because this becomes an [`anyhow::Error`],
    /// which is `'static`; the alternative would be for every caller to keep its
    /// labels alive for the life of the error.
    pub label: String,
    /// Which mode was asked for. Carried because the two modes fail differently and
    /// so are *remedied* differently — an all-of failure can be answered by asking
    /// for any one algorithm, which is the whole of `any-of`.
    pub mode: HashMode,
    /// rel path -> the requested algorithms it could not obtain.
    pub missing: BTreeMap<String, Vec<HashAlg>>,
    /// Requested algorithm -> (paths holding it, undecided paths).
    ///
    /// Keyed by name rather than by [`HashAlg`] because the rendered line is the
    /// contract here: `{:<12} {}/{}`. See [`fmt::Display`].
    pub coverage: BTreeMap<String, (usize, usize)>,
}

impl fmt::Display for Coverage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let undecided: usize = self.coverage.values().next().map_or(0, |c| c.1);
        let flag = match self.mode {
            HashMode::AllOf => "--hash-all-of",
            HashMode::AnyOf => "--hash-any-of",
        };
        writeln!(
            f,
            "this run cannot compare: the {} side cannot supply every digest it was asked for",
            self.side
        )?;
        // Named up front, because the two modes are different requests and the
        // remedies below are mode-specific — a reader who does not know which one
        // failed cannot tell whether `--hash-any-of` is offered as the fix or as the
        // thing that was already tried.
        writeln!(f, "\n  asked under {flag}")?;
        writeln!(f, "  {}: {}", self.side, self.label)?;
        writeln!(
            f,
            "  {} of {} undecided path(s) uncovered:",
            self.missing.len(),
            undecided
        )?;
        for (rel, algos) in self.missing.iter().take(EXAMPLE_PATHS) {
            // Joined by name, so the rendered line is the list of spellings and not
            // whatever an algorithm serialises as. `AlgoSet` deliberately has no
            // `Display`: three call sites need three separators.
            let names: Vec<&str> = algos.iter().map(|a| a.as_str()).collect();
            writeln!(f, "\n    {rel:<32} {}", names.join(", "))?;
        }
        if self.missing.len() > EXAMPLE_PATHS {
            writeln!(
                f,
                "\n    ... and {} more",
                self.missing.len() - EXAMPLE_PATHS
            )?;
        }
        if undecided > 0 {
            writeln!(f, "\n  coverage over the {undecided} undecided path(s):")?;
            for (algo, (have, total)) in &self.coverage {
                writeln!(f, "    {algo:<12} {have}/{total}")?;
            }
        }
        // The requirement, stated in the mode the user actually asked for. The two
        // sentences differ in exactly one clause, and that clause is the whole
        // reason the mode exists — so it is worth the two branches.
        let requirement = match self.mode {
            HashMode::AllOf => {
                "Every requested algorithm must be available on both sides of a \
                 pair that\nsize+mtime cannot settle."
            }
            HashMode::AnyOf => {
                "At least one requested algorithm must be available on both sides \
                 of a pair that\nsize+mtime cannot settle, and none of them is."
            }
        };
        writeln!(
            f,
            "\n{requirement} This side has no filesystem to read a missing digest from, so the \
             pair\nwould fall back to size+mtime and this run would report a confident answer \
             about\ncontent it never read."
        )?;
        // Remedies a user can act on without reading the rest of the message. Every
        // one of them names a flag that exists.
        writeln!(f, "\nTo answer this question, one of:")?;
        // Only worth offering when some single algorithm would actually cover the
        // undecided set — otherwise it is a command that fails the same way.
        let narrower = self
            .coverage
            .iter()
            .find(|(_, (have, total))| *total > 0 && have == total)
            .map(|(a, _)| a.clone());
        match self.mode {
            HashMode::AllOf => {
                if let Some(a) = narrower {
                    writeln!(
                        f,
                        "  - narrow the request to what it already holds: --hash-all-of {a}"
                    )?;
                } else if self.coverage.len() > 1 {
                    writeln!(
                        f,
                        "  - require only one of them: --hash-any-of {}",
                        self.coverage.keys().cloned().collect::<Vec<_>>().join(" ")
                    )?;
                }
            }
            HashMode::AnyOf => {
                writeln!(
                    f,
                    "  - require every one of them, which is stricter but equally \
                     unanswerable:\n    --hash-all-of {}",
                    self.coverage.keys().cloned().collect::<Vec<_>>().join(" ")
                )?;
            }
        }
        writeln!(
            f,
            "  - repopulate it, then run this again: girsync update --dir <the folder \
             this record describes>"
        )?;
        writeln!(
            f,
            "  - compare by stat only, and accept that content is unchecked: \
             --hash-all-of none"
        )?;
        Ok(())
    }
}

impl std::error::Error for Coverage {}

/// How many uncovered paths the message names in full.
///
/// Enough to recognise the shape of the problem, few enough that the message stays
/// readable on a tree with thousands of uncovered paths — the count and the
/// per-algorithm coverage carry the rest.
const EXAMPLE_PATHS: usize = 5;

#[cfg(test)]
mod tests {
    use crate::planner::HashMode;
    use crate::planner::fixtures::{Shape, cached_entry, dir, file, map};
    use crate::planner::stat_trust::StatTrust;

    // -- coverage -----------------------------------------------------------
    //
    // The rule in one line: a side must hold, or be able to compute, every
    // requested algorithm for every *undecided* pair. A folder can always
    // compute, so only a record can be short - which is the property, not a
    // special case. These cases assert on the *message*, because the message is
    // the part a user acts on.

    /// A record missing a requested digest fails the run rather than letting the
    /// pair fall back to size+mtime.
    #[test]
    fn a_record_missing_a_requested_algorithm_fails_the_run() {
        let s = map(vec![("a.txt", cached_entry(&["sha256"]))]);
        let d = map(vec![("a.txt", cached_entry(&["md5", "sha256"]))]);
        let err = Shape::record_src(&["md5", "sha256"])
            .run(&s, &d, true, StatTrust::default())
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("src"), "names the side: {msg}");
        assert!(msg.contains("a.txt"), "names the path: {msg}");
        assert!(msg.contains("md5"), "names the algorithm: {msg}");
        assert!(msg.contains("1/1"), "reports coverage: {msg}");
    }

    /// **Scoped to the undecided set.** A record full of stat-differing or
    /// one-sided paths needs no coverage, and failing on it would fail runs that
    /// are perfectly answerable — which is why the check lives in the planner and
    /// not in the record loader, which sees the whole record and cannot tell which
    /// of its paths are answerable.
    #[test]
    fn coverage_is_scoped_to_the_undecided_set() {
        // Undecided but covered: fine.
        let s = map(vec![("ok.txt", cached_entry(&["md5"]))]);
        let d = map(vec![("ok.txt", cached_entry(&["md5"]))]);
        assert!(
            Shape::record_src(&["md5"])
                .run(&s, &d, true, StatTrust::default())
                .is_ok()
        );

        // Stat differs: already decided, so an empty record row costs nothing.
        let s = map(vec![("d.txt", file(10, 100))]);
        let d = map(vec![("d.txt", file(11, 100))]);
        assert!(
            Shape::record_src(&["md5"])
                .run(&s, &d, true, StatTrust::default())
                .is_ok()
        );

        // Record-only path: a MISSING verdict, decided by presence.
        let s = map(vec![("only_rec.txt", file(10, 100))]);
        let d = map(vec![]);
        assert!(
            Shape::record_src(&["md5"])
                .run(&s, &d, true, StatTrust::default())
                .is_ok()
        );

        // A dir on both sides: compared by presence, needs no digest.
        let s = map(vec![("sub", dir())]);
        let d = map(vec![("sub", dir())]);
        assert!(
            Shape::record_src(&["md5"])
                .run(&s, &d, true, StatTrust::default())
                .is_ok()
        );
    }

    /// Trust is **not** part of availability. `--no-trust-cached-hashes src` on a
    /// record asks it to re-read, which it cannot do, so it changes nothing it
    /// owes. Folding trust in would make the flag fatal for every record - and
    /// `tests/convert.rs` passes that flag against a record, so this is live.
    ///
    /// And the two halves stay independent: distrusting the *record* changes
    /// nothing, while distrusting the folder side still makes it re-read.
    #[test]
    fn distrusting_a_record_does_not_change_what_it_owes() {
        let s = map(vec![("a.txt", cached_entry(&["md5"]))]);
        let d = map(vec![("a.txt", cached_entry(&["md5"]))]);
        // The record is fully covered, so distrusting it is not a shortfall.
        assert!(
            Shape::record_src(&["md5"])
                .distrusting(true, false)
                .run(&s, &d, true, StatTrust::default())
                .is_ok()
        );
        // The folder side trusts: it already holds md5.
        let p = Shape::record_src(&["md5"])
            .distrusting(true, false)
            .run(&s, &d, true, StatTrust::default())
            .unwrap();
        assert!(p.dst.pending().is_empty());
        // The folder side distrusts, and it *can* re-read, so it does.
        let p = Shape::record_src(&["md5"])
            .distrusting(true, true)
            .run(&s, &d, true, StatTrust::default())
            .unwrap();
        assert_eq!(p.dst.pending(), vec!["a.txt"]);
    }

    /// A record that *can* meet the request is never asked to compute, because it
    /// cannot. `plan_one` skips such a side, so the plan carries no entry for it at
    /// all — which is asserted here rather than left as an unobservable consequence
    /// of the skip.
    #[test]
    fn a_covered_record_is_never_asked_to_hash() {
        let s = map(vec![("a.txt", cached_entry(&["md5"]))]);
        let d = map(vec![("a.txt", file(10, 100))]);
        let p = Shape::record_src(&["md5"])
            .run(&s, &d, true, StatTrust::default())
            .unwrap();
        assert!(
            p.src.pending().is_empty(),
            "a record has nothing to compute"
        );
        assert_eq!(p.dst.pending(), vec!["a.txt"], "the folder side pays");
    }

    /// Both sides can be records (`compare --src A/girpr-cache --dst B/girpr-cache`),
    /// and then either may be the short one. `src` is reported first so the message
    /// leads with a fixed side rather than whichever happened to be scanned first.
    #[test]
    fn two_records_report_the_src_shortfall_first() {
        let s = map(vec![("a.txt", file(10, 100))]);
        let d = map(vec![("a.txt", file(10, 100))]);
        let err = Shape::both_records(&["md5"])
            .run(&s, &d, true, StatTrust::default())
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("the src side"), "{msg}");
    }

    /// An unrequested algorithm is neither a shortfall nor a substitute. A record
    /// carrying `blake3` that this crate has never heard of covers `md5`
    /// perfectly well - which is `tests/convert.rs`'s shape, and the reason
    /// availability is a set membership test rather than a count.
    #[test]
    fn an_unrequested_algorithm_is_neither_a_shortfall_nor_a_substitute() {
        let s = map(vec![("a.txt", cached_entry(&["md5", "blake3"]))]);
        let d = map(vec![("a.txt", cached_entry(&["md5", "blake3"]))]);
        assert!(
            Shape::record_src(&["md5"])
                .run(&s, &d, true, StatTrust::default())
                .is_ok(),
            "md5 is what was asked for and it is there"
        );
        // And an algorithm the record does *not* hold is a shortfall even though
        // it holds something else.
        let s = map(vec![("a.txt", cached_entry(&["blake3"]))]);
        let d = map(vec![("a.txt", cached_entry(&["md5", "blake3"]))]);
        assert!(
            Shape::record_src(&["md5"])
                .run(&s, &d, true, StatTrust::default())
                .is_err()
        );
    }

    /// A folder is never short of anything it can reach, so `sync` - two folders -
    /// cannot fail coverage. That is what makes the shared check safe there.
    #[test]
    fn a_folder_side_is_never_uncovered_however_little_it_has_cached() {
        let s = map(vec![("a.txt", file(10, 100))]);
        let d = map(vec![("a.txt", file(10, 100))]);
        let p = Shape {
            hashable: (true, true),
            algos: &["md5", "sha256"],
            no_trust: (false, false),
            mode: HashMode::AllOf,
        }
        .run(&s, &d, true, StatTrust::default())
        .unwrap();
        assert_eq!(p.src.pending(), vec!["a.txt"]);
        assert_eq!(p.dst.pending(), vec!["a.txt"]);
    }
}
