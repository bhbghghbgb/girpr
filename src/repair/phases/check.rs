//! `--check-only`: verify every planned file and report damage, write nothing
//! (docs/02 step 6.5).
//!
//! This is the one place that answers "is my install intact?" without touching
//! it: no `config.ini` bump, no purge, no `*_tmp` sweep, no audio scan write.
//! Exit code is `0` when clean, `4` when any file fails size, MD5, or is
//! unreadable.
//!
//! `files_skipped` counts verified-clean files and `files_failed` counts damaged
//! ones, so the `SUMMARY`/`PROGRESS` counters read the same as a repair run.

use std::path::Path;
use std::sync::atomic::Ordering;

use crate::report::{self, Summary};
use crate::util;

use super::plan::RepairPlan;
use crate::repair::scan::ScanReport;

/// Verify the whole plan; returns the number of damaged files.
///
/// When the run carried a `--scan`, the scan's verdicts are used instead of
/// reading the files again: the scan already read them, and the answer it
/// carries is the same one. That is what makes `--check-only --scan` a
/// single read of the install.
pub fn verify_plan(
    game_dir: &Path,
    plan: &RepairPlan,
    summary: &Summary,
    output: report::OutputFormat,
    scan: Option<&ScanReport>,
) -> u64 {
    let mut bad = 0u64;
    let mut damaged_chunks = 0u64;
    let mut last_emit = std::time::Instant::now();
    for (i, f) in plan.files.iter().enumerate() {
        // Resolve once. A containment failure is a *diagnostic*: `build_plan`
        // already refused an escaping rel, so this is defence in depth, and it
        // must not be mislabelled as a size mismatch (which is what `.ok()`
        // did here).
        let path = util::contained_join(game_dir, &f.rel);
        match scan.and_then(|s| s.files.get(&f.rel)) {
            Some(v) => match &path {
                Err(e) => {
                    bad += 1;
                    tracing::warn!(
                        file = %f.rel,
                        error = %format!("{e:#}"),
                        "CHECK fail: path escapes the game dir"
                    );
                }
                Ok(path) => {
                    // The scan is a shortcut for the *read*, not for the "is
                    // it there" answer: an absent or unreadable file's scan
                    // verdict carries the empty digest, and a legitimate empty
                    // file's manifest entry has exactly that digest. Presence
                    // and length are therefore checked here too, the same way
                    // the fallback below does it, so the two spellings of
                    // `--check-only` agree on a missing empty file.
                    let actual_len = util::file_len(path);
                    if actual_len != Some(f.size as u64) {
                        bad += 1;
                        tracing::warn!(file = %f.rel, expect_size = f.size, actual_size = ?actual_len, "CHECK fail: size");
                    } else if v.file_md5.eq_ignore_ascii_case(&f.md5) {
                        // intact
                    } else {
                        bad += 1;
                        damaged_chunks += v.damaged_chunks();
                        tracing::warn!(
                            file = %f.rel,
                            expect_md5 = %f.md5,
                            actual_md5 = %v.file_md5,
                            damaged_chunks = v.damaged_chunks(),
                            "CHECK fail: md5"
                        );
                    }
                }
            },
            None => match &path {
                Err(e) => {
                    bad += 1;
                    tracing::warn!(
                        file = %f.rel,
                        error = %format!("{e:#}"),
                        "CHECK fail: path escapes the game dir"
                    );
                }
                Ok(path) => {
                    let actual_len = util::file_len(path);
                    if actual_len != Some(f.size as u64) {
                        bad += 1;
                        tracing::warn!(file = %f.rel, expect_size = f.size, actual_size = ?actual_len, "CHECK fail: size");
                    } else {
                        match util::md5_file(path) {
                            Ok(h) if h.eq_ignore_ascii_case(&f.md5) => {}
                            Ok(h) => {
                                bad += 1;
                                tracing::warn!(file = %f.rel, expect_md5 = %f.md5, actual_md5 = %h, "CHECK fail: md5");
                            }
                            Err(e) => {
                                bad += 1;
                                tracing::warn!(file = %f.rel, "CHECK fail: unreadable: {e:#}");
                            }
                        }
                    }
                }
            },
        }
        // Keep the counters live so PROGRESS reflects work done so far.
        summary
            .files_skipped
            .store((i as u64 + 1) - bad.min(i as u64 + 1), Ordering::Relaxed);
        summary.files_failed.store(bad, Ordering::Relaxed);
        if last_emit.elapsed().as_secs() >= report::PROGRESS_INTERVAL_SECS {
            report::emit_progress(summary, output);
            last_emit = std::time::Instant::now();
        }
    }
    tracing::info!(
        "check-only: total={} bad={} damaged_chunks={}",
        plan.files.len(),
        bad,
        damaged_chunks
    );
    bad
}
