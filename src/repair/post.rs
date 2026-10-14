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
//! The deprecated list itself is **not** fetched here — it arrives as an
//! argument, having been read during the plan phase (docs/05 M0).

use std::path::Path;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::config::{RunCtx, RunFailure};
use crate::report::Summary;
use crate::util;

use super::metadata::Meta;
use super::plan::{RepairPlan, purge};
use super::scope::PathScope;

/// Run the post-phase. Every failure here is fatal (`exit 3`): by this point
/// the bytes are already repaired, so a half-finished bookkeeping write would
/// desync the launcher from the game.
///
/// `deprecated` is the server's deprecated-file list, already fetched (and
/// already possibly empty, if the call failed) during the plan phase.
pub async fn run(
    ctx: &RunCtx,
    game_dir: &Path,
    plan: &RepairPlan,
    meta: &Meta,
    summary: &Summary,
    deprecated: &[String],
) -> Result<(), RunFailure> {
    delete_deprecated(game_dir, deprecated, summary, &ctx.path_scope);
    move_audio_cache(game_dir, &meta.config, &ctx.path_scope);
    if ctx.purge_after {
        let purge_plan = purge::plan_purge(game_dir, plan, &purge::keep_set(), &ctx.path_scope);
        let bytes = purge::apply_purge(&purge_plan, false).map_err(RunFailure::write)?;
        summary
            .deleted_extra_bytes
            .fetch_add(bytes, Ordering::Relaxed);
    }
    // docs/04 §2.2: `config.ini` is bumped last so the on-disk version only
    // advances once the bytes are in place. Two situations make that premise
    // false by construction — the bytes were deliberately *not* put in place:
    //
    // - an active path scope (M2): the run only repaired a subset of the tree,
    // - `--purge-only` (M6): a cleanup put no bytes in place at all.
    //
    // Bumping would be exactly the launcher desync the ordering exists to
    // prevent. Skip it, say so, and let `SUMMARY` carry `config_bumped=0` so a
    // test can assert it instead of reading the file.
    if ctx.path_scope.is_active() {
        tracing::warn!(
            "path scope active: not bumping config.ini, so its game_version still describes the un-scoped parts of the install"
        );
        return Ok(());
    }
    if ctx.purge_only {
        tracing::warn!("--purge-only: not bumping config.ini; no bytes were repaired");
        return Ok(());
    }
    let (ch, sub, cps) = ctx.biz.channel_tuple();
    util::write_config_ini(
        game_dir,
        &meta.latest,
        ctx.biz.as_str(),
        (ch, sub, cps),
        false,
    )
    .context("write config.ini")
    .map_err(RunFailure::write)?;
    Ok(())
}

/// Delete every path the server still lists as deprecated. The list was
/// fetched during the plan phase and is advisory: an empty list (failed call)
/// simply means there is nothing to do. A path outside the active scope does
/// not exist, so it is not deleted either.
fn delete_deprecated(game_dir: &Path, list: &[String], summary: &Summary, scope: &PathScope) {
    for n in list {
        if !scope.is_visible(n) {
            tracing::debug!("deprecated path out of scope, kept: {n}");
            continue;
        }
        let p = game_dir.join(util::normalize_rel(n));
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
///
/// A scoped run does not relocate audio: a file whose source or destination is
/// outside the scope is left alone, since moving it is a mutation of a path the
/// scope may not have admitted (docs/04 §2).
fn move_audio_cache(game_dir: &Path, config: &crate::hyp::GameConfig, scope: &PathScope) {
    let (cache_dir, res_dir) = (&config.audio_pkg_cache_dir, &config.audio_pkg_res_dir);
    if cache_dir.is_empty() || res_dir.is_empty() || cache_dir == res_dir {
        return;
    }
    // Rel paths are `/`-separated everywhere except at the filesystem boundary.
    let cache_rel = cache_dir.replace('\\', "/");
    let res_rel = res_dir.replace('\\', "/");
    if !scope.is_visible(&cache_rel) || !scope.is_visible(&res_rel) {
        tracing::debug!("audio cache->res out of scope, skipped");
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
        if !scope.is_visible(&join_rel(&cache_rel, rel)) {
            continue;
        }
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

/// Join two rel path fragments into the `/`-separated form the scope matches on.
fn join_rel(dir_rel: &str, name: &std::path::Path) -> String {
    let name = name.to_string_lossy().replace('\\', "/");
    if dir_rel.is_empty() {
        name
    } else {
        format!("{dir_rel}/{name}")
    }
}
