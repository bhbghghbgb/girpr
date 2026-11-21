//! The girpr report plane: run counters and every stdout line of the contract.
//!
//! One rule lives here: **a stdout line is a [`Record`]**, not a string built at
//! the call site. A record is an event label plus named fields, held once and
//! rendered twice — `--output text` prints `REPORT total=…` and `--output json`
//! prints `{"event":"report","total":…}` from the same field list, so the two
//! planes cannot drift.
//!
//! The crate owns the whole `Summary`/`Meter`/`StoppedBy` group together. Those
//! types are mutually recursive (`Summary` holds two `Meter`s; `StoppedBy`
//! reads a `Summary`), so the boundary sits around the group rather than
//! cutting the cycle inside the facade. The repair pipeline consumes the group
//! directly, so an edit to repair internals leaves this crate and its tests
//! cached.
//!
//! The ordered-JSON and line-writing mechanics stay in `girpr-report-core`.
//! This crate keeps girpr's event-first key order, unsigned counts and 1/0
//! flags; girsync and gircap keep theirs. Nothing here changes a byte: key
//! order, casing, spacing, the pinned `format_timestamp` and the three
//! distinct report vocabularies are all outside this crate.

pub mod emit;
pub mod ledger;
pub mod phase;
pub mod record;
pub mod summary;

#[cfg(test)]
mod tests;

pub use self::emit::{
    ProgressGuard, ReportProvenance, emit, emit_progress, progress_record, report_record,
    summary_record,
};
pub use self::ledger::{Meter, StoppedBy};
pub use self::phase::{PhaseTimer, millis_between};
pub use self::record::{OutputFormat, PROGRESS_INTERVAL_SECS, Record, Val};
pub use self::summary::{Ceiling, Extra, Summary, SummaryFlags};
