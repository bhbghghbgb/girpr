//! `girsync compare` — report how dst differs from src, changing nothing itself.

use anyhow::Result;
use tracing::info;

use crate::config::{CompareOpts, LogCtx, ScanMode};
use crate::diff::diff_maps;
use crate::effective::{Side, classify, ensure_distinct_sides, load_side};
use crate::util::elapsed_s;

use super::report_diff;

/// Diff two sides and print one line per difference plus a `SUMMARY`.
///
/// Each side may independently be a folder root or a `girpr-cache*` record
/// directory, so all four combinations work. Exits `4` if anything differs.
pub fn cmd_compare(opts: CompareOpts, log: &LogCtx) -> Result<i32> {
    let t0 = std::time::Instant::now();
    let CompareOpts {
        src,
        dst,
        trust,
        common,
    } = opts;
    let span = tracing::info_span!(
        "girsync.compare",
        pid = std::process::id(),
        src = %src.display(),
        dst = %dst.display(),
        algos = ?common.algos,
        no_trust_src = trust.no_trust_src,
        no_trust_dst = trust.no_trust_dst,
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
    // A folder side writes its cache while scanning, so naming one cache twice
    // would diff a record against the view the run itself is mutating.
    ensure_distinct_sides(&s, &d)?;
    info!(src = %src.display(), src_kind = side_kind(&s), "load src side");
    // compare only reads, so it never writes cache rows and never dry-runs.
    let sm = load_side(
        &s,
        &common,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_src,
            dry_run: false,
        },
    )?;
    info!(side = "src", entries = sm.len(), "side loaded");
    info!(dst = %dst.display(), dst_kind = side_kind(&d), "load dst side");
    let dm = load_side(
        &d,
        &common,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_dst,
            dry_run: false,
        },
    )?;
    info!(side = "dst", entries = dm.len(), "side loaded");
    info!(src_entries = sm.len(), dst_entries = dm.len(), "diffing");
    let diff = diff_maps(&sm, &dm, &common.algos, common.case_sensitive);
    let code = report_diff(&diff);
    info!(
        missing = diff.missing.len(),
        extra = diff.extra.len(),
        changed = diff.changed.len(),
        type_conflict = diff.type_conflict.len(),
        case_mismatch = diff.case_mismatch.len(),
        total_diff = diff.total(),
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
