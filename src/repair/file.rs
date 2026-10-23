//! Per-file repair: skip check, in-place patch, chunk reuse vs download,
//! final MD5 (docs/02 step 6).
//!
//! Per file, in order:
//! 1. `size + whole-file MD5` match → [`Repaired::Skipped`], 0 bytes touched.
//! 2. Otherwise the file is repaired **in place**: the target is opened
//!    `read+write` and only the bytes that are wrong are written, at the
//!    offsets they belong at. No `_tmp` file, and the peak extra disk is the
//!    file's *growth* rather than its size.
//! 3. `--repair-mode temp` writes the file the other way: `{path}_tmp` is
//!    written (kept, not truncated) and renamed over the target, so the
//!    target file is never modified and the swap is atomic. That is the
//!    property to pick the mode for, and it is also the fallback for a
//!    target that cannot be opened for writing.
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
//! Every byte written lands in a region that was **already wrong at the
//! moment of the write**, and the whole-file MD5 decides whether the file is
//! good.
//!
//! A *downloaded* chunk cannot be hash-gated before it lands: the digest of a
//! decoded stream is only known when the stream ends, and the stream ends
//! after the last byte has been written. What replaces the gate is checkable
//! in the code rather than hopeful:
//!
//! - the in-place path writes only to the chunks the scan found wrong, and to
//!   the ones the whole-file MD5 still finds wrong after a re-do — so the
//!   bytes it lands replace bytes that were already bad;
//! - a downloaded chunk's bytes come from the network, not from the file, so
//!   it consumes no source region: the salvage rule is not *weakened*, it
//!   just loses a candidate when a download fails its digest — the region it
//!   overwrote may have been another chunk's source, and the next scan
//!   re-checks every source, so the cost is a download, never a wrong byte;
//! - the in-place path has **no rollback**, and does not need one: it has no
//!   resume artefact, and the next attempt's scan finds the chunk wrong and
//!   rewrites it. The temp mode is the other way round — it resumes by
//!   length, so a chunk that failed its digest is rolled back to its offset
//!   (see `rollback_tmp`);
//! - the temp path is safe by construction: the `_tmp` is promoted only after
//!   the whole-file MD5 passes, and the rollback keeps its *resume* honest;
//! - a staged source's MD5 is still checked before it is staged, so that path
//!   keeps the stronger property it always had.
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
use crate::repair::scope::ledger::StoppedBy;
use crate::report::Summary;
use crate::sleep::Sleep;
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
    /// **not** repaired, and the work already done for it is kept: under
    /// `--repair-mode temp` the `*_tmp` is left in place and the next run
    /// resumes from it, and in place the chunks that were already correct
    /// stay correct. Either way an aborted run is a smaller run rather than
    /// a broken one. Not counted in any counter.
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
    /// The bytes this file had written into the install **from sources that
    /// passed their gate**, accumulated across its attempts.
    ///
    /// This is the file's half of the run's settled `write_bytes` figure, and
    /// it is what `kept_bytes` is the complement of: the bytes the repair did
    /// not have to touch. It is the *file's* figure rather than an attempt's,
    /// because a chunk written by an attempt that later failed is still in the
    /// install — the next attempt re-verifies the file, finds that chunk
    /// correct, and does not write it again.
    pub settled_writes: u64,
    /// Which path wrote this file.
    pub mode: Mode,
}

/// The bytes one file has had written into the install, across its attempts.
///
/// This is the per-file half of the run's **settled** write figure, and it
/// outlives an attempt: an attempt that wrote a chunk and then failed on the
/// next one leaves those bytes in the install, and the next attempt does not
/// write them again. The per-attempt [`ChunkStats`] is dropped when the
/// attempt fails, so it cannot carry that figure — and `kept_bytes`, which is
/// the complement of this one, over-reported while it was derived from it.
#[derive(Debug, Default, Clone, Copy)]
struct FileWrites {
    /// The bytes written from a source that passed its gate.
    settled: u64,
}

/// How one file was written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Repaired in place: the target was written where it lies.
    #[default]
    Inplace,
    /// Written through `{path}_tmp` and renamed over the target.
    Temp,
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
    sleep: Arc<dyn Sleep>,
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
        sleep,
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
                    Ok(Repaired::Repaired(stats)) => {
                        // The file is repaired, so its downloaded bytes are the
                        // **settled** ones — the bytes of the files that were
                        // actually repaired. The live figure already carries
                        // them, and it carries the abandoned attempts' bytes
                        // too: those crossed the wire, and a budget is about
                        // the traffic, not about the install.
                        if !ctx.summary.settle_download_bytes(stats.download_bytes) {
                            // The **verified** ceiling bound: the file is
                            // repaired and counted, and the run stops here.
                            // The line is crossed by a whole file, because a
                            // file is the atom the figure is counted in.
                            ctx.summary.stop(StoppedBy::VerifiedDownload);
                            tracing::warn!(
                                limit = ?ctx.summary.max_verified_download_bytes(),
                                have = ctx.summary.download.settled(),
                                "verified-download-bytes ceiling reached -> stopping the run"
                            );
                        }
                        ctx.summary.files_repaired.fetch_add(1, Ordering::Relaxed);
                        // Which path wrote it. The disk claim of the run is
                        // "how many files were written where they lie", and
                        // this is the number that says it.
                        match stats.mode {
                            Mode::Inplace => {
                                ctx.summary.files_inplace.fetch_add(1, Ordering::Relaxed);
                            }
                            Mode::Temp => {
                                ctx.summary.files_temp.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        // The bytes that did not have to be written: the file's
                        // size minus what was written. The in-place path
                        // leaves the correct chunks alone; the temp path
                        // rewrites the whole file and keeps nothing.
                        //
                        // The subtraction is against the file's **settled**
                        // writes, accumulated across its attempts: a chunk an
                        // abandoned attempt wrote is in the install, so it is
                        // not a byte the repair did not have to touch.
                        let kept = (ctx.plan.files[idx].size.max(0) as u64)
                            .saturating_sub(stats.settled_writes);
                        ctx.summary.kept_bytes.fetch_add(kept, Ordering::Relaxed);
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
    /// How each file is repaired.
    repair_mode: RepairMode,
    /// Injectable sleep: production waits for real, tests use [`crate::sleep::NoSleep`]
    /// so retry backoff is instant.
    sleep: Arc<dyn Sleep>,
}

/// Skip check, then up to [`MAX_ATTEMPTS`] repair attempts with linear backoff.
async fn repair_one_file(ctx: &FileCtx, idx: usize) -> Result<Repaired> {
    // A runtime hard stop already fired: drop out without touching the file.
    // The work already done for it is kept — under `--repair-mode temp` the
    // `*_tmp` stays and the next run resumes from it, in place the chunks
    // that are already right stay right — so an aborted run is a smaller
    // run, not a broken one.
    if ctx.summary.stopped_by() != StoppedBy::None || ctx.summary.aborted() {
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
    // This is the `--repair-mode temp` path, and the way a file is written
    // when its target cannot be opened for writing; the in-place path is the
    // default and never creates a `_tmp`.
    // See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L330-L331
    // and https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L443-L459
    let tmp_path = PathBuf::from(format!("{}_tmp", final_path.display()));

    // The mode for this file. `--repair-mode temp` never uses the in-place
    // path; the rest decide from what the target is.
    let mut mode = resolve_mode(ctx.repair_mode, &final_path);
    let mut inplace = mode == Mode::Inplace;

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
    // The file's settled writes, accumulated across its attempts. It outlives
    // an attempt on purpose: a chunk an abandoned attempt wrote is in the
    // install, and `kept_bytes` is the complement of this figure.
    let mut writes = FileWrites::default();
    while attempt <= max_attempts {
        let outcome = if inplace {
            attempt_inplace(ctx, file, &prefix, &final_path, &mut writes).await
        } else {
            attempt_temp(ctx, file, &prefix, &tmp_path, &final_path, &mut writes).await
        };
        match outcome {
            Ok(Inplace::Skipped) => return Ok(Repaired::Skipped),
            Ok(Inplace::DryRun) => return Ok(Repaired::DryRun),
            Ok(Inplace::Repaired(stats)) => {
                let mut stats = stats;
                stats.mode = mode;
                stats.settled_writes = writes.settled;
                return Ok(Repaired::Repaired(stats));
            }
            // The target cannot be opened for writing, so this file cannot
            // be written where it lies. The temp path can: it does not need
            // write access to the target. The switch is permanent for this
            // file — an attempt is not spent on it.
            Ok(Inplace::Unavailable(reason)) => {
                tracing::warn!(file = %file.rel, reason, "cannot write the target in place -> writing it through a temp file");
                mode = Mode::Temp;
                inplace = false;
                continue;
            }
            // The ceiling, not the file: this must not be counted as a
            // failure and must not be retried.
            Err(_e) if ctx.summary.stopped_by() != StoppedBy::None => {
                tracing::warn!(file = %file.rel, "byte ceiling reached -> stopped");
                return Ok(Repaired::Stopped);
            }
            Err(e) => {
                tracing::warn!(attempt, "attempt failed: {e:#}");
                // `--fail-fast` cuts the backoff short. Without it, one bad
                // file spends 5 × linear sleep (and each of those attempts has
                // its own 5× chunk retry inside `stream_chunk_to`) before the
                // run even starts on the next file.
                if attempt == max_attempts || ctx.summary.aborted() {
                    return Err(e);
                }
                ctx.sleep.sleep(std::time::Duration::from_secs(attempt as u64)).await;
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

/// How one file will be repaired.
///
/// The table:
///
/// | the target              | mode                          |
/// |-------------------------|-------------------------------|
/// | does not exist          | in place — it is created      |
/// | an existing regular file| in place                      |
/// | a directory, a symlink  | temp — it is not a file       |
///
/// `--repair-mode` decides whether the in-place path is tried at all:
/// `temp` never is, and `discard` deletes the target first and then repairs
/// it in place.
///
/// A target that turns out to be unopenable for writing is **not** decided
/// here: the attempt finds out and answers `Inplace::Unavailable`, and the
/// caller falls back to the temp path with a warning. Deciding it from the
/// metadata alone would be a guess; deciding it from the open is a fact.
///
/// `pub(super)` because the plan's disk estimates ask the same question: the
/// estimate and the repair must not disagree about which path a file takes.
pub(super) fn resolve_mode(requested: RepairMode, path: &std::path::Path) -> Mode {
    if requested == RepairMode::Temp {
        return Mode::Temp;
    }
    match std::fs::symlink_metadata(path) {
        // An existing regular file: repair it where it lies.
        Ok(m) if m.is_file() => Mode::Inplace,
        // A directory, a symlink, a device: not a file we can write into.
        // The temp path replaces the path with a regular file, which is what
        // a manifest entry means.
        Ok(_) => Mode::Temp,
        // Not there: the in-place path creates it.
        Err(_) => Mode::Inplace,
    }
}

/// The temp path: the original size + MD5 check, then the attempt.
///
/// The repair is assembled in `{path}_tmp` and swapped over the target with
/// an atomic rename, so **the target file is never modified** and a repair
/// that fails leaves it exactly as it was. That is the property this mode
/// exists for; what it costs is the disk — the whole file has to exist
/// twice, for as long as the repair takes.
async fn attempt_temp(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    tmp_path: &std::path::Path,
    final_path: &std::path::Path,
    writes: &mut FileWrites,
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
    let stats = repair_attempt(ctx, file, prefix, tmp_path, writes).await?;
    Ok(Inplace::Repaired(stats))
}

/// Repair one file **in place**.
///
/// The file is written where it lies: only the bytes that are wrong are
/// written, at the offsets they belong at, and no `_tmp` file is created.
/// The peak extra disk is the file's *growth* rather than its size, which is
/// the whole point of the path.
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
/// A target that cannot be opened for writing answers [`Inplace::Unavailable`]
/// and the caller writes that file the other way — through a `_tmp`, which
/// does not need write access to the target.
async fn attempt_inplace(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    final_path: &std::path::Path,
    writes: &mut FileWrites,
) -> Result<Inplace> {
    let mut stats = ChunkStats {
        chunks_total: file.chunks.len() as u64,
        ..Default::default()
    };
    // An abandoned attempt settles nothing: the bytes it fetched were spent
    // (the live figure has them) but they are not part of the install, so the
    // reported `download_bytes` — "the bytes of the files that were actually
    // repaired" — does not count them. The settle happens where the file is
    // promoted, in `repair_all`.
    attempt_inplace_inner(ctx, file, prefix, final_path, &mut stats, writes).await
}

/// The body of the in-place repair — see [`attempt_inplace`].
async fn attempt_inplace_inner(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    final_path: &std::path::Path,
    stats: &mut ChunkStats,
    writes: &mut FileWrites,
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
        handle = open_rw(final_path).with_context(|| format!("create {}", final_path.display()))?;
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
    tracing::debug!(bad = bad.len(), kept = stats.chunks_kept, "in-place plan");

    // The eager trim: when no reuse candidate reaches past the target's
    // end, the tail is dead weight and can be released before a single byte
    // is written.
    //
    // **The other half is logged too.** The trim's outcome is not in the
    // output contract — the file ends at the target size either way, and the
    // difference is the *peak* disk usage, which nothing reports. The log is
    // the only place the outcome is visible, so both branches say which one
    // they took.
    let have = handle.metadata().map(|m| m.len()).unwrap_or(0);
    if have > file.size.max(0) as u64 {
        if trim_is_free(file, &file.chunks, &sources) {
            tracing::debug!(before = have, after = file.size, "trimming the dead tail");
            handle.set_len(file.size.max(0) as u64)?;
        } else {
            // The deferred trim: a reuse candidate reaches past the target's
            // end, so the tail is not dead weight. The trim waits for the end
            // and the file grows during the repair instead.
            tracing::debug!(
                before = have,
                after = file.size,
                "a reuse candidate reaches past the target's end: the trim is \
                 deferred to the end (the file grows during the repair)"
            );
        }
    }

    // 3. the tier, and the staging that goes with it.
    //
    // `Staged` reads the bytes that have to move into RAM, so **every**
    // source survives and the writes run forward. `Ordered` reads each
    // source from the file at the moment it is written, and the write
    // order is the one that keeps the most of them alive.
    //
    // The budget picks the tier; it never refuses the file. A file that
    // cannot get its reservation is written in the ordered direction, which
    // produces the same file with less reuse.
    let payload: u64 = bad
        .iter()
        .filter(|&&i| sources[i].is_some() && scan.source_ok[i])
        .map(|&i| file.chunks[i].uncompressed_size.max(0) as u64)
        .sum();
    let (tier, _reservation) = match ctx.summary.max_ram_bytes() {
        // "stage nothing at all" is a mode, not a shortage.
        Some(0) => (Tier::Ordered, None),
        _ if payload == 0 => (Tier::Staged, None),
        _ => {
            if ctx.summary.reserve_ram_bytes(payload) {
                (
                    Tier::Staged,
                    Some(RamReservation {
                        summary: &ctx.summary,
                        bytes: payload,
                    }),
                )
            } else {
                tracing::info!(
                    file = %file.rel,
                    payload,
                    "the moving bytes do not fit the RAM budget -> ordered tier"
                );
                (Tier::Ordered, None)
            }
        }
    };

    let mut captured: Vec<Option<Vec<u8>>> = vec![None; file.chunks.len()];
    if matches!(tier, Tier::Staged) && payload > 0 {
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

    // The order (ordered tier only): the direction that keeps the most
    // sources alive, and which sources those are. Only the sources the scan
    // found intact can be salvaged — the rest have to be downloaded.
    let order = match tier {
        Tier::Staged => None,
        Tier::Ordered => {
            let chunks: Vec<SophonChunk> = bad.iter().map(|&i| file.chunks[i].clone()).collect();
            let srcs: Vec<Option<u64>> = bad
                .iter()
                .map(|&i| if scan.source_ok[i] { sources[i] } else { None })
                .collect();
            let usable = handle.metadata().map(|m| m.len()).unwrap_or(0);
            let o = reuse::salvage_order(&chunks, &srcs, usable);
            tracing::info!(
                file = %file.rel,
                ascending = o.ascending,
                salvaged = o.live_count(),
                of = chunks.len(),
                "ordered tier"
            );
            Some(o)
        }
    };

    // 4. patch. Ascending in the staged tier (forward writes), in the
    // salvage order in the ordered tier.
    let mut write_order: Vec<usize> = (0..bad.len()).collect();
    if let Some(o) = &order
        && !o.ascending
    {
        write_order.reverse();
    }
    for k in write_order {
        let i = bad[k];
        let c = &file.chunks[i];
        let dst = c.offset.max(0) as u64;
        if let Some(bytes) = &captured[i] {
            // A staged source: the bytes are already verified.
            write_at(ctx, writes, &mut handle, dst, bytes)?;
            stats.chunks_reused += 1;
        } else if let Some(o) = &order
            && o.live[k]
            && let Some(src) = sources[i]
        {
            // The source survives the order: copy it from inside the file.
            copy_within(
                ctx,
                writes,
                &mut handle,
                src,
                dst,
                c.uncompressed_size.max(0) as u64,
            )?;
            stats.chunks_reused += 1;
        } else {
            write_downloaded_chunk(ctx, prefix, c, &mut handle, stats, writes).await?;
        }
    }

    // A hole in the chunk coverage would keep the old file's stale bytes,
    // so it is zero-filled. The fill happens **after** the writes: in the
    // ordered tier a hole can still hold a source that a later chunk needs.
    let layout = file.layout();
    for (offset, len) in &layout.gaps {
        zero_fill(ctx, writes, &mut handle, *offset, *len)?;
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
        write_downloaded_chunk(ctx, prefix, c, &mut handle, stats, writes).await?;
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

/// Which way the file is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tier {
    /// The bytes that have to move inside the file are staged in RAM, so
    /// every source survives and the writes run forward.
    Staged,
    /// No staging: the sources are read from the file at the moment they are
    /// written, and the write order is the one that keeps the most of them
    /// alive.
    Ordered,
}

/// A write that passed its gate.
///
/// The bytes are counted twice, on purpose. The run's **settled** figure is
/// the reported one: the bytes written into the install from sources that
/// passed their gate. The **live** figure that the `--max-write-bytes`
/// ceiling reads is charged separately, by [`reserve_write`], at the moment the
/// bytes land — so the ceiling bounds the disk, and the reported figure stays
/// a function of the input state.
///
/// The streamed download calls this **after** its digest has been checked, so
/// a chunk that never validated contributes nothing to the reported figure.
///
/// The **verified** ceiling (`--max-verified-write-bytes`) is checked here,
/// where the bytes are settled: the figure is only known once the source has
/// passed its gate, so this is the earliest the question can be asked. The
/// bytes are counted either way — the write happened — and the stop is the
/// caller's.
fn note_write(ctx: &FileCtx, writes: &mut FileWrites, n: u64) -> Result<()> {
    writes.settled += n;
    if !ctx.summary.note_write_bytes(n) {
        ctx.summary.stop(StoppedBy::VerifiedWrite);
        tracing::warn!(
            limit = ?ctx.summary.max_verified_write_bytes(),
            have = ctx.summary.write.settled(),
            "verified-write-bytes ceiling reached -> stopping the run"
        );
        anyhow::bail!("verified-write-bytes ceiling reached");
    }
    Ok(())
}

/// Write `bytes` at `offset`.
fn write_at(
    ctx: &FileCtx,
    writes: &mut FileWrites,
    f: &mut File,
    offset: u64,
    bytes: &[u8],
) -> Result<()> {
    reserve_write(ctx, bytes.len() as u64)?;
    f.seek(SeekFrom::Start(offset))?;
    f.write_all(bytes)?;
    note_write(ctx, writes, bytes.len() as u64)?;
    Ok(())
}

/// Copy `len` bytes from `src` to `dst` inside the same file.
///
/// The copy is done in pieces, and each piece is read before it is written,
/// so the source and the destination never have to be disjoint in the
/// *handle* — only in the *file*, which the write order guarantees.
fn copy_within(
    ctx: &FileCtx,
    writes: &mut FileWrites,
    f: &mut File,
    src: u64,
    dst: u64,
    len: u64,
) -> Result<()> {
    if len == 0 {
        return Ok(());
    }
    let mut buf = vec![0u8; COPY_BUF.min(len as usize)];
    let mut done = 0u64;
    while done < len {
        let n = ((len - done) as usize).min(buf.len());
        f.seek(SeekFrom::Start(src + done))?;
        f.read_exact(&mut buf[..n])?;
        reserve_write(ctx, n as u64)?;
        f.seek(SeekFrom::Start(dst + done))?;
        f.write_all(&buf[..n])?;
        note_write(ctx, writes, n as u64)?;
        done += n as u64;
    }
    Ok(())
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
            let f =
                open_rw(path).with_context(|| format!("open for writing {}", path.display()))?;
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
/// free when no **reuse candidate** reaches past the target's end: a source
/// the trim would delete is a source that is gone.
///
/// **The candidates are the plan's sources, not every entry in the local reuse
/// map.** The map carries one entry per *distinct chunk identity* of the file,
/// so an entry the repair never reads is in it too — and the map's last entry
/// always ends at the old file's end, because the file's chunks tile it. With
/// the map's rule the tail is therefore never dead weight: the eager trim
/// could not fire for **any** file, and the branch was dead code.
///
/// The plan's sources are the map entries the repair actually reads — one per
/// chunk, the first match for its identity — and a chunk with no candidate has
/// none. That is the same set the salvage rule uses, and it is exactly what
/// "no reuse candidate has `r + s > size`" says.
///
/// The test is still **conservative** where it matters: the sources are read
/// from the file the trim shortens, so every one of them has to be inside the
/// target's end, and a candidate that is not is a candidate the trim would
/// destroy.
fn trim_is_free(file: &PlannedFile, chunks: &[SophonChunk], sources: &[Option<u64>]) -> bool {
    let size = file.size.max(0) as u64;
    chunks.iter().zip(sources).all(|(c, src)| {
        let Some(r) = *src else { return true };
        let s = c.uncompressed_size.max(0) as u64;
        r.checked_add(s).is_none_or(|end| end <= size)
    })
}

/// Write `len` zeros at `offset`.
///
/// A hole in the chunk coverage is a region no chunk claims, so nothing else
/// would ever write it: without this it would keep the old file's bytes.
fn zero_fill(
    ctx: &FileCtx,
    writes: &mut FileWrites,
    out: &mut File,
    offset: u64,
    len: u64,
) -> Result<()> {
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
        note_write(ctx, writes, n as u64)?;
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
    writes: &mut FileWrites,
) -> Result<ChunkStats> {
    let mut stats = ChunkStats {
        chunks_total: file.chunks.len() as u64,
        ..Default::default()
    };
    // An abandoned attempt settles nothing — the same rule the in-place path
    // follows. Its bytes were spent (the live figure has them) but they are
    // not part of the install, so the reported `download_bytes` — "the bytes
    // of the files that were actually repaired" — does not count them. The
    // settle happens where the file is promoted, in `repair_all`.
    repair_attempt_inner(ctx, file, prefix, tmp_path, &mut stats, writes).await
}

/// One full pass over a file's chunks, then verify + promote. The caller
/// ([`repair_attempt`]) owns the accounting of a failed attempt.
async fn repair_attempt_inner(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    tmp_path: &std::path::Path,
    stats: &mut ChunkStats,
    writes: &mut FileWrites,
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
        match try_reuse_slice(ctx, file, c, reuse_entries, &mut f, writes)? {
            ChunkOutcome::Resumed => stats.chunks_resumed += 1,
            ChunkOutcome::Reused => stats.chunks_reused += 1,
            ChunkOutcome::Download => {
                if let Err(e) = write_downloaded_chunk(ctx, prefix, c, &mut f, stats, writes).await
                {
                    // The chunk's bytes may have landed in the `_tmp` before
                    // its digest was known, and the temp mode resumes by
                    // **length**: a `_tmp` that grew past this chunk would be
                    // taken as "already complete" by the next run, and the
                    // corrupt bytes would be kept. Truncating back to the
                    // chunk's offset makes the resume honest — the chunks
                    // before it were each digest-checked when they were
                    // written, and everything from it on is gone.
                    rollback_tmp(&mut f, c.offset.max(0) as u64)?;
                    return Err(e);
                }
            }
        }
    }
    f.flush()?;
    drop(f);
    promote_tmp(ctx, file, tmp_path, stats, writes)
}

/// What one chunk needed. `Resumed` (already complete) and `Reused` (copied
/// from a verified local slice) are both zero-network paths, but they are
/// counted separately because they mean different things in the log.
enum ChunkOutcome {
    Resumed,
    Reused,
    Download,
}

/// Truncate the `_tmp` back to `offset`, so a chunk that failed cannot be
/// resumed as "already complete".
///
/// **Why the temp mode needs this and the in-place path does not.** The temp
/// mode resumes by **length**: a chunk whose end is inside the `_tmp` is
/// taken as done, with no hash check. A streamed chunk's bytes are written
/// before its digest is known, so a chunk that fails its digest leaves a
/// `_tmp` that has grown past it — and the next attempt would keep the
/// corrupt bytes and call the file complete. The rollback makes the resume
/// point honest.
///
/// **What the truncation guarantees.** The `_tmp` is cut back to the failed
/// chunk's offset, so the chunk itself cannot be taken as "already complete"
/// and the bytes after it are gone. The bytes *before* it are either
/// digest-checked (the chunks this attempt wrote) or the prefix a previous
/// run left — the temp mode resumes by length, with no hash check of its own,
/// which is the whole reason the rollback has to exist. Either way the
/// whole-file MD5 gates the promotion.
///
/// The truncation never extends: a `_tmp` shorter than the offset is left
/// alone.
fn rollback_tmp(f: &mut File, offset: u64) -> Result<()> {
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len <= offset {
        return Ok(());
    }
    f.set_len(offset)?;
    tracing::debug!(
        tmp_bytes = len,
        offset,
        "the failed chunk is rolled back out of the tmp"
    );
    Ok(())
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
    writes: &mut FileWrites,
) -> Result<ChunkOutcome> {
    let end = (c.offset + c.uncompressed_size) as u64;
    let cur_len = out.metadata().map(|m| m.len()).unwrap_or(0);
    // Resume-by-length: the temp mode's resume property (see README). A
    // completed prefix is kept without per-chunk re-verification; the final
    // whole-file MD5 gates promotion, so a corrupt prefix only wastes work
    // before failing shut. Mirrors Starward's
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
                note_write(ctx, writes, n as u64)?;
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

/// The sink of a streamed chunk: the decoded bytes go straight into the
/// target file, at the chunk's offset.
///
/// **The write ceiling is charged per piece**, the way the reuse copy charges
/// it (`copy_within`): a chunk is no longer one indivisible write, so a
/// ceiling that trips in the middle of a chunk stops the run in the middle of
/// a chunk. The README already documents that a runtime hard stop leaves the
/// work done in place, and the next run re-verifies and continues.
struct ChunkFileSink<'a> {
    ctx: &'a FileCtx,
    file: &'a mut File,
    offset: u64,
    written: u64,
}

impl<'a> ChunkFileSink<'a> {
    fn new(ctx: &'a FileCtx, file: &'a mut File, offset: u64) -> Self {
        Self {
            ctx,
            file,
            offset,
            written: 0,
        }
    }

    /// How many bytes have landed in the file.
    fn written(&self) -> u64 {
        self.written
    }
}

impl Write for ChunkFileSink<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // The write ceiling counts this piece: it goes into the install, and
        // the ceiling counts what lands there.
        reserve_write(self.ctx, buf.len() as u64).map_err(std::io::Error::other)?;
        // The seek is per piece, and it is deliberate: the sink is
        // position-independent, so the caller cannot get it wrong by having
        // left the file's cursor somewhere else. A syscall per 32 KB is
        // nothing next to the decode.
        self.file
            .seek(SeekFrom::Start(self.offset + self.written))?;
        self.file.write_all(buf)?;
        self.written += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl sophon::ChunkSink for ChunkFileSink<'_> {
    fn restart(&mut self) -> std::io::Result<()> {
        // A retry starts the chunk over. The failed attempt's bytes are in the
        // file, and they are about to be overwritten by the same chunk's
        // bytes — the region was wrong before the write and it is wrong now.
        self.written = 0;
        self.file.seek(SeekFrom::Start(self.offset))?;
        Ok(())
    }
}

/// Download → zstd decode → digest → write at the chunk offset.
///
/// The chunk is **streamed**: the compressed body is decoded as it arrives and
/// the decoded bytes are written into the target as they are produced, so the
/// chunk is never resident as a whole. The digest is therefore known when the
/// stream ends — one byte too late to gate the write, which is the invariant
/// this path changes (see the module doc).
///
/// NOTE(starward-parity): Starward writes the decompressed chunk straight into
/// the file and gates on the whole-file MD5; this is the same shape.
/// See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L374-L412
async fn write_downloaded_chunk(
    ctx: &FileCtx,
    prefix: &str,
    c: &SophonChunk,
    out: &mut File,
    stats: &mut ChunkStats,
    writes: &mut FileWrites,
) -> Result<()> {
    // Reserve before fetching, not after: the pre-flight estimate is a *lower
    // bound* (a reuse slice whose MD5 fails falls back to a download), so this
    // compare-and-swap is what actually bounds the run. Reserving the manifest's
    // `compressed_size` keeps the stop exact under `--io-threads N` instead of
    // overshooting by up to N-1 chunks, and the real byte count is folded back
    // in below so `SUMMARY download_bytes` stays the true figure.
    let reserved = c.compressed_size.max(0) as u64;
    if !ctx.summary.reserve_download_bytes(reserved) {
        ctx.summary.stop(StoppedBy::Download);
        tracing::warn!(
            limit = ?ctx.summary.max_download_bytes(),
            have = ctx.summary.download.live(),
            "download-bytes ceiling reached -> stopping the run"
        );
        anyhow::bail!("download-bytes ceiling reached before chunk {}", c.id);
    }
    let expected = c.uncompressed_size.max(0) as u64;
    let mut sink = ChunkFileSink::new(ctx, &mut *out, c.offset.max(0) as u64);
    // A failed fetch delivers nothing, so the reservation goes straight back.
    // Without this the `?` below would leak `compressed_size` per attempt, for
    // bytes that never arrived — and with no ceiling set the sentinel is
    // `u64::MAX`, so the leak is not gated on `--max-download-bytes`.
    let digest = match sophon::stream_chunk_to(
        &ctx.http,
        ctx.sleep.as_ref(),
        prefix,
        &c.id,
        expected,
        &mut sink,
    )
    .await
    {
        Ok(d) => d,
        Err(e) => {
            ctx.summary.adjust_download_bytes(-(reserved as i64));
            return Err(e).with_context(|| format!("chunk {}", c.id));
        }
    };
    let actual = digest.compressed_bytes;
    ctx.summary
        .adjust_download_bytes(actual as i64 - reserved as i64);
    stats.download_bytes += actual;
    tracing::trace!(chunk = %c.id, compressed_bytes = actual, decompressed_bytes = digest.bytes, "chunk decoded");
    // The size and the digest are checked **after** the stream ends, which is
    // after the last byte has been written. A mismatch fails the attempt, and
    // the attempt's bytes are handed back by the caller.
    //
    // The write is counted **after** the checks, so a chunk that never
    // validated contributes nothing to the reported `write_bytes`. The bytes
    // are still spent — the live counter the ceiling reads has them — but the
    // run's answer to "how much did the repair rewrite" does not move because
    // the network was flaky.
    if digest.bytes != expected {
        anyhow::bail!(
            "chunk {} size mismatch: expect {} got {}",
            c.id,
            c.uncompressed_size,
            digest.bytes
        );
    }
    if !digest.md5.eq_ignore_ascii_case(&c.uncompressed_md5) {
        tracing::debug!(chunk = %c.id, expect_md5 = %c.uncompressed_md5, actual_md5 = %digest.md5, "chunk md5 mismatch");
        anyhow::bail!("chunk {} md5 mismatch", c.id);
    }
    note_write(ctx, writes, sink.written())?;
    stats.chunks_downloaded += 1;
    Ok(())
}

/// Reserve `n` bytes of disk write against `--max-write-bytes`, or trip the
/// runtime hard stop.
///
/// Reserving **before** the write is what makes the ceiling meaningful: the
/// bytes land in the install as soon as `write_all` returns, so a
/// check-then-write under `--io-threads N` would overshoot by up to N-1
/// buffers. On refusal the file is left where it is, and whatever was
/// already correct in it stays correct: the next run re-verifies and
/// continues.
fn reserve_write(ctx: &FileCtx, n: u64) -> Result<()> {
    if ctx.summary.reserve_write_bytes(n) {
        return Ok(());
    }
    ctx.summary.stop_writes();
    tracing::warn!(
        limit = ?ctx.summary.max_write_bytes(),
        have = ctx.summary.write.live(),
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
    writes: &FileWrites,
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
        settled_writes: writes.settled,
        mode: Mode::Temp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The resolution table, as a table.
    ///
    /// The mode is decided from what the target **is**, not from what the
    /// repair would like it to be: a path that is not a regular file cannot
    /// be written in place, and the temp path is the one that replaces a
    /// path with a regular file.
    #[test]
    fn the_resolution_table_is_pinned() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_resolve", "table");

        // Not there: the in-place path creates it.
        let missing = dir.join("missing.dat");
        assert_eq!(resolve_mode(RepairMode::Auto, &missing), Mode::Inplace);
        assert_eq!(resolve_mode(RepairMode::Inplace, &missing), Mode::Inplace);

        // An existing regular file: in place.
        let file = dir.join("f.dat");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(resolve_mode(RepairMode::Auto, &file), Mode::Inplace);

        // A directory: not a file we can write into.
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(resolve_mode(RepairMode::Auto, &sub), Mode::Temp);

        // A symlink: the temp path replaces it with a regular file, which is
        // what a manifest entry means. (Windows needs the privilege, so the
        // case is skipped where it cannot be made.)
        #[cfg(unix)]
        {
            let link = dir.join("link.dat");
            std::os::unix::fs::symlink(&file, &link).unwrap();
            assert_eq!(resolve_mode(RepairMode::Auto, &link), Mode::Temp);
        }

        // `--repair-mode temp` never uses the in-place path.
        assert_eq!(resolve_mode(RepairMode::Temp, &file), Mode::Temp);
        assert_eq!(resolve_mode(RepairMode::Temp, &missing), Mode::Temp);
    }

    /// The mode a file was repaired with is carried on its stats, so the run
    /// can count the paths it used.
    #[test]
    fn the_mode_is_part_of_the_file_stats() {
        let mut s = ChunkStats::default();
        assert_eq!(s.mode, Mode::Inplace, "the default is the disk-saving one");
        s.mode = Mode::Temp;
        assert_eq!(s.mode, Mode::Temp);
        assert_ne!(Mode::Inplace, Mode::Temp);
    }

    /// The trim's test: no **reuse candidate** reaches past the target's end.
    ///
    /// The candidates are the plan's **sources** — one per chunk, the first
    /// match for its identity — and not every entry in the local reuse map. The
    /// map's last entry always ends at the old file's end, because the file's
    /// chunks tile it, so with the map's rule the eager trim could never fire
    /// for any file at all.
    #[test]
    fn the_trim_is_free_when_no_reuse_candidate_reaches_past_the_target() {
        let chunk = |offset: i64, size: i64| SophonChunk {
            id: format!("c{offset}"),
            uncompressed_md5: format!("m{offset}"),
            offset,
            compressed_size: size,
            uncompressed_size: size,
            unknown: 0,
            compressed_md5: String::new(),
        };
        let chunks = vec![chunk(0, 10), chunk(10, 10)];
        let file = PlannedFile {
            rel: "f.bin".into(),
            size: 20,
            md5: "m".into(),
            chunks: chunks.clone(),
        };

        // Both candidates inside the target's end: the tail is dead weight.
        assert!(trim_is_free(&file, &chunks, &[Some(0), Some(10)]));
        // The second one reaches past it: the tail is not dead weight.
        assert!(!trim_is_free(&file, &chunks, &[Some(0), Some(15)]));
        // A chunk with no candidate at all: nothing to lose.
        assert!(trim_is_free(&file, &chunks, &[None, Some(10)]));
        // The test is against the **target's** end, so a bigger target frees it.
        let bigger = PlannedFile {
            size: 40,
            ..file.clone()
        };
        assert!(trim_is_free(&bigger, &chunks, &[Some(0), Some(30)]));
    }
}
