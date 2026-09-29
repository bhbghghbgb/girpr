//! `girsync sync` — mirror src onto dst.
//!
//! This module is only the coordinator; the work lives in the submodules:
//!
//! - [`rename`] — fix dst casing to match src (insensitive mode)
//! - [`plan`] — turn the diff into a work list, and print it for `--dry-run`
//! - [`apply`] — execute that work list, in an order that is load-bearing
//!
//! Phase order overall:
//! 1. validate inputs, back up both caches, snapshot dst's pre-sync state,
//! 2. open both caches and build the two effective maps,
//! 3. rename dst paths to src's casing, so the diff lines up on exact keys,
//! 4. plan, then either print it (`--dry-run`) or apply it.
//!
//! A record path (`girpr-cache*`) is not a valid sync input — that is
//! `compare`-only. Errors propagate as `Err`; the exit code is always 0.

mod apply;
mod plan;
mod rename;

use anyhow::{Result, bail};
use tracing::info;

use crate::cache::{
    CACHE_PREFIX, CacheDb, CacheOpen, backup_db, open_db, remove_cache_path, snapshot_old,
};
use crate::config::{LogCtx, ScanMode, SyncOpts};
use crate::diff::diff_maps;
use crate::effective::{build_effective_folder, classify, ensure_distinct_sides};
use crate::util::{elapsed_s, is_record_path};

use apply::Applier;
use plan::build_plan;
use rename::rename_to_src_casing;

/// Mirror `src` onto `dst`. Returns the process exit code.
pub fn cmd_sync(opts: SyncOpts, log: &LogCtx) -> Result<i32> {
    let t0 = std::time::Instant::now();
    let SyncOpts {
        src,
        dst,
        trust,
        missing_only,
        keep_extra,
        dry_run,
        jobs,
        common,
    } = opts;

    let span = tracing::info_span!(
        "girsync.sync",
        pid = std::process::id(),
        src = %src.display(),
        dst = %dst.display(),
        algos = ?common.algos,
        no_trust_src = trust.no_trust_src,
        no_trust_dst = trust.no_trust_dst,
        missing_only,
        keep_extra,
        dry_run,
        jobs,
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

    validate_inputs(&src, &dst, jobs)?;

    // 1. Backups and snapshots, before either cache is touched.
    let src_db_path = src.join(CACHE_PREFIX);
    let dst_db_path = dst.join(CACHE_PREFIX);
    if dry_run {
        info!("dry-run: skipping backups and cache writes");
    } else {
        if src_db_path.exists() {
            backup_db(&src_db_path)?;
        }
        if dst_db_path.exists() {
            backup_db(&dst_db_path)?;
            // old-state snapshot before any dst changes
            snapshot_old(&dst_db_path)?;
        }
        if common.ignore_cache {
            for p in [&src_db_path, &dst_db_path] {
                if p.exists() {
                    remove_cache_path(p)?;
                    info!(path = %p.display(), "ignore-cache removed");
                }
            }
        }
    }

    // 2. Open both caches and resolve each side's effective map.
    let (src_db, dst_db) = open_caches(&src_db_path, &dst_db_path, &common, dry_run)?;
    info!("loading src effective map");
    let sm = build_effective_folder(
        &src,
        &src_db,
        &common,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_src,
            dry_run,
        },
    )?;
    info!("loading dst effective map");
    let mut dm = build_effective_folder(
        &dst,
        &dst_db,
        &common,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_dst,
            dry_run,
        },
    )?;
    info!(src_entries = sm.len(), dst_entries = dm.len(), "maps ready");

    // 3. Align casing before diffing, so the diff can be case-sensitive.
    let renamed = if common.case_sensitive {
        0
    } else {
        rename_to_src_casing(&sm, &dst, &dst_db, &mut dm, dry_run)?
    };

    // 4. Plan, then print or apply.
    let diff = diff_maps(
        &sm,
        &dm,
        &common.algos,
        true, /* post-rename: exact keys */
    );
    let plan = build_plan(&sm, &dm, &diff, missing_only, keep_extra);
    info!(
        renamed,
        mkdir = plan.mkdir.len(),
        copy = plan.copy.len(),
        delete_files = plan.delete_files.len(),
        fix_dirs = plan.fix_dirs.len(),
        dry_run,
        "plan"
    );
    tracing::debug!(copy = ?plan.copy, "plan copy list");
    tracing::debug!(mkdir = ?plan.mkdir, "plan mkdir list");

    if dry_run {
        plan::print_dry_run(&plan, renamed, missing_only, keep_extra);
        info!(
            renamed,
            mkdir = plan.mkdir.len(),
            copy = plan.copy.len(),
            delete = plan.deletes(),
            dry_run = true,
            elapsed_s = elapsed_s(t0),
            exit = 0,
            "end"
        );
        return Ok(0);
    }

    let applied = Applier {
        src: &src,
        dst: &dst,
        dst_db: &dst_db,
        sm: &sm,
        dm: &dm,
        common: &common,
        jobs,
    }
    .apply(&plan)?;
    // No final flush: every apply phase commits its own mutations, and the
    // pre-drop commit before the filesystem changes is the crash-safety
    // point (a rerun re-copies rather than trusting half-written files).

    println!(
        "SUMMARY renamed={} mkdir={} copied={} deleted={} rmdir={} missing_only={} keep_extra={}",
        renamed,
        plan.mkdir.len(),
        applied.copied,
        applied.deleted,
        applied.removed_dirs,
        missing_only,
        keep_extra
    );
    info!(
        renamed,
        mkdir = plan.mkdir.len(),
        copied = applied.copied,
        deleted = applied.deleted,
        rmdir = applied.removed_dirs,
        missing_only,
        keep_extra,
        dry_run = false,
        elapsed_s = elapsed_s(t0),
        exit = 0,
        "end"
    );
    Ok(0)
}

/// Reject the inputs that make a mirror meaningless, before anything is touched.
fn validate_inputs(src: &std::path::Path, dst: &std::path::Path, jobs: usize) -> Result<()> {
    if is_record_path(src) || is_record_path(dst) {
        bail!("sync needs folder vs folder (record inputs are compare-only)");
    }
    if !src.is_dir() || !dst.is_dir() {
        bail!("src and dst must both be directories");
    }
    // One cache per run: two spellings of one folder would otherwise make the
    // mirror delete src out from under itself, on top of the handle clash.
    ensure_distinct_sides(&classify(src), &classify(dst))?;
    if jobs == 0 {
        bail!("--jobs must be >= 1");
    }
    Ok(())
}

/// Open both caches. Under `--dry-run` neither one is ever opened for writing.
///
/// A dry run has nothing to persist: `build_effective_folder` skips its batch
/// handle and `rename_to_src_casing` returns before touching disk, so the only
/// writes a read/write open could perform are the ones a dry run promises not to
/// do — creating a missing cache, and rewriting `meta` when the case mode
/// disagrees. So an existing cache is opened read-only, and a missing one falls
/// back to an in-memory DB that leaves the folder exactly as it was. A corrupt
/// cache falls back the same way: a dry run should still produce a plan.
fn open_caches(
    src_db_path: &std::path::Path,
    dst_db_path: &std::path::Path,
    common: &crate::config::CommonOpts,
    dry_run: bool,
) -> Result<(CacheDb, CacheDb)> {
    let open = |p: &std::path::Path| -> Result<CacheDb> {
        if !dry_run {
            return open_db(
                p,
                common.case_sensitive,
                CacheOpen::ReadWrite {
                    ignore_cache: false,
                    backup_first: false,
                },
            );
        }
        // `--dry-run --ignore-cache` asks for the cache to be rebuilt, and a dry
        // run may not rebuild it. Act as though it were absent: the folder is
        // then scanned from disk alone and the real cache is left untouched.
        if common.ignore_cache || !p.exists() {
            info!(path = %p.display(), "dry-run: using an in-memory cache");
            return CacheDb::open_temp(common.case_sensitive);
        }
        open_db(p, common.case_sensitive, CacheOpen::ReadOnly)
            .or_else(|_| CacheDb::open_temp(common.case_sensitive))
    };
    Ok((open(src_db_path)?, open(dst_db_path)?))
}
