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
//! runs. Records are attributed to the thread that started the repair loop,
//! not to the thread that happens to poll a file task: file tasks run on the
//! blocking pool, so a task carries its origin's thread id onto the pool
//! thread while it runs. Only the test seam reads the record; a shipped run
//! writes to it and never looks at it, so no output byte, exit code, log line
//! or CLI flag changes. Nextest runs one test per process, so a `take` after a
//! run sees exactly that run's decisions.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::thread::ThreadId;

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
    /// An unexpected I/O failure on the temp reuse path (open, seek, read, or
    /// a short source) fell back to a download. The expected MD5 mismatch is
    /// deliberately not counted: it is the ordinary "candidate did not verify"
    /// outcome, not a failure worth surfacing.
    pub reuse_io_fallbacks: u64,
}

/// Records by originating thread. Keyed by thread rather than held in TLS
/// because a file task records from a blocking-pool thread (see
/// [`DiagnosticsOwner`]).
static RECORDS: OnceLock<Mutex<HashMap<ThreadId, RepairDiagnostics>>> = OnceLock::new();

fn records() -> &'static Mutex<HashMap<ThreadId, RepairDiagnostics>> {
    RECORDS.get_or_init(|| Mutex::new(HashMap::new()))
}

thread_local! {
    /// The id records from this thread are attributed to.
    static OWNER: ThreadId = std::thread::current().id();
    /// Override set on a pool thread while it runs a moved file task.
    static ADOPTED: Cell<Option<ThreadId>> = const { Cell::new(None) };
}

fn owner() -> ThreadId {
    ADOPTED
        .with(|a| a.get())
        .unwrap_or_else(|| OWNER.with(|id| *id))
}

fn with_record<R>(f: impl FnOnce(&mut RepairDiagnostics) -> R) -> R {
    let key = owner();
    let mut map = records().lock().unwrap_or_else(|e| e.into_inner());
    f(map.entry(key).or_default())
}

/// Attribute the current thread's diagnostics to `owner` while this guard is
/// alive. A file task moved to the blocking pool adopts the thread that
/// started the repair loop, so a `take` on that thread sees its decisions.
pub(crate) struct DiagnosticsOwner(Option<ThreadId>);

impl DiagnosticsOwner {
    pub(crate) fn adopt(owner: ThreadId) -> Self {
        Self(ADOPTED.with(|a| a.replace(Some(owner))))
    }
}

impl Drop for DiagnosticsOwner {
    fn drop(&mut self) {
        ADOPTED.with(|a| a.set(self.0));
    }
}

/// Drain the originating thread's record, leaving a fresh one behind.
pub(crate) fn take() -> RepairDiagnostics {
    let key = owner();
    let mut map = records().lock().unwrap_or_else(|e| e.into_inner());
    map.remove(&key).unwrap_or_default()
}

/// Record the ordered tier's write direction and salvage figures.
pub(crate) fn record_order(ascending: bool, salvaged: u64, of: u64) {
    with_record(|d| {
        d.ascending = Some(ascending);
        d.salvaged = Some(salvaged);
        d.salvaged_of = Some(of);
    });
}

/// Record the in-place writer's chunk accounting.
pub(crate) fn record_chunk_stats(stats: &super::file::ChunkStats) {
    with_record(|d| {
        d.chunks_total = Some(stats.chunks_total);
        d.chunks_kept = Some(stats.chunks_kept);
        d.chunks_reused = Some(stats.chunks_reused);
        d.chunks_downloaded = Some(stats.chunks_downloaded);
    });
}

/// Record that the guarded plan-scan verdict was reused.
pub(crate) fn record_plan_scan_reused() {
    with_record(|d| d.plan_scan_reused = true);
}

/// Record that a RAM shortage (not a zero-byte mode) picked the ordered tier.
pub(crate) fn record_ram_shortage() {
    with_record(|d| d.ram_shortage = true);
}

/// Record that the eager dead-tail trim ran.
pub(crate) fn record_dead_tail_trimmed() {
    with_record(|d| d.dead_tail_trimmed = true);
}

/// Record that the trim was deferred to the end.
pub(crate) fn record_trim_deferred() {
    with_record(|d| d.trim_deferred = true);
}

/// Record an unexpected reuse-path I/O failure that fell back to a download.
pub(crate) fn record_reuse_io_fallback() {
    with_record(|d| d.reuse_io_fallbacks += 1);
}
