//! The post-phase: Starward-handling deletes + `config.ini` bump, then the
//! optional files-cleanup (docs/02 step 7).
//!
//! Order matters and is load-bearing:
//! 1. deprecated files (they are not in the live manifest, so the purge below
//!    would delete them too — the explicit loop keeps them cleaned even without
//!    any purge flag),
//! 2. audio cache → res move (Starward parity; prevents stranded duplicates),
//! 3. `--purge-after` files-cleanup,
//! 4. `config.ini` bump last, so the on-disk version only advances once the
//!    bytes are in place.
//!
//! The whole phase is skipped in read-only modes and whenever any file failed.

use std::path::Path;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::config::{RunCtx, RunFailure};
use crate::report::Summary;
use crate::util;

use super::metadata::Meta;
use super::plan::RepairPlan;
use super::purge;

/// Run the post-phase. Every failure here is fatal (`exit 3`): by this point
/// the bytes are already repaired, so a half-finished bookkeeping write would
/// desync the launcher from the game.
pub async fn run(
    ctx: &RunCtx,
    game_dir: &Path,
    plan: &RepairPlan,
    meta: &Meta,
    summary: &Summary,
) -> Result<(), RunFailure> {
    delete_deprecated(game_dir, meta, summary).await;
    move_audio_cache(game_dir, &meta.config);
    if ctx.purge_after {
        let bytes = purge::purge_extra(game_dir, plan, &purge::keep_set(), false)
            .map_err(RunFailure::write)?;
        summary.deleted_extra_bytes.fetch_add(bytes, Ordering::Relaxed);
    }
    let (ch, sub, cps) = ctx.biz.channel_tuple();
    util::write_config_ini(game_dir, &meta.latest, ctx.biz.as_str(), (ch, sub, cps), false)
        .context("write config.ini")
        .map_err(RunFailure::write)?;
    Ok(())
}

/// Delete every path the server still lists as deprecated. Best-effort: the
/// list is advisory, so a failed call only warns and the run continues.
async fn delete_deprecated(game_dir: &Path, meta: &Meta, summary: &Summary) {
    let list = match meta.hyp.deprecated_files().await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("deprecated list unavailable: {e:#}");
            return;
        }
    };
    for n in list {
        let p = game_dir.join(util::normalize_rel(&n));
        if p.is_file() {
            let sz = util::file_len(&p).unwrap_or(0);
            if std::fs::remove_file(&p).is_ok() {
                summary.deleted_extra_bytes.fetch_add(sz, Ordering::Relaxed);
                tracing::info!("deleted deprecated {n}");
            }
        }
    }
}

/// Move `audio_pkg_cache_dir/**/*` into `audio_pkg_res_dir`, Starward parity
/// (`GameInstallService.cs:427-445,643-661`).
///
/// Both dirs must be configured and distinct. Leftover empty cache dirs are
/// cleaned by the files-cleanup's emptied-dir sweep, whenever a purge is
/// enabled.
fn move_audio_cache(game_dir: &Path, config: &crate::hyp::GameConfig) {
    let (cache_dir, res_dir) = (&config.audio_pkg_cache_dir, &config.audio_pkg_res_dir);
    if cache_dir.is_empty() || res_dir.is_empty() || cache_dir == res_dir {
        return;
    }
    let cache = game_dir.join(util::normalize_rel(cache_dir));
    let res = game_dir.join(util::normalize_rel(res_dir));
    if !cache.is_dir() {
        return;
    }
    let files: Vec<std::path::PathBuf> = WalkDir::new(&cache)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .collect();
    for src in files {
        let Ok(rel) = src.strip_prefix(&cache) else {
            continue;
        };
        let target = res.join(rel);
        // `rename` first (cheap, same volume); fall back to copy+remove when
        // the file is in use or the rename is otherwise refused.
        if std::fs::create_dir_all(target.parent().unwrap_or(&res)).is_ok()
            && std::fs::rename(&src, &target).is_err()
        {
            let _ = std::fs::copy(&src, &target);
            let _ = std::fs::remove_file(&src);
        }
    }
    tracing::info!("moved audio cache -> res");
}
