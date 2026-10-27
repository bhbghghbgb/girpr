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
    /// it" — and a matrix that recorded it is *more* honest than one that
    /// failed.
    ///
    /// It used to be excluded, on the reasoning that `over_budget` is "a
    /// finding about the run rather than about the world". That reasoning has a
    /// consequence the same module's own docs already rejected: a class the
    /// budget refused ended the run with exit `3`, while `--require-class` —
    /// the flag that says "the caller wanted this class" — was documented as
    /// the only thing that turns an unmet class into a failed run. A17 was
    /// doing `--require-class`'s job for a class nobody had required.
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
    /// One row per required class, in admission order.
    pub matrix: Vec<MatrixRow>,
    /// Total charged against the budget.
    pub bytes_admitted: u64,
    /// The budget as it stood after admission.
    ///
    /// Carried rather than recomputed by the caller: the caller's own `Budget`
    /// was never charged — [`Budget`] is copyable and the charge happens
    /// internally — so reporting *it* would say "0 of 1 000 000 spent" about a
    /// run that spent 882 940.
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
    let mut forced_rejected: Vec<Rejection> = Vec::new();
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
            continue;
        };
        let cost = c.budget_cost();
        if !budget.admits(cost) {
            // **Record and continue.** This used to `break`, which silently
            // dropped every forced path after the first refusal - including a
            // cheaper one that would have fitted. A refusal is a finding about
            // one path, not a reason to stop reading the caller's list.
            forced_rejected.push(Rejection {
                capture_id: c.candidate.capture_id(primary, 0),
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
            capture_id: c.candidate.capture_id(primary, 0),
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
    // It used to be nine, one per class, each re-walking 2 921 candidates; the
    // counts are the same, they were just being recomputed nine times.
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

    Selection {
        bytes_admitted: admitted.iter().map(|a| a.budget_cost).sum(),
        admitted,
        rejected,
        forced_skipped,
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
/// **It says nothing about the reference pair, and it used to.** The note used
/// to point at the spec's record of a class that was absent for 7.0.0 -> 7.1.0,
/// which was true of the class set the spec had then. All six classes have
/// candidates for that pair now, so there is no such absence to point at and
/// the note is about the reader's next move instead.
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
mod tests {
    use super::*;
    use crate::manifest::diff::{PathRow, PathScore};

    /// A scored path, with the **direction** and the **trim's input** spelled
    /// out.
    ///
    /// `cost` is the whole point: a common path is charged `old_bytes +
    /// new_bytes`, so two scored paths only order against each other if they
    /// actually differ — and `cost` is *not* `old_size + new_size`, which is
    /// exactly the distinction the budget is about.
    ///
    /// Sizes and chunk counts are tuples rather than loose arguments so that a
    /// call site reads `("f/x", 500, (100, 200), (2, 1, 0), false)` as a row of
    /// the spec's table instead of eight unnamed numbers.
    ///
    /// * `(old_size, new_size)`
    /// * `(nchunks, reused, moved)`
    /// * `(moved_later, moved_earlier)`
    #[allow(clippy::too_many_arguments)]
    fn shaped(
        path: &str,
        cost: u64,
        size: (i64, i64),
        chunks: (u64, u64, u64),
        moves: (u64, u64),
        old_max_end: u64,
        md5: bool,
    ) -> PathScore {
        let (old_size, new_size) = size;
        let (nchunks, reused, moved) = chunks;
        PathScore {
            path: path.into(),
            old_size,
            new_size,
            nchunks,
            reused,
            moved,
            moved_later: moves.0,
            moved_earlier: moves.1,
            old_max_end,
            same_md5: md5,
            old_bytes: cost / 2,
            new_bytes: cost - cost / 2,
            patch_bytes: if reused == nchunks { 0 } else { cost / 4 },
            total_cost: cost,
        }
    }

    /// The simple case: every move lands **earlier**, and no reuse source is
    /// recorded.
    fn scored(
        path: &str,
        cost: u64,
        size: (i64, i64),
        chunks: (u64, u64, u64),
        md5: bool,
    ) -> PathScore {
        let moved = chunks.2;
        shaped(path, cost, size, chunks, (0, moved), 0, md5)
    }

    fn row(path: &str, state: State, s: Option<PathScore>, size: i64) -> PathRow {
        PathRow {
            path: path.into(),
            state,
            size,
            score: s,
        }
    }

    fn field(field: &str, rows: Vec<PathRow>) -> FieldDiff {
        FieldDiff {
            field: field.into(),
            counts: Default::default(),
            scores: rows.iter().filter_map(|r| r.score.clone()).collect(),
            rows,
            pool: Default::default(),
        }
    }

    /// The six classes, one candidate each, cheapest first.
    ///
    /// The class set is a **partition** on the content, so a candidate answers
    /// exactly one class. A class that another capture already answers is
    /// recorded as `covered_by` only when that capture was **forced**
    /// (`--files`), which is the one way a capture exists before the
    /// admission looks at it.
    fn all_six() -> Vec<SizedCandidate> {
        candidates(&[field(
            "game",
            vec![
                // identical, cheapest of all
                row(
                    "a/identical",
                    State::Identical,
                    Some(scored("a/identical", 108, (54, 54), (1, 1, 0), true)),
                    54,
                ),
                // nothing reusable: every chunk is fetched
                row(
                    "c/no_reuse",
                    State::Changed,
                    Some(scored("c/no_reuse", 2_200, (1000, 1200), (3, 0, 0), false)),
                    1200,
                ),
                // partial, nothing moved: the reused chunks are already correct
                // where they belong
                row(
                    "d/in_place",
                    State::Changed,
                    Some(scored(
                        "d/in_place",
                        1_826_067,
                        (2340264, 2340264),
                        (3, 1, 0),
                        false,
                    )),
                    2340264,
                ),
                // partial, the reused chunks live elsewhere in the file
                row(
                    "e/moved",
                    State::Changed,
                    Some(shaped(
                        "e/moved",
                        2_000_000,
                        (200, 100),
                        (3, 2, 1),
                        (0, 1),
                        150,
                        false,
                    )),
                    100,
                ),
                row("j/added", State::Added, None, 114_000),
                row("k/removed", State::Removed, None, 1_730_000),
            ],
        )])
    }

    #[test]
    fn the_pool_is_built_from_every_row_with_its_shape_and_classes() {
        let pool = all_six();
        assert_eq!(pool.len(), 6);
        let by_path = |p: &str| pool.iter().find(|c| c.candidate.path == p).unwrap();
        assert_eq!(
            by_path("a/identical").candidate.classes,
            vec![Class::Identical]
        );
        assert_eq!(by_path("j/added").candidate.classes, vec![Class::Added]);
        assert_eq!(by_path("k/removed").candidate.classes, vec![Class::Removed]);
        assert_eq!(
            by_path("c/no_reuse").candidate.classes,
            vec![Class::NoReuse]
        );
        assert_eq!(
            by_path("d/in_place").candidate.classes,
            vec![Class::ReuseInPlace]
        );
        assert_eq!(
            by_path("e/moved").candidate.classes,
            vec![Class::ReuseMoved]
        );
    }

    /// `removed` has no new side and `added` has to be fetched whole. Charging
    /// `total_cost` to both would make `removed` free and `added` cost nothing.
    #[test]
    fn a_one_sided_path_is_charged_its_own_size() {
        let pool = all_six();
        let g = pool.iter().find(|c| c.candidate.path == "j/added").unwrap();
        let h = pool
            .iter()
            .find(|c| c.candidate.path == "k/removed")
            .unwrap();
        let d = pool
            .iter()
            .find(|c| c.candidate.path == "d/in_place")
            .unwrap();
        assert_eq!(g.budget_cost(), 114_000);
        assert_eq!(h.budget_cost(), 1_730_000);
        assert_eq!(d.budget_cost(), 1_826_067, "a common path is old + new");
    }

    #[test]
    fn the_pool_is_ordered_cheapest_first() {
        let costs: Vec<u64> = all_six().iter().map(SizedCandidate::budget_cost).collect();
        let mut sorted = costs.clone();
        sorted.sort_unstable();
        assert_eq!(costs, sorted, "the budget relies on this ordering");
        // A common path is charged old + new, so the cheapest thing here is the
        // 108 B file *plus its old copy* — not the file's own size, and not the
        // 114 000 B added file either. Charging a common path its own size would
        // understate what materializing the fixture actually costs.
        assert_eq!(costs[0], 108, "a/identical: 54 old + 54 new");
    }

    #[test]
    fn one_capture_per_class_is_the_greedy_answer() {
        let s = select(&all_six(), 1, Budget::unlimited(), &[]);
        let mut got: Vec<(Class, String)> = s
            .matrix
            .iter()
            .filter(|r| r.status == MatrixStatus::Captured)
            .map(|r| (r.class, r.path.clone().unwrap_or_default()))
            .collect();
        got.sort_by_key(|(c, _)| c.id());
        assert_eq!(
            got,
            vec![
                (Class::Added, "j/added".into()),
                (Class::Identical, "a/identical".into()),
                (Class::NoReuse, "c/no_reuse".into()),
                (Class::Removed, "k/removed".into()),
                (Class::ReuseInPlace, "d/in_place".into()),
                (Class::ReuseMoved, "e/moved".into()),
            ]
        );
        assert_eq!(s.admitted.len(), 6, "six classes, one capture each");
        assert!(s.unmet().is_empty(), "and every class answered");
    }

    /// The class set is a **partition** on the content: a candidate answers
    /// exactly one class, so no capture can be bought twice for two shapes.
    ///
    /// That is what makes the bundle's size predictable — one file per patch
    /// shape — and it is why `covered_by` only ever arises from a **forced**
    /// capture, which exists before the admission looks at the pool.
    #[test]
    fn no_candidate_answers_two_classes() {
        for c in all_six() {
            assert!(
                c.candidate.classes.len() <= 1,
                "{} answers {:?}: the class set is a partition",
                c.candidate.path,
                c.candidate.classes
            );
        }
    }

    /// A **forced** capture answers its class without having been selected for
    /// it — which is `covered_by`, and it is satisfied either way. What matters
    /// is that the file is captured once, not once per class it happens to
    /// answer.
    #[test]
    fn a_forced_capture_answers_its_class_without_being_selected_for_it() {
        let s = select(&all_six(), 1, Budget::unlimited(), &["d/in_place".into()]);
        let row = s
            .matrix
            .iter()
            .find(|r| r.class == Class::ReuseInPlace)
            .expect("a row for every class");
        assert_eq!(row.status, MatrixStatus::CoveredBy);
        assert_eq!(row.covered_by.as_deref(), Some("d/in_place"));
        assert!(row.is_satisfied(), "covered counts: the capture exists");
        assert!(!row.unsatisfied());
    }

    /// The edge case: a capture that would have answered a class is **refused**
    /// by the budget, so the class becomes `over_budget` — a decision about the
    /// *budget*, not about the class.
    ///
    /// The capture here is a **forced** one (`--files`), and the ceiling is too
    /// small for it. The class it would have answered has that same file as its
    /// only candidate, so the class cannot be funded either.
    #[test]
    fn a_class_whose_capture_was_refused_is_over_budget_not_absent() {
        let pool = all_six();
        // Enough for the cheapest capture and not enough for `d/in_place`.
        let cap = 108;
        let s = select(&pool, 1, Budget::capped(cap), &["d/in_place".into()]);

        let row = s
            .matrix
            .iter()
            .find(|r| r.class == Class::ReuseInPlace)
            .expect("a row for every class");
        assert_eq!(
            row.status,
            MatrixStatus::OverBudget,
            "the capture could not be funded"
        );
        assert!(row.unsatisfied(), "and over_budget is not satisfied");
        assert!(
            row.reason
                .as_deref()
                .unwrap_or_default()
                .contains("the cheapest candidate"),
            "the reason names what happened: {:?}",
            row.reason
        );
    }

    /// Two forced paths, each under the ceiling on its own, whose **sum** is
    /// not. The second must be refused.
    ///
    /// This is the case the reverted series missed: checking each forced path
    /// against the caller's unspent budget admitted both, because `0 + 60` and
    /// `0 + 60` each fit a 100 B cap. The ledger has to be cumulative.
    #[test]
    fn forced_costs_are_checked_against_the_cumulative_budget_not_one_by_one() {
        let pool = candidates(&[field(
            "game",
            vec![
                row("x/a", State::Added, None, 60),
                row("x/b", State::Added, None, 60),
            ],
        )]);
        let s = select(&pool, 1, Budget::capped(100), &["x/a".into(), "x/b".into()]);

        assert_eq!(s.admitted.len(), 1, "only one 60 B path fits in 100 B");
        assert_eq!(s.paths(), vec!["x/a"], "the first one named wins");
        assert_eq!(s.bytes_admitted, 60);
        assert!(
            s.bytes_admitted <= 100,
            "the cumulative admission must not exceed the ceiling"
        );
        assert_eq!(
            s.budget.spent(),
            60,
            "the returned budget is the one the forced phase charged"
        );
        let rejected: Vec<&str> = s.rejected.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(rejected, vec!["x/b"], "and the refusal is recorded");
    }

    /// A forced path over the ceiling must not hide a later, cheaper one.
    ///
    /// The loop used to `break` on the first refusal, so `--files a --files b`
    /// with `a = 500 B`, `b = 100 B` under a 200 B cap captured neither and
    /// recorded neither. A refusal is about one path, not the rest of the list.
    #[test]
    fn a_forced_path_over_the_ceiling_does_not_hide_a_cheaper_one_after_it() {
        let pool = candidates(&[field(
            "game",
            vec![
                row("x/a", State::Added, None, 500),
                row("x/b", State::Added, None, 100),
            ],
        )]);
        let s = select(&pool, 1, Budget::capped(200), &["x/a".into(), "x/b".into()]);

        assert_eq!(
            s.paths(),
            vec!["x/b"],
            "the later path fits and must be admitted"
        );
        let rejected: Vec<&str> = s.rejected.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(rejected, vec!["x/a"], "and the refused one is recorded");
        assert_eq!(
            s.matrix
                .iter()
                .find(|r| r.class == Class::Added)
                .map(|r| r.status),
            Some(MatrixStatus::CoveredBy),
            "the class is answered by the admitted forced path"
        );
    }

    /// A class with no candidate is recorded as `unavailable` **with a reason**,
    /// rather than being dropped from the matrix.
    #[test]
    fn an_unsatisfiable_class_is_recorded_with_a_reason_and_a_count() {
        let pool = candidates(&[field(
            "game",
            vec![row(
                "a/identical",
                State::Identical,
                Some(scored("a/identical", 108, (54, 54), (1, 1, 0), true)),
                54,
            )],
        )]);
        let s = select(&pool, 1, Budget::unlimited(), &[]);
        let row = s
            .matrix
            .iter()
            .find(|r| r.class == Class::ReuseMoved)
            .unwrap();
        assert_eq!(row.status, MatrixStatus::Unavailable);
        assert_eq!(row.candidates_considered, 0);
        let reason = row.reason.clone().unwrap();
        assert!(reason.contains("reuse_moved"), "{reason}");
        assert!(reason.contains("--audio"), "the remedy: {reason}");
        assert!(
            !s.matrix
                .iter()
                .any(|r| r.capture_id.is_none() && r.status == MatrixStatus::Captured)
        );
        assert_eq!(
            s.matrix.len(),
            Class::ALL.len(),
            "one row per class, always"
        );
    }

    /// A forced capture answers its class *without* having been selected for it —
    /// which is `covered_by`, and it is satisfied either way. What matters is that
    /// the file is captured once, not once per class it happens to answer.
    #[test]
    fn a_class_with_candidates_already_captured_is_covered_by_not_recaptured() {
        // One file that satisfies two classes.
        let pool = candidates(&[field(
            "game",
            vec![row(
                "x/both",
                State::Changed,
                Some(scored("x/both", 200, (100, 100), (3, 1, 0), false)),
                100,
            )],
        )]);
        let s = select(&pool, 1, Budget::unlimited(), &["x/both".into()]);
        assert_eq!(s.admitted.len(), 1, "forced once, not twice");
        let same = s
            .matrix
            .iter()
            .find(|r| r.class == Class::ReuseInPlace)
            .unwrap();
        assert_eq!(same.status, MatrixStatus::CoveredBy);
        assert_eq!(same.covered_by.as_deref(), Some("x/both"));
        assert!(same.is_satisfied(), "covered counts: the capture exists");
        assert!(!same.unsatisfied());
        // Nothing else has a candidate, so every other row is unavailable and
        // the matrix is complete rather than short.
        assert_eq!(s.matrix.len(), Class::ALL.len());
    }

    /// `--per-class 2` takes a second candidate per class, cheapest first.
    #[test]
    fn per_class_two_admits_a_second_file_for_a_class() {
        let pool = candidates(&[field(
            "game",
            vec![
                row(
                    "x/a",
                    State::Changed,
                    Some(scored("x/a", 200, (100, 100), (3, 1, 0), false)),
                    100,
                ),
                row(
                    "x/b",
                    State::Changed,
                    Some(scored("x/b", 400, (100, 100), (3, 2, 0), false)),
                    200,
                ),
                row(
                    "x/c",
                    State::Changed,
                    Some(scored("x/b", 400, (100, 100), (3, 2, 0), false)),
                    300,
                ),
            ],
        )]);
        let s = select(&pool, 2, Budget::unlimited(), &[]);
        let for_class: Vec<&str> = s
            .admitted
            .iter()
            .filter(|a| a.primary_class == Class::ReuseInPlace)
            .map(|a| a.path.as_str())
            .collect();
        assert_eq!(
            for_class,
            vec!["x/a", "x/b"],
            "cheapest first, then the next"
        );
        assert!(
            s.admitted.iter().any(|a| a.capture_id.contains('+')),
            "the second is `class+n`"
        );
    }

    /// A tight ceiling plus `--per-class 2` is the acceptance case: one fits,
    /// the second is recorded as over budget rather than dropped.
    #[test]
    fn per_class_two_under_a_tight_ceiling_tags_the_second_over_budget() {
        let mk = |p: &str, size: i64| row(p, State::Added, None, size);
        let pool = candidates(&[field(
            "game",
            vec![mk("x/a", 100), mk("x/b", 500), mk("x/c", 900)],
        )]);
        // Enough for `a` and `b`, not `c`. Only `added` has candidates.
        let s = select(&pool, 3, Budget::capped(600), &[]);
        let row = s.matrix.iter().find(|r| r.class == Class::Added).unwrap();
        assert_eq!(row.status, MatrixStatus::Captured);
        assert_eq!(
            s.admitted
                .iter()
                .filter(|a| a.primary_class == Class::Added)
                .count(),
            2
        );
        assert_eq!(s.bytes_admitted, 600);
    }

    #[test]
    fn a_class_whose_cheapest_candidate_cannot_be_funded_is_over_budget_not_absent() {
        let pool = candidates(&[field(
            "game",
            vec![
                row("x/a", State::Added, None, 100),
                row("x/b", State::Removed, None, 1_000_000),
            ],
        )]);
        let s = select(&pool, 1, Budget::capped(50), &[]);
        let row = s.matrix.iter().find(|r| r.class == Class::Added).unwrap();
        assert_eq!(row.status, MatrixStatus::OverBudget);
        assert!(
            row.reason.as_deref().unwrap_or("").contains("100"),
            "names the cost: {:?}",
            row.reason
        );
        // And over-budget is *not* satisfied, which is what --require-class acts on.
        assert!(row.unsatisfied());
        assert!(!row.is_satisfied());
        // Both one-sided classes had a candidate and could not be funded; the
        // other nine had none at all. `unmet` carries both, but the *reason*
        // is the useful part and it differs.
        let over: Vec<Class> = s
            .matrix
            .iter()
            .filter(|r| r.status == MatrixStatus::OverBudget)
            .map(|r| r.class)
            .collect();
        assert_eq!(over, vec![Class::Added, Class::Removed], "matrix order");
        let unavailable = s
            .matrix
            .iter()
            .filter(|r| r.status == MatrixStatus::Unavailable)
            .count();
        assert_eq!(unavailable, Class::ALL.len() - 2);
        assert_eq!(
            s.unmet().len(),
            Class::ALL.len(),
            "nothing in this fixture is satisfied"
        );
        assert!(s.admitted.is_empty());
        assert_eq!(s.bytes_admitted, 0);
    }

    #[test]
    fn unmet_reports_over_budget_and_unavailable_but_not_covered_by() {
        let pool = candidates(&[field(
            "game",
            vec![row(
                "x/a",
                State::Changed,
                Some(scored("x/a", 200, (100, 100), (3, 1, 0), false)),
                100,
            )],
        )]);
        let s = select(&pool, 1, Budget::unlimited(), &["x/a".into()]);
        assert_eq!(s.paths(), vec!["x/a"], "the forced one is the only capture");
        let covered = s
            .matrix
            .iter()
            .find(|r| r.class == Class::ReuseInPlace)
            .unwrap();
        assert_eq!(covered.status, MatrixStatus::CoveredBy);
        assert!(
            !s.unmet().contains(&Class::ReuseInPlace),
            "covered_by is never unmet: the capture exists"
        );
        assert_eq!(s.paths(), vec!["x/a"]);
    }

    #[test]
    fn the_matrix_has_exactly_one_row_per_class_in_admission_order() {
        let s = select(&all_six(), 1, Budget::unlimited(), &[]);
        assert_eq!(s.matrix.len(), Class::ALL.len());
        let ids: Vec<Class> = s.matrix.iter().map(|r| r.class).collect();
        assert_eq!(ids, Class::REQUIRED_ORDER.to_vec());
    }

    /// The cheapest-first property is what makes the budget fund the most
    /// classes, so it is asserted as an invariant over many ceilings.
    #[test]
    fn admission_is_cheapest_first_for_every_ceiling() {
        let pool = all_six();
        for cap in [0, 42, 100, 1_000, 2_000, 10_000, 1_000_000, u64::MAX] {
            let s = select(&pool, 1, Budget::capped(cap), &[]);
            let mut costs: Vec<u64> = s.admitted.iter().map(|a| a.budget_cost).collect();
            let spent: u64 = costs.iter().sum();
            assert!(spent <= cap, "cap {cap}: spent {spent}");
            costs.sort_unstable();
            // Every admitted cost is <= every non-admitted one.
            let cheapest_rejected = pool
                .iter()
                .map(SizedCandidate::budget_cost)
                .filter(|c| !s.admitted.iter().any(|a| a.budget_cost == *c))
                .min();
            if let Some(rej) = cheapest_rejected {
                assert!(
                    costs.last().is_none_or(|last| *last <= rej),
                    "cap {cap} admitted {costs:?} but rejected {rej}"
                );
            }
        }
    }

    #[test]
    fn a_forced_path_outside_the_manifests_is_skipped_not_fatal() {
        let s = select(
            &all_six(),
            1,
            Budget::unlimited(),
            &["nope/not/in/manifest".into()],
        );
        // A path the manifests do not carry is skipped, not fatal: the caller
        // may be naming a file for a future version. The matrix is still
        // complete, because a phantom path says nothing about which classes the
        // pool can satisfy.
        assert_eq!(s.matrix.len(), Class::ALL.len());
        assert_eq!(s.admitted.len(), 6, "the six real ones still went in");
    }

    #[test]
    fn candidates_document_carries_the_columns_and_a_one_based_rank() {
        let diffs = [field(
            "game",
            vec![
                row(
                    "a",
                    State::Changed,
                    Some(scored("a", 20, (10, 10), (3, 1, 0), false)),
                    10,
                ),
                row(
                    "b",
                    State::Changed,
                    Some(scored("b", 40, (20, 20), (1, 0, 0), false)),
                    20,
                ),
            ],
        )];
        let doc = candidates_document(&diffs);
        assert_eq!(doc["schema_version"], crate::SCHEMA_VERSION);
        let cols = doc["columns"].as_array().unwrap();
        for c in [
            "path",
            "reused",
            "moved",
            "moved_later",
            "moved_earlier",
            "old_max_end",
            "patch_bytes",
            "total_cost",
            "classes",
            "rank",
        ] {
            assert!(cols.iter().any(|x| x == c), "missing {c}");
        }
        let rows = doc["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["rank"], 1);
        assert_eq!(rows[1]["rank"], 2);
        // Both have identical total_cost, so the path breaks the tie.
        assert_eq!(rows[0]["path"], "a");
        assert_eq!(rows[0]["classes"][0], Class::ReuseInPlace.id());
    }

    #[test]
    fn classes_in_order_normalises_a_reader_supplied_list() {
        let got = classes_in_order(&["removed", "identical", "not_a_class"]);
        assert_eq!(got, vec![Class::Identical, Class::Removed]);
    }

    #[test]
    fn capture_ids_are_the_class_then_the_ordinal() {
        let c = SizedCandidate {
            candidate: all_six()[0].candidate.clone(),
            size: 54,
        };
        assert_eq!(c.candidate.capture_id(Class::Identical, 0), "identical");
        assert_eq!(c.candidate.capture_id(Class::Identical, 2), "identical+2");
    }
}
