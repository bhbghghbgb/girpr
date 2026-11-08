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
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use anyhow::{Context, Result};
use tokio::task::JoinError;

#[cfg(test)]
use crate::config::RunExit;
use crate::config::{RunCtx, RunFailure};
use crate::hyp::{ChunkBuild, GameBranchPackage, GameConfig, HypClient, production_bases};
use crate::util;

use crate::repair::{audio, spawn_blocking_traced};

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

/// Define the variants and the exhaustive list together, so [`LocalRead::ALL`]
/// cannot drift from the enum: adding a name here adds it to both.
macro_rules! local_reads {
    ($($variant:ident),+ $(,)?) => {
        /// The blocking local reads, and the exit class each one's **join**
        /// failure takes.
        ///
        /// The class here is the one for a task-level failure (a panic or a
        /// cancelled task on the blocking pool), not necessarily the class of
        /// the error the read itself returns. `Ignore` and `Blacklist` map an
        /// unreadable file to [`ReadClass::Usage`] inside the task, while their
        /// join failure is [`ReadClass::Metadata`]; `Version` and `Audio` use
        /// `Usage` for both. The join classes live in one table because
        /// [`Self::join_failure`] is their only reader.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum LocalRead {
            $($variant),+
        }

        impl LocalRead {
            /// Every read, in the order the normal path reports a failure.
            #[cfg(test)]
            const ALL: &[Self] = &[$(Self::$variant),+];
        }
    };
}

local_reads!(Version, Ignore, Blacklist, Audio);

impl LocalRead {
    /// The label the join failure and the task's own context share.
    const fn label(self) -> &'static str {
        match self {
            Self::Version => "read local game version",
            Self::Ignore => "read res_category ignore list",
            Self::Blacklist => "read blacklist",
            Self::Audio => "resolve audio selection",
        }
    }

    /// The documented exit class for this read's **join** failure.
    ///
    /// An `Ignore` or `Blacklist` read that cannot read its file reports
    /// [`ReadClass::Usage`]; its join failure is metadata. Failing to resolve
    /// the configured directory is metadata inside the task as well.
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

/// Await a blocking read whose value is no longer needed, so an early return
/// does not detach it from the run.
async fn drain<T>(read: tokio::task::JoinHandle<T>) {
    let _ = read.await;
}

/// A sibling future that may have settled while the primary was awaited.
enum Settled<F: Future> {
    /// The sibling finished first; its answer is kept.
    Ready(F::Output),
    /// The sibling is still running and can be awaited or dropped.
    Running(Pin<Box<F>>),
}

impl<F: Future> Settled<F> {
    /// Take the sibling's answer, waiting for it if it is still running.
    async fn wait(self) -> F::Output {
        match self {
            Self::Ready(value) => value,
            Self::Running(future) => future.await,
        }
    }
}

/// Await `primary` while keeping `sibling` alive; return as soon as `primary`
/// settles.
///
/// A sibling that settles first is retained rather than polled again, so the
/// caller can still use its answer when the primary succeeds. When the primary
/// settles first the returned [`Settled::Running`] is dropped by an early
/// return, which cancels the sibling's remaining retry schedule instead of
/// waiting it out. The primary is never cancelled, so an authoritative result
/// (including this phase's exit-code precedence) is not lost.
async fn settle_primary<P, S>(primary: P, sibling: S) -> (P::Output, Settled<S>)
where
    P: Future,
    S: Future,
{
    tokio::pin!(primary);
    let mut sibling = Box::pin(sibling);
    let mut settled = None;
    loop {
        tokio::select! {
            result = &mut primary => {
                let sibling = match settled {
                    Some(value) => Settled::Ready(value),
                    None => Settled::Running(sibling),
                };
                return (result, sibling);
            }
            result = &mut sibling, if settled.is_none() => settled = Some(result),
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
        spawn_blocking_traced(move || {
            util::read_game_version(&game_dir)
                .context(LocalRead::Version.label())
                .map_err(RunFailure::usage)
        })
    };

    // 2. First server wave: the two calls are independent. The config call is
    // the primary result: its mode decides the documented exit class, so it is
    // never cancelled. The branch answer is retained when it settles first, and
    // cancelled when the config call fails or refuses the mode.
    let (config_result, branch_pending) =
        settle_primary(hyp.game_config(), hyp.game_branch()).await;
    let mut config = match config_result
        .context("getGameConfigs")
        .map_err(RunFailure::metadata)
    {
        Ok(config) => config,
        Err(failure) => {
            drain(version_read).await;
            return Err(failure);
        }
    };
    // The response names the game it describes; a run for another biz must not
    // trust any of its paths or mode. An absent or empty `game.biz` has no
    // identity to check and is accepted.
    //
    // This check deliberately precedes the download-mode refusal below: every
    // field of a response for another game belongs to that game, so its mode is
    // not evidence about this run. A wrong-biz answer is therefore metadata
    // even when its mode would also be unsupported.
    if let Some(declared) = config.biz_mismatch(ctx.biz.as_str()) {
        drain(version_read).await;
        return Err(RunFailure::metadata(anyhow::anyhow!(
            "getGameConfigs answered biz {declared:?}, but this run requested {:?}",
            ctx.biz.as_str()
        )));
    }
    let local_version = match version_read.await {
        Ok(inner) => inner?,
        Err(e) => return Err(LocalRead::Version.join_failure(e)),
    };
    tracing::info!("local game_version: {local_version:?}");

    // Chunk repair is the only mode this tool implements; FILE and LDIFF modes
    // have no per-file chunk manifests to patch from, so refuse rather than
    // guess (a wrong mode would otherwise surface later as a -202 from getBuild,
    // i.e. exit 2 instead of the documented usage error). The mode check stays
    // authoritative even though the branch call has already gone out; the
    // branch future is dropped here, so its retry schedule is not awaited.
    // See `Starward.Core/HoYoPlay/GameConfig.cs:DownloadMode` for the value set.
    if config.default_download_mode != crate::hyp::DOWNLOAD_MODE_CHUNK {
        return Err(RunFailure::usage(anyhow::anyhow!(
            "game download mode {:?} is not chunk mode; chunk repair not supported (unexpected for Genshin)",
            config.default_download_mode
        )));
    }
    let game_branch = branch_pending
        .wait()
        .await
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
        spawn_blocking_traced(move || -> Result<HashSet<String>, RunFailure> {
            if dir.is_empty() {
                return Ok(HashSet::new());
            }
            let p = util::contained_join(&game_dir, &dir)
                .context("res_category_dir")
                .map_err(RunFailure::metadata)?;
            Ok(util::read_ignore_categories(&p)
                .context(LocalRead::Ignore.label())
                .map_err(RunFailure::usage)?
                .unwrap_or_default())
        })
    };
    let blacklist_read = {
        let game_dir = game_dir.to_path_buf();
        let enabled = config.enable_resource_blacklist && !config.blacklist_dir.is_empty();
        let dir = config.blacklist_dir.clone();
        spawn_blocking_traced(move || -> Result<HashSet<String>, RunFailure> {
            if !enabled {
                return Ok(HashSet::new());
            }
            let p = util::contained_join(&game_dir, &dir)
                .context("blacklist_dir")
                .map_err(RunFailure::metadata)?;
            Ok(util::read_blacklist(&p)
                .context(LocalRead::Blacklist.label())
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
        spawn_blocking_traced(move || {
            audio::resolve_effective_audio(&game_dir, &scan_dir, explicit.as_ref())
                .context(LocalRead::Audio.label())
                .map_err(RunFailure::usage)
        })
    };

    // 4. Second server wave: the latest build is fatal; the local one only
    // feeds chunk reuse (S2). A missing local version, or a server that no
    // longer serves that tag (retcode -202), just means more downloads — never
    // a wrong result. The latest call is primary, so a local-build retry
    // schedule is cancelled when the latest call fails.
    let local_build_fut = async {
        match local_version.as_deref() {
            Some(lv) => Some(hyp.chunk_build(&branch, Some(lv)).await),
            None => None,
        }
    };
    let (latest_result, local_pending) =
        settle_primary(hyp.chunk_build(&branch, None), local_build_fut).await;
    let latest_build = match latest_result
        .context("getBuild(latest)")
        .map_err(RunFailure::metadata)
    {
        Ok(build) => match build.tag_mismatch(&latest) {
            Some(declared) => {
                drain(ignore_read).await;
                drain(blacklist_read).await;
                drain(audio_read).await;
                return Err(RunFailure::metadata(anyhow::anyhow!(
                    "getBuild(latest) answered tag {declared:?}, but the branch requested {latest:?}"
                )));
            }
            None => build,
        },
        Err(failure) => {
            drain(ignore_read).await;
            drain(blacklist_read).await;
            drain(audio_read).await;
            return Err(failure);
        }
    };
    let local_build = match (local_version.as_deref(), local_pending.wait().await) {
        (Some(lv), Some(Ok(b))) => {
            // A local build is only a reuse optimization. A response whose own
            // tag names a different build is not the local one, so treat it
            // like an unavailable lookup rather than failing the run.
            if let Some(declared) = b.tag_mismatch(lv) {
                tracing::warn!(
                    "local build answered tag {declared:?}, not the local version {lv:?}; full-fetch fallback"
                );
                None
            } else {
                tracing::info!("local build manifest loaded for dedup (tag={lv})");
                Some(b)
            }
        }
        (_, Some(Err(e))) => {
            tracing::warn!("local build unavailable ({e}); full-fetch fallback");
            None
        }
        _ => None,
    };

    // Await every read before classifying, so one failing read does not leave
    // its siblings detached. The order below keeps the existing precedence.
    let ignore_result = ignore_read.await;
    let blacklist_result = blacklist_read.await;
    let audio_result = audio_read.await;
    let ignore = match ignore_result {
        Ok(inner) => inner?,
        Err(e) => return Err(LocalRead::Ignore.join_failure(e)),
    };
    let blacklist = match blacklist_result {
        Ok(inner) => inner?,
        Err(e) => return Err(LocalRead::Blacklist.join_failure(e)),
    };
    let audio = match audio_result {
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
    use crate::config::RepairMode;
    use crate::repair::scope::{Limits, PathScope};
    use crate::report::OutputFormat;
    use crate::sleep::{NoSleep, Sleep};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A `RunCtx` pointed at one loopback base, with every flag at its
    /// default so only the metadata phase is exercised.
    fn ctx(game_dir: std::path::PathBuf, base: String) -> RunCtx {
        RunCtx {
            game_dir,
            biz: crate::Biz::Hk4eGlobal,
            audio: HashSet::new(),
            audio_explicit: false,
            path_scope: PathScope::default(),
            limits: Limits::default(),
            repair_mode: RepairMode::Auto,
            repair_mode_explicit: false,
            jobs: 1,
            check_only: false,
            scan: false,
            purge_only: false,
            fail_fast: false,
            dry_run: false,
            print_plan: false,
            plan_out: None,
            purge_after: false,
            purge_before: false,
            output: OutputFormat::Text,
            hyp_base_override: Some(base.clone()),
            sophon_base_override: Some(base),
            sleep: Arc::new(NoSleep),
        }
    }

    /// Run `f` with a debug-level subscriber whose lines are captured.
    ///
    /// `set_default` is thread-local and the tests run in parallel, so the
    /// subscriber cannot leak into another test. The phase's blocking reads use
    /// `spawn_blocking_traced`, which installs the same dispatcher on the pool
    /// thread, so their lines are captured too.
    async fn logs_of<F: std::future::Future>(f: F) -> (F::Output, String) {
        #[derive(Clone, Default)]
        struct Captured(Arc<std::sync::Mutex<Vec<u8>>>);

        impl std::io::Write for Captured {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .expect("captured log lock")
                    .extend_from_slice(buf);
                Ok(buf.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let buf = Captured::default();
        let writer = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        let out = f.await;
        drop(guard);
        let text = String::from_utf8(buf.0.lock().expect("captured log lock").clone())
            .expect("tracing writes UTF-8");
        (out, text)
    }

    #[derive(Clone)]
    enum Reply {
        Body(&'static str),
        Status(&'static str),
    }

    const NOT_FOUND: Reply = Reply::Status("404 Not Found");

    /// Serve `getGameConfigs` and `getGameBranches` on an ephemeral loopback
    /// port; any other target is a `404`.
    async fn serve(config: Reply, branch: Reply) -> String {
        serve_metadata(config, branch, NOT_FOUND, None).await
    }

    /// Serve the metadata targets. A `getBuild` carrying a `tag` is answered by
    /// `local` when one is given, and by `latest` otherwise.
    async fn serve_metadata(
        config: Reply,
        branch: Reply,
        latest: Reply,
        local: Option<Reply>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    continue;
                };
                let config = config.clone();
                let branch = branch.clone();
                let latest = latest.clone();
                let local = local.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let mut got = 0;
                    while got < buf.len() {
                        match socket.read(&mut buf[got..]).await {
                            Ok(0) => break,
                            Ok(n) => {
                                got += n;
                                if buf[..got].windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            Err(_) => return,
                        }
                    }
                    let head = String::from_utf8_lossy(&buf[..got]).to_string();
                    let target = head
                        .lines()
                        .next()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .unwrap_or("/");
                    let reply = if target.contains("getGameBranches") {
                        &branch
                    } else if target.contains("getGameConfigs") {
                        &config
                    } else if target.contains("getBuild") {
                        match (&local, target.contains("tag=")) {
                            (Some(local), true) => local,
                            _ => &latest,
                        }
                    } else {
                        &NOT_FOUND
                    };
                    let (status, body) = match reply {
                        Reply::Body(body) => ("200 OK", body.as_bytes()),
                        Reply::Status(status) => (*status, &[][..]),
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(body).await;
                    let _ = socket.flush().await;
                });
            }
        });
        base
    }

    /// Each local read's class table names the class its join failure takes,
    /// and `join_failure` goes through that class rather than choosing one
    /// itself.
    ///
    /// The wildcard-free `match` below is also the exhaustiveness check: a new
    /// variant makes this test stop compiling, so it cannot silently be left
    /// out of the coverage.
    #[tokio::test]
    async fn every_local_read_join_failure_keeps_its_documented_exit_class() {
        for &read in LocalRead::ALL {
            let (label, class, exit) = match read {
                LocalRead::Version => ("read local game version", ReadClass::Usage, RunExit::Usage),
                LocalRead::Ignore => (
                    "read res_category ignore list",
                    ReadClass::Metadata,
                    RunExit::Metadata,
                ),
                LocalRead::Blacklist => ("read blacklist", ReadClass::Metadata, RunExit::Metadata),
                LocalRead::Audio => ("resolve audio selection", ReadClass::Usage, RunExit::Usage),
            };
            assert_eq!(read.label(), label, "{read:?}");
            assert_eq!(read.class(), class, "{read:?}");
            assert_eq!(read.class().exit(), exit, "{read:?}");
            let failure = read.class().failure(anyhow::anyhow!("boom"));
            assert_eq!(failure.exit_code, exit, "{read:?}");
            assert!(format!("{failure:#}").contains("boom"), "{read:?}");

            // An aborted task yields a real `JoinError`, so this exercises the
            // same constructor `fetch` uses rather than only the table.
            let join = {
                let task = tokio::task::spawn(std::future::pending::<()>());
                task.abort();
                task.await.expect_err("an aborted task fails to join")
            };
            let failure = read.join_failure(join);
            assert_eq!(failure.exit_code, exit, "{read:?}: join_failure");
            let text = format!("{failure:#}");
            assert!(text.contains(label), "{read:?}: {text}");
        }
    }

    /// A primary that settles first leaves the sibling unawaited: the caller
    /// can drop it, which is the cancellation an early return performs.
    #[tokio::test]
    async fn a_settled_primary_cancels_the_sibling_retry_schedule() {
        let dropped = Arc::new(AtomicBool::new(false));
        let attempts = Arc::new(AtomicUsize::new(0));

        struct Mark(Arc<AtomicBool>);
        impl Drop for Mark {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let mark = Mark(dropped.clone());
        let sibling = {
            let attempts = attempts.clone();
            async move {
                // Held by the future from construction, so dropping it before
                // the first poll still marks the cancellation.
                let _mark = mark;
                // The first retry's wait is instant, but the attempt after it
                // never settles. Awaiting this sibling would hang the phase.
                NoSleep.sleep(Duration::from_secs(1)).await;
                attempts.fetch_add(1, Ordering::SeqCst);
                std::future::pending::<()>().await;
            }
        };

        let (primary, pending) = settle_primary(std::future::ready("config failed"), sibling).await;
        assert_eq!(primary, "config failed");
        drop(pending);
        assert!(
            dropped.load(Ordering::SeqCst),
            "the sibling future was dropped, not awaited"
        );
        assert!(
            attempts.load(Ordering::SeqCst) <= 1,
            "the retry schedule was not followed"
        );
    }

    /// A sibling that settles first keeps its answer, including an error, for
    /// the primary's success path.
    #[tokio::test]
    async fn a_sibling_error_is_kept_when_the_primary_succeeds() {
        let sibling = async {
            tokio::task::yield_now().await;
            Err::<(), _>("branch failed")
        };
        let primary = async {
            tokio::task::yield_now().await;
            Ok::<_, &str>("config")
        };

        let (config, pending) = settle_primary(primary, sibling).await;
        assert_eq!(config, Ok("config"));
        assert_eq!(pending.wait().await, Err("branch failed"));
    }

    /// Draining a read waits for it rather than detaching it.
    #[tokio::test]
    async fn drain_waits_for_the_read_to_finish() {
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let read = tokio::task::spawn(async move {
            flag.store(true, Ordering::SeqCst);
        });

        drain(read).await;
        assert!(
            done.load(Ordering::SeqCst),
            "drain returns only after the read has completed"
        );
    }

    /// The config call's terminal failure is the phase's error even while the
    /// branch call is still retrying; the branch answer is never substituted.
    #[tokio::test]
    async fn a_config_failure_is_authoritative_over_a_retrying_branch() {
        let base = serve(
            Reply::Body(r#"{"retcode":-1001,"message":"invalid params","data":null}"#),
            Reply::Status("500 Internal Server Error"),
        )
        .await;
        let dir = girpr_tempdir::TempDir::new("girpr_test_metadata", "config_failure");
        let ctx = ctx(dir.to_path_buf(), base);

        let failure = match fetch(&ctx, &dir).await {
            Ok(_) => panic!("a terminal config failure must stop the phase"),
            Err(failure) => failure,
        };
        assert_eq!(failure.exit_code, RunExit::Metadata);
        let text = format!("{:#}", failure);
        assert!(text.contains("getGameConfigs"), "{text}");
        assert!(text.contains("-1001"), "{text}");
        assert!(
            !text.contains("getGameBranches"),
            "the branch error must not replace the config one: {text}"
        );
    }

    /// The branch call's terminal error is what the phase reports when the
    /// config call succeeds.
    ///
    /// Both first-wave answers are immediate and neither is retried, so the
    /// result does not depend on which one the selector sees first: a branch
    /// error that settles early is retained, and one still running is awaited
    /// after the config answer.
    #[tokio::test]
    async fn a_branch_error_is_reported_when_the_config_call_succeeds() {
        let base = serve(
            Reply::Body(
                r#"{"retcode":0,"message":"OK","data":{"launch_configs":[{"default_download_mode":"DOWNLOAD_MODE_CHUNK"}]}}"#,
            ),
            Reply::Body(r#"{"retcode":-1001,"message":"invalid params","data":null}"#),
        )
        .await;
        let dir = girpr_tempdir::TempDir::new("girpr_test_metadata", "branch_failure");
        let ctx = ctx(dir.to_path_buf(), base);

        let failure = match fetch(&ctx, &dir).await {
            Ok(_) => panic!("a terminal branch failure must stop the phase"),
            Err(failure) => failure,
        };
        assert_eq!(failure.exit_code, RunExit::Metadata);
        let text = format!("{:#}", failure);
        assert!(text.contains("getGameBranches"), "{text}");
        assert!(text.contains("-1001"), "{text}");
        assert!(
            !text.contains("getGameConfigs"),
            "the config call succeeded, so it must not be blamed: {text}"
        );
    }

    const CHUNK_CONFIG: &str = r#"{"retcode":0,"message":"OK","data":{"launch_configs":[{"game":{"id":"gopR6Cufr3","biz":"hk4e_global"},"default_download_mode":"DOWNLOAD_MODE_CHUNK"}]}}"#;
    const BRANCH: &str = r#"{"retcode":0,"message":"OK","data":{"game_branches":[{"main":{"package_id":"pkg","branch":"main","password":"","tag":"5.1.0"}}]}}"#;
    const LATEST_BUILD: &str =
        r#"{"retcode":0,"message":"OK","data":{"build_id":"b","tag":"5.1.0","manifests":[]}}"#;

    /// A response that names the biz and tag the run asked for is accepted, and
    /// its identity reaches the caller unchanged.
    #[tokio::test]
    async fn matching_biz_and_tag_are_accepted() {
        let base = serve_metadata(
            Reply::Body(CHUNK_CONFIG),
            Reply::Body(BRANCH),
            Reply::Body(LATEST_BUILD),
            None,
        )
        .await;
        let dir = girpr_tempdir::TempDir::new("girpr_test_metadata", "identity_match");
        let ctx = ctx(dir.to_path_buf(), base);

        let meta = fetch(&ctx, &dir)
            .await
            .expect("a matching identity is accepted");
        assert_eq!(meta.latest, "5.1.0");
        assert_eq!(meta.latest_build.tag, "5.1.0");
    }

    /// A config response for another business is metadata, not a run to
    /// continue with: its paths and download mode belong to a different game.
    #[tokio::test]
    async fn a_config_response_for_another_biz_is_refused_as_metadata() {
        const WRONG_BIZ: &str = r#"{"retcode":0,"message":"OK","data":{"launch_configs":[{"game":{"id":"1Z8W5NHUQb","biz":"hk4e_cn"},"default_download_mode":"DOWNLOAD_MODE_CHUNK"}]}}"#;
        let base = serve_metadata(
            Reply::Body(WRONG_BIZ),
            Reply::Body(BRANCH),
            Reply::Body(LATEST_BUILD),
            None,
        )
        .await;
        let dir = girpr_tempdir::TempDir::new("girpr_test_metadata", "wrong_biz");
        let ctx = ctx(dir.to_path_buf(), base);

        let failure = fetch(&ctx, &dir)
            .await
            .err()
            .expect("a response for another biz must be refused");
        assert_eq!(failure.exit_code, RunExit::Metadata);
        let text = format!("{:#}", failure);
        for needle in ["getGameConfigs", "hk4e_cn", "hk4e_global"] {
            assert!(
                text.contains(needle),
                "the refusal must name {needle}: {text}"
            );
        }
    }

    /// A response for another game is refused as metadata even when the
    /// download mode it names is one this tool would refuse: identity is
    /// checked first, because none of the other game's fields — including its
    /// mode — can be trusted.
    #[tokio::test]
    async fn a_wrong_biz_response_is_refused_before_its_download_mode() {
        const WRONG_BIZ_FILE_MODE: &str = r#"{"retcode":0,"message":"OK","data":{"launch_configs":[{"game":{"id":"1Z8W5NHUQb","biz":"hk4e_cn"},"default_download_mode":"DOWNLOAD_MODE_FILE"}]}}"#;
        let base = serve_metadata(
            Reply::Body(WRONG_BIZ_FILE_MODE),
            Reply::Body(BRANCH),
            Reply::Body(LATEST_BUILD),
            None,
        )
        .await;
        let dir = girpr_tempdir::TempDir::new("girpr_test_metadata", "wrong_biz_file_mode");
        let ctx = ctx(dir.to_path_buf(), base);

        let failure = fetch(&ctx, &dir)
            .await
            .err()
            .expect("a response for another biz must be refused");
        assert_eq!(failure.exit_code, RunExit::Metadata);
        let text = format!("{:#}", failure);
        assert!(text.contains("hk4e_cn"), "{text}");
        assert!(text.contains("hk4e_global"), "{text}");
        assert!(
            !text.contains("not chunk mode"),
            "the identity refusal must not be replaced by the mode one: {text}"
        );
    }

    /// The latest build's own tag is a second identity check: a build served
    /// for a different version than the branch resolved is metadata.
    #[tokio::test]
    async fn a_latest_build_for_another_tag_is_refused_as_metadata() {
        const WRONG_BUILD: &str =
            r#"{"retcode":0,"message":"OK","data":{"build_id":"b","tag":"5.0.0","manifests":[]}}"#;
        let base = serve_metadata(
            Reply::Body(CHUNK_CONFIG),
            Reply::Body(BRANCH),
            Reply::Body(WRONG_BUILD),
            None,
        )
        .await;
        let dir = girpr_tempdir::TempDir::new("girpr_test_metadata", "wrong_latest_tag");
        let ctx = ctx(dir.to_path_buf(), base);

        let failure = fetch(&ctx, &dir)
            .await
            .err()
            .expect("a build for another tag must be refused");
        assert_eq!(failure.exit_code, RunExit::Metadata);
        let text = format!("{:#}", failure);
        for needle in ["getBuild(latest)", "5.0.0", "5.1.0"] {
            assert!(
                text.contains(needle),
                "the refusal must name {needle}: {text}"
            );
        }
    }

    /// Absent identity fields are not disagreements: a config with no `game`
    /// and a build with no `tag` are accepted.
    #[tokio::test]
    async fn absent_biz_and_tag_are_tolerated() {
        const CONFIG_WITHOUT_GAME: &str = r#"{"retcode":0,"message":"OK","data":{"launch_configs":[{"default_download_mode":"DOWNLOAD_MODE_CHUNK"}]}}"#;
        const BUILD_WITHOUT_TAG: &str =
            r#"{"retcode":0,"message":"OK","data":{"build_id":"b","manifests":[]}}"#;
        let base = serve_metadata(
            Reply::Body(CONFIG_WITHOUT_GAME),
            Reply::Body(BRANCH),
            Reply::Body(BUILD_WITHOUT_TAG),
            None,
        )
        .await;
        let dir = girpr_tempdir::TempDir::new("girpr_test_metadata", "identity_absent");
        let ctx = ctx(dir.to_path_buf(), base);

        let meta = fetch(&ctx, &dir)
            .await
            .expect("an absent identity has nothing to check");
        assert_eq!(meta.latest_build.tag, "");
    }

    /// The local build is only a reuse optimization, so a tag mismatch degrades
    /// to the full-fetch fallback instead of failing the run; a matching tag is
    /// still kept.
    #[tokio::test]
    async fn a_local_build_tag_mismatch_degrades_to_the_fallback() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_metadata", "local_tag_mismatch");
        std::fs::write(dir.join("config.ini"), "[General]\ngame_version=5.0.0\n")
            .expect("write config.ini");

        const LOCAL_FOR_ANOTHER_TAG: &str = r#"{"retcode":0,"message":"OK","data":{"build_id":"old","tag":"4.9.0","manifests":[]}}"#;
        let base = serve_metadata(
            Reply::Body(CHUNK_CONFIG),
            Reply::Body(BRANCH),
            Reply::Body(LATEST_BUILD),
            Some(Reply::Body(LOCAL_FOR_ANOTHER_TAG)),
        )
        .await;
        let ref_ctx = ctx(dir.to_path_buf(), base);

        let (fetched, logs) = logs_of(fetch(&ref_ctx, &dir)).await;
        let meta = fetched.expect("a local build mismatch is not fatal");
        assert!(
            meta.local_build.is_none(),
            "a build for another tag must not be used as the local one"
        );
        assert!(
            logs.contains(
                r#"local build answered tag "4.9.0", not the local version "5.0.0"; full-fetch fallback"#
            ),
            "the fallback warning names both tags: {logs}"
        );
        assert!(
            logs.contains("no audio scan file; defaulting to"),
            "a line emitted by the blocking audio read is captured: {logs}"
        );

        const LOCAL_BUILD: &str = r#"{"retcode":0,"message":"OK","data":{"build_id":"old","tag":"5.0.0","manifests":[]}}"#;
        let base = serve_metadata(
            Reply::Body(CHUNK_CONFIG),
            Reply::Body(BRANCH),
            Reply::Body(LATEST_BUILD),
            Some(Reply::Body(LOCAL_BUILD)),
        )
        .await;
        let second_ctx = ctx(dir.to_path_buf(), base);

        let meta = fetch(&second_ctx, &dir)
            .await
            .expect("a matching local build is kept");
        assert_eq!(meta.local_build.expect("local build").tag, "5.0.0");
    }
}
