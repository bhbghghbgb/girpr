use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::plan::{PlannedFile, RepairPlan};
use crate::report::Summary;
use crate::sophon;
use crate::util::{self, normalize_rel};
use tracing::Instrument;

/// Per-chunk counters of one repaired file (also logged in the `repaired` event).
#[derive(Default, Debug)]
pub(crate) struct ChunkStats {
    download_bytes: u64,
    chunks_total: u64,
    chunks_reused: u64,
    chunks_downloaded: u64,
    chunks_resumed: u64,
}

// TODO: rename `Repaired::Repaired` (e.g. `Done`) — touches match sites at repair_all_files; kept to avoid logic churn.
#[allow(clippy::enum_variant_names)] // variant mirrors Starward parity naming; rename would touch 7 match sites
pub(crate) enum Repaired {
    Skipped,
    Repaired(ChunkStats),
    DryRun,
}

/// Repair every planned file with bounded **file** parallelism (chunks inside a
/// file stay sequential → HDD friendly). Each task gets a `file{seq,total,task,path}`
/// span so its progress can be grouped in logs; `task` is the tokio async-task id
/// (stable across thread hops), and the `started on thread` event records which
/// worker picked the file up. Counters land in `sum`.
pub(crate) async fn repair_all_files(
    http: reqwest::Client,
    game_dir: PathBuf,
    plan: Arc<RepairPlan>,
    local_map: HashMap<String, Vec<(String, i64, i64)>>,
    sum: Arc<Summary>,
    jobs: usize,
    dry_run: bool,
) {
    let total_files = plan.files.len();
    let sem = Arc::new(tokio::sync::Semaphore::new(jobs.max(1)));
    let game_dir_a = Arc::new(game_dir);
    let http_a = Arc::new(http);
    // NOTE: this map holds every local chunk (md5+size+offset per chunk, ~10^4 entries).
    // It must be shared by Arc: a per-task deep clone here once cost ~6GB across ~3k tasks.
    let local_map_a = Arc::new(local_map);
    let mut handles = Vec::new();
    for idx in 0..plan.files.len() {
        let sem = sem.clone();
        let game_dir = game_dir_a.clone();
        let http = http_a.clone();
        let plan = plan.clone();
        let sum = sum.clone();
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
    if let Some(parent) = final_path.parent()
        && !dry_run
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
            tracing::debug!(chunk = %c.id, expect_md5 = c.uncompressed_md5, actual_md5 = %dh, "chunk md5 mismatch");
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
