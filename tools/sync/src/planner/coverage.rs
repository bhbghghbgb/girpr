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

use crate::effective::Repopulate;
use crate::hash::{AlgoSet, HashAlg};

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
    /// Where a repopulation of this side would run.
    ///
    /// A record that **is** a folder's own `girpr-cache` lives in that folder,
    /// and that folder is what `girsync update --dir` takes. A record that is
    /// not a folder's own cache — a `girpr-cache-backup-*` copy, say — has no
    /// folder to repopulate: a repopulation there rebuilds the folder's *own*
    /// cache, which is a different file. The planner is handed a side's
    /// *contents* and not the path of the file they came from, so the caller is
    /// the one who says which case this is.
    ///
    /// **Required, in two named cases.** [`Repopulate::Folder`] is the folder a
    /// repopulation would run on and [`Repopulate::NotAFolderCache`] is a side
    /// that has none. The field is not an `Option` because a caller that left it
    /// out would lose the remedy bullet without saying so, and the bullet is the
    /// one line in this message a user pastes.
    pub repopulate: Repopulate,
    /// Which mode was asked for. Carried because the two modes fail differently and
    /// so are *remedied* differently — an all-of failure can be answered by asking
    /// for any one algorithm, which is the whole of `any-of`.
    pub mode: HashMode,
    /// rel path -> the requested algorithms it could not obtain.
    pub missing: BTreeMap<String, AlgoSet>,
    /// Requested algorithm -> (paths holding it, undecided paths).
    ///
    /// Keyed by [`HashAlg`], so the map is ordered by *canonical* order rather than
    /// by name. Those are the same thing for the two algorithms that exist — `md5`
    /// then `sha256` is both declaration order and alphabetical — which is why the
    /// rendered lines and the two remedies below are unchanged. They stop being the
    /// same the day a third algorithm is added, and at that point the remedy text
    /// reorders silently rather than failing. That is the trade, and it is stated
    /// here because the rendered message is what a user acts on.
    pub coverage: BTreeMap<HashAlg, (usize, usize)>,
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
            let names: Vec<&str> = algos.iter().map(HashAlg::as_str).collect();
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
        // Whether a repopulation would rebuild this side at all, said **before**
        // the remedy list rather than inside it: a list that is one line shorter
        // is a question the user cannot answer from the list alone, and the
        // answer is not the user's to guess.
        if let Repopulate::NotAFolderCache = &self.repopulate {
            writeln!(
                f,
                "\nThis side is not a folder's own cache, so nothing would rebuild it: a \
                 repopulation\nrebuilds a folder's own `girpr-cache`, which is a different file."
            )?;
        }
        // Remedies a user can act on without reading the rest of the message. Every
        // one of them names a flag that exists.
        writeln!(f, "\nTo answer this question, one of:")?;
        // Only worth offering when some single algorithm would actually cover the
        // undecided set — otherwise it is a command that fails the same way.
        let narrower = self
            .coverage
            .iter()
            .find(|(_, (have, total))| *total > 0 && have == total)
            .map(|(a, _)| *a);
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
                        "  - require only one of them: {}",
                        self.request("--hash-any-of")
                    )?;
                }
            }
            HashMode::AnyOf => {
                writeln!(
                    f,
                    "  - require every one of them, which is stricter but equally \
                     unanswerable:\n    {}",
                    self.request("--hash-all-of")
                )?;
            }
        }
        // The one remedy that names a path, so it is the one that has to be
        // copy-pasteable: the folder the caller named, never a placeholder.
        if let Repopulate::Folder(folder) = &self.repopulate {
            writeln!(
                f,
                "  - repopulate it, then run this again: girsync update --dir {}",
                folder.display()
            )?;
        }
        writeln!(
            f,
            "  - compare by stat only, and accept that content is unchecked: \
             --hash-all-of none"
        )?;
        Ok(())
    }
}

impl std::error::Error for Coverage {}

impl Coverage {
    /// The requested algorithms as a command line: the flag, then one occurrence
    /// of it per algorithm.
    ///
    /// **One algorithm per occurrence**, because that is what the parser takes.
    /// A flag that named two algorithms in one occurrence would be refused
    /// before the run started, so a remedy printed in that shape would be a
    /// command the user cannot paste — the one thing a remedy must not be.
    ///
    /// A helper rather than a third inline `join`, because the two remedies that
    /// name the whole request must not be able to spell it differently from each
    /// other.
    fn request(&self, flag: &str) -> String {
        self.coverage
            .keys()
            .map(|a| format!("{flag} {}", a.as_str()))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// How many uncovered paths the message names in full.
///
/// Enough to recognise the shape of the problem, few enough that the message stays
/// readable on a tree with thousands of uncovered paths — the count and the
/// per-algorithm coverage carry the rest.
const EXAMPLE_PATHS: usize = 5;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Coverage, Repopulate};
    use crate::hash::{AlgoSet, HashAlg};
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

    /// **A known algorithm under an unusual spelling is still that algorithm.**
    ///
    /// The counterpart to the case above: there the record's `sha256` sat in
    /// the foreign arm under its canonical spelling — its length was
    /// unexpected — and the planner found it by name, so only `md5` was short.
    /// Here the spelling is unusual too, and the planner must reach the same
    /// answer. A row that holds the digest is a row that *has* the algorithm;
    /// refusing it for coverage turns the user away from work they could have
    /// had, over a difference of case.
    ///
    /// Asserted by **comparing the two spellings**, so the case states what it
    /// is about — the two rows must plan identically — rather than that the run
    /// happened not to fail. A test that passed whether the planner accepted or
    /// refused the row would assert nothing.
    #[test]
    fn a_record_holding_a_known_algorithm_under_another_spelling_is_accepted() {
        let canonical = map(vec![("a.txt", cached_entry(&["sha256"]))]);
        let unusual = map(vec![("a.txt", cached_entry(&["SHA256"]))]);
        let d = map(vec![("a.txt", file(10, 100))]);

        let by_canonical = Shape::record_src(&["sha256"])
            .run(&canonical, &d, true, StatTrust::default())
            .expect("the record holds sha256, whatever it spells it");
        let by_unusual = Shape::record_src(&["sha256"])
            .run(&unusual, &d, true, StatTrust::default())
            .expect("the record holds sha256, whatever it spells it");

        assert_eq!(
            by_unusual, by_canonical,
            "the unusual spelling is the same algorithm as the canonical one"
        );
        assert!(
            by_unusual.src.pending().is_empty(),
            "a record has nothing to compute, so it owes nothing"
        );
        assert_eq!(
            by_unusual.dst.pending(),
            vec!["a.txt"],
            "and the folder side still reads the file it has no digest for"
        );
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
    /// `tools/sled2redb/tests/convert.rs` passes that flag against a record, so
    /// this is live.
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
    /// perfectly well - which is `tools/sled2redb/tests/convert.rs`'s shape,
    /// and the reason availability is a set membership test rather than a count.
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

    /// **The rendered message is the part a user acts on**, and until now only two
    /// substrings of it were pinned — the algorithm name and the coverage fraction.
    /// The rest is a command line they copy and paste: the padded per-algorithm
    /// column, and the remedies. Those are asserted whole here.
    ///
    /// Both remedies that name the whole request go through one helper, so there is
    /// no way for them to spell it differently from each other, and the padded
    /// column is `{:<12}` — which only lines up because `Display for HashAlg` uses
    /// `f.pad` rather than `f.write_str`.
    #[test]
    fn the_message_names_the_algorithms_and_the_remedies_exactly() {
        use std::collections::BTreeMap;

        let msg = format!(
            "{:#}",
            Coverage {
                side: "src",
                label: "rec".into(),
                repopulate: Repopulate::Folder(PathBuf::from("D:\\game\\old")),
                mode: HashMode::AllOf,
                missing: BTreeMap::from([(
                    "a.txt".to_string(),
                    AlgoSet::from_iter([HashAlg::Md5, HashAlg::Sha256])
                )]),
                coverage: BTreeMap::from([
                    (HashAlg::Md5, (0usize, 2usize)),
                    (HashAlg::Sha256, (1, 2)),
                ]),
            }
        );
        for want in [
            // The padded column: name left-aligned in twelve, then the fraction.
            "    md5          0/2",
            "    sha256       1/2",
            // The shortfall line: the path, then the algorithms it lacks.
            "    a.txt                            md5, sha256",
            // Neither algorithm alone covers, so the remedy is the weaker mode
            // over the whole request — the string the user will paste, and it
            // names one algorithm per occurrence because that is what the
            // parser takes.
            "  - require only one of them: --hash-any-of md5 --hash-any-of sha256",
            // And the repopulation names the folder itself, so the line is a
            // command that can be pasted as it stands.
            "  - repopulate it, then run this again: girsync update --dir D:\\game\\old",
        ] {
            assert!(
                msg.contains(want),
                "missing from the message:\n{want}\n\n{msg}"
            );
        }
    }

    /// **The remedy is a command the user can paste.**
    ///
    /// The remedy is the only command line this crate prints for the user, so
    /// the printed text and the parser have to agree about what a flag looks
    /// like. A flag takes **one algorithm per occurrence**, so a remedy that
    /// named two in one occurrence would be refused before the run started — the
    /// line would be a dead end rather than a way out.
    ///
    /// Asserted by parsing the printed text rather than by comparing it with a
    /// second spelling written out here: a string written twice is a second
    /// thing that can drift. The order the remedy prints is checked too,
    /// because the request's order is a contract with the user.
    #[test]
    fn the_remedy_is_a_command_the_parser_accepts() {
        use std::collections::BTreeMap;

        use clap::Parser;

        use crate::cli::{Cli, Cmd};
        use crate::config::CommonOpts;

        let coverage = |mode| Coverage {
            side: "src",
            label: "rec".into(),
            repopulate: Repopulate::NotAFolderCache,
            mode,
            missing: BTreeMap::new(),
            // Neither algorithm covers the undecided set, so the all-of remedy
            // is the weaker mode over the whole request rather than a narrower
            // one.
            coverage: BTreeMap::from([(HashAlg::Md5, (0usize, 2usize)), (HashAlg::Sha256, (1, 2))]),
        };

        for (mode, flag) in [
            (HashMode::AllOf, "--hash-any-of"),
            (HashMode::AnyOf, "--hash-all-of"),
        ] {
            let msg = format!("{:#}", coverage(mode));
            // The remedy line carries the command after its label, and the
            // any-of remedy puts it on the next line — so the command is what
            // follows the flag wherever it sits.
            let command = msg
                .lines()
                .find_map(|l| l.find(flag).map(|at| l[at..].trim()))
                .unwrap_or_else(|| panic!("the message offers {flag}:\n{msg}"));
            let argv: Vec<String> = ["girsync", "compare", "--src", "s", "--dst", "d"]
                .iter()
                .map(|s| s.to_string())
                .chain(command.split_whitespace().map(str::to_string))
                .collect();
            let cli = Cli::try_parse_from(&argv)
                .unwrap_or_else(|e| panic!("the remedy must parse: {command:?}: {e}"));
            let common = match cli.cmd {
                Cmd::Compare { common, .. } => common,
                _ => unreachable!("the argv names compare"),
            };
            let opts = CommonOpts::try_from(common).expect("the remedy is a valid request");
            let asked: Vec<&str> = opts.algos.iter().map(|a| a.as_str()).collect();
            assert_eq!(
                asked,
                vec!["md5", "sha256"],
                "the remedy asks for the algorithms in the order it prints them: {command}"
            );
        }
    }

    /// **The remedy is copy-pasteable, so it names a folder and not a
    /// placeholder.** The planner is handed a side's contents and not the path
    /// of the file they came from, so the caller says which folder the record
    /// lives in and the message ends with that.
    ///
    /// Two cases, and the second is the one that matters: a side that has
    /// nothing to repopulate offers **no** repopulation at all, rather than a
    /// line with a placeholder in it that nobody can paste — and it *says* why,
    /// so the shorter list is an answer rather than an absence.
    #[test]
    fn the_repopulate_remedy_names_a_folder_or_says_why_there_is_none() {
        use std::collections::BTreeMap;

        let coverage = |repopulate: Repopulate| Coverage {
            side: "src",
            label: "record D:\\game\\old\\girpr-cache".into(),
            repopulate,
            mode: HashMode::AllOf,
            missing: BTreeMap::new(),
            coverage: BTreeMap::from([(HashAlg::Md5, (0usize, 1usize))]),
        };

        let named = format!(
            "{:#}",
            coverage(Repopulate::Folder(PathBuf::from("D:\\game\\old")))
        );
        assert!(
            named.contains("girsync update --dir D:\\game\\old"),
            "the remedy names the folder the record lives in:\n{named}"
        );
        assert!(
            !named.contains("the folder this record describes"),
            "and no placeholder is left in it:\n{named}"
        );

        let unnamed = format!("{:#}", coverage(Repopulate::NotAFolderCache));
        assert!(
            !unnamed.contains("girsync update"),
            "a side with nothing to repopulate offers no repopulation:\n{unnamed}"
        );
        assert!(
            unnamed.contains("This side is not a folder's own cache"),
            "and the message says why there is none:\n{unnamed}"
        );
    }

    /// One algorithm *does* cover the undecided set, so the remedy is to narrow the
    /// request rather than weaken it — and `narrower` picks the first algorithm in
    /// map order that fully covers, which is canonical order.
    #[test]
    fn a_covered_algorithm_offers_a_narrower_request_first() {
        use std::collections::BTreeMap;

        let msg = format!(
            "{:#}",
            Coverage {
                side: "src",
                label: "rec".into(),
                repopulate: Repopulate::NotAFolderCache,
                mode: HashMode::AllOf,
                missing: BTreeMap::new(),
                coverage: BTreeMap::from([
                    (HashAlg::Md5, (0usize, 2usize)),
                    (HashAlg::Sha256, (2, 2)),
                ]),
            }
        );
        assert!(
            msg.contains("  - narrow the request to what it already holds: --hash-all-of sha256"),
            "{msg}"
        );
        assert!(
            !msg.contains("--hash-any-of"),
            "and the weaker mode is not offered when a narrower request would do: {msg}"
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
