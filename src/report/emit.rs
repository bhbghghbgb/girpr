use std::collections::HashSet;
use std::sync::atomic::Ordering;

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
    println!("{line}");
    tracing::info!("{line}");
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

/// Background `PROGRESS` ticker for the repair phase. The first tick fires
/// immediately; it is skipped so we only report after a full interval.
///
/// Under `--output json` this is one more NDJSON line: the same record, same
/// shape, in the middle of the stream where the tick actually landed.
pub fn spawn_progress_reporter(
    sum: std::sync::Arc<Summary>,
    format: OutputFormat,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(PROGRESS_INTERVAL_SECS));
        interval.tick().await;
        loop {
            interval.tick().await;
            emit_progress(&sum, format);
        }
    })
}

/// The one-time begin report, written right after the metadata calls. Parsers
/// get it before any repair starts: `local_version` is the on-disk `config.ini`
/// value (`none` when absent), `latest_version` is the "will be updated to" tag;
/// the rest come from the HoYoPlay/Sophon APIs, not from config. `audio_langs`
/// is the effective set.
///
/// Provenance: Starward `GameInstallService.cs:788-849` (the same field set the
/// launcher shows the user before installing).
#[allow(clippy::too_many_arguments)] // stable report formatter; params-struct would churn CLI parsing
pub fn report_record(
    local_version: &Option<String>,
    latest_version: &str,
    biz: &str,
    cfg: &GameConfig,
    pkg: &GameBranchPackage,
    build: &ChunkBuild,
    audio_langs: &HashSet<String>,
) -> Record {
    let mut langs: Vec<&String> = audio_langs.iter().collect();
    langs.sort();
    let audio_joined: Vec<&str> = langs.iter().map(|s| s.as_str()).collect();
    let local = local_version.as_deref().unwrap_or("none");
    Record::keyed("REPORT")
        .put("local_version", local)
        .put("latest_version", latest_version)
        .put("biz", biz)
        .put("exe", cfg.exe_file_name.as_str())
        .put("download_mode", cfg.default_download_mode.as_str())
        .put("branch", pkg.branch.as_str())
        .put("package_id", pkg.package_id.as_str())
        .put("build_id", build.build_id.as_str())
        .put("audio_langs", audio_joined.join(","))
        .put("diff_tags", pkg.diff_tags.join(","))
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
