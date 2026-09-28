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

/// Verify the whole plan; returns the number of damaged files.
pub fn verify_plan(game_dir: &Path, plan: &RepairPlan, summary: &Summary) -> u64 {
    let mut bad = 0u64;
    let mut last_emit = std::time::Instant::now();
    for (i, f) in plan.files.iter().enumerate() {
        let p = game_dir.join(util::normalize_rel(&f.rel));
        let actual_len = util::file_len(&p);
        if actual_len != Some(f.size as u64) {
            bad += 1;
            tracing::warn!(file = %f.rel, expect_size = f.size, actual_size = ?actual_len, "CHECK fail: size");
        } else {
            match util::md5_file(&p) {
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
        // Keep the counters live so PROGRESS reflects work done so far.
        summary
            .files_skipped
            .store((i as u64 + 1) - bad.min(i as u64 + 1), Ordering::Relaxed);
        summary.files_failed.store(bad, Ordering::Relaxed);
        if last_emit.elapsed().as_secs() >= report::PROGRESS_INTERVAL_SECS {
            report::emit_progress(summary);
            last_emit = std::time::Instant::now();
        }
    }
    tracing::info!("check-only: total={} bad={}", plan.files.len(), bad);
    bad
}
