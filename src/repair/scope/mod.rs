//! Scope of work: *which paths exist* and *how much work* (docs/04).
//!
//! Two tiers, deliberately separate because they answer different questions
//! and fail differently (docs/04 §1.4):
//!
//! | tier | module | question | failure if wrong |
//! |---|---|---|---|
//! | **path scope** | [`filter`] | which paths exist | a typo repairs the wrong files |
//! | **resource ceilings** | [`limits`] + [`ledger`] | how much work | a typo lets a 100 GB run through |
//!
//! The ceilings are worthless without the selector being right most of the
//! time, and the selector is worthless without a backstop — so both live here.
//!
//! The load-bearing invariant, from girsync and docs/04 §2: **a filtered-out
//! path is treated as nonexistent on *both* sides** — it is absent from the
//! plan *and* invisible to the files-cleanup. Narrowing the plan alone would
//! leave the purge with a *narrower* keep-set than an unfiltered run, which is
//! the worst possible outcome.
//!
//! | module | concern |
//! |---|---|
//! | [`filter`] | which paths exist (`--include` / `--exclude`) |
//! | [`limits`] | how much work is allowed (the pre-flight ceilings) |
//! | [`ledger`] | how much work has been done (the runtime reservations) |
//! | [`report`] | what the plan was, as a `PLAN` line and a JSON document |

pub mod filter;
pub mod ledger;
pub mod limits;
pub mod report;

pub use filter::PathScope;
pub use limits::{LimitHit, Limits, Preflight};
pub use report::{PlanDocument, PlanTotals, plan_record};
