//! The run's counters and every stdout line the automation contract promises.
//!
//! One rule lives here: **a stdout line is a [`Record`]**, not a string built at
//! the call site. A record is an event label plus named fields, held once and
//! rendered twice — `--output text` prints `REPORT total=…` and `--output json`
//! prints `{"event":"report","total":…}` from the same field list, so the two
//! planes cannot drift the way two independent `format!`s can.
//!
//! One record per line, rendered once, is what removes the possibility of a
//! record that is text on one plane and a differently-typed JSON object on the
//! other.
//!
//! Records reach stdout *and* the log, so a caller that captures either stream
//! sees the same `REPORT` / `PROGRESS` / `PLAN` / `SUMMARY` sequence. Building
//! records is separated from rendering them, so both planes are unit-testable
//! without a subscriber installed.
//!
//! Diagnostics are a different channel and do not come through here: they are
//! `tracing` events on stderr (and optionally a `TRACE` log file — see
//! [`crate::logging`]).
//!
//! - [`record`] — `Record`, `Val`, `OutputFormat`, and the two renderings.
//! - [`summary`] — `Summary`, `SummaryFlags`, and the conditional tail.
//! - [`emit`] — `emit`, `progress_record`, `report_record`, `summary_record`.

#[cfg(test)]
mod tests;

pub mod emit;
pub mod record;
pub mod summary;

pub use self::emit::{
    emit, emit_progress, progress_record, report_record, spawn_progress_reporter, summary_record,
};
pub use self::record::{OutputFormat, PROGRESS_INTERVAL_SECS, Record, Val};
pub use self::summary::{Extra, Summary, SummaryFlags};
