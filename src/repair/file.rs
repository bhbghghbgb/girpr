//! Per-file repair: skip check, in-place patch, `_tmp` resume, chunk reuse vs
//! download, final MD5 (docs/02 step 6, docs/07 §4).
//!
//! Per file, in order:
//! 1. `size + whole-file MD5` match → [`Repaired::Skipped`], 0 bytes touched.
//! 2. Otherwise the file is repaired **in place**: the target is opened
//!    `read+write` and only the bytes that are wrong are written, at the
//!    offsets they belong at. No `_tmp` file, and the peak extra disk is the
//!    file's *growth* rather than its size.
//! 3. The temp path is the fallback and the escape hatch: `{path}_tmp` is
//!    written (kept, not truncated) and renamed over the target, so the
//!    original is never touched. It is what `--repair-mode temp` asks for.
//!
//! The in-place path, in detail:
//! - **scan** — one read that answers everything: the whole-file MD5 (the
//!   skip check), which chunks are already correct where they belong, and
//!   which reuse sources are intact;
//! - **stage** — the bytes of the chunks that have to *move* inside the file
//!   are read into RAM, so the write order cannot destroy them;
//! - **patch** — the wrong chunks are written, ascending: from the staged
//!   bytes when there are any, from a download otherwise;
//! - **verify** — one more read. The whole-file MD5 is the gate; if it does
//!   not match, the per-chunk verdicts name the culprit and only those are
//!   redone.
//!
//! Every byte written is hash-gated before it lands, whichever path wrote
//! it: a downloaded chunk's MD5 is checked before the write, and a staged
//! source's MD5 is checked before it is staged and again before it is
//! written.
//!
//! Files run in parallel, bounded by a semaphore of `jobs`; chunks inside a
//! file are strictly sequential.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use tracing::Instrument;

use crate::config::RepairMode;
use crate::report::Summary;
use crate::sophon::{self, SophonChunk};
use crate::util;

use super::manifest::LocalChunkMap;
use super::plan::{PlannedFile, RepairPlan};
use super::reuse;

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
    /// A runtime hard stop fired. The file was **not** failed and
    /// **not** repaired: its `*_tmp` is left in place and the next run resumes
    /// from it (S3), which is what makes an aborted run a smaller run rather
    /// than a broken one. Not counted in any counter.
    Stopped,
}

/// Per-file chunk accounting (logged on success, aggregated into the run).
#[derive(Default, Debug, Clone)]
pub struct ChunkStats {
    pub download_bytes: u64,
    pub chunks_total: u64,
    pub chunks_reused: u64,
    pub chunks_downloaded: u64,
    pub chunks_resumed: u64,
    /// Chunks that were already correct where they belong and were not
    /// written at all. The in-place path's own "kept" count.
    pub chunks_kept: u64,
}

/// Run the repair loop over the whole plan, bounded to `jobs` concurrent files.
///
/// Per-file failures are counted, never fatal; the caller decides that a
/// non-zero `files_failed` skips the post-phase.
///
/// `output` rides along rather than being read from a global: the progress
/// ticker this spawns emits records, and a record's plane has to come from the
/// same place the rest of the run's does.
#[allow(clippy::too_many_arguments)] // one parameter per input the repair loop needs
pub async fn repair_all(
    http: reqwest::Client,
    game_dir: PathBuf,
    plan: Arc<RepairPlan>,
    local_map: LocalChunkMap,
    summary: Summary,
    jobs: usize,
    dry_run: bool,
    output: crate::report::OutputFormat,
    repair_mode: RepairMode,
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
        repair_mode,
    });
    let progress = crate::report::spawn_progress_reporter(summary.clone(), output);

    let mut handles = Vec::with_capacity(total_files);
    for idx in 0..total_files {
        // `--fail-fast`: once a file has exhausted its retries, no further file
        // is started. The unstarted ones are simply not counted — the caller
        // sees `done < total` in `PROGRESS` and `failed > 0` in `SUMMARY`.
        if ctx.summary.aborted() {
            tracing::warn!(
                remaining = total_files - idx,
                "fail-fast: not starting the remaining files"
            );
            break;
        }
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
                    Ok(Repaired::Repaired(_stats)) => {
                        // `download_bytes` is already live: it is incremented
                        // per chunk inside `write_downloaded_chunk`, because the
                        // runtime hard stop has to read it mid-flight.
                        ctx.summary.files_repaired.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Repaired::DryRun) | Ok(Repaired::Stopped) => {}
                    Err(e) => {
                        ctx.summary.files_failed.fetch_add(1, Ordering::Relaxed);
                        tracing::error!("repair failed: {e:#}");
                        // `--fail-fast`: the file that failed is honestly
                        // counted and the run now stops instead of grinding
                        // through hours of retry backoff. No new counter — the
                        // fact is already visible as `failed > 0` + exit 3.
                        if ctx.summary.fail_fast() {
                            ctx.summary.abort();
                        }
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
    /// How each file is repaired (docs/07 §4).
    repair_mode: RepairMode,
}

/// Skip check, then up to [`MAX_ATTEMPTS`] repair attempts with linear backoff.
async fn repair_one_file(ctx: &FileCtx, idx: usize) -> Result<Repaired> {
    // A runtime hard stop already fired: drop out without touching the file.
    // Its `*_tmp` (if any) stays, and the next run resumes from it.
    if ctx.summary.download_stopped() || ctx.summary.write_stopped() || ctx.summary.aborted() {
        return Ok(Repaired::Stopped);
    }
    let file = &ctx.plan.files[idx];
    let final_path = ctx.game_dir.join(util::normalize_rel(&file.rel));
    if let Some(parent) = final_path.parent()
        && !ctx.dry_run
    {
        std::fs::create_dir_all(parent)?;
    }
    tracing::debug!(expect_size = file.size, expect_md5 = %file.md5, chunks = file.chunks.len(), "check start");
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
    // This is the **fallback** and the `--repair-mode temp` path; the
    // in-place path is the default and never creates a `_tmp`.
    // See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L330-L331
    // and https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L443-L459
    let tmp_path = PathBuf::from(format!("{}_tmp", final_path.display()));

    // `--repair-mode temp` is today's behaviour, exactly. Every other mode
    // starts with the in-place path, which is the one that saves disk.
    let mut inplace = ctx.repair_mode != RepairMode::Temp;

    // `--fail-fast` caps a file's attempts at one instead of [`MAX_ATTEMPTS`].
    // docs/04 E1 asks for "the run reaches its SUMMARY in seconds", and it cannot
    // mean that while the *first* file alone spends 5 × (15 s of chunk-fetch
    // backoff + 10 s of file-level backoff) ≈ 125 s of pure sleeping before anyone
    // else is spared. The flag is an explicit "tell me fast"; a user who wants the
    // full budget does not pass it. Counter semantics are unchanged: a file that
    // fails on its first attempt is still just `files_failed`.
    let max_attempts = if ctx.summary.fail_fast() {
        1
    } else {
        MAX_ATTEMPTS
    };
    let mut attempt = 1;
    while attempt <= max_attempts {
        let outcome = if inplace {
            attempt_inplace(ctx, file, &prefix, &final_path).await
        } else {
            attempt_temp(ctx, file, &prefix, &tmp_path, &final_path).await
        };
        match outcome {
            Ok(Inplace::Skipped) => return Ok(Repaired::Skipped),
            Ok(Inplace::DryRun) => return Ok(Repaired::DryRun),
            Ok(Inplace::Repaired(stats)) => return Ok(Repaired::Repaired(stats)),
            // The in-place path cannot run for this file. The temp path can:
            // it does not need write access to the target. The switch is
            // permanent for this file — an attempt is not spent on it.
            Ok(Inplace::Unavailable(reason)) => {
                tracing::warn!(file = %file.rel, reason, "in-place repair unavailable -> temp");
                inplace = false;
                continue;
            }
            // The ceiling, not the file: this must not be counted as a
            // failure and must not be retried.
            Err(_e) if ctx.summary.download_stopped() || ctx.summary.write_stopped() => {
                tracing::warn!(file = %file.rel, "byte ceiling reached -> stopped");
                return Ok(Repaired::Stopped);
            }
            Err(e) => {
                tracing::warn!(attempt, "attempt failed: {e:#}");
                // `--fail-fast` cuts the backoff short. Without it, one bad
                // file spends 5 × linear sleep (and each of those attempts has
                // its own 5× chunk retry inside `fetch_chunk_bytes`) before the
                // run even starts on the next file.
                if attempt == max_attempts || ctx.summary.aborted() {
                    return Err(e);
                }
                tokio::time::sleep(std::time::Duration::from_secs(attempt as u64)).await;
                attempt += 1;
            }
        }
    }
    unreachable!("attempt loop always returns")
}

/// What one attempt at a file decided.
///
/// Both paths answer in the same vocabulary, so the caller does not have to
/// know which one ran.
enum Inplace {
    /// The file is already correct.
    Skipped,
    /// A dry run: the file would be repaired, nothing was written.
    DryRun,
    /// The file was repaired.
    Repaired(ChunkStats),
    /// This path cannot run for this file; the caller should use the other
    /// one. The reason is for the log.
    Unavailable(&'static str),
}

/// The temp path: the original size + MD5 check, then the attempt.
///
/// This is today's behaviour, unchanged: the target is never modified, the
/// work happens in `{path}_tmp`, and the final whole-file MD5 gates the
/// atomic rename over the target.
async fn attempt_temp(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    tmp_path: &std::path::Path,
    final_path: &std::path::Path,
) -> Result<Inplace> {
    match util::file_len(final_path) {
        None => tracing::debug!("check: missing local file"),
        Some(n) if n != file.size as u64 => {
            tracing::debug!(actual_size = n, "check: size mismatch -> needs repair");
        }
        Some(_) => match util::md5_file(final_path) {
            Ok(h) if h.eq_ignore_ascii_case(&file.md5) => {
                tracing::debug!(actual_md5 = %h, "check: md5 match -> skip");
                return Ok(Inplace::Skipped);
            }
            Ok(h) => tracing::debug!(actual_md5 = %h, "check: md5 mismatch -> needs repair"),
            Err(e) => tracing::debug!("check: unreadable ({e:#}) -> needs repair"),
        },
    }
    if ctx.dry_run {
        tracing::info!("would repair");
        return Ok(Inplace::DryRun);
    }
    let stats = repair_attempt(ctx, file, prefix, tmp_path).await?;
    Ok(Inplace::Repaired(stats))
}

/// Repair one file **in place**.
///
/// The file is written where it lies: only the bytes that are wrong are
/// written, at the offsets they belong at, and no `_tmp` file is created.
/// The peak extra disk is the file's *growth* rather than its size, which is
/// the whole point of the path (docs/07 §4.2).
///
/// The steps, in order:
///
/// 1. **open** the target `read+write` (created when it is not there),
///    clearing the read-only attribute if that is what stopped the open;
/// 2. **scan** — one read that answers everything: the whole-file MD5 (the
///    skip check), which chunks are already correct where they belong, and
///    which reuse sources are intact;
/// 3. **stage** — the bytes of the chunks that have to *move* inside the
///    file are read into RAM, so the write order cannot destroy them;
/// 4. **patch** — the wrong chunks are written, ascending: from the staged
///    bytes when there are any, from a download otherwise;
/// 5. **verify** — one more read. The whole-file MD5 is the gate; if it does
///    not match, the per-chunk verdicts name the culprit and only those are
///    redone.
///
/// A file that cannot be repaired in place — the target cannot be opened for
/// writing, or its moving bytes do not fit the RAM budget — answers
/// [`Inplace::Unavailable`] and the caller uses the temp path instead.
async fn attempt_inplace(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    final_path: &std::path::Path,
) -> Result<Inplace> {
    let mut stats = ChunkStats {
        chunks_total: file.chunks.len() as u64,
        ..Default::default()
    };
    let outcome = attempt_inplace_inner(ctx, file, prefix, final_path, &mut stats).await;
    if outcome.is_err() {
        // The same rule the temp path follows: an abandoned attempt gives its
        // downloaded bytes back, so `SUMMARY download_bytes` means "the bytes
        // of files that were actually repaired".
        ctx.summary
            .adjust_download_bytes(-(stats.download_bytes as i64));
    }
    outcome
}

/// The body of the in-place repair — see [`attempt_inplace`].
async fn attempt_inplace_inner(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    final_path: &std::path::Path,
    stats: &mut ChunkStats,
) -> Result<Inplace> {
    // The reuse candidates: where each chunk's bytes could be copied from.
    let entries: &[(String, i64, i64)] = match ctx.local_map.get(&file.rel) {
        Some(v) => v.as_slice(),
        None => &[],
    };
    let sources = reuse::sources_for(&file.chunks, entries);

    // The scan's two lists, both parallel to the chunks: where each chunk
    // *belongs*, and where its bytes could be copied from. A chunk with no
    // candidate gets an empty source range, so the two verdicts line up
    // index for index.
    let dest_ranges: Vec<util::ScanRange<'_>> = file
        .chunks
        .iter()
        .map(|c| {
            (
                c.offset.max(0) as u64,
                c.uncompressed_size.max(0) as u64,
                c.uncompressed_md5.as_str(),
            )
        })
        .collect();
    let source_ranges: Vec<util::ScanRange<'_>> = file
        .chunks
        .iter()
        .zip(sources.iter())
        .map(|(c, src)| {
            (
                src.unwrap_or(0),
                c.uncompressed_size.max(0) as u64,
                c.uncompressed_md5.as_str(),
            )
        })
        .collect();

    // 1. open. A dry run changes nothing: no file is created and no
    // attribute is cleared, so the target is only read.
    let mut handle = match open_inplace(ctx, final_path) {
        Ok(Some(f)) => f,
        Ok(None) => {
            // A dry run on a file that is not there: nothing is correct, so
            // the file would be repaired.
            tracing::info!("would repair");
            return Ok(Inplace::DryRun);
        }
        Err(e) => {
            // The target cannot be opened for writing. That is not a failed
            // file: the temp path does not need write access to the target,
            // so the repair still happens — through the other path.
            tracing::debug!(error = %format!("{e:#}"), "in-place open failed");
            return Ok(Inplace::Unavailable(
                "the target cannot be opened for writing",
            ));
        }
    };

    // 2. scan. One read: the whole-file MD5, which chunks are already
    // correct, and which sources are intact.
    let mut scan = util::scan_reader(&mut handle, &dest_ranges, &source_ranges)
        .with_context(|| format!("scan {}", file.rel))?;
    tracing::debug!(
        actual_md5 = %scan.file_md5,
        expect_md5 = %file.md5,
        on_disk = scan.file_len,
        "scanned"
    );
    if scan.file_md5.eq_ignore_ascii_case(&file.md5) {
        tracing::debug!(actual_md5 = %scan.file_md5, "check: md5 match -> skip");
        return Ok(Inplace::Skipped);
    }
    if ctx.dry_run {
        tracing::info!("would repair");
        return Ok(Inplace::DryRun);
    }

    // `--repair-mode discard`: the target is deleted and written from
    // scratch. Zero peak extra disk, and a full download for the file — the
    // trade the mode exists to offer. The delete happens *after* the skip
    // check, so an intact file is never deleted.
    if ctx.repair_mode == RepairMode::Discard {
        drop(handle);
        std::fs::remove_file(final_path).ok();
        handle = open_rw(final_path)
            .with_context(|| format!("create {}", final_path.display()))?;
        // The file is gone: every chunk has to be written, and there is
        // nothing left to reuse from.
        scan = util::Scan::absent(dest_ranges.len(), source_ranges.len());
    }

    // The chunks that are not correct where they belong. `file.chunks` is
    // sorted by offset (the plan guarantees it), so this list is ascending
    // too — which is the order the patch loop writes in.
    let bad: Vec<usize> = (0..file.chunks.len())
        .filter(|&i| !scan.dest_ok[i])
        .collect();
    stats.chunks_kept = (file.chunks.len() - bad.len()) as u64;
    tracing::debug!(
        bad = bad.len(),
        kept = stats.chunks_kept,
        "in-place plan"
    );

    // The eager trim: when no reuse candidate reaches past the target's
    // end, the tail is dead weight and can be released before a single byte
    // is written.
    if trim_is_free(file, entries) {
        let have = handle.metadata().map(|m| m.len()).unwrap_or(0);
        if have > file.size.max(0) as u64 {
            tracing::debug!(before = have, after = file.size, "trimming the dead tail");
            handle.set_len(file.size.max(0) as u64)?;
        }
    }

    // 3. stage. A chunk needs staging only if its reuse source is
    // *elsewhere* in the file and the chunk is actually wrong: a source at
    // the chunk's own offset is the bad chunk itself, so it is not a source
    // at all. The common repair case therefore stages nothing.
    let payload: u64 = bad
        .iter()
        .filter(|&&i| sources[i].is_some() && scan.source_ok[i])
        .map(|&i| file.chunks[i].uncompressed_size.max(0) as u64)
        .sum();
    // The reservation is held until the end of the attempt: the guard gives
    // the RAM back when it is dropped.
    let _reservation = if payload > 0 {
        if !ctx.summary.reserve_ram_bytes(payload) {
            // The budget is a budget, not a refusal: the file is repaired by
            // the other path instead. (The ordered tier replaces this
            // fallback; until it lands, the temp path is the safe answer.)
            return Ok(Inplace::Unavailable(
                "the bytes to move do not fit the RAM budget",
            ));
        }
        Some(RamReservation {
            summary: &ctx.summary,
            bytes: payload,
        })
    } else {
        None
    };

    let mut captured: Vec<Option<Vec<u8>>> = vec![None; file.chunks.len()];
    if payload > 0 {
        for &i in &bad {
            if !scan.source_ok[i] {
                continue;
            }
            let Some(off) = sources[i] else { continue };
            let c = &file.chunks[i];
            let bytes = util::read_range(&mut handle, off, c.uncompressed_size.max(0) as u64)
                .with_context(|| format!("capture chunk {} of {}", c.id, file.rel))?;
            // Every byte written is hash-gated before it lands, and a staged
            // source is a write's source: gate it here too.
            let h = format!("{:x}", md5::compute(&bytes));
            if !h.eq_ignore_ascii_case(&c.uncompressed_md5) {
                tracing::debug!(chunk = %c.id, "staged bytes do not match -> download");
                continue;
            }
            captured[i] = Some(bytes);
        }
    }

    // A hole in the chunk coverage would keep the old file's stale bytes,
    // so it is zero-filled. The staged bytes are already in RAM, so a hole
    // that overlaps a source does not lose it.
    let layout = file.layout();
    for (offset, len) in &layout.gaps {
        zero_fill(ctx, &mut handle, *offset, *len)?;
    }

    // 4. patch. Ascending, and only the wrong chunks.
    for &i in &bad {
        let c = &file.chunks[i];
        match &captured[i] {
            Some(bytes) => {
                handle.seek(SeekFrom::Start(c.offset.max(0) as u64))?;
                reserve_write(ctx, bytes.len() as u64)?;
                handle.write_all(bytes)?;
                stats.chunks_reused += 1;
            }
            None => {
                write_downloaded_chunk(ctx, prefix, c, &mut handle, stats).await?;
            }
        }
    }

    // The file must end at its manifest size: shorter means the tail is
    // missing, longer means it is dead weight.
    handle.set_len(file.size.max(0) as u64)?;
    handle.flush()?;

    // 5. verify. One more read: the whole-file MD5 is the gate, and the
    // per-chunk verdicts name the culprit when it does not match.
    let after = util::scan_reader(&mut handle, &dest_ranges, &[])?;
    if after.file_md5.eq_ignore_ascii_case(&file.md5) {
        tracing::info!(
            chunks_total = stats.chunks_total,
            chunks_kept = stats.chunks_kept,
            chunks_reused = stats.chunks_reused,
            chunks_downloaded = stats.chunks_downloaded,
            download_bytes = stats.download_bytes,
            final_md5 = %after.file_md5,
            "repaired in place"
        );
        return Ok(Inplace::Repaired(stats.clone()));
    }

    // Redo only the chunks that are still wrong. A chunk that fails here
    // passed its own MD5, so the damage is elsewhere: a second pass names
    // it instead of re-fetching the whole file.
    let still_bad: Vec<usize> = (0..file.chunks.len())
        .filter(|&i| !after.dest_ok[i])
        .collect();
    if still_bad.is_empty() {
        anyhow::bail!(
            "final md5 mismatch for {}: expect {} got {} (the bytes outside the chunks are wrong)",
            file.rel,
            file.md5,
            after.file_md5
        );
    }
    tracing::warn!(
        file = %file.rel,
        chunks = still_bad.len(),
        "final md5 mismatch -> redoing the mismatching chunks"
    );
    for &i in &still_bad {
        let c = &file.chunks[i];
        write_downloaded_chunk(ctx, prefix, c, &mut handle, stats).await?;
    }
    handle.set_len(file.size.max(0) as u64)?;
    handle.flush()?;
    let final_scan = util::scan_reader(&mut handle, &dest_ranges, &[])?;
    if !final_scan.file_md5.eq_ignore_ascii_case(&file.md5) {
        anyhow::bail!(
            "final md5 mismatch for {}: expect {} got {}",
            file.rel,
            file.md5,
            final_scan.file_md5
        );
    }
    tracing::info!(
        chunks_total = stats.chunks_total,
        chunks_kept = stats.chunks_kept,
        chunks_reused = stats.chunks_reused,
        chunks_downloaded = stats.chunks_downloaded,
        download_bytes = stats.download_bytes,
        final_md5 = %final_scan.file_md5,
        "repaired in place"
    );
    Ok(Inplace::Repaired(stats.clone()))
}

/// A RAM reservation that gives itself back when it is dropped.
struct RamReservation<'a> {
    summary: &'a Summary,
    bytes: u64,
}

impl Drop for RamReservation<'_> {
    fn drop(&mut self) {
        if self.bytes > 0 {
            self.summary.release_ram_bytes(self.bytes);
        }
    }
}

/// Open the target for the in-place repair.
///
/// `read+write`, created when it is not there. `None` means "the file is not
/// there and the run may not create it" — a dry run, which changes nothing.
///
/// A file that cannot be opened for writing is **not** a failure: the temp
/// path does not need write access to the target, so the caller falls back
/// to it. The usual reason a game file cannot be opened for writing is the
/// read-only attribute, and that is cleared first — Starward does the same
/// before it writes (GameInstallService.cs:232-236).
fn open_inplace(ctx: &FileCtx, path: &std::path::Path) -> Result<Option<File>> {
    if ctx.dry_run {
        // A dry run changes nothing: no file is created and no attribute is
        // cleared. Reading is all it needs.
        if !path.exists() {
            return Ok(None);
        }
        let f = File::open(path).with_context(|| format!("open {}", path.display()))?;
        return Ok(Some(f));
    }
    match open_rw(path) {
        Ok(f) => Ok(Some(f)),
        Err(first) => {
            tracing::debug!(
                path = %path.display(),
                error = %first,
                "in-place open failed -> clearing the read-only attribute"
            );
            clear_readonly(path)?;
            let f = open_rw(path).with_context(|| format!("open for writing {}", path.display()))?;
            Ok(Some(f))
        }
    }
}

/// Open a file for the in-place repair: read + write, created when absent,
/// never truncated.
fn open_rw(path: &std::path::Path) -> std::io::Result<File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// Clear the read-only attribute of a file.
///
/// On Windows this is the attribute that stops a write; on Unix it is the
/// write bit, and clearing it is what makes the file writable by its owner
/// — which it has to be for the repair to happen at all.
fn clear_readonly(path: &std::path::Path) -> Result<()> {
    let mut perms = std::fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .permissions();
    if !perms.readonly() {
        return Ok(());
    }
    // The lint is about the blanket "make it writable" reflex. This is the
    // opposite: the file is *already* read-only, and the repair has to
    // write to it, so the attribute is cleared and nothing else is touched.
    #[allow(clippy::permissions_set_readonly_false)]
    {
        perms.set_readonly(false);
    }
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("clear the read-only attribute of {}", path.display()))?;
    tracing::debug!(path = %path.display(), "read-only attribute cleared");
    Ok(())
}

/// Is trimming the file's dead tail free?
///
/// The tail past the manifest's size is dead weight, and releasing it before
/// a single byte is written is the whole point of the eager trim. It is only
/// free when no reuse candidate reaches past the end: an entry in the local
/// reuse map past the target's end can only ever be a source, and a source
/// the trim would delete is a source that is gone.
fn trim_is_free(file: &PlannedFile, entries: &[(String, i64, i64)]) -> bool {
    let size = file.size.max(0) as u64;
    entries.iter().all(|(_, sz, off)| {
        let off = (*off).max(0) as u64;
        let sz = (*sz).max(0) as u64;
        off.checked_add(sz).is_none_or(|end| end <= size)
    })
}

/// Write `len` zeros at `offset`.
///
/// A hole in the chunk coverage is a region no chunk claims, so nothing else
/// would ever write it: without this it would keep the old file's bytes.
fn zero_fill(ctx: &FileCtx, out: &mut File, offset: u64, len: u64) -> Result<()> {
    if len == 0 {
        return Ok(());
    }
    // The zero-fill is a write like any other: it lands in the install and
    // the write ceiling counts it.
    reserve_write(ctx, len)?;
    out.seek(SeekFrom::Start(offset))?;
    let zeros = vec![0u8; COPY_BUF.min(len as usize)];
    let mut left = len;
    while left > 0 {
        let n = (left as usize).min(zeros.len());
        out.write_all(&zeros[..n])?;
        left -= n as u64;
    }
    Ok(())
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
    let outcome = repair_attempt_inner(ctx, file, prefix, tmp_path, &mut stats).await;
    if outcome.is_err() {
        // This attempt is being abandoned, so the bytes it fetched are given
        // back. That is what keeps `SUMMARY download_bytes` meaning
        // **the bytes of files that were actually promoted** — rather than
        // "bytes that crossed the wire at some point, including on the four
        // attempts that failed". The ceiling still saw them while they
        // were in flight, which is the whole reason the counter is live; this
        // only decides what a *finished* run reports.
        ctx.summary
            .adjust_download_bytes(-(stats.download_bytes as i64));
    }
    outcome
}

/// One full pass over a file's chunks, then verify + promote. The caller
/// ([`repair_attempt`]) owns the accounting of a failed attempt.
async fn repair_attempt_inner(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    tmp_path: &std::path::Path,
    stats: &mut ChunkStats,
) -> Result<ChunkStats> {
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
                write_downloaded_chunk(ctx, prefix, c, &mut f, stats).await?;
            }
        }
    }
    f.flush()?;
    drop(f);
    promote_tmp(ctx, file, tmp_path, stats)
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
                // The write ceiling counts *this* copy: it goes into `*_tmp` and
                // appears in neither `download_bytes` nor the plan's estimate.
                reserve_write(ctx, n as u64)?;
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
    // Reserve before fetching, not after: the pre-flight estimate is a *lower
    // bound* (a reuse slice whose MD5 fails falls back to a download), so this
    // compare-and-swap is what actually bounds the run. Reserving the manifest's
    // `compressed_size` keeps the stop exact under `--io-threads N` instead of
    // overshooting by up to N-1 chunks, and the real `blob.len()` is folded back
    // in below so `SUMMARY download_bytes` stays the true figure.
    let reserved = c.compressed_size.max(0) as u64;
    if !ctx.summary.reserve_download_bytes(reserved) {
        ctx.summary.stop_downloads();
        tracing::warn!(
            limit = ?ctx.summary.max_download_bytes(),
            have = ctx.summary.download_bytes.load(Ordering::Relaxed),
            "download-bytes ceiling reached -> stopping the run"
        );
        anyhow::bail!("download-bytes ceiling reached before chunk {}", c.id);
    }
    // A failed fetch delivers nothing, so the reservation goes straight back.
    // Without this the `?` below would leak `compressed_size` per attempt, for
    // bytes that never arrived — and with no ceiling set the sentinel is
    // `u64::MAX`, so the leak is not gated on `--max-download-bytes`.
    let blob = match sophon::fetch_chunk_bytes(&ctx.http, prefix, &c.id).await {
        Ok(b) => b,
        Err(e) => {
            ctx.summary.adjust_download_bytes(-(reserved as i64));
            return Err(e).with_context(|| format!("chunk {}", c.id));
        }
    };
    let actual = blob.len() as u64;
    ctx.summary
        .adjust_download_bytes(actual as i64 - reserved as i64);
    stats.download_bytes += actual;
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
    reserve_write(ctx, decoded.len() as u64)?;
    out.write_all(&decoded)?;
    stats.chunks_downloaded += 1;
    Ok(())
}

/// Reserve `n` bytes of disk write against `--max-write-bytes`, or trip the
/// runtime hard stop.
///
/// Reserving **before** the write is what makes the ceiling meaningful: the
/// bytes land in `*_tmp` as soon as `write_all` returns, so a check-then-write
/// under `--io-threads N` would overshoot by up to N-1 buffers. On refusal the
/// file is left where it is and `*_tmp` keeps whatever prefix it already had,
/// which S3 resume-by-length picks up next run.
fn reserve_write(ctx: &FileCtx, n: u64) -> Result<()> {
    if ctx.summary.reserve_write_bytes(n) {
        return Ok(());
    }
    ctx.summary.stop_writes();
    tracing::warn!(
        limit = ?ctx.summary.max_write_bytes(),
        have = ctx.summary.write_bytes.load(Ordering::Relaxed),
        "write-bytes ceiling reached -> stopping the run"
    );
    anyhow::bail!("write-bytes ceiling reached")
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
        chunks_kept: stats.chunks_kept,
    })
}
