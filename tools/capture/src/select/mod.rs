//! Scoring, variant classes, and budget admission.
//!
//! **M5.** The ranking key is `total_cost` — old bytes *plus* new bytes — and
//! not `patch_bytes`, because the old file must be materialized in full for the
//! fixture to be real. That choice is what puts a 1.83 MB DLL ahead of its own
//! 274 KB patch size, and it is the difference between a cheap artifact and a
//! dishonest one.
//!
//! ## The selection pool
//!
//! `game` ∪ the selected audio fields. Every field's manifest is scored and
//! diffed, not just `game`, and ranking prefers `game` on a cost tie — so a
//! voice pack is reached only when no game file could satisfy the class, and the
//! reason a capture came from where it did is recorded either way.
//!
//! ## Admission, in three phases
//!
//! 1. **Wants.** For each class in [`Class::REQUIRED_ORDER`], take up to
//!    `--per-class` cheapest candidates that are not already captured. Stop
//!    early if something already captured satisfies the class — that is
//!    `covered_by`, and it is why the exclusion set exists.
//! 2. **Budget.** Sort the wants by cost and admit what fits; the rest are
//!    `over_budget`, recorded rather than dropped.
//! 3. **Matrix.** One row per class: captured, covered by another capture,
//!    over budget, or genuinely unavailable. Never simply absent.

pub mod budget;
pub mod classes;
pub mod decide;
pub mod require;

use serde::Serialize;

use std::collections::BTreeMap;

use crate::manifest::diff::{FieldDiff, State, by_cost, field_rank};

pub use budget::{Budget, Fate};
pub use classes::{Class, Shape};

/// One path the matrix could capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The `matching_field` the path came from.
    pub field: String,
    /// The manifest-relative path.
    pub path: String,
    /// Common / one-sided.
    pub state: State,
    /// `old_bytes + new_bytes`. Zero for a one-sided path, which is charged by
    /// its own size — see [`SizedCandidate::budget_cost`].
    pub total_cost: u64,
    /// `Σ compressed_size` a patch would fetch. Zero for a one-sided path.
    pub patch_bytes: u64,
    /// The numbers the predicates read.
    pub shape: Shape,
    /// Every class this path satisfies.
    pub classes: Vec<Class>,
}

impl Candidate {
    /// Whether this candidate satisfies `class`.
    pub fn satisfies(&self, class: Class) -> bool {
        class.matches(self.shape)
    }

    /// The primary class it was picked for, spelled `<class>` or `<class>+<n>`.
    pub fn capture_id(&self, primary: Class, nth: usize) -> String {
        if nth == 0 {
            primary.id().to_string()
        } else {
            format!("{}+{nth}", primary.id())
        }
    }
}

/// A candidate plus the size a one-sided row needs to be charged for.
///
/// A pair rather than a field, because the size is only consulted when the row
/// is one-sided — and a `Candidate` carrying a `size` it ignores nine times out
/// of ten is a field someone will eventually read on a common path and trust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizedCandidate {
    /// The candidate.
    pub candidate: Candidate,
    /// The file's declared size on whichever side exists.
    pub size: u64,
}

impl SizedCandidate {
    /// What this capture costs against `--max-download-bytes`.
    ///
    /// `total_cost` for a common path, and **the file's own size** for a
    /// one-sided one. A `removed` path has no new side to materialize, so its
    /// cost is the bytes it occupies in `game/`; an `added` path has to be
    /// fetched whole, so its cost is its compressed size. Charging `total_cost`
    /// to both would make `removed` free and `added` cost nothing, which is
    /// backwards.
    pub fn budget_cost(&self) -> u64 {
        match self.candidate.state {
            State::Added | State::Removed => self.size,
            _ => self.candidate.total_cost,
        }
    }
}

/// Why a class was not captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixStatus {
    /// A capture of its own exists for it.
    Captured,
    /// A capture taken for another class also satisfies it.
    CoveredBy,
    /// A candidate existed but did not fit the budget.
    OverBudget,
    /// No path in the pool satisfies it.
    Unavailable,
}

impl MatrixStatus {
    /// The wire spelling, as `MANIFEST.json → matrix[].status` records it.
    ///
    /// **Not** `format!("{:?}", self).to_lowercase()`, which is what this used
    /// to do and which produced `coveredby` and `overbudget`. Those are not the
    /// spellings A17 checks for and the schema doc writes, so a bundle in which
    /// one class was covered by another wrote a status its own A17 row rejected -
    /// and the run still exited `0`, because the only test that could have seen
    /// it compared against the same wrong string. `Class::id()` is the model: a
    /// variant has one spelling and it is written down here.
    pub const fn id(self) -> &'static str {
        match self {
            MatrixStatus::Captured => "captured",
            MatrixStatus::CoveredBy => "covered_by",
            MatrixStatus::OverBudget => "over_budget",
            MatrixStatus::Unavailable => "unavailable",
        }
    }

    /// Whether A17 accepts this state.
    ///
    /// **All four.** A17 is about the matrix being *complete*: every class has
    /// a row and every row says what happened. `over_budget` is one of those
    /// answers — "a candidate existed and this run's budget would not pay for
    /// it" — and excluding it would make a class the budget refused end the run
    /// with exit `3`, while `--require-class` — the flag that says "the caller
    /// wanted this class" — is the only thing that turns an unmet class into a
    /// failed run. A17 must not do `--require-class`'s job for a class nobody
    /// required.
    ///
    /// The requirement is still enforced: [`MatrixRow::is_satisfied`] excludes
    /// `over_budget`, so `--require-class` still refuses with exit `4`.
    pub const fn is_acceptable(self) -> bool {
        matches!(
            self,
            MatrixStatus::Captured
                | MatrixStatus::CoveredBy
                | MatrixStatus::OverBudget
                | MatrixStatus::Unavailable
        )
    }
}

impl std::fmt::Display for MatrixStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

#[cfg(test)]
mod status_tests {
    use super::MatrixStatus::*;

    /// The spellings are what `MANIFEST.json`, A17 and the schema doc all agree
    /// on, and they are **not** the lowercased Rust variant names.
    #[test]
    fn every_status_has_the_spelling_the_schema_documents() {
        assert_eq!(Captured.id(), "captured");
        assert_eq!(CoveredBy.id(), "covered_by");
        assert_eq!(OverBudget.id(), "over_budget");
        assert_eq!(Unavailable.id(), "unavailable");
        // The bug this replaced: `format!("{:?}", s).to_lowercase()`. It happens to be
        // right for the single-word variants, which is exactly why it survived -
        // two of the four agreed by coincidence and the two that mattered did not.
        assert_eq!(format!("{:?}", CoveredBy).to_lowercase(), "coveredby");
        assert_eq!(format!("{:?}", OverBudget).to_lowercase(), "overbudget");
        assert_ne!(CoveredBy.id(), "coveredby");
        assert_ne!(OverBudget.id(), "overbudget");
    }

    /// `Display` and `id()` cannot drift, because one calls the other.
    #[test]
    fn display_is_the_id() {
        for s in [Captured, CoveredBy, OverBudget, Unavailable] {
            assert_eq!(s.to_string(), s.id());
        }
    }

    /// A17 accepts all four states.
    #[test]
    fn every_state_is_acceptable_to_a17() {
        assert!(Captured.is_acceptable());
        assert!(CoveredBy.is_acceptable());
        assert!(OverBudget.is_acceptable());
        assert!(Unavailable.is_acceptable());
    }

    /// The two spellings cannot disagree about what A17 accepts.
    #[test]
    fn the_selfcheck_and_the_status_agree_on_what_is_acceptable() {
        for s in [Captured, CoveredBy, OverBudget, Unavailable] {
            assert_eq!(
                s.is_acceptable(),
                crate::emit::selfcheck::MatrixRow::ok_status(s.id()),
                "{s:?}: `is_acceptable` and the selfcheck's `ok_status` disagree"
            );
        }
    }
}

/// One row of the matrix: what happened to one required class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatrixRow {
    /// The class.
    pub class: Class,
    /// What happened to it.
    pub status: MatrixStatus,
    /// The capture that answers it, when one does.
    pub capture_id: Option<String>,
    /// The capture it was covered by, when it was.
    pub covered_by: Option<String>,
    /// The path that answers it, when it does.
    pub path: Option<String>,
    /// Why it is not captured, when it is not.
    pub reason: Option<String>,
    /// How many candidates in the pool satisfied the class.
    pub candidates_considered: u64,
}

impl MatrixRow {
    /// Whether this class is answered, one way or another.
    ///
    /// `over_budget` does **not** count: the caller was told to take it and the
    /// tool could not, which is exactly what `--require-class` turns into exit 4.
    pub fn is_satisfied(&self) -> bool {
        matches!(
            self.status,
            MatrixStatus::Captured | MatrixStatus::CoveredBy
        )
    }

    /// The unmet ones, for the refusal message.
    pub fn unsatisfied(&self) -> bool {
        matches!(
            self.status,
            MatrixStatus::OverBudget | MatrixStatus::Unavailable
        )
    }
}

/// What a run decided to capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// The captures, cheapest first.
    pub admitted: Vec<Admission>,
    /// Candidates that were wanted and not funded, with the cost that refused
    /// them.
    ///
    /// Kept rather than dropped because `--per-class N` routinely wants more
    /// than the ceiling can pay for, and "the second capture for `identical`
    /// would have cost 1 775 744 B" is the answer to a question someone will
    /// ask. A silent omission and a shortfall are the same bytes and very
    /// different findings.
    pub rejected: Vec<Rejection>,
    /// `--files` paths the manifests do not carry.
    ///
    /// Kept because a caller who names five files and gets one capture has
    /// otherwise no way to learn what happened to the other four. Not an error -
    /// the caller may be naming a file for a future version - but a silent
    /// omission and a refusal are the same bytes and very different findings, so
    /// the run reports the count and the names on stderr.
    pub forced_skipped: Vec<String>,
    /// `--files` paths the pool carries but which answer **no matrix class**.
    ///
    /// Separate from [`Selection::forced_skipped`] because the two are different
    /// facts: one path is absent from the manifests, the other is present and is
    /// a degenerate patch shape (a one-chunk file with no reuse, for example)
    /// that no class claims. Neither is a capture, and a caller who named the
    /// path needs to be told which one it was. Inventing a class for it would
    /// make the matrix claim something false.
    pub forced_unscored: Vec<String>,
    /// One row per required class, in admission order.
    pub matrix: Vec<MatrixRow>,
    /// Total charged against the budget.
    pub bytes_admitted: u64,
    /// The budget as it stood after admission.
    ///
    /// Carried rather than recomputed by the caller: phase 0 and phase 2 charge
    /// the same ledger, and [`Budget`] is copyable, so a caller holding its own
    /// value would still see `spent == 0` after a run that spent 882 940 B. The
    /// returned value is the one the admissions actually moved.
    pub budget: Budget,
}

/// A candidate the selection wanted and the budget would not fund.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rejection {
    /// `<class>` or `<class>+<n>` — the id it *would* have taken, so a reader
    /// can see which ordinal in the `--per-class` sequence this is.
    pub capture_id: String,
    /// The class it was picked for.
    pub primary_class: Class,
    /// The path.
    pub path: String,
    /// The `matching_field` it came from.
    pub matching_field: String,
    /// Its state.
    pub state: State,
    /// What it would have charged.
    pub budget_cost: u64,
    /// Why it did not.
    pub reason: String,
}

impl Selection {
    /// The unmet classes, in matrix order.
    pub fn unmet(&self) -> Vec<Class> {
        self.matrix
            .iter()
            .filter(|r| r.unsatisfied())
            .map(|r| r.class)
            .collect()
    }

    /// The admitted paths, as a runner-up-free list.
    pub fn paths(&self) -> Vec<&str> {
        self.admitted.iter().map(|a| a.path.as_str()).collect()
    }
}

/// One captured file and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Admission {
    /// `<class>` or `<class>+<n>`.
    pub capture_id: String,
    /// The class it was picked for.
    pub primary_class: Class,
    /// The path.
    pub path: String,
    /// The `matching_field` it came from.
    pub matching_field: String,
    /// Its state.
    pub state: State,
    /// Every class it satisfies — usually one, sometimes more.
    pub also_satisfies: Vec<Class>,
    /// What it charged against the budget.
    pub budget_cost: u64,
    /// What a patch would fetch.
    pub patch_bytes: u64,
}

impl Admission {
    /// Whether this capture answers `class`.
    pub fn satisfies(&self, class: Class) -> bool {
        self.primary_class == class || self.also_satisfies.contains(&class)
    }
}

/// Build the candidate pool from every field's diff, cheapest first.
///
/// Pure over `&[FieldDiff]`: the ordering is spec §5's — `total_cost`, then the
/// field with `game` ranked first, then `moved` desc, then `reused` desc, then
/// `path` — and one-sided paths are placed by their own size.
pub fn candidates(diffs: &[FieldDiff]) -> Vec<SizedCandidate> {
    let mut out: Vec<SizedCandidate> = Vec::new();
    for f in diffs {
        for row in &f.rows {
            let shape = match &row.score {
                Some(s) => Shape::from_row(row.state, s),
                None => match row.state {
                    State::Added => Shape::Added,
                    State::Removed => Shape::Removed,
                    // Unreachable: `diff` gives every common row a score.
                    _ => continue,
                },
            };
            out.push(SizedCandidate {
                candidate: Candidate {
                    field: f.field.clone(),
                    path: row.path.clone(),
                    state: row.state,
                    total_cost: row.score.as_ref().map(|s| s.total_cost).unwrap_or(0),
                    patch_bytes: row.score.as_ref().map(|s| s.patch_bytes).unwrap_or(0),
                    shape,
                    classes: Class::all_satisfied_by(shape),
                },
                size: row.size.max(0) as u64,
            });
        }
    }
    out.sort_by(cost_then_field_then_path);
    out
}

/// How many pool candidates satisfy each class, indexed by [`Class::ALL`].
///
/// One walk of the pool instead of one per class. The numbers are the same ones
/// the per-class `filter().count()` produced.
fn considered_counts(pool: &[SizedCandidate]) -> [u64; Class::ALL.len()] {
    let mut counts = [0_u64; Class::ALL.len()];
    for c in pool {
        for (i, class) in Class::ALL.iter().enumerate() {
            if c.candidate.satisfies(*class) {
                counts[i] += 1;
            }
        }
    }
    counts
}

/// Where a class sits in [`Class::ALL`], for indexing [`considered_counts`].
fn class_idx(class: Class) -> usize {
    Class::ALL
        .iter()
        .position(|c| *c == class)
        .expect("every class is in ALL")
}

/// Cheapest first, then the field with `game` ranked first, then by path.
///
/// **One definition, two call sites.** [`candidates`] sorts the pool with it and
/// [`select`] sorts `wants` with it, and those two orders must agree: a fate
/// list computed over one order and read at the positions of the other admitted
/// a 108 B candidate under a 100 B ceiling, exactly as the comment at the phase-2
/// sort records. The tie-break is part of the artifact, so it is spelled once.
fn cost_then_field_then_path(a: &SizedCandidate, b: &SizedCandidate) -> std::cmp::Ordering {
    a.budget_cost()
        .cmp(&b.budget_cost())
        .then_with(|| field_rank(&a.candidate.field).cmp(&field_rank(&b.candidate.field)))
        .then_with(|| a.candidate.path.cmp(&b.candidate.path))
}

/// The clause that ends both of the artifact's "the budget would not fund this"
/// sentences.
///
/// `Rejection.reason` and `MatrixRow.reason` are two *different* sentences and
/// both already appear in shipped bundles, so they cannot be unified without
/// being a band-3 change to two artifact-visible strings. What can be shared is
/// the part that would silently drift — the verb, the cost and the reason — and
/// that is what this is.
///
/// The subjects differ on purpose: a rejection names the candidate that was
/// refused, a matrix row names the cheapest candidate of a class, and a reader
/// looking at one of those rows is answering a different question.
///
/// **This string is only reachable when a budget is set.** The default capture is
/// unlimited, so the byte-diff harness cannot see it; the wording is pinned by
/// `tests/matrix.rs` instead.
fn unfunded(cost: u64) -> String {
    format!("costs {cost} B and the budget could not fund it")
}

/// How one class was answered, before the budget had its say.
///
/// The budget is what turns a plan into a fate, and the two are kept apart
/// because a `covered_by` row whose covering capture is refused has to become
/// `over_budget` — a decision about the *budget*, not about the class.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Coverage {
    /// The class has candidates of its own.
    Own,
    /// The class is answered by a capture that already exists.
    ByAdmission {
        /// The capture's id.
        capture_id: String,
        /// The path it captured.
        path: String,
    },
    /// The class is answered by a want of an earlier class, which the budget
    /// may yet refuse.
    ByWant {
        /// The index of the covering want.
        want: usize,
    },
    /// Nothing in the pool satisfies it.
    None,
}

/// Run the three admission phases.
///
/// `forced` is what `--files` asked for: already-captured paths, which take
/// precedence over everything and are what produces `covered_by` for free.
pub fn select(
    pool: &[SizedCandidate],
    per_class: usize,
    mut budget: Budget,
    forced: &[String],
) -> Selection {
    // Phase 0 — forced captures, in the caller's order, each one noted with
    // every class it happens to answer.
    //
    // **One ledger, charged here.** The caller's `Budget` is copied into this
    // function and charged for every forced admission, then handed to phase 2
    // as-is. A separate local ledger and a per-path check against the unspent
    // caller budget admitted two 60 B paths under a 100 B ceiling, because each
    // one individually satisfied `0 + 60 <= 100`; the cumulative total is the
    // number the flag is about.
    let mut admitted: Vec<Admission> = Vec::new();
    let mut forced_skipped: Vec<String> = Vec::new();
    let mut forced_unscored: Vec<String> = Vec::new();
    let mut forced_rejected: Vec<Rejection> = Vec::new();
    // **How many forced paths of each class have already taken an id.**
    //
    // A forced path is admitted with ordinal zero only when it is the *first*
    // forced path of its primary class. Two distinct paths that both answer
    // `added` would share the id `added`, so the second `patchmaps/added.json`
    // would replace the first while the run still reported two captures. The
    // ordinal is assigned here rather than at admission so the rejection record
    // carries the same id the path would have written.
    let mut forced_ordinals: BTreeMap<Class, usize> = BTreeMap::new();
    for want in forced {
        let Some(c) = pool.iter().find(|c| c.candidate.path == *want) else {
            // A path the manifests do not carry is not an error here: the caller
            // may be naming a file for a future version. It is *recorded*, which
            // is the difference between "I captured that file" and "I did not,
            // and here is the list".
            forced_skipped.push(want.clone());
            continue;
        };
        let Some(primary) = c.candidate.classes.first().copied() else {
            // The pool carries the path, but the class set is a partition and
            // this shape answers none of it (a degenerate patch). It is still a
            // path the caller named, so it is recorded rather than dropped; it
            // is not a capture, because a capture with no class would make the
            // matrix claim something false.
            forced_unscored.push(want.clone());
            continue;
        };
        let nth = forced_ordinals.get(&primary).copied().unwrap_or(0);
        forced_ordinals.insert(primary, nth + 1);
        let cost = c.budget_cost();
        if !budget.admits(cost) {
            // **Record and continue.** A `break` here would silently drop every
            // forced path after the first refusal — including a cheaper one that
            // cheaper one that would have fitted. A refusal is a finding about
            // one path, not a reason to stop reading the caller's list.
            forced_rejected.push(Rejection {
                capture_id: c.candidate.capture_id(primary, nth),
                primary_class: primary,
                path: c.candidate.path.clone(),
                matching_field: c.candidate.field.clone(),
                state: c.candidate.state,
                budget_cost: cost,
                reason: unfunded(cost),
            });
            continue;
        }
        budget.charge(cost);
        admitted.push(Admission {
            capture_id: c.candidate.capture_id(primary, nth),
            primary_class: primary,
            path: c.candidate.path.clone(),
            matching_field: c.candidate.field.clone(),
            state: c.candidate.state,
            also_satisfies: Class::all_others_satisfied_by(c.candidate.shape, primary),
            budget_cost: cost,
            patch_bytes: c.candidate.patch_bytes,
        });
    }

    // Phase 1 — wants, class by class, cheapest-first and excluding anything
    // already captured.
    //
    // The ordinal rides along because `capture_id` is `<class>` or `<class>+<n>`
    // where `n` counts *within the class*. Phase 2 re-sorts these by cost, so
    // the position a want is admitted at says nothing about which pick it was —
    // `reuse_moved+7` would be a true statement about a global cost
    // order and a useless one about `--per-class`.
    let mut wants: Vec<(Class, usize, SizedCandidate)> = Vec::new();
    let mut notes: Vec<MatrixRow> = Vec::new();
    // How each class was answered, before the budget had its say. The budget
    // is what turns a plan into a fate, and the two are kept apart because a
    // `covered_by` row whose covering capture is refused has to become
    // `over_budget` — a decision about the *budget*, not about the class.
    let mut coverage: Vec<Coverage> = Vec::new();
    // `candidates_considered` for every class, in **one** pass over the pool.
    // Nine passes — one per class — each re-walking 2 921 candidates would
    // recompute the same counts nine times.
    let considered_by_class = considered_counts(pool);
    for class in Class::REQUIRED_ORDER {
        let considered = considered_by_class[class_idx(class)];

        // Already answered by something captured for another reason?
        if let Some(cover) = admitted.iter().find(|a| a.satisfies(class)) {
            notes.push(MatrixRow {
                class,
                status: MatrixStatus::CoveredBy,
                capture_id: None,
                covered_by: None,
                path: None,
                reason: None,
                candidates_considered: considered,
            });
            coverage.push(Coverage::ByAdmission {
                capture_id: cover.capture_id.clone(),
                path: cover.path.clone(),
            });
            continue;
        }

        // Already answered by a file an earlier class is about to capture?
        //
        // **This is the case that decides the bundle's size.** The cheapest
        // `reuse_moved` candidate is the same file as the cheapest
        // `reuse_moved` one, and buying a second copy of that shape would
        // cost 24.9 MB for nothing: the two classes differ only in which of
        // the two trim outcomes the file shows, and one capture shows both.
        //
        // The covering want is the **cheapest** one that satisfies the class,
        // because the rule below — "a class that cannot have the covering
        // capture cannot have a dearer one either" — is only true of the
        // cheapest.
        let cover = wants
            .iter()
            .enumerate()
            .filter(|(_, (_, _, w))| w.candidate.satisfies(class))
            .min_by_key(|(_, (_, _, w))| w.budget_cost())
            .map(|(i, _)| i);
        if let Some(want) = cover {
            notes.push(MatrixRow {
                class,
                status: MatrixStatus::CoveredBy,
                capture_id: None,
                covered_by: None,
                path: None,
                reason: None,
                candidates_considered: considered,
            });
            coverage.push(Coverage::ByWant { want });
            continue;
        }

        let mut taken = 0_usize;
        let mut picked_any = false;
        while taken < per_class {
            let pick = pool.iter().find(|c| {
                c.candidate.satisfies(class)
                    && !admitted.iter().any(|a| a.path == c.candidate.path)
                    && !wants
                        .iter()
                        .any(|(_, _, w)| w.candidate.path == c.candidate.path)
            });
            let Some(pick) = pick else { break };
            picked_any = true;
            wants.push((class, taken, pick.clone()));
            taken += 1;
        }

        if picked_any {
            notes.push(MatrixRow {
                class,
                status: MatrixStatus::Captured,
                capture_id: None,
                covered_by: None,
                path: None,
                reason: None,
                candidates_considered: considered,
            });
            coverage.push(Coverage::Own);
        } else {
            notes.push(MatrixRow {
                class,
                status: MatrixStatus::Unavailable,
                capture_id: None,
                covered_by: None,
                path: None,
                reason: Some(unavailable_reason(class, considered)),
                candidates_considered: considered,
            });
            coverage.push(Coverage::None);
        }
    }

    // Phase 2 — the budget, smallest-first across the whole wanted set.
    //
    // The sort is computed *first* and the costs are read in that order, so the
    // fate list and the walk cannot disagree about which candidate is which.
    // Computing `fates` over `wants` order and then reading it at sorted
    // positions admitted an 108 B candidate under a 100 B ceiling, because the
    // two orders were silently out of step.
    let mut ordered: Vec<usize> = (0..wants.len()).collect();
    ordered.sort_by(|&a, &b| cost_then_field_then_path(&wants[a].2, &wants[b].2));
    let costs: Vec<u64> = ordered.iter().map(|&i| wants[i].2.budget_cost()).collect();
    // The same ledger phase 0 charged: `budget` already carries the forced
    // spend, so `--max-download-bytes` is one cumulative number, not one number
    // per phase.
    let (fates, final_budget) = budget::admit_in_order(&costs, budget);
    let mut rejected: Vec<Rejection> = forced_rejected;

    for (rank, i) in ordered.into_iter().enumerate() {
        let (class, ordinal, c) = &wants[i];
        if fates[rank] != Fate::Admitted {
            // A forced path phase 0 refused can be wanted again for its class -
            // phase 1 has to consider it, or the matrix would say `unavailable`
            // about a path that exists. Phase 2 then refuses it for the same
            // reason, and one path must not appear twice in `rejected`.
            if rejected.iter().any(|r| r.path == c.candidate.path) {
                continue;
            }
            rejected.push(Rejection {
                capture_id: c.candidate.capture_id(*class, *ordinal),
                primary_class: *class,
                path: c.candidate.path.clone(),
                matching_field: c.candidate.field.clone(),
                state: c.candidate.state,
                budget_cost: c.budget_cost(),
                reason: unfunded(c.budget_cost()),
            });
            continue;
        }
        admitted.push(Admission {
            capture_id: c.candidate.capture_id(*class, *ordinal),
            primary_class: *class,
            path: c.candidate.path.clone(),
            matching_field: c.candidate.field.clone(),
            state: c.candidate.state,
            also_satisfies: Class::all_others_satisfied_by(c.candidate.shape, *class),
            budget_cost: c.budget_cost(),
            patch_bytes: c.candidate.patch_bytes,
        });
    }
    tracing::debug!(
        budget = %final_budget.describe(),
        admitted = admitted.len(),
        rejected = rejected.len(),
        "admitted under the ceiling"
    );

    // Phase 3 — the matrix, with ids resolved.
    let mut matrix = Vec::with_capacity(notes.len());
    for (mut row, cover) in notes.into_iter().zip(coverage) {
        match cover {
            // Nothing in the pool satisfies it: the row said so in phase 1.
            Coverage::None => {}
            // A capture that already existed answers it.
            Coverage::ByAdmission { capture_id, path } => {
                row.capture_id = Some(capture_id);
                row.covered_by = Some(path.clone());
                row.path = Some(path);
                row.reason = Some("cheapest candidate already captured for another class".into());
            }
            // A want of an earlier class answers it — **if the budget funded
            // that want**. A class that cannot have the covering capture cannot
            // have a dearer one either, so the row becomes `over_budget`
            // rather than buying a second copy of the same shape.
            Coverage::ByWant { want } => {
                let (class, ordinal, c) = &wants[want];
                let capture_id = c.candidate.capture_id(*class, *ordinal);
                match admitted.iter().find(|a| a.capture_id == capture_id) {
                    Some(a) => {
                        row.status = MatrixStatus::CoveredBy;
                        row.capture_id = Some(a.capture_id.clone());
                        row.covered_by = Some(a.path.clone());
                        row.path = Some(a.path.clone());
                        row.reason =
                            Some("the cheapest candidate is captured for another class".into());
                    }
                    None => {
                        row.status = MatrixStatus::OverBudget;
                        row.reason = Some(format!(
                            "the capture that would have covered it {}",
                            unfunded(c.budget_cost())
                        ));
                    }
                }
            }
            // The class has candidates of its own: either one of them was
            // admitted, or the budget refused them all.
            Coverage::Own => match admitted.iter().find(|a| a.satisfies(row.class)) {
                Some(a) => {
                    row.status = MatrixStatus::Captured;
                    row.capture_id = Some(a.capture_id.clone());
                    row.path = Some(a.path.clone());
                }
                None => {
                    row.status = MatrixStatus::OverBudget;
                    let cheapest = wants
                        .iter()
                        .filter(|(c, _, _)| *c == row.class)
                        .map(|(_, _, c)| c.budget_cost())
                        .min()
                        .unwrap_or(0);
                    row.reason = Some(format!("the cheapest candidate {}", unfunded(cheapest)));
                }
            },
        }
        matrix.push(row);
    }

    let bytes_admitted: u64 = admitted
        .iter()
        .fold(0, |total, a| total.saturating_add(a.budget_cost));
    // The ledger and the sum describe one fact, and both saturate: a crafted
    // manifest can make two admitted costs sum past `u64::MAX`, and the summary
    // must report the ledger's number rather than panic (debug) or wrap
    // (release). A future admission path could still charge one without adding
    // an `Admission` (or the reverse), and the matrix would then report a
    // different spent total from the summary; keep the two tied in debug builds
    // so that change is caught here rather than in a bundle.
    debug_assert_eq!(
        bytes_admitted,
        final_budget.spent(),
        "the admitted sum and the charged budget must be the same number"
    );

    Selection {
        bytes_admitted,
        admitted,
        rejected,
        forced_skipped,
        forced_unscored,
        matrix,
        budget: final_budget,
    }
}

/// Why a class was not captured.
///
/// The two cases read very differently to whoever gets the artifact: "this
/// version pair has no such file" is information about the world, while
/// "everything that matched was already captured" is information about the
/// run's own ordering. Collapsing them into one message would lose that.
fn unavailable_reason(class: Class, considered: u64) -> String {
    if considered == 0 {
        format!(
            "no path in any selected matching_field satisfies `{}` ({}): {}",
            class.id(),
            class.exercises(),
            REPOOL_NOTE
        )
    } else {
        format!(
            "every candidate satisfying `{}` is already captured for another class",
            class.id()
        )
    }
}

/// The note the matrix carries when a class has no candidate at all, so a reader
/// can tell "this version pair does not have one" from "this tool looked
/// somewhere it should not have".
///
/// **It says nothing about the reference pair.** The note once pointed at
/// the spec's record of a class absent for 7.0.0 -> 7.1.0, but all six
/// classes have candidates for that pair now, so there is no such absence
/// to point at and the note is about the reader's next move instead.
const REPOOL_NOTE: &str = "widen --audio, or check the version pair";

/// `diff/candidates.json` — the scoring table, for every common path.
///
/// Column-oriented so a reader can sort without re-parsing objects, and carrying
/// `rank` so "why did it pick that?" has an answer without re-running anything.
pub fn candidates_document(diffs: &[FieldDiff]) -> serde_json::Value {
    let rows: Vec<serde_json::Value> = by_cost(diffs)
        .into_iter()
        .enumerate()
        .map(|(rank, (field, s))| {
            let all = Class::all_satisfied_by(Shape::from_row(
                if s.same_md5 {
                    State::Identical
                } else {
                    State::Changed
                },
                &s,
            ));
            serde_json::json!({
                "path": s.path,
                "matching_field": field,
                "old_size": s.old_size,
                "new_size": s.new_size,
                "nchunks": s.nchunks,
                "reused": s.reused,
                "moved": s.moved,
                "moved_later": s.moved_later,
                "moved_earlier": s.moved_earlier,
                "old_max_end": s.old_max_end,
                "same_md5": s.same_md5,
                "old_bytes": s.old_bytes,
                "new_bytes": s.new_bytes,
                "patch_bytes": s.patch_bytes,
                "total_cost": s.total_cost,
                "classes": all.iter().map(|c| c.id()).collect::<Vec<_>>(),
                "rank": rank + 1,
            })
        })
        .collect();
    serde_json::json!({
        "schema_version": crate::SCHEMA_VERSION,
        "columns": [
            "path", "matching_field", "old_size", "new_size", "nchunks", "reused",
            "moved", "moved_later", "moved_earlier", "old_max_end", "same_md5",
            "old_bytes", "new_bytes", "patch_bytes", "total_cost", "classes", "rank",
        ],
        "rows": rows,
    })
}

/// Every class in a row's `classes` list, in [`Class::ALL`] order.
///
/// Left as the linear `contains`. The plan lists it as a redundant pass, on the
/// assumption that this runs per diff row; it does not — it has one caller, and
/// it is a test. Routing it through [`Class::parse`] would replace nine string
/// comparisons with nine *fallible* parses, each of which allocates an error
/// string for an unparseable id.
pub fn classes_in_order(ids: &[&str]) -> Vec<Class> {
    Class::ALL
        .into_iter()
        .filter(|c| ids.contains(&c.id()))
        .collect()
}

#[cfg(test)]
mod tests;
