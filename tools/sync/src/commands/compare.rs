//! `girsync compare` — report how dst differs from src, changing nothing itself.

use anyhow::Result;
use tracing::{debug, info};

use crate::config::{CompareOpts, LogCtx, ScanMode};
use crate::diff::diff_maps;
use crate::effective::{Side, classify, load_side};
use crate::util::elapsed_s;

/// Diff two sides and print one line per difference plus a `SUMMARY`.
///
/// Each side may independently be a folder root or a `girpr-cache*` record
/// directory, so all four combinations work. Exits `4` if anything differs.
pub fn cmd_compare(opts: CompareOpts, log: &LogCtx) -> Result<i32> {
    let t0 = std::time::Instant::now();
    let CompareOpts {
        src,
        dst,
        fast,
        common,
    } = opts;
    let span = tracing::info_span!(
        "girsync.compare",
        pid = std::process::id(),
        src = %src.display(),
        dst = %dst.display(),
        algos = ?common.algos,
        fast,
        include = ?common.includes,
        exclude = ?common.excludes,
        case_sensitive = common.case_sensitive,
        max_depth = common.max_depth,
        ignore_cache = common.ignore_cache,
        console_level = %log.level.to_ascii_lowercase(),
        file_level = "trace",
        log_file = %log.file_display(),
    );
    let _span_guard = span.enter();
    info!("start");
    let s = classify(&src);
    let d = classify(&dst);
    info!(src = %src.display(), src_kind = side_kind(&s), "load src side");
    // compare only reads, so it never writes cache rows and never dry-runs.
    let mode = ScanMode {
        fast,
        force_hash: false,
        dry_run: false,
    };
    let sm = load_side(&s, &common, mode)?;
    info!(side = "src", entries = sm.len(), "side loaded");
    info!(dst = %dst.display(), dst_kind = side_kind(&d), "load dst side");
    let dm = load_side(&d, &common, mode)?;
    info!(side = "dst", entries = dm.len(), "side loaded");
    info!(src_entries = sm.len(), dst_entries = dm.len(), "diffing");
    let diff = diff_maps(&sm, &dm, &common.algos, common.case_sensitive);
    for r in &diff.missing {
        println!("MISSING {}", r);
        debug!(kind = "missing", rel = %r, "diff");
    }
    for r in &diff.extra {
        println!("EXTRA {}", r);
        debug!(kind = "extra", rel = %r, "diff");
    }
    for r in &diff.changed {
        println!("CHANGED {}", r);
        debug!(kind = "changed", rel = %r, "diff");
    }
    for r in &diff.type_conflict {
        println!("TYPE-CONFLICT {}", r);
        debug!(kind = "type-conflict", rel = %r, "diff");
    }
    for (a, b) in &diff.case_mismatch {
        println!("CASE-MISMATCH {} <=> {}", a, b);
        debug!(kind = "case-mismatch", src_rel = %a, dst_rel = %b, "diff");
    }
    let total = diff.total();
    println!(
        "SUMMARY missing={} extra={} changed={} type_conflict={} case_mismatch={} total_diff={}",
        diff.missing.len(),
        diff.extra.len(),
        diff.changed.len(),
        diff.type_conflict.len(),
        diff.case_mismatch.len(),
        total
    );
    let code = if diff.is_empty() { 0 } else { 4 };
    info!(
        missing = diff.missing.len(),
        extra = diff.extra.len(),
        changed = diff.changed.len(),
        type_conflict = diff.type_conflict.len(),
        case_mismatch = diff.case_mismatch.len(),
        total_diff = total,
        elapsed_s = elapsed_s(t0),
        exit = code,
        "end"
    );
    Ok(code)
}

fn side_kind(s: &Side) -> &'static str {
    match s {
        Side::Record(_) => "record",
        Side::Folder(_) => "folder",
    }
}
