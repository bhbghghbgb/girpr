use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use crate::hyp::{ChunkBuild, GameBranchPackage, GameConfig, HypClient};
use crate::sophon::{self, SophonChunkFile, WantedManifest};
use crate::util;
use crate::Biz;
use tracing::Instrument;

/// Typed failure carrying the process exit code, so the error contract in the
/// README / docs/02 (§1) is enforced at runtime instead of a blanket `2`.
/// `1` usage/config, `2` metadata/network, `3` write/verify.
#[derive(Debug)]
pub struct RunFailure {
    pub exit_code: i32,
    pub source: anyhow::Error,
}

impl RunFailure {
    pub fn usage(e: impl Into<anyhow::Error>) -> Self {
        Self {
            exit_code: 1,
            source: e.into(),
        }
    }
    pub fn metadata(e: impl Into<anyhow::Error>) -> Self {
        Self {
            exit_code: 2,
            source: e.into(),
        }
    }
    pub fn write(e: impl Into<anyhow::Error>) -> Self {
        Self {
            exit_code: 3,
            source: e.into(),
        }
    }
}

impl std::fmt::Display for RunFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.source)
    }
}
impl std::error::Error for RunFailure {}

pub struct RepairPlan {
    pub latest: String,
    pub files: Vec<PlannedFile>,
    pub url_prefix_by_file: HashMap<String, String>,
}

pub struct PlannedFile {
    pub rel: String, // forward-slash rel path
    pub size: i64,
    pub md5: String,
    pub chunks: Vec<sophon::SophonChunk>,
}

#[derive(Default)]
pub struct Summary {
    pub files_total: u64,
    pub files_skipped: AtomicU64,
    pub files_repaired: AtomicU64,
    pub files_failed: AtomicU64,
    pub download_bytes: AtomicU64,
    /// Unified cleanup counter (Collapse files-cleanup parity): every byte
    /// deleted by `--purge-before` / `--purge-after` (temps, orphans,
    /// deprecated files) lands here. No split accounting.
    pub deleted_extra_bytes: AtomicU64,
}

/// How often the background reporter emits `PROGRESS` during long phases.
pub const PROGRESS_INTERVAL_SECS: u64 = 10;

/// Format the one-line progress snapshot. Shared by stdout + log emission.
pub fn format_progress(sum: &Summary) -> String {
    let skipped = sum.files_skipped.load(Ordering::Relaxed);
    let repaired = sum.files_repaired.load(Ordering::Relaxed);
    let failed = sum.files_failed.load(Ordering::Relaxed);
    let done = skipped + repaired + failed;
    let dl = sum.download_bytes.load(Ordering::Relaxed);
    format!(
        "PROGRESS done={}/{} skipped={} repaired={} failed={} download_bytes={}",
        done, sum.files_total, skipped, repaired, failed, dl
    )
}

/// Emit progress to **both** stdout and the log so unattended runs keep it
/// regardless of which stream is captured.
/// NOTE(design-intent): the dual emission is deliberate (automation contract,
/// see README).
pub fn emit_progress(sum: &Summary) {
    let line = format_progress(sum);
    println!("{line}");
    tracing::info!("{line}");
}

/// The one-time begin-report written to stdout right after the metadata calls
/// (mirrored to the log). Parsers get both versions before any repair starts:
/// `local_version` is the on-disk `config.ini` value (`none` when absent),
/// `latest_version` is the "will be updated to" tag; the rest come from the
/// HoYoPlay/Sophon APIs, not from config. `audio_langs` is the effective set.
/// Emitted as a `REPORT key=value` line, or as a JSON object with
/// `--json-summary`.
pub fn format_report_line(
    local_version: &Option<String>,
    latest_version: &str,
    biz: &str,
    cfg: &GameConfig,
    pkg: &GameBranchPackage,
    build: &ChunkBuild,
    audio_langs: &HashSet<String>,
    json: bool,
) -> String {
    let mut langs: Vec<&String> = audio_langs.iter().collect();
    langs.sort();
    let audio_joined: Vec<&str> = langs.iter().map(|s| s.as_str()).collect();
    let audio = audio_joined.join(",");
    let local = local_version.as_deref().unwrap_or("none");
    let diff_tags = pkg.diff_tags.join(",");
    if json {
        serde_json::json!({
            "local_version": local,
            "latest_version": latest_version,
            "biz": biz,
            "exe": cfg.exe_file_name,
            "download_mode": cfg.default_download_mode,
            "branch": pkg.branch,
            "package_id": pkg.package_id,
            "build_id": build.build_id,
            "audio_langs": audio,
            "diff_tags": diff_tags,
        })
        .to_string()
    } else {
        format!(
            "REPORT local_version={local} latest_version={latest_version} biz={biz} exe={} download_mode={} branch={} package_id={} build_id={} audio_langs={audio} diff_tags={diff_tags}",
            cfg.exe_file_name, cfg.default_download_mode, pkg.branch, pkg.package_id, build.build_id
        )
    }
}

fn start_progress_reporter(sum: Arc<Summary>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(PROGRESS_INTERVAL_SECS));
        // First tick fires immediately; skip it so we only report after a full interval.
        interval.tick().await;
        loop {
            interval.tick().await;
            emit_progress(&sum);
        }
    })
}

pub struct RepairCtx {
    pub game_dir: PathBuf,
    pub biz: Biz,
    pub audio: HashSet<String>,
    pub jobs: usize,
    pub check_only: bool,
    pub dry_run: bool,
    pub purge_after: bool,
    pub purge_before: bool,
    pub json_summary: bool,
}

pub async fn run(ctx: RepairCtx) -> Result<(Summary, i32), RunFailure> {
    let mut summary = Summary::default();
    // Read-only modes must not write anything: `--dry-run` logs actions only,
    // `--check-only` verifies only. Every delete/write below is gated on this
    // (previously `--check-only` still swept temps and `--purge-before` still
    // deleted — both fixed by gating on `readonly`, not just `dry_run`).
    let readonly = ctx.dry_run || ctx.check_only;
    if !readonly {
        std::fs::create_dir_all(&ctx.game_dir)
            .context("create game dir")
            .map_err(RunFailure::usage)?;
    }
    let game_dir = ctx
        .game_dir
        .canonicalize()
        .unwrap_or_else(|_| ctx.game_dir.clone());

    // NOTE: no unconditional temp sweep. All cleanup is the single
    // Collapse-style files-cleanup (`collapse_purge_extra`), run before
    // and/or after patching only when the corresponding flag is set.
    // `--purge-before` and `--purge-after` delete the same things; only the
    // timing differs.

    // 1. local state
    let local_version = util::read_game_version(&game_dir);
    tracing::info!("local game_version: {:?}", local_version);

    // 2. server metadata (up to 5 calls)
    let hyp = HypClient::new(ctx.biz).map_err(RunFailure::metadata)?;
    let cfg = hyp
        .game_config()
        .await
        .context("getGameConfigs")
        .map_err(RunFailure::metadata)?;
    tracing::info!(
        "game config: exe={} audio_scan={} audio_res={} audio_cache={} mode={}",
        cfg.exe_file_name,
        cfg.audio_pkg_scan_dir,
        cfg.audio_pkg_res_dir,
        cfg.audio_pkg_cache_dir,
        cfg.default_download_mode
    );
    if cfg.default_download_mode == "DOWNLOAD_MODE_FILE" {
        return Err(RunFailure::usage(anyhow::anyhow!(
            "game is in legacy FILE mode; chunk repair not supported (unexpected for Genshin)"
        )));
    }
    let branch = hyp
        .game_branch()
        .await
        .context("getGameBranches")
        .map_err(RunFailure::metadata)?;
    let latest = branch.main.tag.clone();
    tracing::info!("latest version: {}", latest);
    let latest_build = hyp
        .chunk_build(&branch.main, None)
        .await
        .context("getBuild(latest)")
        .map_err(RunFailure::metadata)?;

    let mut local_build = None;
    if let Some(lv) = local_version.as_deref() {
        match hyp.chunk_build(&branch.main, Some(lv)).await {
            Ok(b) => {
                tracing::info!("local build manifest loaded for dedup (tag={})", lv);
                local_build = Some(b);
            }
            Err(e) => tracing::warn!("local build unavailable ({}); full-fetch fallback", e),
        }
    }

    let ignore = if cfg.res_category_dir.is_empty() {
        HashSet::new()
    } else {
        util::read_ignore_categories(&game_dir.join(normalize_rel(&cfg.res_category_dir)))
    };
    let blacklist = if cfg.enable_resource_blacklist && !cfg.blacklist_dir.is_empty() {
        util::read_blacklist(&game_dir.join(normalize_rel(&cfg.blacklist_dir)))
    } else {
        HashSet::new()
    };

    // effective audio langs
    let mut audio = ctx.audio.clone();
    if audio.is_empty() {
        audio = read_current_audio(&game_dir, &cfg.audio_pkg_scan_dir);
        if audio.is_empty() {
            audio.insert("en-us".to_string());
        }
        tracing::info!("keeping current audio langs: {:?}", audio);
    } else if !ctx.dry_run && !ctx.check_only && !cfg.audio_pkg_scan_dir.is_empty() {
        write_audio_scan(&game_dir, &cfg.audio_pkg_scan_dir, &audio)
            .context("write audio scan file")
            .map_err(RunFailure::write)?;
    }

    // 2.5 begin-report to stdout (mirrored to log): versions + API-sourced
    // metadata so a stdout parser sees what is on disk and what it will become
    // before any repair starts. JSON when `--json-summary`.
    // See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallService.cs#L788-L849
    let report_line = format_report_line(
        &local_version,
        &latest,
        ctx.biz.as_str(),
        &cfg,
        &branch.main,
        &latest_build,
        &audio,
        ctx.json_summary,
    );
    println!("{report_line}");
    tracing::info!("{report_line}");

    // 3. fetch + verify + parse latest manifests (per-manifest lists kept for prefix mapping)
    let http = hyp.http();
    let wanted = sophon::select_manifests(&latest_build, &audio, &ignore);
    let mut per_manifest: Vec<(String, Vec<SophonChunkFile>)> = Vec::new();
    for m in wanted {
        let files = sophon::fetch_manifest(&http, m)
            .await
            .with_context(|| format!("manifest {}", m.matching_field))
            .map_err(RunFailure::metadata)?;
        tracing::info!("manifest {}: {} entries", m.matching_field, files.len());
        per_manifest.push((m.chunk_download.url_prefix.clone(), files));
    }

    // local manifests for chunk-dedup (best effort)
    let mut local_map: HashMap<String, Vec<(String, i64, i64)>> = HashMap::new();
    if let Some(lb) = local_build.as_ref() {
        let local_wanted = sophon::select_manifests(lb, &audio, &ignore);
        let mut wm: Vec<WantedManifest> = Vec::new();
        for m in local_wanted {
            match sophon::fetch_manifest(&http, m).await {
                Ok(files) => wm.push(WantedManifest {
                    meta: m.clone(),
                    files,
                }),
                Err(e) => tracing::warn!("local manifest {} failed: {}", m.matching_field, e),
            }
        }
        local_map = sophon::build_local_chunk_map(&wm);
    }

    let plan = build_plan(per_manifest, &blacklist, &latest);
    tracing::info!("work list: {} files", plan.files.len());

    // Purge-before: same Collapse-style files-cleanup as purge-after,
    // run here to free space for the repair itself.
    // Skipped entirely in `--check-only` (verification reports damage; it must
    // not delete). In `--dry-run` the purge runs in log-only mode.
    // See docs/02 Step 5 and
    // https://github.com/CollapseLauncher/Collapse/blob/dc47259171794596331dffcf90db85a6ac0415ac/CollapseLauncher/Classes/InstallManagement/Genshin/GenshinInstall.cs#L177-L263
    if ctx.purge_before && !ctx.check_only {
        let server_keep = collapse_keep_set();
        let bytes = collapse_purge_extra(&game_dir, &plan, &server_keep, ctx.dry_run)
            .map_err(RunFailure::write)?;
        summary
            .deleted_extra_bytes
            .fetch_add(bytes, Ordering::Relaxed);
    }

    // 4. check-only mode
    if ctx.check_only {
        let mut bad = 0u64;
        summary.files_total = plan.files.len() as u64;
        let mut last_emit = std::time::Instant::now();
        for (i, f) in plan.files.iter().enumerate() {
            let p = game_dir.join(normalize_rel(&f.rel));
            let actual_len = util::file_len(&p);
            if actual_len != Some(f.size as u64) {
                bad += 1;
                tracing::warn!(file = %f.rel, expect_size = f.size, actual_size = ?actual_len, "CHECK fail: size");
            } else {
                match util::md5_file(&p) {
                    Ok(h) if h.eq_ignore_ascii_case(&f.md5) => {}
                    Ok(h) => {
                        bad += 1;
                        tracing::warn!(file = %f.rel, expect_md5 = %f.md5, actual_md5 = %h, "CHECK fail: md5");
                    }
                    Err(e) => {
                        bad += 1;
                        tracing::warn!(file = %f.rel, "CHECK fail: unreadable: {:#}", e);
                    }
                }
            }
            // Keep the check-only counter live so PROGRESS reflects work done so far.
            summary
                .files_skipped
                .store((i as u64 + 1) - bad.min(i as u64 + 1), Ordering::Relaxed);
            summary.files_failed.store(bad, Ordering::Relaxed);
            if last_emit.elapsed().as_secs() >= PROGRESS_INTERVAL_SECS {
                emit_progress(&summary);
                last_emit = std::time::Instant::now();
            }
        }
        tracing::info!("check-only: total={} bad={}", plan.files.len(), bad);
        summary.files_total = plan.files.len() as u64;
        summary.files_failed.store(bad, Ordering::Relaxed);
        return Ok((summary, if bad == 0 { 0 } else { 4 }));
    }

    // 5. repair files (bounded file parallelism, sequential chunks per file).
    // Each file task gets a `file{seq,total,task,path}` span so its progress can be
    // grouped in logs; `task` is the tokio async-task id (stable across thread hops),
    // and the `started on thread` event records which worker picked the file up.
    summary.files_total = plan.files.len() as u64;
    let total_files = plan.files.len();
    let sem = Arc::new(tokio::sync::Semaphore::new(ctx.jobs.max(1)));
    let game_dir_a = Arc::new(game_dir.clone());
    let http_a = Arc::new(http);
    let plan_a = Arc::new(plan);
    // NOTE: this map holds every local chunk (md5+size+offset per chunk, ~10^4 entries).
    // It must be shared by Arc: a per-task deep clone here once cost ~6GB across ~3k tasks.
    let local_map_a = Arc::new(local_map);
    let sum_a = Arc::new(summary);
    let progress = start_progress_reporter(sum_a.clone());
    let dry_run = ctx.dry_run;
    let mut handles = Vec::new();
    for idx in 0..plan_a.files.len() {
        let sem = sem.clone();
        let game_dir = game_dir_a.clone();
        let http = http_a.clone();
        let plan = plan_a.clone();
        let sum = sum_a.clone();
        let local_map = local_map_a.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.unwrap();
            let task_id = tokio::task::try_id();
            let span = tracing::info_span!(
                "file",
                seq = idx + 1,
                total = total_files,
                task = ?task_id,
                path = %plan.files[idx].rel,
            );
            async move {
                tracing::debug!(thread = ?std::thread::current().id(), "picked up by worker");
                match repair_one_file(
                    &http,
                    &game_dir,
                    &plan.files[idx],
                    &plan.url_prefix_by_file,
                    &local_map,
                    dry_run,
                )
                .await
                {
                    Ok(Repaired::Skipped) => {
                        sum.files_skipped.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Repaired::Repaired(stats)) => {
                        sum.files_repaired.fetch_add(1, Ordering::Relaxed);
                        sum.download_bytes
                            .fetch_add(stats.download_bytes, Ordering::Relaxed);
                    }
                    Ok(Repaired::DryRun) => {}
                    Err(e) => {
                        sum.files_failed.fetch_add(1, Ordering::Relaxed);
                        tracing::error!("repair failed: {:#}", e);
                    }
                }
            }
            .instrument(span)
            .await
        }));
    }
    for h in handles {
        let _ = h.await;
    }
    progress.abort();
    let summary = Arc::try_unwrap(sum_a).unwrap_or_else(|a| Summary {
        files_total: a.files_total,
        files_skipped: AtomicU64::new(a.files_skipped.load(Ordering::Relaxed)),
        files_repaired: AtomicU64::new(a.files_repaired.load(Ordering::Relaxed)),
        files_failed: AtomicU64::new(a.files_failed.load(Ordering::Relaxed)),
        download_bytes: AtomicU64::new(a.download_bytes.load(Ordering::Relaxed)),
        deleted_extra_bytes: AtomicU64::new(a.deleted_extra_bytes.load(Ordering::Relaxed)),
    });
    if summary.files_failed.load(Ordering::Relaxed) > 0 {
        return Ok((summary, 3));
    }

    // 6. post phase (see docs/02 Step 7).
    // `readonly` covers both `--dry-run` and `--check-only` (check-only returns
    // before this point, but gating on `readonly` keeps the invariant obvious).
    if !readonly {
        // Starward-handling remainder: deprecated list + audio cache→res move
        // + config.ini bump. Deprecated files are also covered by the purge
        // below when enabled (they are not in the live manifest); the explicit
        // loop keeps them cleaned even without any purge flag.
        match hyp.deprecated_files().await {
            Ok(list) => {
                for n in list {
                    let p = game_dir.join(normalize_rel(&n));
                    if p.is_file() {
                        let sz = util::file_len(&p).unwrap_or(0);
                        if std::fs::remove_file(&p).is_ok() {
                            summary.deleted_extra_bytes.fetch_add(sz, Ordering::Relaxed);
                            tracing::info!("deleted deprecated {}", n);
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("deprecated list unavailable: {:#}", e),
        }
        if !cfg.audio_pkg_cache_dir.is_empty()
            && !cfg.audio_pkg_res_dir.is_empty()
            && cfg.audio_pkg_cache_dir != cfg.audio_pkg_res_dir
        {
            move_audio_cache(&game_dir, &cfg.audio_pkg_cache_dir, &cfg.audio_pkg_res_dir);
        }
        // Purge-after: same files-cleanup as purge-before, run after patching.
        if ctx.purge_after {
            let server_keep = collapse_keep_set();
            let bytes = collapse_purge_extra(&game_dir, &plan_a, &server_keep, false)
                .map_err(RunFailure::write)?;
            summary
                .deleted_extra_bytes
                .fetch_add(bytes, Ordering::Relaxed);
        }
        let (ch, sub, cps) = ctx.biz.channel_tuple();
        util::write_config_ini(
            &game_dir,
            &latest,
            ctx.biz.as_str(),
            (ch, sub, cps),
            "",
            false,
        )
        .context("write config.ini")
        .map_err(RunFailure::write)?;
    } else if ctx.dry_run && ctx.purge_after {
        // Dry-run logging only: list what the files-cleanup would
        // delete (writes nothing; byte count stays 0 by design).
        let server_keep = collapse_keep_set();
        let plan_ref = &*plan_a;
        let _ = collapse_purge_extra(&game_dir, plan_ref, &server_keep, true)
            .map_err(RunFailure::write)?;
    }

    Ok((summary, 0))
}

/// Build sorted, deduped, blacklist-filtered plan. `per_manifest`: (chunk_url_prefix, files).
pub fn build_plan(
    per_manifest: Vec<(String, Vec<SophonChunkFile>)>,
    blacklist: &HashSet<String>,
    latest_tag: &str,
) -> RepairPlan {
    let mut files: Vec<PlannedFile> = Vec::new();
    let mut prefix: HashMap<String, String> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (url_prefix, list) in per_manifest {
        for f in list {
            if f.is_folder || f.file.is_empty() {
                continue;
            }
            let rel = f.file.replace('\\', "/");
            if blacklist.contains(&rel) || !seen.insert(rel.clone()) {
                continue;
            }
            prefix.insert(rel.clone(), url_prefix.clone());
            files.push(PlannedFile {
                rel,
                size: f.size,
                md5: f.md5.clone(),
                chunks: f.chunks,
            });
        }
    }
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    RepairPlan {
        latest: latest_tag.to_string(),
        files,
        url_prefix_by_file: prefix,
    }
}

enum Repaired {
    Skipped,
    Repaired(ChunkStats),
    DryRun,
}

#[derive(Default, Debug)]
struct ChunkStats {
    download_bytes: u64,
    chunks_total: u64,
    chunks_reused: u64,
    chunks_downloaded: u64,
    chunks_resumed: u64,
}

async fn repair_one_file(
    http: &reqwest::Client,
    game_dir: &Path,
    file: &PlannedFile,
    prefix_by_path: &HashMap<String, String>,
    local_map: &HashMap<String, Vec<(String, i64, i64)>>,
    dry_run: bool,
) -> Result<Repaired> {
    let final_path = game_dir.join(normalize_rel(&file.rel));
    if let Some(parent) = final_path.parent() {
        if !dry_run {
            std::fs::create_dir_all(parent)?;
        }
    }
    tracing::debug!(expect_size = file.size, expect_md5 = %file.md5, chunks = file.chunks.len(), "check start");
    // fast skip: size + full MD5
    match util::file_len(&final_path) {
        None => tracing::debug!("check: missing local file"),
        Some(n) if n != file.size as u64 => {
            tracing::debug!(actual_size = n, "check: size mismatch -> needs repair");
        }
        Some(_) => match util::md5_file(&final_path) {
            Ok(h) if h.eq_ignore_ascii_case(&file.md5) => {
                tracing::debug!(actual_md5 = %h, "check: md5 match -> skip");
                return Ok(Repaired::Skipped);
            }
            Ok(h) => tracing::debug!(actual_md5 = %h, "check: md5 mismatch -> needs repair"),
            Err(e) => tracing::debug!("check: unreadable ({:#}) -> needs repair", e),
        },
    }
    if dry_run {
        tracing::info!("would repair");
        return Ok(Repaired::DryRun);
    }
    let prefix = prefix_by_path.get(&file.rel).cloned().unwrap_or_default();
    // NOTE(starward-parity): temp-then-atomic-promote. Starward writes
    // `FullPath + "_tmp"` (OpenOrCreate), verifies MD5, then `Move(tmp, final,
    // true)` and `Delete(tmp)` on mismatch — same contract here, including the
    // suffix, so foreign leftovers are swept on the next run.
    // See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L330-L331
    // and https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L443-L459
    let tmp_path = PathBuf::from(format!("{}_tmp", final_path.display()));

    for attempt in 1..=5u32 {
        match repair_attempt(http, game_dir, file, &prefix, local_map, &tmp_path).await {
            Ok(stats) => return Ok(Repaired::Repaired(stats)),
            Err(e) => {
                tracing::warn!(attempt, "attempt failed: {:#}", e);
                if attempt == 5 {
                    return Err(e);
                }
                tokio::time::sleep(std::time::Duration::from_secs(attempt as u64)).await;
            }
        }
    }
    unreachable!()
}

async fn repair_attempt(
    http: &reqwest::Client,
    game_dir: &Path,
    file: &PlannedFile,
    prefix: &str,
    local_map: &HashMap<String, Vec<(String, i64, i64)>>,
    tmp_path: &Path,
) -> Result<ChunkStats> {
    let mut stats = ChunkStats::default();
    stats.chunks_total = file.chunks.len() as u64;
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(tmp_path)
        .with_context(|| format!("open tmp {}", tmp_path.display()))?;
    let have = f.metadata().map(|m| m.len()).unwrap_or(0);
    if have > file.size as u64 {
        tracing::debug!(tmp_bytes = have, "tmp oversize -> truncate");
        f.set_len(file.size as u64)?;
    }
    tracing::debug!(
        tmp_bytes = f.metadata().map(|m| m.len()).unwrap_or(0),
        expect_size = file.size,
        "tmp resume point"
    );
    let reuse_entries = local_map.get(&file.rel);
    for c in &file.chunks {
        let end = (c.offset + c.uncompressed_size) as u64;
        let cur_len = f.metadata().map(|m| m.len()).unwrap_or(0);
        // V1-SIMPLIFICATION (S3, see README): resume-by-length. A completed
        // prefix is kept without per-chunk re-verification; the final
        // whole-file MD5 gates promotion, so a corrupt prefix only wastes work
        // before failing shut. Mirrors Starward's
        // `if (fs.Length < chunk.Offset + chunk.UncompressedSize)` skip.
        // See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L342-L343
        if cur_len >= end {
            stats.chunks_resumed += 1;
            tracing::trace!(chunk = %c.id, offset = c.offset, size = c.uncompressed_size, "chunk already complete -> keep");
            continue; // completed prefix kept; final whole-file MD5 re-verifies
        }
        f.seek(SeekFrom::Start(c.offset as u64))?;
        // (a) verified local slice reuse (same path, md5+size match).
        // V1-SIMPLIFICATION (S2, see README): same-file-only reuse. Starward
        // maps each chunk to any local file via `OriginalFileFullPath/Offset`
        // (cross-file dedup); we only reuse slices from the same path, which
        // covers ~all Genshin wins at a fraction of the bookkeeping.
        // See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallFile.cs#L100-L131
        // and https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L347-L360
        let mut reused = false;
        match reuse_entries.and_then(|entries| {
            entries.iter().find(|(md5, sz, _)| {
                md5.eq_ignore_ascii_case(&c.uncompressed_md5) && *sz == c.uncompressed_size
            })
        }) {
            None => {
                tracing::trace!(chunk = %c.id, offset = c.offset, size = c.uncompressed_size, "no local reuse candidate -> download")
            }
            Some((_, _, ro)) => {
                let src = game_dir.join(normalize_rel(&file.rel));
                match util::md5_file_slice(&src, *ro as u64, c.uncompressed_size as u64) {
                    Ok(h) if h.eq_ignore_ascii_case(&c.uncompressed_md5) => {
                        tracing::trace!(chunk = %c.id, src_offset = ro, "reuse slice md5 ok -> copy");
                        let mut sf = File::open(&src)?;
                        sf.seek(SeekFrom::Start(*ro as u64))?;
                        let mut remaining = c.uncompressed_size as u64;
                        let mut buf = vec![0u8; 512 * 1024];
                        let mut ok = true;
                        while remaining > 0 {
                            let want = (remaining as usize).min(buf.len());
                            let n = sf.read(&mut buf[..want])?;
                            if n == 0 {
                                ok = false;
                                break;
                            }
                            f.write_all(&buf[..n])?;
                            remaining -= n as u64;
                        }
                        reused = ok;
                        if reused {
                            stats.chunks_reused += 1;
                        }
                    }
                    Ok(h) => {
                        tracing::debug!(chunk = %c.id, slice_md5 = %h, "reuse slice md5 mismatch -> download")
                    }
                    Err(e) => {
                        tracing::debug!(chunk = %c.id, "reuse slice unreadable ({:#}) -> download", e)
                    }
                }
            }
        }
        if reused {
            continue;
        }
        // (b) download chunk blob + zstd decode + write at offset.
        // V1-SIMPLIFICATION (S1, see README): whole-chunk buffering
        // (`fetch` + `decode_all`) instead of Starward's streaming
        // `Pipe + DecompressionStream` pipeline. Simpler and avoids any
        // retained blob store; costs higher peak RAM per active chunk.
        // See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L374-L412
        let blob = sophon::fetch_chunk_bytes(http, prefix, &c.id)
            .await
            .with_context(|| format!("chunk {}", c.id))?;
        stats.download_bytes += blob.len() as u64;
        let decoded = zstd::stream::decode_all(std::io::Cursor::new(&blob))
            .with_context(|| format!("zstd chunk {}", c.id))?;
        tracing::trace!(chunk = %c.id, compressed_bytes = blob.len(), decompressed_bytes = decoded.len(), "chunk decoded");
        if decoded.len() as i64 != c.uncompressed_size {
            anyhow::bail!(
                "chunk {} size mismatch: expect {} got {}",
                c.id,
                c.uncompressed_size,
                decoded.len()
            );
        }
        let dh = format!("{:x}", md5::compute(&decoded));
        if !dh.eq_ignore_ascii_case(&c.uncompressed_md5) {
            tracing::debug!(chunk = %c.id, expect_md5 = %c.uncompressed_md5, actual_md5 = %dh, "chunk md5 mismatch");
            anyhow::bail!("chunk {} md5 mismatch", c.id);
        }
        f.seek(SeekFrom::Start(c.offset as u64))?;
        f.write_all(&decoded)?;
        stats.chunks_downloaded += 1;
    }
    f.flush()?;
    drop(f);
    // final verify + atomic promote
    let len = util::file_len(tmp_path).unwrap_or(0);
    if len != file.size as u64 {
        anyhow::bail!("tmp length {} != expected {}", len, file.size);
    }
    let h = util::md5_file(tmp_path)?;
    tracing::debug!(tmp_size = len, tmp_md5 = %h, "after: tmp ready for promote");
    // NOTE(starward-parity): final MD5 gates promotion; on mismatch the tmp is
    // deleted and the file is retried/failed, never promoted partially.
    // See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L443-L459
    if !h.eq_ignore_ascii_case(&file.md5) {
        std::fs::remove_file(tmp_path).ok();
        anyhow::bail!(
            "final md5 mismatch for {}: expect {} got {}",
            file.rel,
            file.md5,
            h
        );
    }
    let final_path = game_dir.join(normalize_rel(&file.rel));
    std::fs::rename(tmp_path, &final_path).with_context(|| format!("promote {}", file.rel))?;
    tracing::info!(chunks_total = stats.chunks_total, chunks_reused = stats.chunks_reused, chunks_downloaded = stats.chunks_downloaded, chunks_resumed = stats.chunks_resumed, download_bytes = stats.download_bytes, final_md5 = %h, "repaired");
    Ok(stats)
}

/// Collapse-style files-cleanup (v1 scope).
/// NOTE(collapse-parity): set-difference of on-disk files vs the live manifest,
/// the same idea as `GetUnusedFileInfoList`.
/// See https://github.com/CollapseLauncher/Collapse/blob/dc47259171794596331dffcf90db85a6ac0415ac/CollapseLauncher/Classes/InstallManagement/Genshin/GenshinInstall.cs#L177-L263
///
/// V1 GAP (deliberate, documented): Collapse's expected set is the union of
/// the Sophon primary manifests + dispatcher persistent manifests
/// (`res_versions_external`, `data_versions`) + plugin/SDK/WPF zip entries,
/// minus `EliminateUnnecessaryAssetIndex` (unselected audio) and `ctable*`.
/// v1 only uses `{latest Sophon manifest paths}` — no dispatcher, no
/// SDK/WPF/plugin zips (game launches without them; see README v1 limits).
/// Do not "fix" the missing union without reading the v1-limits section.
///
/// There is exactly one cleanup: `--purge-before` and `--purge-after` call
/// this same function; only the timing differs. Temps (`*_tmp`, `*.hdiff`,
/// `chunk/`, `ldiff/`, `staging/`, legacy `*.diff` / `*deletefiles*`) are NOT
/// skipped — they are not in the live manifest, so they purge here like any
/// other orphan, into the single `deleted_extra_bytes` counter.
///
/// `server_keep` comes from [`collapse_keep_set`]: files that live inside
/// `game_dir` but NEVER appear in chunk manifests.
pub fn collapse_purge_extra(
    game_dir: &Path,
    plan: &RepairPlan,
    server_keep: &HashSet<String>,
    dry_run: bool,
) -> Result<u64> {
    let expected: HashSet<String> = plan.files.iter().map(|f| f.rel.clone()).collect();
    let mut deleted_bytes = 0u64;
    let mut candidates: Vec<(PathBuf, u64)> = Vec::new();
    for e in walkdir::WalkDir::new(game_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !e.file_type().is_file() {
            continue;
        }
        let rel = e
            .path()
            .strip_prefix(game_dir)
            .unwrap_or(e.path())
            .to_string_lossy()
            .replace('\\', "/");
        if classify_purge_path(&rel, &expected, server_keep) == PurgeVerdict::Purge {
            let sz = e.metadata().map(|m| m.len()).unwrap_or(0);
            candidates.push((e.path().to_path_buf(), sz));
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    let mut would_be = 0u64;
    for (p, sz) in candidates {
        if dry_run {
            would_be += sz;
            tracing::info!("would purge extra {} ({} bytes)", p.display(), sz);
        } else if std::fs::remove_file(&p).is_ok() {
            deleted_bytes += sz;
            tracing::info!("purged extra {} ({} bytes)", p.display(), sz);
        }
    }
    if dry_run {
        tracing::info!(
            "purge dry-run: {} bytes would be freed (deleted_extra_bytes stays 0 by design)",
            would_be
        );
    }
    if !dry_run {
        let mut dirs: Vec<PathBuf> = walkdir::WalkDir::new(game_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_dir())
            .map(|e| e.path().to_path_buf())
            .collect();
        dirs.sort_by(|a, b| b.components().count().cmp(&a.components().count()));
        for d in dirs {
            if d == game_dir {
                continue;
            }
            let _ = std::fs::remove_dir(d); // only empty dirs
        }
    }
    Ok(deleted_bytes)
}

/// Metadata files the files-cleanup must keep.
///
/// v1: `config.ini` ONLY. It lives inside `game_dir` but NEVER appears in
/// Sophon chunk manifests, and the tool rewrites it after patching — purging
/// it would break the version bookkeeping. Everything else (exe, blacklist /
/// res-category / audio-scan bookkeeping, `ScreenShot/`, logs, temps) is
/// intentionally NOT kept: Collapse's `GenshinInstall.GetUnusedFileInfoList`
/// protects only `FilesCleanupIgnoreList` (empty for Genshin) plus the
/// `Audio_*_pkg_version` regexes, and this tool maximizes free space — if the
/// user wants anything else kept, that needs an explicit `--keep` flag
/// (not implemented in v1).
///
/// NOTE: the game flags unknown executables inside the game folder, so the
/// tool binary itself must never live there; the exe needs no keep rule.
///
/// `audio_lang_*` / `Audio_*_pkg_version` are matched by filename pattern in
/// [`classify_purge_path`], not here, because their location varies by channel.
///
/// All entries are `/`-separated rel paths (same normalization as
/// [`build_plan`] and the purge walk).
pub fn collapse_keep_set() -> HashSet<String> {
    HashSet::from(["config.ini".to_string()])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgeVerdict {
    Keep,
    Purge,
}

/// Decide the fate of one `/`-separated rel path for the files-cleanup.
/// Pure function (no I/O) so it can be unit-tested exhaustively.
///
/// Rule order matters (first match wins); each rule names its provenance:
/// 1. Manifest membership (Collapse expected set).
/// 2. `config.ini` ([`collapse_keep_set`] — the only metadata keep).
/// 3. Collapse audio pkg-version parity (`audio_lang_*` + case-insensitive
///    `Audio_*_pkg_version`, `GenshinInstall.cs:228-247`). v1 matches broadly
///    by pattern rather than per-`audio_lang_14`-line exact regexes.
/// Everything else purges — including temps (`*_tmp`, `*.hdiff`, `chunk/`,
/// `ldiff/`, `staging/`, `*.diff`, `*deletefiles*`), `ScreenShot/`, logs,
/// exe and server bookkeeping files.
pub fn classify_purge_path(
    rel: &str,
    expected: &HashSet<String>,
    server_keep: &HashSet<String>,
) -> PurgeVerdict {
    // 1. In the live manifest → keep.
    if expected.contains(rel) {
        return PurgeVerdict::Keep;
    }
    // 2. config.ini → keep.
    if server_keep.contains(rel) {
        return PurgeVerdict::Keep;
    }
    // 3. Collapse audio pkg-version parity → keep.
    if is_audio_version_file(file_name(rel)) {
        return PurgeVerdict::Keep;
    }
    PurgeVerdict::Purge
}

/// Collapse `GenshinInstall.cs:228-247` parity: `audio_lang_14` (any
/// `audio_lang_*`) plus `Audio_<lang>_pkg_version` (matched case-insensitively
/// — on disk it is `Audio_English_pkg_version`, i.e. capital A).
fn is_audio_version_file(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    lower.starts_with("audio_lang_")
        || (lower.starts_with("audio_") && lower.ends_with("_pkg_version"))
}

fn file_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

pub fn normalize_rel(rel: &str) -> String {
    rel.replace('/', std::path::MAIN_SEPARATOR_STR)
        .replace('\\', std::path::MAIN_SEPARATOR_STR)
}

fn read_current_audio(game_dir: &Path, scan_dir: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    if scan_dir.is_empty() {
        return set;
    }
    let Ok(text) = std::fs::read_to_string(game_dir.join(normalize_rel(scan_dir))) else {
        return set;
    };
    for line in text.lines() {
        match line.trim() {
            "Chinese" => {
                set.insert("zh-cn".to_string());
            }
            s if s.eq_ignore_ascii_case("English(US)") || s.eq_ignore_ascii_case("English") => {
                set.insert("en-us".to_string());
            }
            "Japanese" => {
                set.insert("ja-jp".to_string());
            }
            "Korean" => {
                set.insert("ko-kr".to_string());
            }
            _ => {}
        }
    }
    set
}

fn write_audio_scan(game_dir: &Path, scan_dir: &str, audio: &HashSet<String>) -> Result<()> {
    let mut lines: Vec<&str> = Vec::new();
    if audio.contains("zh-cn") {
        lines.push("Chinese");
    }
    if audio.contains("en-us") {
        lines.push("English(US)");
    }
    if audio.contains("ja-jp") {
        lines.push("Japanese");
    }
    if audio.contains("ko-kr") {
        lines.push("Korean");
    }
    let p = game_dir.join(normalize_rel(scan_dir));
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(p, lines.join("\n"))?;
    Ok(())
}

fn move_audio_cache(game_dir: &Path, cache_dir: &str, res_dir: &str) {
    let cache = game_dir.join(normalize_rel(cache_dir));
    let res = game_dir.join(normalize_rel(res_dir));
    if !cache.is_dir() {
        return;
    }
    let files: Vec<PathBuf> = walkdir::WalkDir::new(&cache)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .collect();
    for src in files {
        if let Ok(rel) = src.strip_prefix(&cache) {
            let target = res.join(rel);
            if std::fs::create_dir_all(target.parent().unwrap_or(&res)).is_ok() {
                if std::fs::rename(&src, &target).is_err() {
                    let _ = std::fs::copy(&src, &target);
                    let _ = std::fs::remove_file(&src);
                }
            }
        }
    }
    tracing::info!("moved audio cache -> res");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_sorted_dedup_blacklist() {
        let files = vec![
            SophonChunkFile {
                file: "b.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 2,
                md5: "m2".into(),
            },
            SophonChunkFile {
                file: "a.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 1,
                md5: "m1".into(),
            },
            SophonChunkFile {
                file: "drop.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 1,
                md5: "m".into(),
            },
        ];
        let per = vec![("http://x".to_string(), files)];
        let bl: HashSet<String> = ["drop.dat".to_string()].into_iter().collect();
        let plan = build_plan(per, &bl, "5.0");
        assert_eq!(
            plan.files
                .iter()
                .map(|f| f.rel.as_str())
                .collect::<Vec<_>>(),
            vec!["a.dat", "b.dat"]
        );
        assert_eq!(plan.latest, "5.0");
    }

    #[test]
    fn progress_format_done_counts() {
        let s = Summary {
            files_total: 10,
            files_skipped: AtomicU64::new(3),
            files_repaired: AtomicU64::new(2),
            files_failed: AtomicU64::new(1),
            download_bytes: AtomicU64::new(123),
            ..Summary::default()
        };
        assert_eq!(
            format_progress(&s),
            "PROGRESS done=6/10 skipped=3 repaired=2 failed=1 download_bytes=123"
        );
    }

    fn purge_fixture() -> (HashSet<String>, HashSet<String>) {
        let expected: HashSet<String> = ["game.dat".to_string()].into_iter().collect();
        let server_keep = collapse_keep_set();
        (expected, server_keep)
    }

    #[test]
    fn purge_keeps_manifest_and_config_only() {
        let (expected, keep) = purge_fixture();
        for rel in ["game.dat", "config.ini"] {
            assert_eq!(
                classify_purge_path(rel, &expected, &keep),
                PurgeVerdict::Keep,
                "{rel} must be kept"
            );
        }
        // v1: exe, server bookkeeping, user data all purge (maximize free
        // space; explicit --keep not implemented).
        for rel in [
            "YuanShen.exe",
            "blacklist.txt",
            "res_category.txt",
            "audio_scan.txt",
            "ScreenShot/shot.png",
            "log/output.txt",
            "login.dat",
        ] {
            assert_eq!(
                classify_purge_path(rel, &expected, &keep),
                PurgeVerdict::Purge,
                "{rel} must be purged"
            );
        }
    }

    #[test]
    fn purge_keeps_audio_version_files_case_insensitive() {
        let (expected, keep) = purge_fixture();
        // Collapse GenshinInstall.cs:228-247 parity (audio_lang_14 + per-lang pkg_version).
        for rel in [
            "audio_lang_14",
            "Audio_English_pkg_version",
            "audio_chinese_pkg_version",
            "AUDIO_JAPANESE_PKG_VERSION",
        ] {
            assert_eq!(
                classify_purge_path(rel, &expected, &keep),
                PurgeVerdict::Keep,
                "{rel} must be kept"
            );
        }
    }

    #[test]
    fn purge_purges_temps_like_any_other_orphan() {
        let (expected, keep) = purge_fixture();
        // Single files-cleanup: temps are not skipped, they purge into the
        // same counter. --purge-before and --purge-after delete the same set.
        for rel in [
            "a.dat_tmp",
            "config.ini.girpr_tmp",
            "x.hdiff",
            "chunk/abc123",
            "ldiff/p1",
            "staging/y",
            "old.diff",
            "patch.deletefiles.txt",
        ] {
            assert_eq!(
                classify_purge_path(rel, &expected, &keep),
                PurgeVerdict::Purge,
                "{rel} must be purged"
            );
        }
    }

    #[test]
    fn purge_dry_run_deletes_nothing_but_reports() {
        let dir = std::env::temp_dir().join("girpr_test_purge_dryrun");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("game.dat"), b"keep").unwrap();
        std::fs::write(dir.join("stray.dat"), b"drop").unwrap();
        let plan = RepairPlan {
            latest: "5.0".into(),
            files: vec![PlannedFile {
                rel: "game.dat".into(),
                size: 4,
                md5: String::new(),
                chunks: vec![],
            }],
            url_prefix_by_file: HashMap::new(),
        };
        let keep: HashSet<String> = ["config.ini".to_string()].into_iter().collect();
        let bytes = collapse_purge_extra(&dir, &plan, &keep, true).unwrap();
        assert_eq!(bytes, 0);
        assert!(dir.join("stray.dat").exists());
        assert!(dir.join("game.dat").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn purge_real_run_deletes_only_unexpected() {
        let dir = std::env::temp_dir().join("girpr_test_purge_real");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("game.dat"), b"keep").unwrap();
        std::fs::write(dir.join("login.dat"), b"drop").unwrap();
        let plan = RepairPlan {
            latest: "5.0".into(),
            files: vec![PlannedFile {
                rel: "game.dat".into(),
                size: 4,
                md5: String::new(),
                chunks: vec![],
            }],
            url_prefix_by_file: HashMap::new(),
        };
        let keep: HashSet<String> = ["config.ini".to_string()].into_iter().collect();
        let bytes = collapse_purge_extra(&dir, &plan, &keep, false).unwrap();
        assert_eq!(bytes, 4);
        assert!(!dir.join("login.dat").exists());
        assert!(dir.join("game.dat").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn keep_set_is_config_only() {
        let keep = collapse_keep_set();
        assert!(keep.contains("config.ini"));
        assert_eq!(keep.len(), 1);
    }

    #[test]
    fn report_line_keyvalue_reports_versions_and_api_fields() {
        use crate::hyp::{ChunkBuild, GameBranchPackage, GameConfig};
        let cfg = GameConfig {
            exe_file_name: "YuanShen.exe".into(),
            audio_pkg_scan_dir: "scan.txt".into(),
            audio_pkg_res_dir: "res".into(),
            audio_pkg_cache_dir: "cache".into(),
            default_download_mode: "DOWNLOAD_MODE_CHUNK".into(),
            res_category_dir: "rc.txt".into(),
            blacklist_dir: "bl.txt".into(),
            enable_resource_blacklist: true,
            game: None,
        };
        let pkg = GameBranchPackage {
            package_id: "pkg1".into(),
            branch: "main".into(),
            password: String::new(),
            tag: "5.1.0".into(),
            diff_tags: vec!["5.0.0".into(), "5.1.0".into()],
        };
        let build = ChunkBuild {
            build_id: "build42".into(),
            tag: "5.1.0".into(),
            manifests: Vec::new(),
        };
        let audio: HashSet<String> = ["ja-jp".into(), "en-us".into()].into_iter().collect();
        let line = format_report_line(
            &Some("4.0.0".to_string()),
            "5.1.0",
            "hk4e_global",
            &cfg,
            &pkg,
            &build,
            &audio,
            false,
        );
        assert!(
            line.starts_with("REPORT local_version=4.0.0 latest_version=5.1.0 biz=hk4e_global"),
            "{line}"
        );
        assert!(line.contains("exe=YuanShen.exe"), "{line}");
        assert!(line.contains("download_mode=DOWNLOAD_MODE_CHUNK"), "{line}");
        assert!(line.contains("branch=main"), "{line}");
        assert!(line.contains("package_id=pkg1"), "{line}");
        assert!(line.contains("build_id=build42"), "{line}");
        assert!(line.contains("audio_langs=en-us,ja-jp"), "{line}");
        assert!(line.contains("diff_tags=5.0.0,5.1.0"), "{line}");
    }

    #[test]
    fn report_line_none_local_and_sorted_json() {
        use crate::hyp::{ChunkBuild, GameBranchPackage, GameConfig};
        let cfg = GameConfig {
            exe_file_name: "GenshinImpact.exe".into(),
            audio_pkg_scan_dir: String::new(),
            audio_pkg_res_dir: String::new(),
            audio_pkg_cache_dir: String::new(),
            default_download_mode: String::new(),
            res_category_dir: String::new(),
            blacklist_dir: String::new(),
            enable_resource_blacklist: false,
            game: None,
        };
        let pkg = GameBranchPackage::default();
        let build = ChunkBuild {
            build_id: "b1".into(),
            tag: "3.0.0".into(),
            manifests: Vec::new(),
        };
        let audio: HashSet<String> = HashSet::new();
        let line = format_report_line(&None, "3.0.0", "hk4e_cn", &cfg, &pkg, &build, &audio, true);
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["local_version"], "none");
        assert_eq!(v["latest_version"], "3.0.0");
        assert_eq!(v["biz"], "hk4e_cn");
        assert_eq!(v["exe"], "GenshinImpact.exe");
        assert_eq!(v["audio_langs"], "");
    }
}
