//! The one writer for the run's stdout data plane.
//!
//! The record order is fixed: `REPORT` once the metadata is known, `PLAN` when
//! the run asked for it, zero or more `PROGRESS` ticks from the phase's single
//! [`ProgressGuard`], and `SUMMARY` last, emitted once by the caller after
//! [`crate::repair::run`] returns. No phase emits `SUMMARY`.
//!
//! Ownership rule: a phase that has long-running work starts **one**
//! [`ProgressGuard`] for its shared [`Summary`] and drops it when the phase
//! returns. The ticker is the only `PROGRESS` producer; per-file loops and
//! blocking tasks update counters but never emit. That is what keeps two
//! tickers from interleaving into the same stream.
//!
//! [`emit`] is the only writer. Its policy for every record: render once,
//! mirror the rendered line to the log, write stdout through [`write_line`],
//! never panic, and never change the exit code.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::record::{OutputFormat, PROGRESS_INTERVAL_SECS, Record};
use super::summary::{Summary, conditional_extras};
use crate::hyp::{ChunkBuild, GameBranchPackage, GameConfig};

/// Emit one record to **both** stdout and the log in the run's chosen plane, so
/// unattended runs keep it regardless of which stream is captured.
///
/// The log carries the rendered line rather than the record, so a log file
/// written under `--output text` reads like the stdout a human saw, and one
/// written under `--output json` is parseable by the same code that reads
/// stdout. That is the point of logging it at all: the two streams carry the
/// same data plane, not merely the same facts.
/// NOTE(design-intent): the dual emission is deliberate (automation contract,
/// see README).
pub fn emit(record: &Record, format: OutputFormat) {
    let line = record.render(format);
    // The log mirror is written first: a consumer that closed stdout must not
    // also cost the run its record in the log.
    tracing::info!("{line}");
    // A closed stdout is the end of the data plane, not a failed run: the
    // repair already happened and the exit code is the one it earned.
    match write_line(&mut std::io::stdout().lock(), &line) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
            tracing::debug!("stdout closed: {e}");
        }
        Err(e) => tracing::warn!("stdout write failed ({:?}): {e}", e.kind()),
    }
}

/// Write one rendered record plus its newline.
///
/// Private seam for the writer tests; the process stdout is the only real
/// caller.
pub(super) fn write_line(out: &mut impl std::io::Write, line: &str) -> std::io::Result<()> {
    out.write_all(line.as_bytes())?;
    out.write_all(b"\n")
}

/// The `PROGRESS` snapshot: where the run is, without touching the disk.
pub fn progress_record(sum: &Summary) -> Record {
    let skipped = sum.files_skipped.load(Ordering::Relaxed);
    let repaired = sum.files_repaired.load(Ordering::Relaxed);
    let failed = sum.files_failed.load(Ordering::Relaxed);
    let done = skipped + repaired + failed;
    let dl = sum.download.settled();
    Record::keyed("PROGRESS")
        .put("done", done)
        .put("total", sum.files_total)
        .put("skipped", skipped)
        .put("repaired", repaired)
        .put("failed", failed)
        .put("download_bytes", dl)
}

/// Emit the current progress snapshot.
pub fn emit_progress(sum: &Summary, format: OutputFormat) {
    emit(&progress_record(sum), format);
}

/// Owns a phase's `PROGRESS` ticker for as long as the phase runs.
///
/// A phase with long-running work starts exactly one guard for its shared
/// counters and drops it on the way out (including an early return or a
/// panic): dropping stops the ticker, so no later phase can inherit a running
/// stream. It is an RAII handle rather than a global because two live guards
/// would otherwise be two silent tickers.
pub struct ProgressGuard {
    task: tokio::task::JoinHandle<()>,
}

impl ProgressGuard {
    /// Start the ticker for a phase whose counters `sum` holds.
    pub fn start(sum: Arc<Summary>, format: OutputFormat) -> Self {
        Self::spawn(sum, format, Duration::from_secs(PROGRESS_INTERVAL_SECS))
    }

    /// The same ticker at a shorter interval, for the start/stop tests.
    #[cfg(test)]
    pub fn start_with_interval(
        sum: Arc<Summary>,
        format: OutputFormat,
        interval: Duration,
    ) -> Self {
        Self::spawn(sum, format, interval)
    }

    fn spawn(sum: Arc<Summary>, format: OutputFormat, interval: Duration) -> Self {
        let task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // The first tick fires immediately; skip it so a phase only reports
            // after a full interval has passed.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                emit_progress(&sum, format);
            }
        });
        Self { task }
    }
}

impl Drop for ProgressGuard {
    fn drop(&mut self) {
        // Abort, do not await: the ticker spends its life asleep between ticks,
        // and joining it would block the runtime for up to a whole interval.
        self.task.abort();
    }
}

/// The provenance fields of the one-time begin report.
///
/// The field order is the report's field order: the caller names every value
/// once, and the formatter turns the group into the `REPORT` record.
pub struct ReportProvenance<'a> {
    /// The on-disk `config.ini` version, or `None` when absent.
    pub local_version: &'a Option<String>,
    /// The version the run is moving to.
    pub latest_version: &'a str,
    /// The game/business channel.
    pub biz: &'a str,
    /// The launcher's game configuration.
    pub cfg: &'a GameConfig,
    /// The selected branch package.
    pub pkg: &'a GameBranchPackage,
    /// The target build.
    pub build: &'a ChunkBuild,
    /// The effective audio language set.
    pub audio_langs: &'a HashSet<String>,
}

/// The one-time begin report, written right after the metadata calls. Parsers
/// get it before any repair starts: `local_version` is the on-disk `config.ini`
/// value (`none` when absent), `latest_version` is the "will be updated to" tag;
/// the rest come from the HoYoPlay/Sophon APIs, not from config. `audio_langs`
/// is the effective set.
///
/// Provenance: Starward `GameInstallService.cs:788-849` (the same field set the
/// launcher shows the user before installing).
pub fn report_record(provenance: &ReportProvenance<'_>) -> Record {
    let mut langs: Vec<&String> = provenance.audio_langs.iter().collect();
    langs.sort();
    let audio_joined: Vec<&str> = langs.iter().map(|s| s.as_str()).collect();
    let local = provenance.local_version.as_deref().unwrap_or("none");
    Record::keyed("REPORT")
        .put("local_version", local)
        .put("latest_version", provenance.latest_version)
        .put("biz", provenance.biz)
        .put("exe", provenance.cfg.exe_file_name.as_str())
        .put(
            "download_mode",
            provenance.cfg.default_download_mode.as_str(),
        )
        .put("branch", provenance.pkg.branch.as_str())
        .put("package_id", provenance.pkg.package_id.as_str())
        .put("build_id", provenance.build.build_id.as_str())
        .put("audio_langs", audio_joined.join(","))
        .put("diff_tags", provenance.pkg.diff_tags.join(","))
}

/// The final `SUMMARY`, ending every run.
///
/// Trailing conditional keys (`limited`, `scoped`, `config_bumped`,
/// `plan_files`, …) are appended only when their flag is
/// active, so an unflagged run's line is byte-identical to a run that passes no
/// flags at all.
pub fn summary_record(sum: &Summary, exit: i32) -> Record {
    let skipped = sum.files_skipped.load(Ordering::Relaxed);
    let repaired = sum.files_repaired.load(Ordering::Relaxed);
    let failed = sum.files_failed.load(Ordering::Relaxed);
    let dl = sum.download.settled();
    let del = sum.deleted_extra_bytes.load(Ordering::Relaxed);
    let base = Record::keyed("SUMMARY")
        .put("total", sum.files_total)
        .put("skipped", skipped)
        .put("repaired", repaired)
        .put("failed", failed)
        .put("download_bytes", dl)
        .put("deleted_extra_bytes", del)
        .put("exit", exit);
    conditional_extras(sum)
        .into_iter()
        .fold(base, |rec, extra| extra.put(rec))
}
