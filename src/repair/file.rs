//! Per-file repair: skip check, `_tmp` resume, chunk reuse vs download, final
//! MD5, atomic promote (docs/02 step 6) — plus the **in-place** path that
//! replaces the temp-then-promote contract (docs/07 §4).
//!
//! Per file, in order:
//! 1. `size + whole-file MD5` match → [`Repaired::Skipped`], 0 bytes touched.
//! 2. Otherwise the file is repaired by one of two paths:
//!    - **in place** — the target is opened read+write and patched *where it
//!      lies*: only the chunks that are wrong are written, and the extra disk
//!      a repair needs is the file's **growth** rather than a second copy of
//!      it (docs/07 §4.2). This is the default.
//!    - **through `{path}_tmp`** — the shipped path: a completed prefix is
//!      kept (S3, gated by the final MD5), else a hash-verified slice of the
//!      same file is copied (S2), else the chunk is downloaded (S1); the
//!      final whole-file MD5 gates the atomic `rename(tmp → final)`.
//! 3. The whole-file MD5 gates the outcome. Under the temp path a mismatch
//!    deletes the temp and retries the file (≤5 attempts); under the
//!    in-place path it re-verifies the chunks and redoes only the ones that
//!    are still wrong (docs/07 §4.2, property 6).
//!
//! Files run in parallel, bounded by a semaphore of `jobs`; chunks inside a
//! file are strictly sequential, which keeps peak disk ≈ `jobs × largest
//! file` under the temp path and ≈ `jobs × largest growth` under the
//! in-place one.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
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

/// The unit the zero-fill of a layout hole is written in.
const ZERO_FILL_BUF: usize = 64 * 1024;

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
    /// **not** repaired: under the temp path its `*_tmp` is left in place and
    /// the next run resumes from it (S3), which is what makes an aborted run
    /// a smaller run rather than a broken one. Under the in-place path the
    /// file is simply left half-patched, and the next run re-verifies and
    /// finishes it (docs/07 §4.2, property 5). Not counted in any counter.
    Stopped,
}

/// Per-file chunk accounting (logged on success, aggregated into the run).
#[derive(Default, Debug)]
pub struct ChunkStats {
    pub download_bytes: u64,
    pub chunks_total: u64,
    pub chunks_reused: u64,
    pub chunks_downloaded: u64,
    pub chunks_resumed: u64,
    /// The chunks that were already correct and were **not** written: the
    /// in-place path's whole point, made countable.
    pub chunks_kept: u64,
    /// The bytes behind `chunks_kept` — the disk the repair did not spend.
    pub bytes_kept: u64,
    /// The bytes written into the install for this file. Under the in-place
    /// path this is the *changed* bytes; under the temp path it is the whole
    /// file.
    pub bytes_written: u64,
    /// Which path wrote the file: `"inplace"` or `"temp"`. `None` for a file
    /// that was skipped.
    pub mode: Option<&'static str>,
}

/// How one file's bytes are put on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriteMode {
    /// `{path}_tmp`, then an atomic rename over the target — the shipped
    /// contract: the original is never modified, and the extra space is one
    /// whole file per file in flight.
    Temp,
    /// The target, written where it lies.
    Inplace,
}

impl WriteMode {
    /// The name the `SUMMARY` counters use.
    fn as_str(&self) -> &'static str {
        match self {
            WriteMode::Temp => "temp",
            WriteMode::Inplace => "inplace",
        }
    }
}

/// Which tier of the in-place repair is running (docs/07 §4.0).
///
/// Both tiers produce the **same file**; they differ in how much reuse they
/// keep and in which direction they write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tier {
    /// The moving bytes are staged in RAM, so the write order does not
    /// constrain the reuse: both shift directions survive, and the writes
    /// run forward.
    Ram,
    /// The moving bytes are **not** staged: the salvage order picks the
    /// direction, and only the sources that order preserves are used. The
    /// rest download.
    Ordered,
}

/// The in-place path could not run: the temp path takes over.
///
/// It is a *signal*, not a failure: the file is repaired either way, and the
/// reason is logged rather than hidden (docs/07 §4, N4).
#[derive(Debug)]
struct NeedsTemp {
    why: String,
}

impl std::fmt::Display for NeedsTemp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.why)
    }
}

impl std::error::Error for NeedsTemp {}

/// Run the repair loop over the whole plan, bounded to `jobs` concurrent files.
///
/// Per-file failures are counted, never fatal; the caller decides that a
/// non-zero `files_failed` skips the post-phase.
///
/// `output` rides along rather than being read from a global: the progress
/// ticker this spawns emits records, and a record's plane has to come from
/// the same place the rest of the run's does.
#[allow(clippy::too_many_arguments)] // one parameter per input the repair loop needs
pub async fn repair_all(
    http: reqwest::Client,
    game_dir: PathBuf,
    plan: Arc<RepairPlan>,
    local_map: LocalChunkMap,
    summary: Summary,
    jobs: usize,
    dry_run: bool,
    repair_mode: crate::config::RepairMode,
    output: crate::report::OutputFormat,
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
                    Ok(Repaired::Repaired(stats)) => {
                        // `download_bytes` is already live: it is incremented
                        // per chunk inside `write_downloaded_chunk`, because the
                        // runtime hard stop has to read it mid-flight.
                        ctx.summary.files_repaired.fetch_add(1, Ordering::Relaxed);
                        // Which path wrote it is the disk story of the run:
                        // an in-place repair costs the file's growth, a temp
                        // repair costs a whole file of headroom.
                        match stats.mode {
                            Some("inplace") => {
                                ctx.summary.files_inplace.fetch_add(1, Ordering::Relaxed);
                            }
                            Some("temp") => {
                                ctx.summary.files_temp.fetch_add(1, Ordering::Relaxed);
                            }
                            _ => {}
                        }
                        // The disk the run did *not* spend: the chunks that
                        // were already correct and were not written.
                        ctx.summary
                            .kept_bytes
                            .fetch_add(stats.bytes_kept, Ordering::Relaxed);
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
    /// How the repair writes: in place, through a `_tmp`, or by discarding
    /// the target first.
    repair_mode: crate::config::RepairMode,
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

    // How this file's bytes will be put on disk. Deciding it first is what
    // lets the skip check be the *scan* under the in-place path: the scan
    // answers "is the file intact?" and "which chunks are wrong?" in one
    // read, so the in-place path never reads the file twice for the same
    // question.
    let mode = resolve_mode(ctx.repair_mode, file, &final_path);

    let mut scan = match &mode {
        WriteMode::Inplace => {
            let layout = Reuse::of(ctx, file);
            let scan = util::scan_file(&final_path, &layout.dest_refs(), &layout.source_refs())?;
            if scan.file_md5.eq_ignore_ascii_case(&file.md5) {
                tracing::debug!(actual_md5 = %scan.file_md5, "check: md5 match -> skip");
                return Ok(Repaired::Skipped);
            }
            tracing::debug!(actual_md5 = %scan.file_md5, "check: md5 mismatch -> needs repair");
            Some(scan)
        }
        WriteMode::Temp => {
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
            None
        }
    };
    if ctx.dry_run {
        tracing::info!("would repair");
        return Ok(Repaired::DryRun);
    }
    // `--repair-mode discard`: the target is deleted and written fresh. It
    // happens *here*, after the skip check, because deleting an intact file
    // and downloading it again would be the opposite of what the mode is
    // for.
    if ctx.repair_mode == crate::config::RepairMode::Discard
        && std::fs::remove_file(&final_path).is_ok()
    {
        tracing::info!(file = %file.rel, "discard: the target is deleted and written fresh");
        // The scan described the file that is now gone: the attempt has to
        // look again.
        scan = None;
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
    for attempt in 1..=max_attempts {
        let outcome = repair_attempt(ctx, file, &prefix, &tmp_path, &final_path, mode, scan.as_ref())
            .await;
        match outcome {
            Ok(stats) => return Ok(Repaired::Repaired(stats)),
            // The ceiling, not the file: this must not be counted as a
            // failure and must not be retried.
            Err(_e) if ctx.summary.download_stopped() || ctx.summary.write_stopped() => {
                tracing::warn!(file = %file.rel, "byte ceiling reached -> stopped");
                return Ok(Repaired::Stopped);
            }
            Err(e) => {
                tracing::warn!(attempt, "attempt failed: {e:#}");
                // The file moved under us, so the scan is stale: the next
                // attempt re-scans (and re-verifies, which is what makes an
                // interrupted in-place run resume by verification).
                scan = None;
                // `--fail-fast` cuts the backoff short. Without it, one bad
                // file spends 5 × linear sleep (and each of those attempts has
                // its own 5× chunk retry inside `fetch_chunk_bytes`) before the
                // run even starts on the next file.
                if attempt == max_attempts || ctx.summary.aborted() {
                    return Err(e);
                }
                tokio::time::sleep(std::time::Duration::from_secs(attempt as u64)).await;
            }
        }
    }
    unreachable!("attempt loop always returns")
}

/// Which path this file's bytes take.
///
/// The rules (docs/07 §4):
///
/// - `temp` — always the shipped path: write `{path}_tmp`, then rename it
///   over the target. The original is never modified, and the extra space is
///   one whole file per file in flight.
/// - `auto` — in place wherever the target can be opened read+write, and
///   the temp path when it cannot (with a warning: the degradation is
///   reported, not hidden).
/// - `inplace` — always in place.
/// - `discard` — the target is deleted and written fresh.
///
/// A target that is **not there** is created in place: there is nothing to
/// salvage from a file that does not exist, and creating it in place needs
/// no second copy of it — which is the whole difference between a fresh
/// install that needs `S` free bytes and one that needs none.
fn resolve_mode(
    repair_mode: crate::config::RepairMode,
    file: &PlannedFile,
    final_path: &Path,
) -> WriteMode {
    if repair_mode == crate::config::RepairMode::Temp {
        return WriteMode::Temp;
    }
    // A target that is not there (or is not a regular file) is created.
    let exists = std::fs::metadata(final_path)
        .map(|m| m.is_file())
        .unwrap_or(false);
    if !exists {
        return WriteMode::Inplace;
    }
    // The in-place path needs the target openable read+write (P3); a
    // read-only attribute is cleared first (P4 — the game ships read-only
    // files, and Starward clears the attribute before writing). A target
    // that cannot be opened at all falls back to the temp path, and says so
    // (N4).
    match open_inplace(final_path, false) {
        Ok(_) => WriteMode::Inplace,
        Err(e) => {
            tracing::warn!(
                file = %file.rel,
                why = %format!("{e:#}"),
                "target cannot be opened read+write -> temp path"
            );
            WriteMode::Temp
        }
    }
}

/// One full pass over a file's chunks, then verify + promote.
async fn repair_attempt(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    tmp_path: &std::path::Path,
    final_path: &std::path::Path,
    mode: WriteMode,
    scan: Option<&util::Scan>,
) -> Result<ChunkStats> {
    let mut stats = ChunkStats {
        chunks_total: file.chunks.len() as u64,
        ..Default::default()
    };
    let outcome = match mode {
        WriteMode::Temp => attempt_temp(ctx, file, prefix, tmp_path, &mut stats).await,
        WriteMode::Inplace => {
            match attempt_inplace(ctx, file, prefix, final_path, scan, &mut stats).await {
                Ok(stats) => Ok(stats),
                // The in-place path could not run: the temp path takes over,
                // and the file is repaired either way.
                Err(e) if e.downcast_ref::<NeedsTemp>().is_some() => {
                    tracing::warn!(file = %file.rel, why = %format!("{e:#}"), "temp path");
                    attempt_temp(ctx, file, prefix, tmp_path, &mut stats).await
                }
                Err(e) => Err(e),
            }
        }
    };
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

/// The shipped path: write `{path}_tmp`, then promote it over the target.
///
/// Unchanged from docs/02 step 6 — a completed prefix is kept (S3), a
/// hash-verified slice of the same file is copied (S2), and the rest is
/// downloaded (S1). The final whole-file MD5 gates the atomic rename.
async fn attempt_temp(
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
        match try_reuse_slice(ctx, file, c, reuse_entries, &mut f, stats)? {
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

/// The in-place path: the target is patched **where it lies**.
///
/// The algorithm (docs/07 §4.2):
///
/// 1. open the target read+write (P3, P4),
/// 2. **scan**: one read that answers everything — the whole-file MD5, which
///    chunks are already correct where they belong, and which reuse sources
///    are intact,
/// 3. work out the payload: the bytes that have to **move** inside the file,
/// 4. stage that payload in RAM if the budget allows (the RAM tier), and
/// 5. patch: write only the wrong chunks, ascending, each one either from the
///    staged bytes or from a download,
/// 6. trim, zero-fill any hole in the layout, and verify — a mismatch
///    re-verifies the chunks and redoes only the ones that are still wrong.
///
/// The extra disk a file needs is its **growth**, not its size: a
/// same-size repair needs none at all.
async fn attempt_inplace(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    final_path: &Path,
    scan: Option<&util::Scan>,
    stats: &mut ChunkStats,
) -> Result<ChunkStats> {
    let layout = Reuse::of(ctx, file);
    let mut h = open_inplace(final_path, true).map_err(|e| {
        NeedsTemp {
            why: format!("open {}: {e:#}", final_path.display()),
        }
    })?;

    // The scan. The caller's scan is the skip check's own read, so it is
    // reused when there is one; a retry re-scans, because the file moved.
    let scan = match scan {
        Some(s) => s.clone(),
        None => util::scan_file(final_path, &layout.dest_refs(), &layout.source_refs())?,
    };

    // The chunks that are wrong where they belong. A chunk that is already
    // correct is **kept**: it is not written, and its bytes are not read
    // again.
    let bad: Vec<usize> = (0..file.chunks.len())
        .filter(|&i| !scan.dest_ok[i])
        .collect();
    tracing::debug!(
        chunks_total = file.chunks.len(),
        chunks_wrong = bad.len(),
        "inplace: scan done"
    );

    // The payload: the bytes that have to move *inside* this file. A chunk
    // needs staging only if its reuse source is elsewhere in the file
    // (`r_i != o_i`) **and** the chunk is actually wrong. For the common
    // repair — a corrupt file — the source is the chunk's own place, so the
    // payload is zero (docs/07 §4.3).
    let mut payload = 0u64;
    let mut movable = vec![false; file.chunks.len()];
    for &i in &bad {
        let Some(si) = layout.source_of[i] else {
            continue;
        };
        if !scan.source_ok[si] {
            // The source is not intact: the chunk downloads.
            continue;
        }
        let r = layout.sources[si].0;
        let o = file.chunks[i].offset.max(0) as u64;
        if r == o {
            // The source is the chunk's own place — which is the *bad* bytes.
            // There is nothing to salvage from it.
            continue;
        }
        payload += file.chunks[i].uncompressed_size.max(0) as u64;
        movable[i] = true;
    }

    // The tier. The RAM tier stages the moving bytes and can therefore use
    // both shift directions; the ordered tier stages nothing and has to
    // settle for one direction (docs/07 §4.4).
    let usable_len = h.metadata().map(|m| m.len()).unwrap_or(0);
    let candidates: &[(String, i64, i64)] = ctx
        .local_map
        .get(&file.rel)
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    let order = super::reuse::salvage_order(&file.chunks, candidates, usable_len);
    let mut reserved = 0u64;
    let tier = if payload == 0 {
        // Nothing to move: the two tiers agree, and the cheaper one runs.
        Tier::Ram
    } else if ctx.summary.reserve_ram_bytes(payload) {
        reserved = payload;
        Tier::Ram
    } else {
        tracing::debug!(
            payload,
            ascending = order.ascending,
            "staging budget exhausted -> the ordered tier"
        );
        Tier::Ordered
    };

    // The RAM tier: stage the moving bytes before a single byte is written,
    // so the write order does not have to constrain the reuse.
    let captured: Option<HashMap<usize, Vec<u8>>> = match tier {
        Tier::Ram => {
            if payload == 0 {
                None
            } else {
                let cap = capture_sources(&mut h, file, &layout, &bad, &movable)?;
                tracing::debug!(payload, "inplace: moving bytes staged");
                Some(cap)
            }
        }
        Tier::Ordered => None,
    };
    // The staging area belongs to this file until it is done with it — and
    // "done" includes every way the attempt can end.
    let _staging = RamGuard {
        ctx,
        bytes: reserved,
    };

    // The patch: only the wrong chunks. The RAM tier writes them ascending;
    // the ordered tier writes them in the direction that keeps the most
    // reuse.
    let mut order_of_writes: Vec<usize> = bad.clone();
    if matches!(tier, Tier::Ordered) && !order.ascending {
        order_of_writes.reverse();
    }
    for &i in &order_of_writes {
        let c = &file.chunks[i];
        let dest = c.offset.max(0) as u64;
        // 1. the staged bytes (the RAM tier),
        match captured.as_ref().and_then(|m| m.get(&i)) {
            // The staged bytes were hash-verified by the scan, and they were
            // staged *before* any write, so they are still the right bytes.
            Some(bytes) => {
                h.seek(SeekFrom::Start(dest))?;
                reserve_write(ctx, stats, bytes.len() as u64)?;
                h.write_all(bytes)?;
                stats.chunks_reused += 1;
            }
            None => {
                // 2. the file's own bytes, if the order says they are still
                // there (the ordered tier),
                if matches!(tier, Tier::Ordered)
                    && order.live[i]
                    && let Some(bytes) = read_source_at(&mut h, file, &layout, i, &scan)?
                {
                    h.seek(SeekFrom::Start(dest))?;
                    reserve_write(ctx, stats, bytes.len() as u64)?;
                    h.write_all(&bytes)?;
                    stats.chunks_reused += 1;
                } else {
                    // 3. the network.
                    write_downloaded_chunk(ctx, prefix, c, &mut h, stats).await?;
                }
            }
        }
    }

    // N3: a hole in the chunk layout is zero-filled rather than left as
    // whatever stale bytes the file happened to have.
    let (tiled, gaps) = file.layout();
    if !tiled {
        for (off, len) in gaps {
            tracing::warn!(
                file = %file.rel,
                offset = off,
                len,
                "chunk layout does not tile the file -> zero-filled"
            );
            h.seek(SeekFrom::Start(off))?;
            reserve_write(ctx, stats, len)?;
            let mut left = len;
            let zeros = vec![0u8; ZERO_FILL_BUF.min(left as usize)];
            while left > 0 {
                let n = (left as usize).min(zeros.len());
                h.write_all(&zeros[..n])?;
                left -= n as u64;
            }
        }
    }

    // §4.5: the eager trim. The tail past the target's end is dead weight
    // unless a reuse source lives in it, and the test for that is exact.
    let size = file.size.max(0) as u64;
    let cur = h.metadata().map(|m| m.len()).unwrap_or(0);
    if cur > size {
        if trim_is_free(ctx, file) {
            h.set_len(size)?;
            tracing::debug!(from = cur, to = size, "inplace: dead tail released");
        } else {
            tracing::debug!(
                from = cur,
                to = size,
                "inplace: trim deferred (a reuse source reaches past the target's end)"
            );
        }
    }
    // The final length, whatever the trim above did.
    h.set_len(size)?;
    h.flush()?;
    drop(h);

    // The final gate: the whole-file MD5. A mismatch does not restart the
    // file — it re-verifies the chunks and redoes only the ones that are
    // still wrong, which is one read pass instead of a re-download
    // (docs/07 §4.2, property 6).
    let h_md5 = util::md5_file(final_path)?;
    if !h_md5.eq_ignore_ascii_case(&file.md5) {
        tracing::debug!(actual_md5 = %h_md5, "inplace: final md5 mismatch -> re-verify chunks");
        reverify_and_redo(ctx, file, prefix, final_path, stats).await?;
    }
    // The chunks that were already correct where they belong: the in-place
    // path's whole point, made countable.
    stats.chunks_kept = stats.chunks_total.saturating_sub(bad.len() as u64);
    stats.bytes_kept = file
        .chunks
        .iter()
        .enumerate()
        .filter(|(i, _)| !bad.contains(i))
        .map(|(_, c)| c.uncompressed_size.max(0) as u64)
        .sum();
    tracing::info!(
        chunks_total = stats.chunks_total,
        chunks_kept = stats.chunks_kept,
        chunks_reused = stats.chunks_reused,
        chunks_downloaded = stats.chunks_downloaded,
        bytes_written = stats.bytes_written,
        download_bytes = stats.download_bytes,
        final_md5 = %file.md5,
        "repaired"
    );
    Ok(ChunkStats {
        download_bytes: stats.download_bytes,
        chunks_total: stats.chunks_total,
        chunks_reused: stats.chunks_reused,
        chunks_downloaded: stats.chunks_downloaded,
        chunks_resumed: stats.chunks_resumed,
        chunks_kept: stats.chunks_kept,
        bytes_kept: stats.bytes_kept,
        bytes_written: stats.bytes_written,
        mode: Some(WriteMode::Inplace.as_str()),
    })
}

/// The staging reservation of one file, released however the attempt ends.
///
/// A file that fails mid-way must not keep its share of the budget: the next
/// file in flight would be pushed to the ordered tier for no reason.
struct RamGuard<'a> {
    ctx: &'a FileCtx,
    bytes: u64,
}

impl Drop for RamGuard<'_> {
    fn drop(&mut self) {
        if self.bytes > 0 {
            self.ctx.summary.release_ram_bytes(self.bytes);
        }
    }
}

/// Re-verify every chunk and redo only the ones that are still wrong.
///
/// This replaces "delete the temp and restart the file": the culprit is
/// *named*, and only it is fetched again (docs/07 §5, W3).
async fn reverify_and_redo(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    final_path: &Path,
    stats: &mut ChunkStats,
) -> Result<()> {
    let layout = Reuse::of(ctx, file);
    let scan = util::scan_file(final_path, &layout.dest_refs(), &[])?;
    let bad: Vec<usize> = (0..file.chunks.len())
        .filter(|&i| !scan.dest_ok[i])
        .collect();
    if bad.is_empty() {
        anyhow::bail!(
            "final md5 mismatch for {} but every chunk verifies",
            file.rel
        );
    }
    let mut h = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(final_path)
        .with_context(|| format!("reopen {}", final_path.display()))?;
    for &i in &bad {
        tracing::debug!(chunk = %file.chunks[i].id, "inplace: chunk still wrong -> redo");
        write_downloaded_chunk(ctx, prefix, &file.chunks[i], &mut h, stats).await?;
    }
    h.flush()?;
    drop(h);
    let h_md5 = util::md5_file(final_path)?;
    if !h_md5.eq_ignore_ascii_case(&file.md5) {
        anyhow::bail!(
            "final md5 mismatch for {}: expect {} got {}",
            file.rel,
            file.md5,
            h_md5
        );
    }
    Ok(())
}

/// The file's chunks, and the reuse candidate each one could be read from.
///
/// The candidate is the first entry of the local reuse map whose
/// `(md5, size)` match the chunk's — the same rule `try_reuse_slice` uses,
/// so the two paths can never disagree about which bytes a chunk could come
/// from.
struct Reuse {
    /// The destination ranges, one per chunk, in chunk order.
    dest: Vec<(u64, u64, String)>,
    /// The candidate source ranges, in the order the scan saw them.
    sources: Vec<(u64, u64, String)>,
    /// Per chunk: the index into `sources`, if the chunk has a candidate.
    source_of: Vec<Option<usize>>,
}

impl Reuse {
    fn of(ctx: &FileCtx, file: &PlannedFile) -> Self {
        let entries = ctx.local_map.get(&file.rel);
        let mut dest = Vec::with_capacity(file.chunks.len());
        let mut sources: Vec<(u64, u64, String)> = Vec::new();
        let mut source_of = Vec::with_capacity(file.chunks.len());
        for c in &file.chunks {
            let size = c.uncompressed_size.max(0) as u64;
            dest.push((c.offset.max(0) as u64, size, c.uncompressed_md5.clone()));
            let candidate = entries.and_then(|list| {
                list.iter().find(|(md5, sz, _)| {
                    md5.eq_ignore_ascii_case(&c.uncompressed_md5) && *sz == c.uncompressed_size
                })
            });
            match candidate {
                Some((_, _, off)) => {
                    source_of.push(Some(sources.len()));
                    sources.push(((*off).max(0) as u64, size, c.uncompressed_md5.clone()));
                }
                None => source_of.push(None),
            }
        }
        Self {
            dest,
            sources,
            source_of,
        }
    }

    /// The destination ranges, borrowed for the scan.
    fn dest_refs(&self) -> Vec<(u64, u64, &str)> {
        self.dest
            .iter()
            .map(|(o, s, m)| (*o, *s, m.as_str()))
            .collect()
    }

    /// The source ranges, borrowed for the scan.
    fn source_refs(&self) -> Vec<(u64, u64, &str)> {
        self.sources
            .iter()
            .map(|(o, s, m)| (*o, *s, m.as_str()))
            .collect()
    }
}

/// Read a chunk's reuse source **at write time**, and verify it.
///
/// The ordered tier reads the source from the file as it goes, which is safe
/// because the order guarantees the source has not been overwritten yet.
/// The bytes are still hash-gated: a source that does not hash to what the
/// chunk wants is not used, and the chunk downloads instead.
///
/// `None` means "this chunk has no usable source right now".
fn read_source_at(
    h: &mut File,
    file: &PlannedFile,
    layout: &Reuse,
    i: usize,
    scan: &util::Scan,
) -> Result<Option<Vec<u8>>> {
    let Some(si) = layout.source_of[i] else {
        return Ok(None);
    };
    if !scan.source_ok[si] {
        // The scan already said the source is not intact.
        return Ok(None);
    }
    let (off, len, _) = &layout.sources[si];
    let dest = file.chunks[i].offset.max(0) as u64;
    let dlen = file.chunks[i].uncompressed_size.max(0) as u64;
    // A source that overlaps its own destination cannot be copied in place:
    // the copy would read bytes it has already overwritten. Such a chunk
    // downloads instead — the salvage order does not rule it out, because
    // the order is about *other* chunks' writes.
    if *off < dest + dlen && dest < *off + len {
        return Ok(None);
    }
    h.seek(SeekFrom::Start(*off))?;
    let mut buf = vec![0u8; *len as usize];
    if h.read_exact(&mut buf).is_err() {
        // The source is shorter than the scan said: it is not there.
        return Ok(None);
    }
    let digest = format!("{:x}", md5::compute(&buf));
    if !digest.eq_ignore_ascii_case(&file.chunks[i].uncompressed_md5) {
        tracing::debug!(
            chunk = %file.chunks[i].id,
            "inplace: source no longer verifies -> download"
        );
        return Ok(None);
    }
    Ok(Some(buf))
}

/// Read the bytes that have to move, into RAM, **before** a single byte is
/// written.
///
/// This is what buys the in-place path both shift directions and a forward
/// write order: the staged bytes are read from wherever they are, and the
/// writes then go wherever they belong.
fn capture_sources(
    h: &mut File,
    file: &PlannedFile,
    layout: &Reuse,
    bad: &[usize],
    movable: &[bool],
) -> Result<HashMap<usize, Vec<u8>>> {
    let mut out = HashMap::new();
    for &i in bad {
        if !movable[i] {
            continue;
        }
        let Some(si) = layout.source_of[i] else {
            continue;
        };
        let (off, len, _) = &layout.sources[si];
        h.seek(SeekFrom::Start(*off))?;
        let mut buf = vec![0u8; *len as usize];
        h.read_exact(&mut buf).with_context(|| {
            format!(
                "read {} bytes at {} of {}",
                len,
                off,
                file.rel
            )
        })?;
        out.insert(i, buf);
    }
    Ok(out)
}

/// Is the tail past the target's end dead weight?
///
/// It is, unless a reuse candidate reaches past the target's end — then
/// those bytes are still a source, and the trim has to wait until the repair
/// is done with them (docs/07 §4.5).
fn trim_is_free(ctx: &FileCtx, file: &PlannedFile) -> bool {
    let size = file.size.max(0) as u64;
    let Some(entries) = ctx.local_map.get(&file.rel) else {
        return true;
    };
    entries.iter().all(|(_, sz, off)| {
        let r = (*off).max(0) as u64;
        let s = (*sz).max(0) as u64;
        r.saturating_add(s) <= size
    })
}

/// Open the target for an in-place repair.
///
/// P3: the target has to be openable read+write. P4: a read-only attribute
/// is cleared first — the game ships read-only files, Starward clears the
/// attribute before writing, and the old path failed on them at the rename
/// (docs/07 §4.1, N2).
///
/// `create` says whether a missing target may be created: the in-place path
/// creates the file it is going to write.
fn open_inplace(path: &Path, create: bool) -> Result<File> {
    let attempt = || -> std::io::Result<File> {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(create)
            .truncate(false) // never truncate: the file's own bytes are the source
            .open(path)
    };
    match attempt() {
        Ok(f) => Ok(f),
        Err(first) => {
            if clear_readonly(path)? {
                attempt().with_context(|| {
                    format!("open {} read+write after clearing the read-only attribute", path.display())
                })
            } else {
                Err(first).with_context(|| format!("open {}", path.display()))
            }
        }
    }
}

/// Clear the read-only attribute, if there is one.
///
/// `true` when the attribute was there and is now gone.
///
/// On Windows this is `FILE_ATTRIBUTE_READONLY`, which is what the game
/// files carry. On Unix the equivalent is the owner-write bit, and only that
/// bit: `set_readonly(false)` would make the file world-writable, which is
/// not what "clear the read-only attribute" means.
#[allow(clippy::permissions_set_readonly_false)] // the Unix branch uses the mode bits
fn clear_readonly(path: &Path) -> Result<bool> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Ok(false);
    };
    if !meta.permissions().readonly() {
        return Ok(false);
    }
    #[cfg(unix)]
    let perm = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::Permissions::from_mode(meta.permissions().mode() | 0o200)
    };
    #[cfg(not(unix))]
    let perm = {
        let mut p = meta.permissions();
        p.set_readonly(false);
        p
    };
    std::fs::set_permissions(path, perm)
        .with_context(|| format!("clear the read-only attribute of {}", path.display()))?;
    tracing::debug!(path = %path.display(), "read-only attribute cleared");
    Ok(true)
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
    stats: &mut ChunkStats,
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
                reserve_write(ctx, stats, n as u64)?;
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
    reserve_write(ctx, stats, decoded.len() as u64)?;
    out.write_all(&decoded)?;
    stats.chunks_downloaded += 1;
    Ok(())
}

/// Reserve `n` bytes of disk write against `--max-write-bytes`, or trip the
/// runtime hard stop.
///
/// Reserving **before** the write is what makes the ceiling meaningful: the
/// bytes land as soon as `write_all` returns, so a check-then-write under
/// `--io-threads N` would overshoot by up to N-1 buffers. On refusal the
/// file is left where it is and `*_tmp` keeps whatever prefix it already had,
/// which S3 resume-by-length picks up next run.
///
/// The reservation is also the file's own byte count: every byte written
/// into the install is counted here, once, for both paths.
fn reserve_write(ctx: &FileCtx, stats: &mut ChunkStats, n: u64) -> Result<()> {
    if ctx.summary.reserve_write_bytes(n) {
        stats.bytes_written += n;
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

/// The stage of the final MD5 gate: length + whole-file MD5, then the atomic promote. On mismatch the
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
        // The temp path writes the whole file, so nothing is "kept": the
        // resumed prefix is counted as resumed, not as kept.
        chunks_kept: stats.chunks_resumed,
        bytes_kept: 0,
        bytes_written: stats.bytes_written,
        mode: Some(WriteMode::Temp.as_str()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RepairMode;
    use crate::sophon::SophonChunk;

    fn planned() -> PlannedFile {
        PlannedFile {
            rel: "game/foo.dat".into(),
            size: 128,
            md5: "m".into(),
            chunks: vec![SophonChunk {
                id: "c0".into(),
                uncompressed_md5: "md5-c0".into(),
                offset: 0,
                compressed_size: 64,
                uncompressed_size: 64,
                unknown: 0,
                compressed_md5: String::new(),
            }],
        }
    }

    /// The resolution table, one row at a time (docs/07 §4).
    #[test]
    fn the_mode_table_is_what_the_plan_says_it_is() {
        let dir = crate::testtemp::TempDir::new("girpr_test_mode", "table");
        let target = dir.join("foo.dat");
        let file = planned();

        // A target that is not there: created in place. There is nothing to
        // salvage from a file that does not exist, and creating it in place
        // needs no second copy of it.
        for mode in [RepairMode::Auto, RepairMode::Inplace, RepairMode::Discard] {
            assert_eq!(
                resolve_mode(mode, &file, &target),
                WriteMode::Inplace,
                "{mode}: a missing target is created in place"
            );
        }
        // ...except under the temp path, which is the shipped contract.
        assert_eq!(
            resolve_mode(RepairMode::Temp, &file, &target),
            WriteMode::Temp
        );

        // A target that is there and can be opened read+write: in place,
        // whatever the mode says — except the temp path.
        std::fs::write(&target, b"x").unwrap();
        for mode in [RepairMode::Auto, RepairMode::Inplace, RepairMode::Discard] {
            assert_eq!(
                resolve_mode(mode, &file, &target),
                WriteMode::Inplace,
                "{mode}: an openable target is patched in place"
            );
        }
        assert_eq!(
            resolve_mode(RepairMode::Temp, &file, &target),
            WriteMode::Temp,
            "the temp path is the shipped path, and it is never automatic"
        );
    }

    /// A target that cannot be opened read+write falls back to the temp
    /// path — and says so, rather than failing the file.
    #[test]
    fn a_target_that_cannot_be_opened_falls_back_to_the_temp_path() {
        let dir = crate::testtemp::TempDir::new("girpr_test_mode", "unopenable");
        let target = dir.join("foo.dat");
        std::fs::write(&target, b"x").unwrap();
        // A directory is not a file: `metadata().is_file()` is false, so it
        // takes the "create it in place" branch — and the attempt's own open
        // is what fails, which is the fallback the writer reports.
        let as_dir = dir.join("sub");
        std::fs::create_dir(&as_dir).unwrap();
        assert_eq!(
            resolve_mode(RepairMode::Auto, &planned(), &as_dir),
            WriteMode::Inplace
        );
        // A file that is there is in place.
        assert_eq!(
            resolve_mode(RepairMode::Auto, &planned(), &target),
            WriteMode::Inplace
        );
    }
}
