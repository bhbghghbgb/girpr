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
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use tracing::Instrument;

use crate::config::RepairMode;
use crate::repair::scope::ledger::StoppedBy;
use crate::report::Summary;
use crate::sleep::Sleep;
#[cfg(test)]
use crate::sophon::SophonChunk;
use crate::util;

use crate::repair::phases::manifest::LocalChunkMap;
use crate::repair::phases::plan::{PlannedFile, RepairPlan, validate_chunk_ranges};

mod inplace;
mod mode;
mod temp;

#[cfg(test)]
use self::inplace::trim_is_free;

/// Max attempts per file before it is counted as failed.
const MAX_ATTEMPTS: u32 = 5;

/// Copy buffer for the local-slice reuse path.
const COPY_BUF: usize = 512 * 1024;

/// The shared zero block for [`zero_fill`]: [`COPY_BUF`] zero bytes, allocated
/// once and never written, so every hole is filled in the same pieces a local
/// copy uses.
fn zero_block() -> &'static [u8] {
    static ZEROS: OnceLock<Vec<u8>> = OnceLock::new();
    ZEROS.get_or_init(|| vec![0u8; COPY_BUF])
}

/// The per-file scratch buffer for [`copy_within`] and the reuse copy, grown to
/// at least `want` bytes.
///
/// A file's chunks are written one at a time, so one buffer per file is enough;
/// it is never shrunk, so the next piece reuses the same allocation.
fn scratch_slice(buf: &mut Vec<u8>, want: usize) -> &mut [u8] {
    if buf.len() < want {
        buf.resize(want, 0);
    }
    &mut buf[..want]
}

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
    root: Option<util::PathRoot>,
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
        root,
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
            let _permit = sem
                .acquire_owned()
                .await
                .expect("the file semaphore is never closed");
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
                        // rewrites every chunk it does not resume, and a `_tmp`
                        // prefix left by an earlier run is kept and counts
                        // here.
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
    join_file_tasks(handles, &summary).await;
    progress.abort();
    // `try_unwrap` often fails: the aborted reporter task may not have dropped
    // its clone yet. Either way `snapshot` reads the live counters.
    Arc::try_unwrap(summary)
        .map(|s| s.snapshot())
        .unwrap_or_else(|a| a.snapshot())
}

/// Await every spawned file task and charge each join error to `files_failed`.
///
/// A file task returns `()`, so its only `Err` is a panic or a cancellation —
/// never an `anyhow::Error`. Dropping it would let a panicked file leave
/// `files_failed == 0`: the run would bump `config.ini` and exit `0` with that
/// file unrepaired, which is the launcher-desync failure the post-phase
/// ordering exists to prevent.
///
/// The handles are awaited in **completion order**, not spawn order. A panic
/// has already finished; waiting for all earlier handles first would let the
/// queued files keep starting under `--fail-fast`, because the abort flag is
/// only set once this loop observes the panic.
async fn join_file_tasks(handles: Vec<tokio::task::JoinHandle<()>>, summary: &Summary) {
    let mut pending: futures::stream::FuturesUnordered<_> = handles.into_iter().collect();
    while let Some(result) = futures::StreamExt::next(&mut pending).await {
        if let Err(e) = result {
            summary.files_failed.fetch_add(1, Ordering::Relaxed);
            tracing::error!(error = %e, "repair task failed");
            if summary.fail_fast() {
                summary.abort();
            }
        }
    }
}

/// Read-only inputs shared by every file task.
struct FileCtx {
    game_dir: PathBuf,
    /// The install root resolved once for the whole repair loop.
    ///
    /// `None` means the base itself could not be resolved; `repair_one_file`
    /// then falls back to `contained_join`, keeping the per-file refusal and
    /// its message.
    root: Option<util::PathRoot>,
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

/// The manifest-relative path whose file task panics in tests.
///
/// Only [`repair_one_file`] and the wiring test use it; a real manifest cannot
/// contain it, because it names no file the repair would ever plan.
#[cfg(test)]
const TEST_PANIC_REL: &str = "__girpr_test_panic__";

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
    // A test-only seam for the wiring, not the helper: the unit test on
    // `join_file_tasks` proves the join loop counts a `JoinError`, but only a
    // panic raised *inside* a real `repair_all` task proves that `repair_all`
    // still calls it. Without this, reverting line 294 to `let _ = h.await`
    // would be invisible to the suite.
    #[cfg(test)]
    if file.rel == TEST_PANIC_REL {
        panic!("injected panic in the file task");
    }
    // `build_plan` already validated every range (exit `2`), so this is the
    // defensive second gate for a plan built by hand: a write path must never
    // seek on a bad range. Dry-run does not seek, so it keeps the old
    // read-only behaviour here.
    if !ctx.dry_run {
        validate_chunk_ranges(file)?;
    }
    let joined = ctx.root.as_ref().map_or_else(
        || util::contained_join(&ctx.game_dir, &file.rel),
        |root| root.join(&file.rel),
    );
    let final_path = joined?;
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
    // Append the suffix to the path's `OsString` rather than its `display()`:
    // `display()` is lossy, so a non-UTF-8 install path would name a different
    // temp file than the one intended.
    let mut tmp = final_path.as_os_str().to_owned();
    tmp.push("_tmp");
    let tmp_path = PathBuf::from(tmp);

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
    // an attempt on purpose: a chunk an abandoned attempt wrote is in the install,
    // and `kept_bytes` is the complement of this figure.
    let mut writes = FileWrites::default();
    while attempt <= max_attempts {
        let outcome = if inplace {
            inplace::attempt_inplace(ctx, file, &prefix, &final_path, &mut writes).await
        } else {
            temp::attempt_temp(ctx, file, &prefix, &tmp_path, &final_path, &mut writes).await
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
                ctx.sleep
                    .sleep(std::time::Duration::from_secs(attempt as u64))
                    .await;
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

/// Write `bytes` at `offset` and settle them.
///
/// The reservation is the caller's: the three write paths reserve at
/// different granularities on purpose (see [`write_at`], [`copy_within`] and
/// [`zero_fill`]), and the ceiling must be charged at exactly the same points.
/// This helper is only the part they share — seek, write, [`note_write`].
fn write_reserved(
    ctx: &FileCtx,
    writes: &mut FileWrites,
    f: &mut File,
    offset: u64,
    bytes: &[u8],
) -> Result<()> {
    f.seek(SeekFrom::Start(offset))?;
    f.write_all(bytes)?;
    note_write(ctx, writes, bytes.len() as u64)
}

/// Write `bytes` at `offset`, reserving the whole slice up front.
fn write_at(
    ctx: &FileCtx,
    writes: &mut FileWrites,
    f: &mut File,
    offset: u64,
    bytes: &[u8],
) -> Result<()> {
    reserve_write(ctx, bytes.len() as u64)?;
    write_reserved(ctx, writes, f, offset, bytes)
}

/// Copy `len` bytes from `src` to `dst` inside the same file.
///
/// The copy is done in pieces, and each piece is reserved and written before
/// the next is read, so the source and the destination never have to be
/// disjoint in the *handle* — only in the *file*, which the write order
/// guarantees.
fn copy_within(
    ctx: &FileCtx,
    writes: &mut FileWrites,
    f: &mut File,
    scratch: &mut Vec<u8>,
    src: u64,
    dst: u64,
    len: u64,
) -> Result<()> {
    if len == 0 {
        return Ok(());
    }
    let buf = scratch_slice(scratch, COPY_BUF.min(len as usize));
    let mut done = 0u64;
    while done < len {
        let n = ((len - done) as usize).min(buf.len());
        f.seek(SeekFrom::Start(src + done))?;
        f.read_exact(&mut buf[..n])?;
        reserve_write(ctx, n as u64)?;
        write_reserved(ctx, writes, f, dst + done, &buf[..n])?;
        done += n as u64;
    }
    Ok(())
}

/// Write `len` zeros at `offset`.
///
/// A hole in the chunk coverage is a region no chunk claims, so nothing else
/// would ever write it: without this it would keep the old file's bytes.
///
/// The whole length is reserved once, before the first piece lands; the
/// pieces are then written without touching the ceiling again.
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
    let zeros = zero_block();
    let mut left = len;
    let mut pos = offset;
    while left > 0 {
        let n = (left as usize).min(zeros.len());
        write_reserved(ctx, writes, out, pos, &zeros[..n])?;
        left -= n as u64;
        pos += n as u64;
    }
    Ok(())
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

    /// F-8: the seam is reached **through `repair_all`**, not just through
    /// `join_file_tasks`. Reverting `repair_all`'s join loop to a bare
    /// `let _ = handle.await` (or dropping the handles) would leave the panic
    /// invisible: the run would report success with the file unrepaired. This
    /// test fails in exactly that case, because it never calls the join helper
    /// itself.
    #[tokio::test]
    async fn a_panic_inside_a_file_task_fails_the_run() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_panic_wiring", "run");
        let plan = Arc::new(RepairPlan {
            latest: "1.0".into(),
            files: vec![PlannedFile {
                rel: TEST_PANIC_REL.into(),
                size: 4,
                md5: String::new(),
                chunks: Vec::new(),
            }],
            url_prefix_by_file: Default::default(),
        });
        let summary = repair_all(
            reqwest::Client::new(),
            dir.to_path_buf(),
            None,
            plan,
            LocalChunkMap::new(),
            Summary::new(),
            1,
            false,
            crate::report::OutputFormat::Text,
            RepairMode::Inplace,
            Arc::new(crate::sleep::NoSleep),
        )
        .await;
        assert_eq!(
            summary.files_failed.load(Ordering::Relaxed),
            1,
            "a panic in the file task must surface as a failed file"
        );
    }

    /// F-1: a panicked file task is a **failed file**, not a silent success.
    ///
    /// The task body returns `()`, so the only `Err` from its join handle is a
    /// panic or a cancellation. The join loop has to count that error: with
    /// `files_failed == 0` the run would reach the post phase, bump
    /// `config.ini`, and exit `0` with the file unrepaired.
    #[tokio::test]
    async fn a_panicked_file_task_is_counted_as_a_failure() {
        let mut summary = Summary::new();
        summary.set_fail_fast(true);
        let handles = vec![
            tokio::spawn(async {}),
            tokio::spawn(async { panic!("injected file-task panic") }),
        ];
        join_file_tasks(handles, &summary).await;
        assert_eq!(
            summary.files_failed.load(Ordering::Relaxed),
            1,
            "the panicked task must be the failed file"
        );
        assert!(summary.aborted(), "fail-fast must trip on the join error");
    }

    /// N-7: the join loop must observe a panic as soon as its task finishes.
    /// Sequential awaiting in spawn order would wait for every earlier (here:
    /// deliberately slower) handle first, so under `--fail-fast` the queued
    /// files would keep starting after the run already knew it had failed.
    ///
    /// The slower task samples the abort flag when it finishes: it must
    /// already be set by then. With spawn-order awaiting it is not, because
    /// the panicked handle comes after it.
    #[tokio::test]
    async fn a_panic_is_observed_before_a_slower_earlier_task() {
        let summary = Arc::new({
            let mut s = Summary::new();
            s.set_fail_fast(true);
            s
        });
        let saw_abort = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let slow_summary = Arc::clone(&summary);
        let slow_saw = Arc::clone(&saw_abort);
        let slow = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            slow_saw.store(slow_summary.aborted(), Ordering::SeqCst);
        });
        let panicking = tokio::spawn(async { panic!("injected file-task panic") });
        join_file_tasks(vec![slow, panicking], &summary).await;
        assert!(
            saw_abort.load(Ordering::SeqCst),
            "the panic must be observed before the slower task finishes"
        );
        assert_eq!(summary.files_failed.load(Ordering::Relaxed), 1);
    }
}
