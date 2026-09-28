//! Local state + server metadata (docs/02 steps 1–2).
//!
//! Five HoYoPlay/Sophon calls, in order: `getGameConfigs` → `getGameBranches` →
//! `getBuild(latest)` → `getBuild(local, best-effort)`. The deprecated-file list
//! is *not* fetched here; it is lazy and lives in the post-phase.
//!
//! The audio selection is resolved here rather than in step 1 because it needs
//! `audio_pkg_scan_dir` from `getGameConfigs`; the work is split between this
//! module (which calls it) and [`super::audio`] (which owns the file format).

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::{RunCtx, RunFailure};
use crate::hyp::{ChunkBuild, GameBranchPackage, GameConfig, HypClient, production_bases};
use crate::util;

use super::audio;

/// Everything steps 3+ need from steps 1–2.
pub struct Meta {
    /// Client, kept for the lazy `deprecated_files` call in the post-phase.
    pub hyp: HypClient,
    pub config: GameConfig,
    /// `getGameBranches` `main` package (branch/package_id/password/tag).
    pub branch: GameBranchPackage,
    pub latest: String,
    pub latest_build: ChunkBuild,
    /// `getBuild(local)` when the local version is known and still served;
    /// only ever an optimization for chunk reuse.
    pub local_build: Option<ChunkBuild>,
    /// On-disk `config.ini` version; `None` = fresh install.
    pub local_version: Option<String>,
    /// `res_category_dir` ignore set (manifests to drop).
    pub ignore: HashSet<String>,
    /// `blacklist_dir` set (files to drop from the work list).
    pub blacklist: HashSet<String>,
    /// Effective audio selection, canonical codes; empty = game-only.
    pub audio: HashSet<String>,
}

/// Build the API client, honoring the offline/test endpoint overrides
/// (explicit > `GIRPR_*_BASE` env > production). Production runs use
/// [`HypClient::new`].
fn hyp_client_for(ctx: &RunCtx) -> Result<HypClient> {
    let (hyp, sophon) = ctx.api_bases();
    let (default_hyp, default_sophon) = production_bases(ctx.biz.endpoints().0);
    match (hyp, sophon) {
        (Some(h), Some(s)) => HypClient::new_with_bases(ctx.biz, h, s),
        (Some(h), None) => HypClient::new_with_bases(ctx.biz, h, default_sophon),
        (None, Some(s)) => HypClient::new_with_bases(ctx.biz, default_hyp, s),
        (None, None) => HypClient::new(ctx.biz),
    }
}

/// Read local state, then fetch the server metadata that defines the target.
pub async fn fetch(ctx: &RunCtx, game_dir: &Path) -> Result<Meta, RunFailure> {
    // 1. local state
    let local_version = util::read_game_version(game_dir);
    tracing::info!("local game_version: {local_version:?}");

    // 2. server metadata (up to 5 calls)
    let hyp = hyp_client_for(ctx).map_err(RunFailure::metadata)?;
    let config = hyp
        .game_config()
        .await
        .context("getGameConfigs")
        .map_err(RunFailure::metadata)?;
    tracing::info!(
        "game config: exe={} audio_scan={} audio_res={} audio_cache={} mode={}",
        config.exe_file_name,
        config.audio_pkg_scan_dir,
        config.audio_pkg_res_dir,
        config.audio_pkg_cache_dir,
        config.default_download_mode
    );
    // Chunk repair is the only mode this tool implements; FILE and LDIFF modes
    // have no per-file chunk manifests to patch from, so refuse rather than
    // guess (a wrong mode would otherwise surface later as a -202 from getBuild,
    // i.e. exit 2 instead of the documented usage error).
    // See `Starward.Core/HoYoPlay/GameConfig.cs:DownloadMode` for the value set.
    if config.default_download_mode != crate::hyp::DOWNLOAD_MODE_CHUNK {
        return Err(RunFailure::usage(anyhow::anyhow!(
            "game download mode {:?} is not chunk mode; chunk repair not supported (unexpected for Genshin)",
            config.default_download_mode
        )));
    }

    let game_branch = hyp
        .game_branch()
        .await
        .context("getGameBranches")
        .map_err(RunFailure::metadata)?;
    let branch = game_branch.main;
    let latest = branch.tag.clone();
    tracing::info!("latest version: {latest}");
    let latest_build = hyp
        .chunk_build(&branch, None)
        .await
        .context("getBuild(latest)")
        .map_err(RunFailure::metadata)?;

    // The local build only feeds chunk reuse (S2). A missing local version, or a
    // server that no longer serves that tag (retcode -202), just means more
    // downloads — never a wrong result.
    let mut local_build = None;
    if let Some(lv) = local_version.as_deref() {
        match hyp.chunk_build(&branch, Some(lv)).await {
            Ok(b) => {
                tracing::info!("local build manifest loaded for dedup (tag={lv})");
                local_build = Some(b);
            }
            Err(e) => tracing::warn!("local build unavailable ({e}); full-fetch fallback"),
        }
    }

    let ignore = if config.res_category_dir.is_empty() {
        HashSet::new()
    } else {
        util::read_ignore_categories(&game_dir.join(util::normalize_rel(&config.res_category_dir)))
    };
    let blacklist = if config.enable_resource_blacklist && !config.blacklist_dir.is_empty() {
        util::read_blacklist(&game_dir.join(util::normalize_rel(&config.blacklist_dir)))
    } else {
        HashSet::new()
    };

    // Effective audio langs: explicit `--audio` (incl. `none` = game-only) wins
    // and overwrites the scan file; omitted flag keeps the detected set,
    // defaulting to `en-us` when undetectable.
    let explicit = ctx.audio_explicit.then_some(&ctx.audio);
    let audio = audio::resolve_effective_audio(game_dir, &config.audio_pkg_scan_dir, explicit);
    if ctx.audio_explicit && !ctx.readonly() && !config.audio_pkg_scan_dir.is_empty() {
        audio::write_scan_file(game_dir, &config.audio_pkg_scan_dir, &audio)
            .context("write audio scan file")
            .map_err(RunFailure::write)?;
    }

    Ok(Meta {
        hyp,
        config,
        branch,
        latest,
        latest_build,
        local_build,
        local_version,
        ignore,
        blacklist,
        audio,
    })
}
