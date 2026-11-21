//! Structured, in-process repair diagnostics.
//!
//! The per-file repair decides things the stdout `SUMMARY` does not carry:
//! which direction the ordered tier wrote in, how many chunks that direction
//! salvaged, whether the eager trim ran, and whether a RAM budget refusal
//! picked the ordered tier. Today those decisions are visible only as
//! `tracing` lines, so a test that needs them spawns the binary and parses its
//! log file.
//!
//! This module records the same decisions as plain values while the repair
//! runs. The record lives in thread-local storage that only the test seam
//! reads: a shipped run writes to it and never looks at it, so no output byte,
//! exit code, log line or CLI flag changes. Nextest runs one test per process
//! on a current-thread runtime, so a `take` after a run sees exactly that
//! run's decisions.

use std::cell::RefCell;

/// The structured decisions of one in-process repair run.
///
/// Every field is a plain value so a test can assert on the decision itself
/// instead of on the prose a logger rendered for it. `None` means the run
/// never made that decision (the staged tier has no salvage direction, and a
/// run that repaired no file in place has no in-place chunk accounting).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RepairDiagnostics {
    /// The direction the ordered tier chose.
    pub ascending: Option<bool>,
    /// Chunks the ordered direction kept alive, and the number it considered.
    /// Set together with [`Self::ascending`].
    pub salvaged: Option<u64>,
    pub salvaged_of: Option<u64>,
    /// The chunk accounting the `repaired in place` line carries.
    pub chunks_total: Option<u64>,
    pub chunks_kept: Option<u64>,
    pub chunks_reused: Option<u64>,
    pub chunks_downloaded: Option<u64>,
    /// The guarded plan-scan verdict was reused in the in-place writer.
    pub plan_scan_reused: bool,
    /// A staged payload did not fit the RAM budget, so the ordered tier was
    /// chosen because of a shortage. A `--max-ram-bytes 0` mode is *not* one.
    pub ram_shortage: bool,
    /// The eager dead-tail trim ran.
    pub dead_tail_trimmed: bool,
    /// The trim was deferred because a reuse candidate reaches past the end.
    pub trim_deferred: bool,
}

thread_local! {
    static CURRENT: RefCell<RepairDiagnostics> = RefCell::new(RepairDiagnostics::default());
}

/// Drain this thread's record, leaving a fresh one behind.
pub(crate) fn take() -> RepairDiagnostics {
    CURRENT.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// Record the ordered tier's write direction and salvage figures.
pub(crate) fn record_order(ascending: bool, salvaged: u64, of: u64) {
    CURRENT.with(|c| {
        let mut d = c.borrow_mut();
        d.ascending = Some(ascending);
        d.salvaged = Some(salvaged);
        d.salvaged_of = Some(of);
    });
}

/// Record the in-place writer's chunk accounting.
pub(crate) fn record_chunk_stats(stats: &super::file::ChunkStats) {
    CURRENT.with(|c| {
        let mut d = c.borrow_mut();
        d.chunks_total = Some(stats.chunks_total);
        d.chunks_kept = Some(stats.chunks_kept);
        d.chunks_reused = Some(stats.chunks_reused);
        d.chunks_downloaded = Some(stats.chunks_downloaded);
    });
}

/// Record that the guarded plan-scan verdict was reused.
pub(crate) fn record_plan_scan_reused() {
    CURRENT.with(|c| c.borrow_mut().plan_scan_reused = true);
}

/// Record that a RAM shortage (not a zero-byte mode) picked the ordered tier.
pub(crate) fn record_ram_shortage() {
    CURRENT.with(|c| c.borrow_mut().ram_shortage = true);
}

/// Record that the eager dead-tail trim ran.
pub(crate) fn record_dead_tail_trimmed() {
    CURRENT.with(|c| c.borrow_mut().dead_tail_trimmed = true);
}

/// Record that the trim was deferred to the end.
pub(crate) fn record_trim_deferred() {
    CURRENT.with(|c| c.borrow_mut().trim_deferred = true);
}
