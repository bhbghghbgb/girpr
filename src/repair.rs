use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use crate::hyp::HypClient;
use crate::sophon::{self, SophonChunkFile, WantedManifest};
use crate::util;
use crate::Biz;
use tracing::Instrument;

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
    pub deleted_extra_bytes: AtomicU64,
    pub freed_temp_bytes: AtomicU64,
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
pub fn emit_progress(sum: &Summary) {
    let line = format_progress(sum);
    println!("{line}");
    tracing::info!("{line}");
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
    pub purge_extra: bool,
    pub purge_before: bool,
}

pub async fn run(ctx: RepairCtx) -> Result<(Summary, i32)> {
    let mut summary = Summary::default();
    std::fs::create_dir_all(&ctx.game_dir)?;
    let game_dir = ctx.game_dir.canonicalize().unwrap_or_else(|_| ctx.game_dir.clone());

    // 0. pre-clean temps (reclaim space first)
    let (_, freed) = util::sweep_temps(&game_dir, ctx.dry_run);
    summary.freed_temp_bytes.fetch_add(freed, Ordering::Relaxed);

    // 1. local state
    let local_version = util::read_game_version(&game_dir);
    tracing::info!("local game_version: {:?}", local_version);

    // 2. server metadata (4 calls max)
    let hyp = HypClient::new(ctx.biz)?;
    let cfg = hyp.game_config().await.context("getGameConfigs")?;
    tracing::info!(
        "game config: exe={} audio_scan={} audio_res={} audio_cache={} mode={}",
        cfg.exe_file_name,
        cfg.audio_pkg_scan_dir,
        cfg.audio_pkg_res_dir,
        cfg.audio_pkg_cache_dir,
        cfg.default_download_mode
    );
    if cfg.default_download_mode == "DOWNLOAD_MODE_FILE" {
        anyhow::bail!("game is in legacy FILE mode; chunk repair not supported (unexpected for Genshin)");
    }
    let branch = hyp.game_branch().await.context("getGameBranches")?;
    let latest = branch.main.tag.clone();
    tracing::info!("latest version: {}", latest);
    let latest_build = hyp
        .chunk_build(&branch.main, None)
        .await
        .context("getBuild(latest)")?;

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
        write_audio_scan(&game_dir, &cfg.audio_pkg_scan_dir, &audio)?;
    }

    // 3. fetch + verify + parse latest manifests (per-manifest lists kept for prefix mapping)
    let http = hyp.http();
    let wanted = sophon::select_manifests(&latest_build, &audio, &ignore);
    let mut per_manifest: Vec<(String, Vec<SophonChunkFile>)> = Vec::new();
    for m in wanted {
        let files = sophon::fetch_manifest(&http, m)
            .await
            .with_context(|| format!("manifest {}", m.matching_field))?;
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

    // purge-before (optional, frees space for the repair itself)
    if ctx.purge_before {
        let bytes = purge_extra_files(&game_dir, &plan, ctx.dry_run)?;
        summary.deleted_extra_bytes.fetch_add(bytes, Ordering::Relaxed);
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
            summary.files_skipped.store(
                (i as u64 + 1) - bad.min(i as u64 + 1),
                Ordering::Relaxed,
            );
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
                match repair_one_file(&http, &game_dir, &plan.files[idx], &plan.url_prefix_by_file, &local_map, dry_run).await {
                    Ok(Repaired::Skipped) => {
                        sum.files_skipped.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Repaired::Repaired(stats)) => {
                        sum.files_repaired.fetch_add(1, Ordering::Relaxed);
                        sum.download_bytes.fetch_add(stats.download_bytes, Ordering::Relaxed);
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
        freed_temp_bytes: AtomicU64::new(a.freed_temp_bytes.load(Ordering::Relaxed)),
    });
    if summary.files_failed.load(Ordering::Relaxed) > 0 {
        return Ok((summary, 3));
    }

    // 6. post phase
    if !ctx.dry_run {
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
        let (_, freed) = util::sweep_temps(&game_dir, false);
        summary.freed_temp_bytes.fetch_add(freed, Ordering::Relaxed);
        if ctx.purge_extra {
            let bytes = purge_extra_files(&game_dir, &plan_a, false)?;
            summary.deleted_extra_bytes.fetch_add(bytes, Ordering::Relaxed);
        }
        let (ch, sub, cps) = ctx.biz.channel_tuple();
        util::write_config_ini(&game_dir, &latest, ctx.biz.as_str(), (ch, sub, cps), "", false)?;
    } else if ctx.purge_extra {
        let plan_ref = &*plan_a;
        let _ = purge_extra_files(&game_dir, plan_ref, true)?;
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
    // Starward-style temp name so foreign leftovers are also swept next run.
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
    tracing::debug!(tmp_bytes = f.metadata().map(|m| m.len()).unwrap_or(0), expect_size = file.size, "tmp resume point");
    let reuse_entries = local_map.get(&file.rel);
    for c in &file.chunks {
        let end = (c.offset + c.uncompressed_size) as u64;
        let cur_len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if cur_len >= end {
            stats.chunks_resumed += 1;
            tracing::trace!(chunk = %c.id, offset = c.offset, size = c.uncompressed_size, "chunk already complete -> keep");
            continue; // completed prefix kept; final whole-file MD5 re-verifies
        }
        f.seek(SeekFrom::Start(c.offset as u64))?;
        // (a) verified local slice reuse (same path, md5+size match)
        let mut reused = false;
        match reuse_entries.and_then(|entries| {
            entries.iter().find(|(md5, sz, _)| {
                md5.eq_ignore_ascii_case(&c.uncompressed_md5) && *sz == c.uncompressed_size
            })
        }) {
            None => tracing::trace!(chunk = %c.id, offset = c.offset, size = c.uncompressed_size, "no local reuse candidate -> download"),
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
                    Ok(h) => tracing::debug!(chunk = %c.id, slice_md5 = %h, "reuse slice md5 mismatch -> download"),
                    Err(e) => tracing::debug!(chunk = %c.id, "reuse slice unreadable ({:#}) -> download", e),
                }
            }
        }
        if reused {
            continue;
        }
        // (b) download chunk blob + zstd decode + write at offset
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
    if !h.eq_ignore_ascii_case(&file.md5) {
        std::fs::remove_file(tmp_path).ok();
        anyhow::bail!("final md5 mismatch for {}: expect {} got {}", file.rel, file.md5, h);
    }
    let final_path = game_dir.join(normalize_rel(&file.rel));
    std::fs::rename(tmp_path, &final_path).with_context(|| format!("promote {}", file.rel))?;
    tracing::info!(chunks_total = stats.chunks_total, chunks_reused = stats.chunks_reused, chunks_downloaded = stats.chunks_downloaded, chunks_resumed = stats.chunks_resumed, download_bytes = stats.download_bytes, final_md5 = %h, "repaired");
    Ok(stats)
}

/// Expected-set purge (Collapse GetUnusedFileInfoList, simplified, no SDK/WPF zips).
/// Keeps: manifest paths + config.ini + audio_lang_* + ScreenShot/** + log dirs.
pub fn purge_extra_files(game_dir: &Path, plan: &RepairPlan, dry_run: bool) -> Result<u64> {
    let expected: HashSet<String> = plan.files.iter().map(|f| f.rel.clone()).collect();
    let mut deleted_bytes = 0u64;
    let mut candidates: Vec<(PathBuf, u64)> = Vec::new();
    for e in walkdir::WalkDir::new(game_dir).into_iter().filter_map(|e| e.ok()) {
        if !e.file_type().is_file() {
            continue;
        }
        let rel = e
            .path()
            .strip_prefix(game_dir)
            .unwrap_or(e.path())
            .to_string_lossy()
            .replace('\\', "/");
        if expected.contains(&rel) {
            continue;
        }
        if rel == "config.ini"
            || rel.starts_with("ScreenShot/")
            || rel.starts_with("ScreenShots/")
            || rel.starts_with("log")
            || rel.starts_with("logs/")
            || file_name(&rel).starts_with("audio_lang_")
            || rel.ends_with("_tmp")
            || rel.ends_with(".hdiff")
            || rel.ends_with(".girpr_tmp")
        {
            continue;
        }
        // keep screenshots nested under data dirs too
        if rel.contains("ScreenShot") {
            continue;
        }
        let sz = e.metadata().map(|m| m.len()).unwrap_or(0);
        candidates.push((e.path().to_path_buf(), sz));
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    for (p, sz) in candidates {
        if dry_run {
            tracing::info!("would purge extra {} ({} bytes)", p.display(), sz);
        } else if std::fs::remove_file(&p).is_ok() {
            deleted_bytes += sz;
            tracing::info!("purged extra {} ({} bytes)", p.display(), sz);
        }
    }
    if !dry_run {
        let mut dirs: Vec<PathBuf> = walkdir::WalkDir::new(game_dir)
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
            plan.files.iter().map(|f| f.rel.as_str()).collect::<Vec<_>>(),
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
}
