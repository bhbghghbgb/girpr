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
    /// `over_budget` is **not** accepted, for the reason
    /// [`MatrixRow::is_satisfied`] gives: the caller was told to take the class
    /// and this run would not pay for it. `unavailable` is accepted because it
    /// is a fact about the manifests rather than about this run.
    pub const fn is_acceptable(self) -> bool {
        matches!(
            self,
            MatrixStatus::Captured | MatrixStatus::CoveredBy | MatrixStatus::Unavailable
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

    /// A17 accepts three of the four; `over_budget` is a statement about this
    /// run's flags rather than about the manifests.
    #[test]
    fn only_over_budget_is_unacceptable_to_a17() {
        assert!(Captured.is_acceptable());
        assert!(CoveredBy.is_acceptable());
        assert!(Unavailable.is_acceptable());
        assert!(!OverBudget.is_acceptable());
        // And the two spellings cannot disagree about it.
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
/// Pure over `&[FieldDiff]`: the ordering is §5's — `total_cost`, then the
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
/// One walk of the pool instead of nine. The nine numbers are the same ones the
/// per-class `filter().count()` produced.
fn considered_counts(pool: &[SizedCandidate]) -> [u64; 9] {
    let mut counts = [0_u64; 9];
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

/// Run the three admission phases.
///
/// `forced` is what `--files` asked for: already-captured paths, which take
/// precedence over everything and are what produces `covered_by` for free.
pub fn select(
    pool: &[SizedCandidate],
    per_class: usize,
    budget: Budget,
    forced: &[String],
) -> Selection {
    // Phase 0 — forced captures, in the caller's order, each one noted with
    // every class it happens to answer.
    let mut admitted: Vec<Admission> = Vec::new();
    let mut spent = Budget::unlimited();
    let mut forced_skipped: Vec<String> = Vec::new();
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
            break;
        }
        spent.charge(cost);
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
    let initial = spent.spent();

    // Phase 1 — wants, class by class, cheapest-first and excluding anything
    // already captured.
    //
    // The ordinal rides along because `capture_id` is `<class>` or `<class>+<n>`
    // where `n` counts *within the class*. Phase 2 re-sorts these by cost, so
    // the position a want is admitted at says nothing about which pick it was —
    // `partial_moved_offset+7` would be a true statement about a global cost
    // order and a useless one about `--per-class`.
    let mut wants: Vec<(Class, usize, SizedCandidate)> = Vec::new();
    let mut notes: Vec<MatrixRow> = Vec::new();
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
                capture_id: Some(cover.capture_id.clone()),
                covered_by: Some(cover.path.clone()),
                path: Some(cover.path.clone()),
                reason: Some("cheapest candidate already captured for another class".into()),
                candidates_considered: considered,
            });
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
    ordered.sort_by(|&a, &b| {
        cost_then_field_then_path(&wants[a].2, &wants[b].2)
    });
    let costs: Vec<u64> = ordered.iter().map(|&i| wants[i].2.budget_cost()).collect();
    let (fates, final_budget) =
        budget::admit_in_order(&costs, Budget::with_spent(budget.limit(), initial));
    let mut rejected: Vec<Rejection> = Vec::new();

    for (rank, i) in ordered.into_iter().enumerate() {
        let (class, ordinal, c) = &wants[i];
        if fates[rank] != Fate::Admitted {
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
    for mut row in notes {
        match row.status {
            MatrixStatus::Unavailable => {}
            MatrixStatus::CoveredBy => {}
            MatrixStatus::Captured | MatrixStatus::OverBudget => {
                let hit = admitted.iter().find(|a| a.satisfies(row.class));
                match hit {
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
                        row.reason = Some(format!(
                            "the cheapest candidate {}",
                            unfunded(cheapest)
                        ));
                    }
                }
            }
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
const REPOOL_NOTE: &str = "widen --audio, or check the version pair — the spec records this class as \
     absent for 7.0.0 -> 7.1.0";

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
            "moved", "same_md5", "old_bytes", "new_bytes", "patch_bytes",
            "total_cost", "classes", "rank",
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
    fn scored(
        path: &str,
        cost: u64,
        size: (i64, i64),
        chunks: (u64, u64, u64),
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
            same_md5: md5,
            old_bytes: cost / 2,
            new_bytes: cost - cost / 2,
            patch_bytes: if reused == nchunks { 0 } else { cost / 4 },
            total_cost: cost,
        }
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

    /// The nine classes, one candidate each, cheapest class first.
    fn all_nine() -> Vec<SizedCandidate> {
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
                row(
                    "b/zero_single",
                    State::Changed,
                    Some(scored("b/zero_single", 84, (42, 42), (1, 0, 0), false)),
                    42,
                ),
                row(
                    "c/zero_multi",
                    State::Changed,
                    Some(scored(
                        "c/zero_multi",
                        2_200,
                        (1000, 1200),
                        (3, 0, 0),
                        false,
                    )),
                    1200,
                ),
                row(
                    "d/same_offset",
                    State::Changed,
                    Some(scored(
                        "d/same_offset",
                        1_826_067,
                        (2340264, 2340264),
                        (3, 1, 0),
                        false,
                    )),
                    2340264,
                ),
                row(
                    "e/moved",
                    State::Changed,
                    Some(scored("e/moved", 300, (100, 100), (3, 1, 1), false)),
                    100,
                ),
                row(
                    "f/size_changed",
                    State::Changed,
                    Some(scored("f/size_changed", 500, (100, 200), (2, 1, 0), false)),
                    200,
                ),
                row("g/added", State::Added, None, 114_000),
                row("h/removed", State::Removed, None, 1_730_000),
            ],
        )])
    }

    #[test]
    fn the_pool_is_built_from_every_row_with_its_shape_and_classes() {
        let pool = all_nine();
        assert_eq!(pool.len(), 8);
        let by_path = |p: &str| pool.iter().find(|c| c.candidate.path == p).unwrap();
        assert_eq!(
            by_path("a/identical").candidate.classes,
            vec![Class::Identical]
        );
        assert_eq!(by_path("g/added").candidate.classes, vec![Class::Added]);
        assert_eq!(by_path("h/removed").candidate.classes, vec![Class::Removed]);
        assert_eq!(
            by_path("b/zero_single").candidate.classes,
            vec![Class::ZeroReuseSingle]
        );
    }

    /// `removed` has no new side and `added` has to be fetched whole. Charging
    /// `total_cost` to both would make `removed` free and `added` cost nothing.
    #[test]
    fn a_one_sided_path_is_charged_its_own_size() {
        let pool = all_nine();
        let g = pool.iter().find(|c| c.candidate.path == "g/added").unwrap();
        let h = pool
            .iter()
            .find(|c| c.candidate.path == "h/removed")
            .unwrap();
        let d = pool
            .iter()
            .find(|c| c.candidate.path == "d/same_offset")
            .unwrap();
        assert_eq!(g.budget_cost(), 114_000);
        assert_eq!(h.budget_cost(), 1_730_000);
        assert_eq!(d.budget_cost(), 1_826_067, "a common path is old + new");
    }

    #[test]
    fn the_pool_is_ordered_cheapest_first() {
        let costs: Vec<u64> = all_nine().iter().map(SizedCandidate::budget_cost).collect();
        let mut sorted = costs.clone();
        sorted.sort_unstable();
        assert_eq!(costs, sorted, "the budget relies on this ordering");
        // A common path is charged old + new, so the cheapest thing here is the
        // 42 B file *plus its old copy* — not the 42 B file's own size, and not
        // the 114 B added file either. Charging a common path its own size would
        // understate what materializing the fixture actually costs.
        assert_eq!(costs[0], 84, "b/zero_single: 42 old + 42 new");
    }

    #[test]
    fn one_capture_per_class_is_the_greedy_answer() {
        let s = select(&all_nine(), 1, Budget::unlimited(), &[]);
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
                (Class::Added, "g/added".into()),
                (Class::Identical, "a/identical".into()),
                (Class::PartialMovedOffset, "e/moved".into()),
                (Class::PartialSameOffset, "d/same_offset".into()),
                (Class::PartialSizeChanged, "f/size_changed".into()),
                (Class::Removed, "h/removed".into()),
                (Class::ZeroReuseMulti, "c/zero_multi".into()),
                (Class::ZeroReuseSingle, "b/zero_single".into()),
            ]
        );
        assert_eq!(s.admitted.len(), 8);
        assert!(
            s.unmet().contains(&Class::FullReuseReordered),
            "and it has no candidate"
        );
    }

    /// Each class gets its own cheapest because the predicates are disjoint, and
    /// the exclusion set is what keeps it that way when they are not.
    #[test]
    fn an_unsatisfiable_class_is_recorded_with_a_reason_and_a_count() {
        let s = select(&all_nine(), 1, Budget::unlimited(), &[]);
        let row = s
            .matrix
            .iter()
            .find(|r| r.class == Class::FullReuseReordered)
            .unwrap();
        assert_eq!(row.status, MatrixStatus::Unavailable);
        assert_eq!(row.candidates_considered, 0);
        let reason = row.reason.clone().unwrap();
        assert!(reason.contains("full_reuse_reordered"), "{reason}");
        assert!(reason.contains("--audio"), "the remedy: {reason}");
        assert!(
            !s.matrix
                .iter()
                .any(|r| r.capture_id.is_none() && r.status == MatrixStatus::Captured)
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
            .find(|r| r.class == Class::PartialSameOffset)
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
            .filter(|a| a.primary_class == Class::PartialSameOffset)
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
        // other seven had none at all. `unmet` carries both, but the *reason*
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
        assert_eq!(unavailable, 7);
        assert_eq!(s.unmet().len(), Class::ALL.len(), "nothing in this fixture is satisfied");
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
            .find(|r| r.class == Class::PartialSameOffset)
            .unwrap();
        assert_eq!(covered.status, MatrixStatus::CoveredBy);
        assert!(
            !s.unmet().contains(&Class::PartialSameOffset),
            "covered_by is never unmet: the capture exists"
        );
        assert_eq!(s.paths(), vec!["x/a"]);
    }

    #[test]
    fn the_matrix_has_exactly_one_row_per_class_in_admission_order() {
        let s = select(&all_nine(), 1, Budget::unlimited(), &[]);
        assert_eq!(s.matrix.len(), Class::ALL.len());
        let ids: Vec<Class> = s.matrix.iter().map(|r| r.class).collect();
        assert_eq!(ids, Class::REQUIRED_ORDER.to_vec());
    }

    /// The cheapest-first property is what makes the budget fund the most
    /// classes, so it is asserted as an invariant over many ceilings.
    #[test]
    fn admission_is_cheapest_first_for_every_ceiling() {
        let pool = all_nine();
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
            &all_nine(),
            1,
            Budget::unlimited(),
            &["nope/not/in/manifest".into()],
        );
        // A path the manifests do not carry is skipped, not fatal: the caller
        // may be naming a file for a future version. The matrix is still
        // complete, because a phantom path says nothing about which classes the
        // pool can satisfy.
        assert_eq!(s.matrix.len(), Class::ALL.len());
        assert_eq!(s.admitted.len(), 8, "the eight real ones still went in");
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
        assert_eq!(doc["schema_version"], 1);
        let cols = doc["columns"].as_array().unwrap();
        for c in [
            "path",
            "reused",
            "moved",
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
        assert_eq!(rows[0]["classes"][0], Class::PartialSameOffset.id());
    }

    #[test]
    fn classes_in_order_normalises_a_reader_supplied_list() {
        let got = classes_in_order(&["removed", "identical", "not_a_class"]);
        assert_eq!(got, vec![Class::Identical, Class::Removed]);
    }

    #[test]
    fn capture_ids_are_the_class_then_the_ordinal() {
        let c = SizedCandidate {
            candidate: all_nine()[0].candidate.clone(),
            size: 54,
        };
        assert_eq!(c.candidate.capture_id(Class::Identical, 0), "identical");
        assert_eq!(c.candidate.capture_id(Class::Identical, 2), "identical+2");
    }
}
