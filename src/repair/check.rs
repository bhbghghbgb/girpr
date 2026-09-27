use std::path::Path;
use std::sync::atomic::Ordering;

use crate::plan::RepairPlan;
use crate::report::{PROGRESS_INTERVAL_SECS, Summary, emit_progress};
use crate::util::{file_len, md5_file, normalize_rel};

/// `--check-only` verification pass: size + MD5 of every planned file, writes
/// nothing, and feeds the shared [`Summary`] so the `PROGRESS` line and the
/// final exit code (0 clean / 4 damaged) match a repair run's shape.
pub(crate) fn run_check_only(game_dir: &Path, plan: &RepairPlan, summary: &mut Summary) -> i32 {
    let mut bad = 0u64;
    summary.files_total = plan.files.len() as u64;
    let mut last_emit = std::time::Instant::now();
    for (i, f) in plan.files.iter().enumerate() {
        let p = game_dir.join(normalize_rel(&f.rel));
        let actual_len = file_len(&p);
        if actual_len != Some(f.size as u64) {
            bad += 1;
            tracing::warn!(file = %f.rel, expect_size = f.size, actual_size = ?actual_len, "CHECK fail: size");
        } else {
            match md5_file(&p) {
                Ok(h) if h.eq_ignore_ascii_case(&f.md5) => {}
                Ok(h) => {
                    bad += 1;
                    tracing::warn!(file = %f.rel, expect_md5 = %f.md5, actual_md5 = %h, "CHECK fail: md5");
                }
                Err(e) => {
                    bad += 1;
                    tracing::warn!(file = %f.rel, "CHECK fail: unreadable: {:#}", e);
                }
            }
        }
        // Keep the check-only counter live so PROGRESS reflects work done so far.
        summary
            .files_skipped
            .store((i as u64 + 1) - bad.min(i as u64 + 1), Ordering::Relaxed);
        summary.files_failed.store(bad, Ordering::Relaxed);
        if last_emit.elapsed().as_secs() >= PROGRESS_INTERVAL_SECS {
            emit_progress(summary);
            last_emit = std::time::Instant::now();
        }
    }
    tracing::info!("check-only: total={} bad={}", plan.files.len(), bad);
    summary.files_total = plan.files.len() as u64;
    summary.files_failed.store(bad, Ordering::Relaxed);
    if bad == 0 { 0 } else { 4 }
}
