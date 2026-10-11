//! Deciding which digests a run has to compute.
//!
//! Phase A reports what a side already knows, [`HashPlan`] turns that into the set
//! of digests each side still has to produce, and phase C executes it. Three
//! properties follow from the split, and each is load-bearing:
//!
//! - **Phase A never hashes.** It reports what the cache knows, nothing more.
//!   That is what makes the cache *consultable* rather than fused with disk
//!   access, and it is what lets this module pick an algorithm per path instead
//!   of being handed whatever a scan already decided to read.
//! - **The decision is made here and nowhere else.** Phase A does not widen the
//!   request, and phase C does not narrow it. `--hash-any-of` is a different
//!   predicate in [`HashPlan`] rather than a new pass over the tree.
//! - **The decision is made before any read.** A plan is total over the run, so
//!   a run that cannot answer everything fails having read nothing.
//!
//! There are two predicates, and they answer different questions. One side with no
//! counterpart is [`HashPlan::plan_one_side`] — every requested algorithm for every
//! file, which is `update`. Two sides are [`plan_pairs`], which is
//! [`plan_one_side`] plus a pairing step and a coverage requirement.
//!
//! ## Two questions, kept apart
//!
//! [`HashPlan`] answers *what must be computed* and honours trust. Coverage
//! answers *whether the run can be answered at all*, and it does **not**: a side's
//! availability is `cached ∪ hashable`, with no trust term. Both halves matter,
//! because a side whose availability included trust could not be short of
//! anything:
//!
//! | question | honours `no_trust` | who can fail |
//! | --- | --- | --- |
//! | what must be computed | **yes** — it means "re-read rather than reuse" | nobody; a side that cannot re-read is simply never asked |
//! | can the pair be answered | **no** — a record has nothing to distrust | a side that cannot produce a digest from disk, i.e. a record |
//!
//! Folding trust into coverage would make `--no-trust-cached-hashes src` fatal for
//! every record, including ones holding exactly the right digests — and a record is
//! not a cache with a stale entry, it is a fixed body of data with no filesystem
//! behind it.
//!
//! ## The submodules
//!
//! | module | what it owns |
//! | --- | --- |
//! | `stat_trust` | [`StatTrust`] and the field vocabulary: **which stat fields may settle a pair** |
//! | `plan` | [`HashPlan`] and [`Required`]: **what must be computed, and what settles each pair** |
//! | `pairs` | [`plan_pairs`]: **the two-sided decision**, including the pairing step |
//! | `coverage` | [`Coverage`]: **whether the run can be answered at all**, and what it says when it cannot |

mod coverage;
mod pairs;
mod plan;
mod stat_trust;

#[cfg(test)]
mod fixtures;

pub use coverage::Coverage;
pub use pairs::{PairPlan, SideRequest, plan_pairs};
pub use plan::{HashPlan, Required};
pub use stat_trust::{DifferingFields, StatField, StatTrust};

/// How many of the requested algorithms an undecided pair must be able to answer
/// with.
///
/// Both modes ask a real question about content, and the split between them is the
/// whole of their point. Without a requirement, a pair whose two sides hold no
/// comparable digest falls back to a size+mtime that already agreed, and the run
/// reports a content comparison it never performed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HashMode {
    /// `--hash-all-of`, and the default: **every** requested algorithm must be
    /// available on both sides of every undecided pair.
    ///
    /// A folder side that lacks one backfills it by hashing. A record side cannot,
    /// so that is fatal — which is what makes a comparison a comparison.
    #[default]
    AllOf,
    /// `--hash-any-of`: **at least one** must be, and the planner picks one per
    /// pair, cheapest first and then in the order the user gave.
    ///
    /// Weaker than all-of by design, and never weaker than the pick: every
    /// algorithm it chooses is obtainable on *both* sides, so a pair settled this
    /// way is still settled by a digest both sides hold rather than by a digest one
    /// of them is missing. That is why the narrower mode cannot admit a pair whose
    /// content was never read.
    AnyOf,
}
