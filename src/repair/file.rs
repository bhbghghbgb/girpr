//! Per-file repair: skip check, `_tmp` resume, chunk reuse vs download, final
//! MD5, atomic promote (docs/02 step 6).
//!
//! Per file, in order:
//! 1. `size + whole-file MD5` match → [`Repaired::Skipped`], 0 bytes touched.
//! 2. Otherwise open `{path}_tmp` (kept, not truncated) and walk the manifest's
//!    chunks in offset order:
//!    - a completed prefix is kept (S3, gated by the final MD5),
//!    - else copy a hash-verified slice of the same file (S2),
//!    - else download → zstd → verify → write at the chunk offset (S1).
//! 3. Final whole-file MD5 gates the atomic `rename(tmp → final)`; on mismatch
//!    the tmp is deleted and the file is retried (≤5 attempts), then counted as
//!    failed. The original is never modified in place.
//!
//! Files run in parallel, bounded by a semaphore of `jobs`; chunks inside a file
//! are strictly sequential, which keeps peak disk ≈ `jobs × largest_file`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use tracing::Instrument;

use crate::report::Summary;
use crate::sophon::{self, SophonChunk};
use crate::util;

use super::manifest::LocalChunkMap;
use super::plan::{PlannedFile, RepairPlan};

/// Max attempts per file before it is counted as failed.
const MAX_ATTEMPTS: u32 = 5;

/// Copy buffer for the local-slice reuse path.
const COPY_BUF: usize = 512 * 1024;

/// Outcome of one file task.
// TODO: rename `Repaired::Repaired` (e.g. `Done`) — touches match sites at
// `repair_one_file`/`repair_attempt`; kept to avoid logic churn.
#[allow(clippy::enum_variant_names)] // variant mirrors Starward parity naming; rename would touch 7 match sites
pub enum Repaired {
    Skipped,
    Repaired(ChunkStats),
    /// `--dry-run`: the file would be repaired, nothing was written.
    DryRun,
}

/// Per-file chunk accounting (logged on success, aggregated into the run).
#[derive(Default, Debug)]
pub struct ChunkStats {
    pub download_bytes: u64,
    pub chunks_total: u64,
    pub chunks_reused: u64,
    pub chunks_downloaded: u64,
    pub chunks_resumed: u64,
}

/// Run the repair loop over the whole plan, bounded to `jobs` concurrent files.
///
/// Per-file failures are counted, never fatal; the caller decides that a
/// non-zero `files_failed` skips the post-phase.
pub async fn repair_all(
    http: reqwest::Client,
    game_dir: PathBuf,
    plan: Arc<RepairPlan>,
    local_map: LocalChunkMap,
    summary: Summary,
    jobs: usize,
    dry_run: bool,
) -> Summary {
    let summary = Arc::new(summary);
    let total_files = plan.files.len();
    let sem = Arc::new(tokio::sync::Semaphore::new(jobs.max(1)));
    // NOTE: these are shared by Arc on purpose. `local_map` holds every local
    // chunk (~10^4 entries) and the plan holds every planned file, so a
    // per-task deep clone once cost ~6GB across ~3k tasks.
    let ctx = Arc::new(FileCtx {
        game_dir,
        http,
        plan: Arc::clone(&plan),
        local_map,
        summary: summary.clone(),
        dry_run,
    });
    let progress = crate::report::spawn_progress_reporter(summary.clone());

    let mut handles = Vec::with_capacity(total_files);
    for idx in 0..total_files {
        let sem = sem.clone();
        let ctx = ctx.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.unwrap();
            // Each file task gets a `file{seq,total,task,path}` span so its
            // progress can be grouped in logs; `task` is the tokio async-task id
            // (stable across thread hops), and the `picked up by worker` event
            // records which worker took the file.
            let span = tracing::info_span!(
                "file",
                seq = idx + 1,
                total = total_files,
                task = ?tokio::task::try_id(),
                path = %ctx.plan.files[idx].rel,
            );
            async move {
                tracing::debug!(thread = ?std::thread::current().id(), "picked up by worker");
                match repair_one_file(&ctx, idx).await {
                    Ok(Repaired::Skipped) => {
                        ctx.summary.files_skipped.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Repaired::Repaired(stats)) => {
                        ctx.summary.files_repaired.fetch_add(1, Ordering::Relaxed);
                        ctx.summary
                            .download_bytes
                            .fetch_add(stats.download_bytes, Ordering::Relaxed);
                    }
                    Ok(Repaired::DryRun) => {}
                    Err(e) => {
                        ctx.summary.files_failed.fetch_add(1, Ordering::Relaxed);
                        tracing::error!("repair failed: {e:#}");
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
    // `try_unwrap` often fails: the aborted reporter task may not have dropped
    // its clone yet. Either way `snapshot` reads the live counters.
    Arc::try_unwrap(summary)
        .map(|s| s.snapshot())
        .unwrap_or_else(|a| a.snapshot())
}

/// Read-only inputs shared by every file task.
struct FileCtx {
    game_dir: PathBuf,
    http: reqwest::Client,
    plan: Arc<RepairPlan>,
    local_map: LocalChunkMap,
    summary: Arc<Summary>,
    dry_run: bool,
}

/// Skip check, then up to [`MAX_ATTEMPTS`] repair attempts with linear backoff.
async fn repair_one_file(ctx: &FileCtx, idx: usize) -> Result<Repaired> {
    let file = &ctx.plan.files[idx];
    let final_path = ctx.game_dir.join(util::normalize_rel(&file.rel));
    if let Some(parent) = final_path.parent()
        && !ctx.dry_run
    {
        std::fs::create_dir_all(parent)?;
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
            Err(e) => tracing::debug!("check: unreadable ({e:#}) -> needs repair"),
        },
    }
    if ctx.dry_run {
        tracing::info!("would repair");
        return Ok(Repaired::DryRun);
    }
    let prefix = ctx
        .plan
        .url_prefix_by_file
        .get(&file.rel)
        .cloned()
        .unwrap_or_default();
    // NOTE(starward-parity): temp-then-atomic-promote. Starward writes
    // `FullPath + "_tmp"` (OpenOrCreate), verifies MD5, then `Move(tmp, final,
    // true)` and `Delete(tmp)` on mismatch — same contract here, including the
    // suffix, so foreign leftovers are swept on the next run.
    // See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L330-L331
    // and https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L443-L459
    let tmp_path = PathBuf::from(format!("{}_tmp", final_path.display()));

    for attempt in 1..=MAX_ATTEMPTS {
        match repair_attempt(ctx, file, &prefix, &tmp_path).await {
            Ok(stats) => return Ok(Repaired::Repaired(stats)),
            Err(e) => {
                tracing::warn!(attempt, "attempt failed: {e:#}");
                if attempt == MAX_ATTEMPTS {
                    return Err(e);
                }
                tokio::time::sleep(std::time::Duration::from_secs(attempt as u64)).await;
            }
        }
    }
    unreachable!("attempt loop always returns")
}

/// One full pass over a file's chunks, then verify + promote.
async fn repair_attempt(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    tmp_path: &std::path::Path,
) -> Result<ChunkStats> {
    let mut stats = ChunkStats {
        chunks_total: file.chunks.len() as u64,
        ..Default::default()
    };
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false) // keep tmp resume data; explicit to satisfy clippy::suspicious_open_options
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
    let reuse_entries = ctx.local_map.get(&file.rel);
    for c in &file.chunks {
        match try_reuse_slice(ctx, file, c, reuse_entries, &mut f)? {
            ChunkOutcome::Resumed => stats.chunks_resumed += 1,
            ChunkOutcome::Reused => stats.chunks_reused += 1,
            ChunkOutcome::Download => {
                write_downloaded_chunk(ctx, prefix, c, &mut f, &mut stats).await?;
            }
        }
    }
    f.flush()?;
    drop(f);
    promote_tmp(ctx, file, tmp_path, &stats)
}

/// What one chunk needed. `Resumed` (already complete) and `Reused` (copied
/// from a verified local slice) are both zero-network paths, but they are
/// counted separately because they mean different things in the log.
enum ChunkOutcome {
    Resumed,
    Reused,
    Download,
}

/// S2: copy `c` from a hash-verified slice of the same local file. Returns
/// `false` when there is no candidate or the slice does not verify, so the
/// caller falls back to downloading.
///
/// V1-SIMPLIFICATION (S2, see README): same-file-only reuse. Starward maps each
/// chunk to any local file via `OriginalFileFullPath/Offset` (cross-file dedup);
/// we only reuse slices from the same path, which covers ~all Genshin wins at a
/// fraction of the bookkeeping.
/// See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallFile.cs#L100-L131
/// and https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L347-L360
fn try_reuse_slice(
    ctx: &FileCtx,
    file: &PlannedFile,
    c: &SophonChunk,
    reuse_entries: Option<&Vec<(String, i64, i64)>>,
    out: &mut File,
) -> Result<ChunkOutcome> {
    let end = (c.offset + c.uncompressed_size) as u64;
    let cur_len = out.metadata().map(|m| m.len()).unwrap_or(0);
    // S3 (see README): resume-by-length. A completed prefix is kept without
    // per-chunk re-verification; the final whole-file MD5 gates promotion, so a
    // corrupt prefix only wastes work before failing shut. Mirrors Starward's
    // `if (fs.Length < chunk.Offset + chunk.UncompressedSize)` skip.
    // See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L342-L343
    if cur_len >= end {
        tracing::trace!(chunk = %c.id, offset = c.offset, size = c.uncompressed_size, "chunk already complete -> keep");
        return Ok(ChunkOutcome::Resumed);
    }
    out.seek(SeekFrom::Start(c.offset as u64))?;

    let candidate = reuse_entries.and_then(|entries| {
        entries.iter().find(|(md5, sz, _)| {
            md5.eq_ignore_ascii_case(&c.uncompressed_md5) && *sz == c.uncompressed_size
        })
    });
    let Some((_, _, ro)) = candidate else {
        tracing::trace!(chunk = %c.id, offset = c.offset, size = c.uncompressed_size, "no local reuse candidate -> download");
        return Ok(ChunkOutcome::Download);
    };

    let src = ctx.game_dir.join(util::normalize_rel(&file.rel));
    match util::md5_file_slice(&src, *ro as u64, c.uncompressed_size as u64) {
        Ok(h) if h.eq_ignore_ascii_case(&c.uncompressed_md5) => {
            tracing::trace!(chunk = %c.id, src_offset = ro, "reuse slice md5 ok -> copy");
            let mut sf = File::open(&src)?;
            sf.seek(SeekFrom::Start(*ro as u64))?;
            let mut remaining = c.uncompressed_size as u64;
            let mut buf = vec![0u8; COPY_BUF];
            while remaining > 0 {
                let want = (remaining as usize).min(buf.len());
                let n = sf.read(&mut buf[..want])?;
                if n == 0 {
                    // Short slice: fall through to the download, which re-seeks
                    // to `c.offset` and overwrites the partial copy.
                    tracing::debug!(chunk = %c.id, "reuse slice truncated -> download");
                    return Ok(ChunkOutcome::Download);
                }
                out.write_all(&buf[..n])?;
                remaining -= n as u64;
            }
            Ok(ChunkOutcome::Reused)
        }
        Ok(h) => {
            tracing::debug!(chunk = %c.id, slice_md5 = %h, "reuse slice md5 mismatch -> download");
            Ok(ChunkOutcome::Download)
        }
        Err(e) => {
            tracing::debug!(chunk = %c.id, "reuse slice unreadable ({e:#}) -> download");
            Ok(ChunkOutcome::Download)
        }
    }
}

/// Download → zstd decode → size/MD5 verify → write at the chunk offset.
///
/// V1-SIMPLIFICATION (S1, see README): whole-chunk buffering (`fetch` +
/// `decode_all`) instead of Starward's streaming `Pipe +
/// DecompressionStream` pipeline. Simpler and avoids any retained blob store;
/// costs higher peak RAM per active chunk.
/// See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L374-L412
async fn write_downloaded_chunk(
    ctx: &FileCtx,
    prefix: &str,
    c: &SophonChunk,
    out: &mut File,
    stats: &mut ChunkStats,
) -> Result<()> {
    let blob = sophon::fetch_chunk_bytes(&ctx.http, prefix, &c.id)
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
    out.seek(SeekFrom::Start(c.offset as u64))?;
    out.write_all(&decoded)?;
    stats.chunks_downloaded += 1;
    Ok(())
}

/// Final gate: length + whole-file MD5, then the atomic promote. On mismatch the
/// tmp is deleted so the next attempt starts from a clean state and a bad file
/// can never be promoted partially.
///
/// NOTE(starward-parity): final MD5 gates promotion; on mismatch the tmp is
/// deleted and the file is retried/failed, never promoted partially.
/// See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L443-L459
fn promote_tmp(
    ctx: &FileCtx,
    file: &PlannedFile,
    tmp_path: &std::path::Path,
    stats: &ChunkStats,
) -> Result<ChunkStats> {
    let len = util::file_len(tmp_path).unwrap_or(0);
    if len != file.size as u64 {
        anyhow::bail!("tmp length {} != expected {}", len, file.size);
    }
    let h = util::md5_file(tmp_path)?;
    tracing::debug!(tmp_size = len, tmp_md5 = %h, "after: tmp ready for promote");
    if !h.eq_ignore_ascii_case(&file.md5) {
        std::fs::remove_file(tmp_path).ok();
        anyhow::bail!(
            "final md5 mismatch for {}: expect {} got {}",
            file.rel,
            file.md5,
            h
        );
    }
    let final_path = ctx.game_dir.join(util::normalize_rel(&file.rel));
    std::fs::rename(tmp_path, &final_path).with_context(|| format!("promote {}", file.rel))?;
    tracing::info!(chunks_total = stats.chunks_total, chunks_reused = stats.chunks_reused, chunks_downloaded = stats.chunks_downloaded, chunks_resumed = stats.chunks_resumed, download_bytes = stats.download_bytes, final_md5 = %h, "repaired");
    Ok(ChunkStats {
        download_bytes: stats.download_bytes,
        chunks_total: stats.chunks_total,
        chunks_reused: stats.chunks_reused,
        chunks_downloaded: stats.chunks_downloaded,
        chunks_resumed: stats.chunks_resumed,
    })
}
