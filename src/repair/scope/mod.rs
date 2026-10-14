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
//! M1 lands [`filter`] and its semantics. Wiring, the ceilings and the report
//! arrive in the milestones after it.

pub mod filter;

pub use filter::PathScope;
