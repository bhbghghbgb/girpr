//! `girsync compare-self` — report how a folder's own cache has drifted from
//! the folder, changing nothing.
//!
//! This is `compare --src <DIR>/girpr-cache --dst <DIR>` collapsed to one
//! argument, with the one thing that made that spelling impossible: the cache
//! is opened read-only. A plain `compare` of a record against a folder is
//! barred (see [`crate::effective::ensure_distinct_sides`]) precisely because a
//! folder side rewrites its cache while scanning — the audit would repair the
//! drift it was meant to report, and a second run would come back clean. Here
//! the disk side is read without ever opening the cache for writing, so the
//! report is the first thing that happens to the drift rather than the last.
//!
//! **What the default does and does not check.** The disk side is still built
//! with the cache available, so a file whose size+mtime match keeps its
//! recorded digest. That is what makes this cheap, and it is why the default
//! reports *stat* drift only: content that changed while preserving both size
//! and mtime is invisible until `--no-trust-cached-hashes` forces a rehash.
//! The flag is the difference between "is the cache in step with the folder's
//! shape" and "is it in step with the folder's bytes"; the name deliberately
//! promises no more than the former.

use anyhow::{Result, bail};
use tracing::info;

use crate::cache::{CACHE_PREFIX, CacheOpen, open_db};
use crate::config::{CompareSelfOpts, LogCtx, ScanMode};
use crate::diff::diff_maps;
use crate::effective::{build_effective_folder, load_record_side_from};
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
    );
    let _span_guard = span.enter();
    info!("start");

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

    // One read-only handle serves both sides. It is the read-only *open* that
    // carries the no-write promise, not the dry-run scan mode below: the handle
    // has no write path at all, so a cache write could not commit even if some
    // future code path asked for one.
    let cache = open_db(&db_path, common.case_sensitive, CacheOpen::ReadOnly)?;
    info!(cache = %db_path.display(), entries = cache.load_all()?.len(), "cache opened read-only");
    let rec = load_record_side_from(&cache, &common, &db_path.display().to_string())?;
    info!(side = "record", entries = rec.len(), "record side loaded");
    let disk = build_effective_folder(
        &dir,
        &cache,
        &common,
        ScanMode {
            no_trust_cached_hashes,
            // No cache writes. The FS half of a dry run is moot: this command
            // never touches the filesystem.
            dry_run: true,
        },
    )?;
    info!(side = "disk", entries = disk.len(), "disk side loaded");

    info!(
        record_entries = rec.len(),
        disk_entries = disk.len(),
        "diffing"
    );
    let diff = diff_maps(&rec, &disk, &common.algos, common.case_sensitive);
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
