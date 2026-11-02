//! Local state + server metadata (docs/02 steps 1–2).
//!
//! Four HoYoPlay calls: `getGameConfigs` and `getGameBranches` run as one
//! wave, then `getBuild(latest)` and `getBuild(local, best-effort)` as another,
//! while the local files are read in parallel. The deprecated-file list is *not*
//! fetched here: `repair::plan_phase` fetches it (for a run that reaches the
//! post-phase or a dry run) and passes it to the post-phase.
//!
//! The audio selection is resolved here rather than in step 1 because it needs
//! `audio_pkg_scan_dir` from `getGameConfigs`; the work is split between this
//! module (which calls it) and `audio` (which owns the file format).

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use tokio::task::JoinError;

#[cfg(test)]
use crate::config::RunExit;
use crate::config::{RunCtx, RunFailure};
use crate::hyp::{ChunkBuild, GameBranchPackage, GameConfig, HypClient, production_bases};
use crate::util;

use crate::repair::audio;

/// Everything steps 3+ need from steps 1–2.
pub struct Meta {
    /// Client, kept for the plan-phase manifest fetches and the
    /// `deprecated_files` call.
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

/// The blocking local reads and the exit class each one's join failure takes.
///
/// The classification lives in one place rather than at each `await`: the
/// version and audio reads are user-facing setup (exit 1), while the ignore and
/// blacklist lists are inputs to the repair metadata (exit 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalRead {
    Version,
    Ignore,
    Blacklist,
    Audio,
}

impl LocalRead {
    /// Every read, in the order the normal path reports a failure.
    #[cfg(test)]
    const ALL: [Self; 4] = [Self::Version, Self::Ignore, Self::Blacklist, Self::Audio];

    /// The label the join failure and the task's own context share.
    const fn label(self) -> &'static str {
        match self {
            Self::Version => "read local game version",
            Self::Ignore => "read res_category ignore list",
            Self::Blacklist => "read blacklist",
            Self::Audio => "resolve audio selection",
        }
    }

    /// The documented exit class for this read's failure.
    const fn class(self) -> ReadClass {
        match self {
            Self::Version | Self::Audio => ReadClass::Usage,
            Self::Ignore | Self::Blacklist => ReadClass::Metadata,
        }
    }

    /// The join failure for this read, with the read's label as context.
    fn join_failure(self, error: JoinError) -> RunFailure {
        self.class()
            .failure(anyhow::anyhow!("{}: {error}", self.label()))
    }
}

/// The two exit classes a local read can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadClass {
    Usage,
    Metadata,
}

impl ReadClass {
    /// The `RunFailure` constructor for this class.
    fn failure(self, source: anyhow::Error) -> RunFailure {
        match self {
            Self::Usage => RunFailure::usage(source),
            Self::Metadata => RunFailure::metadata(source),
        }
    }

    /// The exit code this class resolves to, for the classification test.
    #[cfg(test)]
    const fn exit(self) -> RunExit {
        match self {
            Self::Usage => RunExit::Usage,
            Self::Metadata => RunExit::Metadata,
        }
    }
}

/// Normalise one server/config directory spelling to the `/` form every gate
/// and sink shares, and validate its containment at the boundary.
///
/// The path scope matches the cleaned form (`./Audio/scan.txt` and
/// `Audio/scan.txt` are the same file), so the scope decision must not be made
/// on the raw spelling while the filesystem sink cleans it. Empty stays empty:
/// every caller treats an empty config value as "not configured".
fn clean_config_dir(game_dir: &Path, name: &str, dir: &str) -> Result<String, RunFailure> {
    if dir.is_empty() {
        return Ok(String::new());
    }
    let rel = util::clean_rel(dir)
        .with_context(|| name.to_string())
        .map_err(RunFailure::metadata)?;
    // The lexical cleaning above is what the gate uses; this also resolves the
    // nearest existing ancestor, so a symlink escape is refused here too.
    util::contained_join(game_dir, &rel)
        .with_context(|| name.to_string())
        .map_err(RunFailure::metadata)?;
    Ok(rel)
}

/// Read local state, then fetch the server metadata that defines the target.
///
/// The calls are overlapped where the data dependencies allow it: the two
/// config/branch lookups form one wave, the latest and local builds form a
/// second, and the local files are read on the blocking pool while the
/// network work runs.
pub async fn fetch(ctx: &RunCtx, game_dir: &Path) -> Result<Meta, RunFailure> {
    let hyp = hyp_client_for(ctx).map_err(RunFailure::metadata)?;

    // 1. Local state. An absent `config.ini` is a fresh install; an unreadable
    // one is an error, not a fresh install that the post-phase would then
    // overwrite the version of. The read needs only `game_dir`, so it starts
    // together with the first network wave rather than before it.
    let version_read = {
        let game_dir = game_dir.to_path_buf();
        tokio::task::spawn_blocking(move || {
            util::read_game_version(&game_dir)
                .context("read local game version")
                .map_err(RunFailure::usage)
        })
    };

    // 2. First server wave: the two calls are independent. `join!`, not
    // `try_join!`, so the error chosen below depends on the inputs rather than
    // on which future happened to be polled first.
    let (config_result, branch_result) = tokio::join!(hyp.game_config(), hyp.game_branch());
    let mut config = config_result
        .context("getGameConfigs")
        .map_err(RunFailure::metadata)?;
    let local_version = match version_read.await {
        Ok(inner) => inner?,
        Err(e) => return Err(LocalRead::Version.join_failure(e)),
    };
    tracing::info!("local game_version: {local_version:?}");

    // Chunk repair is the only mode this tool implements; FILE and LDIFF modes
    // have no per-file chunk manifests to patch from, so refuse rather than
    // guess (a wrong mode would otherwise surface later as a -202 from getBuild,
    // i.e. exit 2 instead of the documented usage error). The mode check stays
    // authoritative even though the branch call has already gone out.
    // See `Starward.Core/HoYoPlay/GameConfig.cs:DownloadMode` for the value set.
    if config.default_download_mode != crate::hyp::DOWNLOAD_MODE_CHUNK {
        return Err(RunFailure::usage(anyhow::anyhow!(
            "game download mode {:?} is not chunk mode; chunk repair not supported (unexpected for Genshin)",
            config.default_download_mode
        )));
    }
    let game_branch = branch_result
        .context("getGameBranches")
        .map_err(RunFailure::metadata)?;
    let branch = game_branch.main;
    let latest = branch.tag.clone();
    tracing::info!("latest version: {latest}");

    // These three dirs feed both a path-scope gate and a filesystem sink. Clean
    // each spelling once, here, so the gate and the sink compare the same
    // string; cleaning only inside `contained_join` would let a `./`-spelled
    // path pass a scope that hides its plain spelling.
    config.audio_pkg_scan_dir =
        clean_config_dir(game_dir, "audio_pkg_scan_dir", &config.audio_pkg_scan_dir)?;
    config.audio_pkg_cache_dir =
        clean_config_dir(game_dir, "audio_pkg_cache_dir", &config.audio_pkg_cache_dir)?;
    config.audio_pkg_res_dir =
        clean_config_dir(game_dir, "audio_pkg_res_dir", &config.audio_pkg_res_dir)?;
    tracing::info!(
        "game config: exe={} audio_scan={} audio_res={} audio_cache={} mode={}",
        config.exe_file_name,
        config.audio_pkg_scan_dir,
        config.audio_pkg_res_dir,
        config.audio_pkg_cache_dir,
        config.default_download_mode
    );

    // 3. Local reads: they need only `game_dir` and the cleaned config, so they
    // run on the blocking pool while the build manifests are fetched. The
    // audio dirs were normalised above; they feed a **move** sink and the
    // `--audio` pre-flight below, so the gate there and the sink use the
    // cleaned string rather than the raw server spelling.
    let ignore_read = {
        let game_dir = game_dir.to_path_buf();
        let dir = config.res_category_dir.clone();
        tokio::task::spawn_blocking(move || -> Result<HashSet<String>, RunFailure> {
            if dir.is_empty() {
                return Ok(HashSet::new());
            }
            let p = util::contained_join(&game_dir, &dir)
                .context("res_category_dir")
                .map_err(RunFailure::metadata)?;
            Ok(util::read_ignore_categories(&p)
                .context("read res_category ignore list")
                .map_err(RunFailure::usage)?
                .unwrap_or_default())
        })
    };
    let blacklist_read = {
        let game_dir = game_dir.to_path_buf();
        let enabled = config.enable_resource_blacklist && !config.blacklist_dir.is_empty();
        let dir = config.blacklist_dir.clone();
        tokio::task::spawn_blocking(move || -> Result<HashSet<String>, RunFailure> {
            if !enabled {
                return Ok(HashSet::new());
            }
            let p = util::contained_join(&game_dir, &dir)
                .context("blacklist_dir")
                .map_err(RunFailure::metadata)?;
            Ok(util::read_blacklist(&p)
                .context("read blacklist")
                .map_err(RunFailure::usage)?
                .unwrap_or_default())
        })
    };
    // Effective audio langs: explicit `--audio` (incl. `none` = game-only) wins
    // and overwrites the scan file; omitted flag keeps the detected set. An
    // absent scan file falls back to `en-us`; an unreadable one fails the run
    // rather than silently replacing the selection (R-2).
    let audio_read = {
        let game_dir = game_dir.to_path_buf();
        let scan_dir = config.audio_pkg_scan_dir.clone();
        let explicit = ctx.audio_explicit.then(|| ctx.audio.clone());
        tokio::task::spawn_blocking(move || {
            audio::resolve_effective_audio(&game_dir, &scan_dir, explicit.as_ref())
                .context("resolve audio selection")
                .map_err(RunFailure::usage)
        })
    };

    // 4. Second server wave: the latest build is fatal; the local one only
    // feeds chunk reuse (S2). A missing local version, or a server that no
    // longer serves that tag (retcode -202), just means more downloads — never
    // a wrong result.
    let local_build_fut = async {
        match local_version.as_deref() {
            Some(lv) => Some(hyp.chunk_build(&branch, Some(lv)).await),
            None => None,
        }
    };
    let (latest_result, local_result) =
        tokio::join!(hyp.chunk_build(&branch, None), local_build_fut);
    let latest_build = latest_result
        .context("getBuild(latest)")
        .map_err(RunFailure::metadata)?;
    let local_build = match (local_version.as_deref(), local_result) {
        (Some(lv), Some(Ok(b))) => {
            tracing::info!("local build manifest loaded for dedup (tag={lv})");
            Some(b)
        }
        (_, Some(Err(e))) => {
            tracing::warn!("local build unavailable ({e}); full-fetch fallback");
            None
        }
        _ => None,
    };

    let ignore = match ignore_read.await {
        Ok(inner) => inner?,
        Err(e) => return Err(LocalRead::Ignore.join_failure(e)),
    };
    let blacklist = match blacklist_read.await {
        Ok(inner) => inner?,
        Err(e) => return Err(LocalRead::Blacklist.join_failure(e)),
    };
    let audio = match audio_read.await {
        Ok(inner) => inner?,
        Err(e) => return Err(LocalRead::Audio.join_failure(e)),
    };
    // The write itself is deferred to the execution phase: the repair reads
    // the in-memory `audio` above, never the file.
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
        let scan_rel = &config.audio_pkg_scan_dir;
        if ctx.path_scope.is_active() && !ctx.path_scope.is_visible(scan_rel) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Each local read keeps the documented exit class, and the join-failure
    /// constructor follows it.
    #[test]
    fn every_local_read_keeps_its_documented_exit_class() {
        let table = [
            (
                LocalRead::Version,
                "read local game version",
                ReadClass::Usage,
                RunExit::Usage,
            ),
            (
                LocalRead::Ignore,
                "read res_category ignore list",
                ReadClass::Metadata,
                RunExit::Metadata,
            ),
            (
                LocalRead::Blacklist,
                "read blacklist",
                ReadClass::Metadata,
                RunExit::Metadata,
            ),
            (
                LocalRead::Audio,
                "resolve audio selection",
                ReadClass::Usage,
                RunExit::Usage,
            ),
        ];
        assert_eq!(
            LocalRead::ALL.len(),
            table.len(),
            "the table covers every local read"
        );
        for (read, label, class, exit) in table {
            assert!(LocalRead::ALL.contains(&read), "{read:?} is in ALL");
            assert_eq!(read.label(), label, "{read:?}");
            assert_eq!(read.class(), class, "{read:?}");
            assert_eq!(read.class().exit(), exit, "{read:?}");
            let failure = read.class().failure(anyhow::anyhow!("boom"));
            assert_eq!(failure.exit_code, exit, "{read:?}");
            assert!(format!("{failure:#}").contains("boom"), "{read:?}");
        }
    }
}
