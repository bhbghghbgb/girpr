//! `girsync compare` — report how dst differs from src.

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
///
/// **This command does not modify either tree, but it is not read-only**: a
/// folder side updates its own cache as it resolves, which is a disk write. So
/// `--dry-run` here means "write nothing at all", and it is honoured per side —
/// an existing cache is opened read-only, a missing one is served from memory.
/// The report is identical either way; only the writes differ. That is the whole
/// contract: `--dry-run` answers the same question, it does not answer a
/// cheaper one. See [`ScanMode::dry_run`].
pub fn cmd_compare(opts: CompareOpts, log: &LogCtx) -> Result<i32> {
    let t0 = std::time::Instant::now();
    let CompareOpts {
        src,
        dst,
        trust,
        dry_run,
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
        dry_run,
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
    // Both handles stay live across the plan and the resolve, which is sound only
    // because `ensure_distinct_sides` already ran: redb allows one writable
    // handle per cache file. Under `--dry-run` neither is writable, but the
    // lifetime requirement is the same.
    let src_mode = ScanMode {
        no_trust_cached_hashes: trust.no_trust_src,
        dry_run,
    };
    let mut s = open_side(&s, &common, src_mode)?;
    info!(
        side = "src",
        entries = s.phase_a.map.len(),
        "src side opened"
    );
    info!(dst = %dst.display(), dst_kind = side_kind(&d), "load dst side");
    let dst_mode = ScanMode {
        no_trust_cached_hashes: trust.no_trust_dst,
        dry_run,
    };
    let mut d = open_side(&d, &common, dst_mode)?;
    info!(
        side = "dst",
        entries = d.phase_a.map.len(),
        "dst side opened"
    );

    // One decision, both sides in. A path is read only if it is a file on *both*
    // sides with equal size and mtime; every other pair state is already
    // decided, and `diff_maps` short-circuits the digest comparison anyway.
    let plans = plan_pairs(
        SideRequest {
            entries: &s.phase_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_src,
        },
        SideRequest {
            entries: &d.phase_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_dst,
        },
        common.case_sensitive,
    );
    info!(
        src_pending = plans.src.by_rel.len(),
        dst_pending = plans.dst.by_rel.len(),
        "planned"
    );
    let sm = resolve_side(&mut s, src_mode, &plans.src)?;
    let dm = resolve_side(&mut d, dst_mode, &plans.dst)?;
    info!(
        src_hashed = sm.stats.hashed,
        dst_hashed = dm.stats.hashed,
        "resolved"
    );
    let (sm, dm) = (sm.map, dm.map);
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
