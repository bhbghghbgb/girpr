//! Local state + server metadata (docs/02 steps 1–2).
//!
//! Five HoYoPlay/Sophon calls, in order: `getGameConfigs` → `getGameBranches` →
//! `getBuild(latest)` → `getBuild(local, best-effort)`. The deprecated-file list
//! is *not* fetched here; it is lazy and lives in the post-phase.
//!
//! The audio selection is resolved here rather than in step 1 because it needs
//! `audio_pkg_scan_dir` from `getGameConfigs`; the work is split between this
//! module (which calls it) and `audio` (which owns the file format).

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::{RunCtx, RunFailure};
use crate::hyp::{ChunkBuild, GameBranchPackage, GameConfig, HypClient, production_bases};
use crate::util;

use crate::repair::audio;

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
    let sleep = ctx.sleep.clone();
    match (hyp, sophon) {
        (Some(h), Some(s)) => HypClient::new_with_bases(ctx.biz, h, s, sleep.clone()),
        (Some(h), None) => HypClient::new_with_bases(ctx.biz, h, default_sophon, sleep.clone()),
        (None, Some(s)) => HypClient::new_with_bases(ctx.biz, default_hyp, s, sleep.clone()),
        (None, None) => HypClient::new_with_bases(ctx.biz, default_hyp, default_sophon, sleep),
    }
}

/// Read local state, then fetch the server metadata that defines the target.
pub async fn fetch(ctx: &RunCtx, game_dir: &Path) -> Result<Meta, RunFailure> {
    // 1. local state. An absent `config.ini` is a fresh install; an unreadable
    // one is an error, not a fresh install that the post-phase would then
    // overwrite the version of.
    let local_version = util::read_game_version(game_dir)
        .context("read local game version")
        .map_err(RunFailure::usage)?;
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
        let p = util::contained_join(game_dir, &config.res_category_dir)
            .context("res_category_dir")
            .map_err(RunFailure::metadata)?;
        util::read_ignore_categories(&p)
            .context("read res_category ignore list")
            .map_err(RunFailure::usage)?
            .unwrap_or_default()
    };
    let blacklist = if config.enable_resource_blacklist && !config.blacklist_dir.is_empty() {
        let p = util::contained_join(game_dir, &config.blacklist_dir)
            .context("blacklist_dir")
            .map_err(RunFailure::metadata)?;
        util::read_blacklist(&p)
            .context("read blacklist")
            .map_err(RunFailure::usage)?
            .unwrap_or_default()
    } else {
        HashSet::new()
    };
    // The audio scan dir is server data too, and an explicit `--audio` writes
    // it in the execution phase. Validate it with the other config dirs: a
    // path out of the install is metadata, refused before the scan read and
    // before anything can be written.
    if !config.audio_pkg_scan_dir.is_empty() {
        util::contained_join(game_dir, &config.audio_pkg_scan_dir)
            .context("audio_pkg_scan_dir")
            .map_err(RunFailure::metadata)?;
    }
    // The cache/res pair feeds the post-phase **move** sink. It is server data
    // exactly like the scan dir above, so a bad value is metadata and refuses
    // the run before any mutation. The post phase still contains both (defence
    // in depth), but reaching it with a hostile value would bump `config.ini`
    // and exit `0` after skipping the move, which says the install is healthy
    // when the server data was not.
    for (name, dir) in [
        ("audio_pkg_cache_dir", &config.audio_pkg_cache_dir),
        ("audio_pkg_res_dir", &config.audio_pkg_res_dir),
    ] {
        if !dir.is_empty() {
            util::contained_join(game_dir, dir)
                .with_context(|| name.to_string())
                .map_err(RunFailure::metadata)?;
        }
    }

    // Effective audio langs: explicit `--audio` (incl. `none` = game-only) wins
    // and overwrites the scan file; omitted flag keeps the detected set. An
    // absent scan file falls back to `en-us`; an unreadable one fails the run
    // rather than silently replacing the selection (R-2).
    let explicit = ctx.audio_explicit.then_some(&ctx.audio);
    let audio = audio::resolve_effective_audio(game_dir, &config.audio_pkg_scan_dir, explicit)
        .context("resolve audio selection")
        .map_err(RunFailure::usage)?;
    // The write itself is deferred to the execution phase. An explicit
    // `--audio` used to write the scan file here, so a run that then refused
    // (a tripped ceiling, an empty `--include`, a hostile manifest rel, an
    // unwritable `--plan-out`) had already changed the install — and on a
    // missing one, created `game_dir` through the scan file's parent. The
    // repair reads the in-memory `audio` above, never the file.
    //
    // An explicit `--audio` under an active path scope is a contradiction, not
    // a merge — the flag would mutate a path the scope may not have admitted.
    // The escape hatch is to include the scan file's own rel path; it is never
    // hardcoded, it comes from `getGameConfigs`, and it is in scope like
    // anything else. This stays here because it is validation, and a refusal
    // must happen before the execution phase.
    if ctx.audio_explicit
        && !ctx.readonly()
        && !ctx.print_plan
        && !config.audio_pkg_scan_dir.is_empty()
    {
        let scan_rel = config.audio_pkg_scan_dir.replace('\\', "/");
        if ctx.path_scope.is_active() && !ctx.path_scope.is_visible(&scan_rel) {
            return Err(RunFailure::usage(anyhow::anyhow!(
                "--audio writes the scan file at '{scan_rel}', which is outside the active \
                 path scope (--include '{}' / --exclude '{}'); include it or drop one of the two flags",
                ctx.path_scope.describe_includes(),
                ctx.path_scope.describe_excludes(),
            )));
        }
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
