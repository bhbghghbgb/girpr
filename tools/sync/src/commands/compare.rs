//! `girsync compare` — report how dst differs from src, changing nothing itself.

use anyhow::Result;
use tracing::info;

use crate::config::{CompareOpts, LogCtx, ScanMode};
use crate::effective::{Side, classify, compare_pair, ensure_distinct_sides, open_side};
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
    // would diff a record against the view the run itself is mutating. This must
    // run before either side is opened: it is what makes holding both handles
    // open across the plan sound.
    ensure_distinct_sides(&s, &d)?;
    info!(src = %src.display(), src_kind = side_kind(&s), "load src side");
    // Both sides are opened and statted first, then planned as a pair, then
    // resolved. A side planned alone would have to assume its counterpart is
    // there and that the metadata matches, and would read every file to find out
    // what the pair already decided.
    // compare only reads the filesystem, so it never dry-runs the FS; the cache
    // half of a dry run is what keeps a compare from persisting digests.
    let s_open = open_side(
        &s,
        &common,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_src,
            dry_run: false,
        },
    )?;
    info!(
        side = "src",
        entries = s_open.phase_a.map.len(),
        "side loaded"
    );
    info!(dst = %dst.display(), dst_kind = side_kind(&d), "load dst side");
    let d_open = open_side(
        &d,
        &common,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_dst,
            dry_run: false,
        },
    )?;
    info!(
        side = "dst",
        entries = d_open.phase_a.map.len(),
        "side loaded"
    );
    info!(
        src_entries = s_open.phase_a.map.len(),
        dst_entries = d_open.phase_a.map.len(),
        "diffing"
    );
    let pair = compare_pair(&s_open, &d_open, &common)?;
    let diff = pair.diff;
    let code = report_diff(&diff);
    info!(
        missing = diff.missing.len(),
        extra = diff.extra.len(),
        changed = diff.changed.len(),
        type_conflict = diff.type_conflict.len(),
        case_mismatch = diff.case_mismatch.len(),
        total_diff = diff.total(),
        src_hashed = pair.src.hashed,
        src_cache_hit = pair.src.cache_hit,
        src_stat_only = pair.src.stat_only,
        dst_hashed = pair.dst.hashed,
        dst_cache_hit = pair.dst.cache_hit,
        dst_stat_only = pair.dst.stat_only,
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
