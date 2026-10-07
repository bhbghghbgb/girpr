//! `girsync compare-self` - report how a folder's own cache has drifted from
//! the folder, changing nothing.
//!
//! This is `compare --src <DIR>/girpr-cache --dst <DIR>` collapsed to one
//! argument, and it differs from that spelling in exactly one respect that
//! matters: **the disk side is given no cache to consult.** The record is read
//! through a read-only handle, the folder is scanned against an *empty* one, and
//! the two are then compared as independent sides. That is what makes this an
//! audit rather than a self-consistency check.
//!
//! ## Why the disk side must have no cache
//!
//! A folder side's phase A takes `kind`, `size` and `mtime_ns` from the
//! filesystem and takes **digests** from its cache. Handing it the record's
//! cache therefore hands it the record's digests, so for every stat-equal pair
//! the content comparison became [`crate::diff::diff_maps`] comparing the
//! record's digest map against itself: always equal, and structurally unable to
//! report anything. Content that changed while preserving both size and mtime
//! was invisible, and `--no-trust-cached-hashes` was the only thing that could
//! surface it — which made the difference between "the cache matches the
//! folder's shape" and "the cache matches the folder's bytes" a matter of
//! remembering a flag, on the one command whose entire job is the second.
//!
//! A shared handle has a second, quieter problem, and it is in the logs of the
//! code this replaced: a folder side prunes rows it does not recognise, and
//! under a shared handle the record's own rows are in scope. Auditing a folder
//! whose disk spells `Case.txt` where the record holds `case.txt` had the disk
//! side decide to drop the record's `case.txt` row, while the record side was
//! holding that very row in memory. `ScanMode::dry_run` suppressed the write, so
//! nothing was lost — but the audit was one flag away from rewriting its own
//! subject, which is the same reason `ensure_distinct_sides` refuses the
//! two-sided spelling.
//!
//! [`CacheDb::open_temp`] is in-memory, so the guarantee is structural rather
//! than a flag that has to be threaded correctly: the disk side cannot reach the
//! record because it is not the same file. The write promise then no longer rests
//! on `ScanMode::dry_run` alone — which stays `true` here as a second,
//! independent reason rather than the only one.
//!
//! ## What this costs
//!
//! A stat-differing pair is still settled by size and mtime with no read at all,
//! so the audit is cheap on a drifted tree. An in-step tree is the other way
//! round: every pair is undecided, and a digest of the folder's bytes is the only
//! thing that can settle it, so the audit reads every file. That is the audit
//! doing its job rather than a regression, and `tests/lazy.rs` pins the counts
//! either way.
//!
//! ## The flags
//!
//! `--no-trust-cached-hashes` is now redundant — there is nothing left for it to
//! distrust — and `--dry-run` was already. Both are still accepted, so a script
//! passing them to every subcommand keeps working, and [`cmd_compare_self`] warns
//! rather than erroring.

use anyhow::{Result, bail};
use tracing::{info, warn};

use crate::cache::{CACHE_PREFIX, CacheDb, CacheOpen, open_db};
use crate::config::{CompareSelfOpts, LogCtx, ScanMode};
use crate::diff::diff_maps;
use crate::effective::{
    SideCapability, load_record_side_from, resolve_folder, resolve_record, scan_stat_only,
};
use crate::planner::{SideRequest, plan_pairs};
use crate::util::elapsed_s;

use super::report_diff;

/// Audit `<dir>` against `<dir>/girpr-cache`. Returns the process exit code:
/// `0` if the cache is in step, `4` if it has drifted.
///
/// The record is the `src` side, so `MISSING` is a row the cache still holds
/// for a path that is gone, `EXTRA` is a path on disk the cache never recorded,
/// and `CHANGED` is a stat or digest disagreement. Same vocabulary, same exit
/// code as `compare`; both print through one shared reporter.
pub fn cmd_compare_self(opts: CompareSelfOpts, log: &LogCtx) -> Result<i32> {
    let t0 = std::time::Instant::now();
    let CompareSelfOpts {
        dir,
        no_trust_cached_hashes,
        common,
    } = opts;
    let span = tracing::info_span!(
        "girsync.compare_self",
        pid = std::process::id(),
        dir = %dir.display(),
        algos = ?common.algos,
        no_trust_cached_hashes,
        include = ?common.includes,
        exclude = ?common.excludes,
        case_sensitive = common.case_sensitive,
        max_depth = common.max_depth,
        console_level = %log.level.to_ascii_lowercase(),
        file_level = "trace",
        log_file = %log.file_display(),
        output = %log.output,
    );
    let _span_guard = span.enter();
    info!("start");

    if common.dry_run {
        // Warned, not rejected, and now for a shared reason with
        // `--no-trust-cached-hashes`: neither flag changes what this command
        // checks any more. Erroring would break a script that passes `--dry-run`
        // to every subcommand to be safe, which is exactly the usage this
        // invites.
        warn!(
            "--dry-run is redundant for compare-self: this command writes nothing \
             (the disk side is scanned against an in-memory cache and the record \
             is opened read-only), so it was already a dry run"
        );
    }
    if no_trust_cached_hashes {
        warn!(
            "--no-trust-cached-hashes is redundant for compare-self: the disk side \
             is scanned with no cache to consult, so an undecided pair is always \
             rehashed. This flag used to be the only way to see content drift that \
             preserved size and mtime"
        );
    }

    if common.ignore_cache {
        // It would back up and delete the very record under audit, leaving
        // nothing to compare against.
        bail!(
            "--ignore-cache has no meaning for compare-self (it would delete the cache being compared)"
        );
    }
    if !dir.is_dir() {
        bail!("--dir {} is not a directory", dir.display());
    }
    let db_path = dir.join(CACHE_PREFIX);
    if !db_path.exists() {
        // Creating one would make the check vacuous on the first run and
        // meaningless on the second.
        bail!(
            "no cache at {} (run `girsync update --dir {}` first)",
            db_path.display(),
            dir.display()
        );
    }

    // Two handles, and they are different files.
    //
    // The record: the subject under audit, read-only. The read-only *open* is
    // what carries the no-write promise for this handle — it has no write path
    // at all, so a cache write could not commit even if some future code asked
    // for one.
    let record = open_db(&db_path, common.case_sensitive, CacheOpen::ReadOnly)?;
    info!(
        cache = %db_path.display(),
        entries = record.load_all()?.len(),
        "cache opened read-only"
    );
    let rec = load_record_side_from(&record, &common, &db_path.display().to_string())?;
    info!(
        side = "record",
        entries = rec.map.len(),
        "record side loaded"
    );

    // The disk side: **an empty handle**, which is the entire point. This is not
    // `open_folder_cache` with different arguments and not a second open policy —
    // there is no folder cache to open. A temp DB is in-memory, so the disk side
    // cannot consult, prune or rewrite the record it is being compared against,
    // and a folder side's only cache-derived input is digests.
    //
    // Its `case_sensitive` matches the run's, so the pairing rule
    // `plan_pairs` applies is the one `diff_maps` will apply.
    let cold = CacheDb::open_temp(common.case_sensitive)?;

    // Phase A for both roles, then one pair plan, then phase C — the shape
    // `compare` uses. The planner needs both sides' maps at once, which is why
    // both handles stay live across the plan.
    let mode = ScanMode {
        no_trust_cached_hashes: false,
        // Unconditionally true, and deliberately not read from the flag. It is
        // belt-and-braces rather than load-bearing — `cold` is in-memory and
        // `record` is read-only, so neither could be written regardless — but
        // keeping it means the no-write guarantee does not rest on one
        // remembered `true` if the handles are ever reworked.
        dry_run: true,
    };
    let disk_a = scan_stat_only(&dir, &cold, &common, mode)?;
    // Labels for a coverage error. The record is `src` here, so the record is what
    // a failure names — which is the side that cannot fill a gap, and the one the
    // remedy is about. The record's label carries the folder too, because on this
    // command the remedy is `girsync update --dir <that folder>` and the user
    // should not have to work out which directory a cache path belongs to.
    let rec_label = format!("record {} (for {})", db_path.display(), dir.display());
    let disk_label = format!("folder {}", dir.display());
    // The `?` stops the run before either side reads anything: a record that
    // cannot supply a requested digest must fail rather than let the pair fall
    // back to size+mtime, which for an audit means reporting "in step" about
    // content nobody compared.
    let plans = plan_pairs(
        SideRequest {
            entries: &rec.map,
            algos: &common.algos,
            // A record has nothing to distrust: its rows are the answer, not a
            // cache with a possibly-stale entry. Trust is absent from coverage
            // for the same reason.
            no_trust: false,
            cap: SideCapability::record(),
            label: &rec_label,
        },
        SideRequest {
            entries: &disk_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: SideCapability::for_folder(&cold, mode),
            label: &disk_label,
        },
        common.case_sensitive,
    )?;
    info!(
        record_pending = plans.src.by_rel.len(),
        disk_pending = plans.dst.by_rel.len(),
        "planned"
    );
    let disk = resolve_folder(&dir, &cold, mode, &disk_a, &plans.dst)?;
    let rec = resolve_record(&rec);
    info!(
        side = "disk",
        entries = disk.map.len(),
        hashed = disk.stats.hashed,
        cache_hit = disk.stats.cache_hit,
        "disk side loaded"
    );

    let (rec, disk) = (rec.map, disk.map);
    info!(
        record_entries = rec.len(),
        disk_entries = disk.len(),
        "diffing"
    );
    let diff = diff_maps(&rec, &disk, &common.algos, common.case_sensitive);
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
