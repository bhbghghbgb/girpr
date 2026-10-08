//! `girsync compare` — report how dst differs from src, changing nothing itself.

use std::path::Path;

use anyhow::Result;
use tracing::info;

use crate::config::{CompareOpts, LogCtx, ScanMode};
use crate::diff::diff_maps;
use crate::effective::{Side, classify, ensure_distinct_sides, open_side, resolve_side};
use crate::planner::{SideRequest, plan_pairs};
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
        output = %log.output,
    );
    let _span_guard = span.enter();
    info!("start");
    if common.dry_run {
        info!(
            "dry-run: no cache will be created, updated, or backed up; \
             both caches come out byte-identical"
        );
    }
    let s = classify(&src);
    let d = classify(&dst);
    // A folder side writes its cache while scanning, so naming one cache twice
    // would diff a record against the view the run itself is mutating.
    ensure_distinct_sides(&s, &d)?;
    info!(src = %src.display(), src_kind = side_kind(&s), "load src side");
    // Both handles stay live across the plan and the resolve, which is sound only
    // because `ensure_distinct_sides` already ran: redb allows one writable
    // handle per cache file.
    //
    // `compare` has no filesystem half to suppress — it never touches the trees —
    // but a folder side does write its cache as it resolves, so `--dry-run` is
    // carried into `ScanMode` and `open_folder_cache` turns that into a read-only
    // or in-memory handle. Every decision below is unchanged either way.
    let mut s = open_side(
        &s,
        &common,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_src,
            dry_run: common.dry_run,
        },
    )?;
    info!(
        side = "src",
        entries = s.phase_a.map.len(),
        "src side opened"
    );
    info!(dst = %dst.display(), dst_kind = side_kind(&d), "load dst side");
    let mut d = open_side(
        &d,
        &common,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_dst,
            dry_run: common.dry_run,
        },
    )?;
    info!(
        side = "dst",
        entries = d.phase_a.map.len(),
        "dst side opened"
    );

    // One decision, both sides in. A path is read only if it is a file on *both*
    // sides with equal size and mtime; every other pair state is already
    // decided, and `diff_maps` short-circuits the digest comparison anyway.
    //
    // The `?` is load-bearing: a side that cannot supply a requested digest —
    // which only a record can be — has to stop the run here, before either side
    // reads anything. Carrying on would report a confident verdict over content
    // the diff silently skipped.
    let s_label = describe(&src);
    let d_label = describe(&dst);
    let plans = plan_pairs(
        SideRequest {
            entries: &s.phase_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_src,
            cap: s.cap,
            label: &s_label,
        },
        SideRequest {
            entries: &d.phase_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_dst,
            cap: d.cap,
            label: &d_label,
        },
        common.case_sensitive,
        common.hash_mode,
    )?;
    info!(
        src_pending = plans.src.by_rel.len(),
        dst_pending = plans.dst.by_rel.len(),
        "planned"
    );
    let sm = resolve_side(
        &mut s,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_src,
            dry_run: common.dry_run,
        },
        &plans.src,
    )?;
    let dm = resolve_side(
        &mut d,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_dst,
            dry_run: common.dry_run,
        },
        &plans.dst,
    )?;
    info!(
        src_hashed = sm.stats.hashed,
        dst_hashed = dm.stats.hashed,
        "resolved"
    );
    let (sm, dm) = (sm.map, dm.map);
    info!(src_entries = sm.len(), dst_entries = dm.len(), "diffing");
    let diff = diff_maps(&sm, &dm, &plans.required, common.case_sensitive);
    let code = report_diff(&diff, &log.report());
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

/// How a coverage error should name this side.
///
/// The planner knows what a side *can do* but not what it *is*, and "the dst side
/// cannot supply…" is not something a user can act on where "the record
/// D:\game\old\girpr-cache cannot supply…" is.
fn describe(p: &Path) -> String {
    let kind = side_kind(&classify(p));
    format!("{kind} {}", p.display())
}
