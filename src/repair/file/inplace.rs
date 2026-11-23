//! The in-place repair path: scan → stage → patch → verify.
//!
//! Moved byte-for-byte from `file.rs`. Imports from `super`:
//! `ChunkStats`, `FileCtx`, `FileWrites`, `Inplace`, `PlannedFile`,
//! `write_at`, `copy_within`, `zero_fill`, plus `super::mode::{open_inplace,
//! open_rw}` and `super::temp::write_downloaded_chunk`. Crate paths for
//! `RepairMode`, the chunk-verify leaf's verdict/remap/verify names, `reuse`,
//! `SophonChunk`, `util`, and the repair-plan leaf's `PlannedFile`, `RepairPlan`
//! and `FileId`.

use std::io::Write;

use futures_util::StreamExt;

use super::mode::{open_inplace, open_rw};
use super::temp::{DownloadedChunk, write_downloaded_chunk};
use super::{
    ChunkStats, FileCtx, FileWrites, Inplace, PlannedFile, copy_within, reusable_verdict, write_at,
    zero_fill,
};
use crate::repair::diagnostics;
use crate::repair::reuse;
use crate::sophon::SophonChunk;
use crate::util::{self, Md5};
use anyhow::{Context, Result};
use girpr_chunk_verify::{
    Verified, scan_ranges, source_ok_by_chunk, source_ok_by_range, verify_in_place,
};
use girpr_repair_plan::FileId;
use girpr_repair_policy::RepairMode;

/// How many downloaded chunks one file may fetch and decode at once.
///
/// The bound is per in-flight chunk, not per staged byte: each window task
/// streams straight into its own file handle at the chunk's offset, so no
/// decoded chunk is held in RAM and the extra memory is one decoder's worth
/// per task. Ordering-sensitive layouts (an overlap or the salvage tier) and
/// the temp path never use this window — they keep the single-handle,
/// single-chunk write order their correctness depends on.
const CHUNK_FETCH_WINDOW: usize = 4;

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
///    which reuse sources are intact. Under `--scan` the plan phase already
///    read the file; when a length and modification-time check proves it has
///    not changed, that verdict is reused instead of reading it again;
/// 3. **stage** — the bytes of the chunks that have to *move* inside the
///    file are read into RAM, so the write order cannot destroy them;
/// 4. **patch** — the wrong chunks are written, ascending: from the staged
///    bytes when there are any, from a download otherwise;
/// 5. **verify** — one read is the gate: the whole-file MD5 decides. Only a
///    mismatch takes a second read, which holds the per-chunk verdicts that
///    name what to redo.
///
/// A target that cannot be opened for writing answers [`Inplace::Unavailable`]
/// and the caller writes that file the other way — through a `_tmp`, which
/// does not need write access to the target.
pub(super) async fn attempt_inplace(
    ctx: &FileCtx,
    id: FileId,
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
    attempt_inplace_inner(ctx, id, file, prefix, final_path, &mut stats, writes).await
}

/// The body of the in-place repair — see [`attempt_inplace`].
async fn attempt_inplace_inner(
    ctx: &FileCtx,
    id: FileId,
    file: &PlannedFile,
    prefix: &str,
    final_path: &std::path::Path,
    stats: &mut ChunkStats,
    writes: &mut FileWrites,
) -> Result<Inplace> {
    // The reuse candidates: where each chunk's bytes could be copied from.
    let sources = reuse::sources_for(&file.chunks, ctx.local_map.get(&file.rel));

    // The scan's two lists, both parallel to the chunks: where each chunk
    // *belongs*, and where its bytes could be copied from. Only chunks with a
    // candidate get a source range (see `girpr_chunk_verify::scan_ranges`); `source_owner`
    // maps those verdicts back to the chunk they belong to.
    let (dest_ranges, source_ranges, source_owner) = scan_ranges(&file.chunks, &sources);

    // 1 + 2. open, scan (or reuse the plan scan), the skip check, dry-run.
    let (mut handle, mut scan) = match open_and_scan(
        ctx,
        id,
        file,
        final_path,
        &dest_ranges,
        &source_ranges,
        &source_owner,
    )? {
        OpenScan::Finished(outcome) => return Ok(outcome),
        OpenScan::Ready { handle, scan } => (handle, scan),
    };

    // `--repair-mode discard`: the target is deleted and written from scratch.
    (handle, scan) = discard_if_requested(
        ctx,
        final_path,
        handle,
        scan,
        dest_ranges.len(),
        source_ranges.len(),
    )?;

    // The chunks that are not correct where they belong, and the stats that
    // describe them.
    let bad = plan_bad_chunks(file, &mut scan, &source_owner, stats);

    // The eager/deferred trim decision.
    trim_dead_tail(file, &sources, &handle, final_path)?;

    // 3. the tier, and the staging that goes with it.
    let (tier, payload, _reservation) = choose_tier(ctx, file, &sources, &scan.source_ok, &bad);
    let captured = stage_sources(
        file,
        &sources,
        &scan.source_ok,
        &bad,
        tier,
        payload,
        &mut handle,
    )?;

    // 4. patch, including the hole fill and the final length.
    patch_chunks(
        ctx,
        writes,
        &mut handle,
        Patch {
            file,
            prefix,
            final_path,
            sources: &sources,
            source_ok: &scan.source_ok,
            bad: &bad,
            captured: &captured,
            tier,
        },
        stats,
    )
    .await?;

    // 5. verify, redo what is still wrong, verify again.
    verify_and_retry(ctx, writes, &mut handle, &dest_ranges, file, prefix, stats).await
}

/// What [`open_and_scan`] ended with.
enum OpenScan {
    /// The attempt is over before any patch: skipped, dry-run, or unavailable.
    Finished(Inplace),
    /// The target is open and scanned; the patch can proceed.
    Ready {
        handle: std::fs::File,
        scan: util::Scan,
    },
}

/// Steps 1 and 2 of the in-place repair: open the target and scan it.
///
/// The scan is one read that answers everything: the whole-file MD5 (the skip
/// check), which chunks are already correct where they belong, and which reuse
/// sources are intact. Under `--scan` the plan phase already took that read;
/// [`reusable_verdict`] decides whether it can be reused.
fn open_and_scan(
    ctx: &FileCtx,
    id: FileId,
    file: &PlannedFile,
    final_path: &std::path::Path,
    dest_ranges: &[util::ScanRange<'_>],
    source_ranges: &[util::ScanRange<'_>],
    source_owner: &[usize],
) -> Result<OpenScan> {
    // 1. open. A dry run changes nothing: no file is created and no
    // attribute is cleared, so the target is only read.
    let mut handle = match open_inplace(ctx, final_path) {
        Ok(Some(f)) => f,
        Ok(None) => {
            // A dry run on a file that is not there: nothing is correct, so
            // the file would be repaired.
            tracing::info!("would repair");
            return Ok(OpenScan::Finished(Inplace::DryRun));
        }
        Err(e) => {
            // The target cannot be opened for writing. That is not a failed
            // file: the temp path does not need write access to the target,
            // so the repair still happens — through the other path.
            tracing::debug!(error = %format!("{e:#}"), "in-place open failed");
            return Ok(OpenScan::Finished(Inplace::Unavailable(
                "the target cannot be opened for writing",
            )));
        }
    };

    // 2. scan. One read: the whole-file MD5, which chunks are already
    // correct, and which sources are intact. Under `--scan` the plan phase
    // already took that read; reuse it only when the guard proves the file is
    // still the one that was scanned.
    let scan = match reusable_verdict(ctx, id, file, handle.metadata()) {
        Some(v) => {
            // How long ago the plan scan ran, not how old the file's mtime is:
            // the verdict records the latter for the guard, and the two are
            // unrelated.
            let scan_age_secs = ctx
                .scan
                .as_deref()
                .and_then(|report| report.scanned_at)
                .and_then(|started| started.elapsed().ok())
                .map(|age| age.as_secs());
            tracing::debug!(
                file = %file.rel,
                scan_age_secs = ?scan_age_secs,
                "reusing the plan scan in the in-place writer"
            );
            diagnostics::record_plan_scan_reused();
            util::Scan {
                file_md5: v.file_md5,
                file_len: v.file_len,
                dest_ok: v.dest_ok.clone(),
                source_ok: source_ok_by_range(&v.source_ok, source_owner),
            }
        }
        None => util::scan_reader(&mut handle, dest_ranges, source_ranges)
            .with_context(|| format!("scan {}", file.rel))?,
    };
    tracing::debug!(
        actual_md5 = %scan.file_md5,
        expect_md5 = %file.md5,
        on_disk = scan.file_len,
        "scanned"
    );
    if scan.file_md5.matches_hex(&file.md5) {
        tracing::debug!(actual_md5 = %scan.file_md5, "check: md5 match -> skip");
        return Ok(OpenScan::Finished(Inplace::Skipped));
    }
    if ctx.dry_run {
        tracing::info!("would repair");
        return Ok(OpenScan::Finished(Inplace::DryRun));
    }
    Ok(OpenScan::Ready { handle, scan })
}

/// `--repair-mode discard`: delete the target and write it from scratch.
///
/// Zero peak extra disk, and a full download for the file — the trade the mode
/// exists to offer. The delete happens *after* the skip check, so an intact
/// file is never deleted. Every other mode returns the open handle and its scan
/// unchanged.
fn discard_if_requested(
    ctx: &FileCtx,
    final_path: &std::path::Path,
    handle: std::fs::File,
    scan: util::Scan,
    dest_len: usize,
    source_len: usize,
) -> Result<(std::fs::File, util::Scan)> {
    if ctx.repair_mode != RepairMode::Discard {
        return Ok((handle, scan));
    }
    drop(handle);
    discard_target(final_path)?;
    let handle = open_rw(final_path).with_context(|| format!("create {}", final_path.display()))?;
    // The file is gone: every chunk has to be written, and there is nothing
    // left to reuse from. `absent` is per range, not per chunk, so it is
    // remapped below like every other scan.
    let scan = util::Scan::absent(dest_len, source_len);
    Ok((handle, scan))
}

/// The chunks that are not correct where they belong, plus the kept-chunk
/// accounting the repaired log reports.
fn plan_bad_chunks(
    file: &PlannedFile,
    scan: &mut util::Scan,
    source_owner: &[usize],
    stats: &mut ChunkStats,
) -> Vec<usize> {
    // Back to one source verdict per chunk, in the plan's order: a chunk with
    // no source has no intact source, by definition. Everything below indexes
    // `source_ok` by chunk, and the discard scan is per range like the rest.
    // The scan answers one verdict per source range, so the two lists are the
    // same length here.
    debug_assert_eq!(
        source_owner.len(),
        scan.source_ok.len(),
        "the scan answers one source verdict per source range"
    );
    scan.source_ok = source_ok_by_chunk(file.chunks.len(), source_owner, &scan.source_ok);

    // The chunks that are not correct where they belong. `file.chunks` is
    // sorted by offset (the plan guarantees it), so this list is ascending
    // too — which is the order the patch loop writes in.
    let bad: Vec<usize> = (0..file.chunks.len())
        .filter(|&i| !scan.dest_ok[i])
        .collect();
    stats.chunks_kept = (file.chunks.len() - bad.len()) as u64;
    tracing::debug!(bad = bad.len(), kept = stats.chunks_kept, "in-place plan");
    bad
}

/// The eager trim: when no reuse candidate reaches past the target's end, the
/// tail is dead weight and can be released before a single byte is written.
///
/// **The other half is logged too.** The trim's outcome is not in the output
/// contract — the file ends at the target size either way, and the difference
/// is the *peak* disk usage, which nothing reports. The log is the only place
/// the outcome is visible, so both branches say which one they took.
fn trim_dead_tail(
    file: &PlannedFile,
    sources: &[Option<u64>],
    handle: &std::fs::File,
    final_path: &std::path::Path,
) -> Result<()> {
    let have = handle_len(handle)
        .with_context(|| format!("stat {} before the trim", final_path.display()))?;
    if have > file.size.max(0) as u64 {
        if trim_is_free(file, &file.chunks, sources) {
            tracing::debug!(before = have, after = file.size, "trimming the dead tail");
            diagnostics::record_dead_tail_trimmed();
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
            diagnostics::record_trim_deferred();
        }
    }
    Ok(())
}

/// Step 3: pick the write tier and reserve the staged payload.
///
/// `Staged` reads the bytes that have to move into RAM, so **every** source
/// survives and the writes run forward. `Ordered` reads each source from the
/// file at the moment it is written, and the write order is the one that keeps
/// the most of them alive.
///
/// The budget picks the tier; it never refuses the file. A file that cannot get
/// its reservation is written in the ordered direction, which produces the same
/// file with less reuse.
fn choose_tier<'a>(
    ctx: &'a FileCtx,
    file: &PlannedFile,
    sources: &[Option<u64>],
    source_ok: &[bool],
    bad: &[usize],
) -> (Tier, u64, Option<RamReservation<'a>>) {
    let payload = staged_payload(&file.chunks, sources, source_ok, bad);
    let (tier, reservation) = match ctx.summary.max_ram_bytes() {
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
                diagnostics::record_ram_shortage();
                (Tier::Ordered, None)
            }
        }
    };
    (tier, payload, reservation)
}

/// Step 3 continued: read every staged source into RAM, hash-gated.
///
/// Every byte written is hash-gated before it lands, and a staged source is a
/// write's source: gate it here too.
fn stage_sources(
    file: &PlannedFile,
    sources: &[Option<u64>],
    source_ok: &[bool],
    bad: &[usize],
    tier: Tier,
    payload: u64,
    handle: &mut std::fs::File,
) -> Result<Vec<Option<Vec<u8>>>> {
    let mut captured: Vec<Option<Vec<u8>>> = vec![None; file.chunks.len()];
    if !matches!(tier, Tier::Staged) || payload == 0 {
        return Ok(captured);
    }
    for &i in bad {
        if !source_ok[i] {
            continue;
        }
        let Some(off) = sources[i] else { continue };
        let c = &file.chunks[i];
        let bytes = util::read_range(handle, off, c.uncompressed_size.max(0) as u64)
            .with_context(|| format!("capture chunk {} of {}", c.id, file.rel))?;
        let h = util::md5::md5_digest(&bytes);
        if !h.matches_hex(&c.uncompressed_md5) {
            tracing::debug!(chunk = %c.id, "staged bytes do not match -> download");
            continue;
        }
        captured[i] = Some(bytes);
    }
    Ok(captured)
}

/// The plan-side inputs to [`patch_chunks`]: what is wrong, where its bytes
/// come from, and the write tier the patch runs in.
struct Patch<'a> {
    file: &'a PlannedFile,
    prefix: &'a str,
    final_path: &'a std::path::Path,
    sources: &'a [Option<u64>],
    source_ok: &'a [bool],
    bad: &'a [usize],
    captured: &'a [Option<Vec<u8>>],
    tier: Tier,
}

/// Step 4: write the wrong chunks, fill the holes, and end at the target size.
async fn patch_chunks(
    ctx: &FileCtx,
    writes: &mut FileWrites,
    handle: &mut std::fs::File,
    patch: Patch<'_>,
    stats: &mut ChunkStats,
) -> Result<()> {
    let Patch {
        file,
        prefix,
        final_path,
        sources,
        source_ok,
        bad,
        captured,
        tier,
    } = patch;
    // The layout is the disjointness proof. With no overlap, each chunk owns a
    // byte range no other chunk ever writes, so its download can be fetched,
    // decoded and written independently of the others. An overlap or the
    // ordered tier keeps the strict sequential order: a source the order
    // keeps alive can live inside another chunk's range.
    let layout = file.layout();
    let disjoint_downloads = tier == Tier::Staged && layout.overlaps.is_empty();

    // The order (ordered tier only): the direction that keeps the most
    // sources alive, and which sources those are. Only the sources the scan
    // found intact can be salvaged — the rest have to be downloaded.
    let order = match tier {
        Tier::Staged => None,
        Tier::Ordered => {
            let chunks: Vec<&SophonChunk> = bad.iter().map(|&i| &file.chunks[i]).collect();
            let srcs: Vec<Option<u64>> = bad
                .iter()
                .map(|&i| if source_ok[i] { sources[i] } else { None })
                .collect();
            let usable = handle_len(handle)
                .with_context(|| format!("stat {} for the salvage order", final_path.display()))?;
            let o = reuse::salvage_order_refs(&chunks, &srcs, usable);
            tracing::info!(
                file = %file.rel,
                ascending = o.ascending,
                salvaged = o.live_count(),
                of = chunks.len(),
                "ordered tier"
            );
            diagnostics::record_order(o.ascending, o.live_count() as u64, chunks.len() as u64);
            Some(o)
        }
    };

    // For a disjoint staged layout, fetch+decode the download chunks in a
    // bounded window before the write loop. Each task streams into its own
    // handle at the chunk's offset; results are folded in chunk order so the
    // per-file accounting and the first error are deterministic even though
    // the network work overlaps. Every task is drained, so a reservation made
    // by an in-flight chunk is settled even when an earlier one fails.
    if disjoint_downloads {
        let idxs: Vec<usize> = bad
            .iter()
            .copied()
            .filter(|&i| captured[i].is_none())
            .collect();
        if !idxs.is_empty() {
            window_download_chunks(ctx, file, prefix, final_path, &idxs, stats, writes).await?;
        }
    }

    // Ascending in the staged tier (forward writes), in the salvage order in
    // the ordered tier. One scratch buffer serves the whole patch: the chunks
    // written on this handle are written one at a time, so the copy pieces
    // reuse it.
    let mut scratch = Vec::new();
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
            write_at(ctx, writes, handle, dst, bytes)?;
            stats.chunks_reused += 1;
        } else if disjoint_downloads {
            // Already fetched, decoded, written and accounted for by the
            // bounded window.
            continue;
        } else if let Some(o) = &order
            && o.live[k]
            && let Some(src) = sources[i]
        {
            // The source survives the order: copy it from inside the file.
            copy_within(
                ctx,
                writes,
                handle,
                &mut scratch,
                src,
                dst,
                c.uncompressed_size.max(0) as u64,
            )?;
            stats.chunks_reused += 1;
        } else {
            write_downloaded_chunk(DownloadedChunk {
                ctx,
                prefix,
                c,
                out: &mut *handle,
                stats: &mut *stats,
                writes: &mut *writes,
            })
            .await?;
        }
    }

    // A hole in the chunk coverage would keep the old file's stale bytes,
    // so it is zero-filled. The fill happens **after** the writes: in the
    // ordered tier a hole can still hold a source that a later chunk needs.
    for (offset, len) in &layout.gaps {
        zero_fill(ctx, writes, handle, *offset, *len)?;
    }

    // The file must end at its manifest size: shorter means the tail is
    // missing, longer means it is dead weight.
    handle.set_len(file.size.max(0) as u64)?;
    handle.flush()?;
    Ok(())
}

/// Fetch+decode+write the download chunks of a disjoint staged layout in a
/// bounded window.
///
/// Each task opens its own read+write handle on the target and drives the
/// existing [`write_downloaded_chunk`] path into it. The layout has already
/// proved the chunks' ranges are disjoint, so the concurrent writes cannot
/// reach the same bytes; the per-chunk reservation and digest checks are the
/// same ones the sequential path uses, and the shared meter is atomic, so
/// settlement is exact regardless of completion order.
///
/// Results are collected in input order and folded only after every task has
/// finished. That keeps the recorded figures and the reported error
/// deterministic, and it drains tasks whose reservation was already charged
/// when a sibling failed — dropping them mid-flight would leak the charge
/// against the download ceiling.
async fn window_download_chunks(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    final_path: &std::path::Path,
    idxs: &[usize],
    stats: &mut ChunkStats,
    writes: &mut FileWrites,
) -> Result<()> {
    let futures = idxs.iter().copied().map(|i| {
        let c = &file.chunks[i];
        async move {
            // A fresh handle has its own cursor, so this task's seek and
            // streamed writes cannot move another task's cursor. The file was
            // opened (and possibly created) before the patch began.
            let mut out = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(final_path)
                .with_context(|| format!("open {} for chunk {}", final_path.display(), c.id))?;
            // Per-task accumulators: the shared `stats`/`writes` are borrowed
            // mutably by the caller and cannot be captured by a window task.
            // Their settled bytes are folded below once all tasks are done.
            let mut local_stats = ChunkStats::default();
            let mut local_writes = FileWrites::default();
            write_downloaded_chunk(DownloadedChunk {
                ctx,
                prefix,
                c,
                out: &mut out,
                stats: &mut local_stats,
                writes: &mut local_writes,
            })
            .await?;
            Ok::<_, anyhow::Error>((local_stats, local_writes))
        }
    });

    let results = bounded_window(CHUNK_FETCH_WINDOW, futures).await;
    let mut first_error = None;
    for result in results {
        match result {
            Ok((chunk_stats, chunk_writes)) => {
                stats.download_bytes += chunk_stats.download_bytes;
                stats.chunks_downloaded += chunk_stats.chunks_downloaded;
                writes.settled += chunk_writes.settled;
            }
            Err(e) if first_error.is_none() => first_error = Some(e),
            Err(_) => {}
        }
    }
    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Drive futures `window` at a time and return their outputs in input order.
///
/// `buffered` keeps at most `window` futures in flight while preserving the
/// order of results, and `collect` runs every future to completion. Both
/// matter here: bounded overlap without losing deterministic accounting, and
/// no in-flight future dropped with an unsettled reservation.
async fn bounded_window<F, T, E>(
    window: usize,
    futures: impl IntoIterator<Item = F>,
) -> Vec<Result<T, E>>
where
    F: std::future::Future<Output = Result<T, E>>,
{
    futures_util::stream::iter(futures)
        .buffered(window.max(1))
        .collect()
        .await
}

/// Step 5: verify, redo only the still-wrong chunks, then verify again.
async fn verify_and_retry(
    ctx: &FileCtx,
    writes: &mut FileWrites,
    handle: &mut std::fs::File,
    dest_ranges: &[util::ScanRange<'_>],
    file: &PlannedFile,
    prefix: &str,
    stats: &mut ChunkStats,
) -> Result<Inplace> {
    // One pass is the gate: the whole-file digest. The per-chunk verdicts are
    // only needed when it fails, so a repaired file does not hash every chunk
    // range as well as the file.
    let still_bad = match verify_in_place(
        dest_ranges,
        |ranges| util::scan_reader(handle, ranges, &[]),
        file.chunks.len(),
        &file.md5,
    )? {
        Verified::Matches(md5) => return Ok(repaired_in_place(stats, md5)),
        Verified::Mismatch { md5, still_bad } => {
            if still_bad.is_empty() {
                return Err(FinalDigestMismatch(format!(
                    "final md5 mismatch for {}: expect {} got {} (the bytes outside the chunks are wrong)",
                    file.rel, file.md5, md5
                ))
                .into());
            }
            still_bad
        }
    };

    // Redo only the chunks the verify pass found still wrong. A chunk that
    // fails here passed its own MD5, so the damage is elsewhere: the range
    // verdicts name it instead of re-fetching the whole file.
    tracing::warn!(
        file = %file.rel,
        chunks = still_bad.len(),
        "final md5 mismatch -> redoing the mismatching chunks"
    );
    for &i in &still_bad {
        let c = &file.chunks[i];
        write_downloaded_chunk(DownloadedChunk {
            ctx,
            prefix,
            c,
            out: &mut *handle,
            stats: &mut *stats,
            writes: &mut *writes,
        })
        .await?;
    }
    handle.set_len(file.size.max(0) as u64)?;
    handle.flush()?;
    // The digest and, when the layout has an overlap, the verdicts the
    // terminal decision needs are the same read: the scan hashes the whole
    // file as it sweeps the ranges.
    let (scan, checked) = read_after_redo(file, dest_ranges, |ranges| {
        util::scan_reader(handle, ranges, &[])
    })?;
    let final_md5 = scan.file_md5;
    if !Md5::parse_hex(&file.md5).is_ok_and(|expected| expected == final_md5) {
        let message = format!(
            "final md5 mismatch for {}: expect {} got {}",
            file.rel, file.md5, final_md5
        );
        if redo_mismatch_is_terminal(file, &scan, checked) {
            return Err(FinalDigestMismatch(message).into());
        }
        return Err(anyhow::anyhow!(message));
    }
    Ok(repaired_in_place(stats, final_md5))
}

/// The one read the post-redo tail takes: the whole-file digest and, when the
/// layout has an overlap, the per-chunk verdicts the terminal decision needs.
///
/// [`util::scan_reader`] hashes the whole file as it sweeps, so its digest is
/// the digest a bare whole-file pass would return. An overlap needs the range
/// verdicts anyway, so asking the scan for them costs no extra read; asking
/// for a digest first and scanning after the comparison failed would read the
/// file twice. The trade is the range digests on an overlap whose redo did
/// settle the digest. A plan with no overlap is proved by the layout alone, so
/// it is given no ranges: the same single pass still answers the digest, just
/// without building per-range digests.
///
/// The second value is how many verdicts the terminal decision reads: the
/// number of ranges the pass was given. A scan that answers a different
/// number is missing or surplus answers, not correct ones — see
/// [`redo_mismatch_is_terminal`].
fn read_after_redo(
    file: &PlannedFile,
    dest_ranges: &[util::ScanRange<'_>],
    read: impl FnOnce(&[util::ScanRange<'_>]) -> Result<util::Scan>,
) -> Result<(util::Scan, usize)> {
    let ranges = overlapping_ranges(file, dest_ranges);
    let checked = ranges.len();
    Ok((read(&ranges)?, checked))
}

/// The chunk ranges the layout says overlap, in the order the scan was given
/// them.
///
/// The layout names the two offsets of every overlap — the chunk that claims
/// the bytes and the chunk that claims them again — so the ranges whose offset
/// it names are the contested ones. Every chunk that claims bytes another
/// chunk claims is named: the layout records an overlap whenever a chunk
/// starts before the coverage ends, and it names both sides of that overlap.
///
/// The chunks it leaves out are the ones **no other chunk claims**. Nothing
/// but their own write moves their bytes, so the redo cannot change their
/// verdict: they are still the chunks the verify pass found, and the redo
/// wrote them correctly when they were wrong. Their verdict is therefore
/// already known, and asking the scan for it again only hashes their bytes a
/// second time.
fn overlapping_ranges<'a>(
    file: &PlannedFile,
    dest_ranges: &'a [util::ScanRange<'a>],
) -> Vec<util::ScanRange<'a>> {
    let layout = file.layout();
    if layout.overlaps.is_empty() {
        // No overlap: the layout is the whole proof, and there is no verdict
        // to ask for.
        return Vec::new();
    }
    let mut contested: Vec<u64> = layout.overlaps.iter().flat_map(|&(a, b)| [a, b]).collect();
    contested.sort_unstable();
    contested.dedup();
    dest_ranges
        .iter()
        .copied()
        .filter(|(offset, _, _)| contested.binary_search(offset).is_ok())
        .collect()
}

/// Can another attempt change the bytes a post-redo mismatch was computed
/// over?
///
/// The redo wrote every chunk the second scan named. A layout with no overlap
/// is the whole proof: the only remaining difference is the zero-filled gaps
/// and the declared length, so a second attempt would write no chunk and
/// repeat the digest. That answer is the layout's, not the caller's: a scan
/// built from an empty range list for an overlapping layout is missing
/// answers, not evidence that nothing can change.
///
/// An overlap can undo a neighbour when the redo rewrites a chunk, so a
/// non-empty overlap list is not proof by itself. Two chunks that claim the
/// same bytes can be genuinely different — at most one of them can match — or
/// identical duplicates, where both match and no write can change either.
/// The layout cannot tell the two apart, because it only knows offsets and
/// lengths; only the bytes can. The scan therefore carries one verdict per
/// overlapping range, and the mismatch is final exactly when every one of
/// them now matches.
///
/// `expected` is how many verdicts the decision reads: the ranges
/// [`read_after_redo`] asked the scan for. A verdict list of another length is
/// not the answer that was asked for: a shorter one is missing verdicts, and a
/// longer one means the scan answered ranges the caller did not ask about, so
/// which verdict belongs to which range is unknown. Both decide nothing and
/// the mismatch keeps its retries — the same defensive shape
/// [`verify_in_place`] uses for the chunks it has to redo.
fn redo_mismatch_is_terminal(file: &PlannedFile, scan: &util::Scan, expected: usize) -> bool {
    if file.layout().overlaps.is_empty() {
        return true;
    }
    expected > 0 && scan.dest_ok.len() == expected && scan.dest_ok.iter().all(|&ok| ok)
}

/// A whole-file digest mismatch that no retry can change.
///
/// The in-place writer assembles the target from bytes it has already checked
/// one chunk at a time: a staged source's MD5, an in-file source range the
/// scan found intact, or a downloaded chunk that passed its own gate. The
/// bytes outside those ranges are the plan's zero-filled gaps and declared
/// length. A whole-file mismatch therefore means the **plan** is inconsistent
/// with itself — bytes no chunk describes, or a declared digest no assembly
/// of them can produce — not that a download or a write went wrong.
///
/// The marker is only used when another attempt cannot change the assembled
/// bytes: either every chunk range already matched, or [`verify_and_retry`]'s
/// redo was followed by a scan that found every range it checked correct (a
/// layout with no overlap asks for no ranges and is proved by that layout).
/// An overlap whose ranges cannot all match keeps the ordinary error, because
/// rewriting one chunk can make its neighbour wrong again and the next attempt
/// may assemble different bytes.
///
/// A **transient** download or write error is still an ordinary `Err` and is
/// retried; this marker is only for the verify gate's deterministic verdict.
/// The temp path's digest gate keeps its retries: it resumes by length, so a
/// corrupt `_tmp` prefix is a cause a retry can remove.
#[derive(Debug)]
pub(super) struct FinalDigestMismatch(String);

impl std::fmt::Display for FinalDigestMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FinalDigestMismatch {}

/// The success settle shared by the first verify and the redo.
///
/// Both paths end the same way: one `repaired in place` line carrying the
/// file's chunk accounting, then the stats the caller promotes. Keeping it in
/// one place stops the two paths' log blocks from drifting apart.
fn repaired_in_place(stats: &ChunkStats, md5: Md5) -> Inplace {
    tracing::info!(
        chunks_total = stats.chunks_total,
        chunks_kept = stats.chunks_kept,
        chunks_reused = stats.chunks_reused,
        chunks_downloaded = stats.chunks_downloaded,
        download_bytes = stats.download_bytes,
        final_md5 = %md5,
        "repaired in place"
    );
    diagnostics::record_chunk_stats(stats);
    Inplace::Repaired(stats.clone())
}

/// What one `File::metadata()` call said about the in-place target.
///
/// Only the length matters at the two sites that stat the handle: before the
/// eager trim and for the salvage order. Keeping the error as its own state
/// lets the failure arm be executed with an injected `io::Error`
/// (`post::classify_deprecated`'s shape) instead of only being read, without a
/// trait or an injection seam. [`handle_len`] turns it back into the exact
/// `io::Result` from `handle.metadata()`, so production still propagates the
/// error with the site's path context and fails the file (exit `3`).
#[derive(Debug)]
enum InplaceLen {
    /// The handle is open; this is its current length.
    Present(u64),
    /// The handle exists but could not be stat'd.
    Failed(std::io::Error),
}

/// Classify one in-place `metadata` result.
///
/// Split from [`handle_len`] so the error arm is unit-testable without a
/// permission fixture, matching `util::classify_len` and
/// `post::classify_deprecated`.
fn classify_len(stat: std::io::Result<std::fs::Metadata>) -> InplaceLen {
    match stat {
        Ok(m) => InplaceLen::Present(m.len()),
        Err(e) => InplaceLen::Failed(e),
    }
}

/// The handle's current length, with the stat error preserved.
///
/// This is the same `io::Result` the two sites got from `handle.metadata()?`,
/// so the `?` path — and the failed-file outcome behind it — is unchanged.
fn handle_len(handle: &std::fs::File) -> std::io::Result<u64> {
    match classify_len(handle.metadata()) {
        InplaceLen::Present(len) => Ok(len),
        InplaceLen::Failed(e) => Err(e),
    }
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
pub(super) fn trim_is_free(
    file: &PlannedFile,
    chunks: &[SophonChunk],
    sources: &[Option<u64>],
) -> bool {
    let size = file.size.max(0) as u64;
    chunks.iter().zip(sources).all(|(c, src)| {
        let Some(r) = *src else { return true };
        let s = c.uncompressed_size.max(0) as u64;
        // An overflowing end is unrepresentable and therefore certainly past
        // any real target: treat it as outside, never as inside.
        r.checked_add(s).is_some_and(|end| end <= size)
    })
}

/// The bytes the staged tier would have to hold in RAM: the sum of the
/// server-declared uncompressed sizes of the bad chunks that still have an
/// intact salvage source.
///
/// `uncompressed_size` is server-controlled and only clamped at zero, not
/// bounded. Three chunks at `i64::MAX` do not fit a `u64`: a plain `.sum()`
/// wraps to `0` in release — the "stage nothing" sentinel with no RAM
/// reservation at all — and panics in debug. Saturate like every other
/// server-size estimate, so a corrupt manifest fails the reservation and
/// falls back to the ordered tier instead of bypassing the budget.
fn staged_payload(
    chunks: &[SophonChunk],
    sources: &[Option<u64>],
    source_ok: &[bool],
    bad: &[usize],
) -> u64 {
    bad.iter()
        .filter(|&&i| sources[i].is_some() && source_ok[i])
        .fold(0u64, |total, &i| {
            total.saturating_add(chunks[i].uncompressed_size.max(0) as u64)
        })
}

/// Delete the target for `--repair-mode discard`.
///
/// `NotFound` is success: the file is already gone, which is what the mode
/// wanted. Every other error is fatal: a refused delete (read-only
/// attribute, ACL, sharing violation) must not become a later
/// `create <path>: access denied` or silently keep old bytes while every
/// chunk is treated as missing.
fn discard_target(path: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("discard {}", path.display())),
    }
}

/// A RAM reservation that gives itself back when it is dropped.
struct RamReservation<'a> {
    summary: &'a crate::report::Summary,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use girpr_chunk_verify::Verdict;

    use super::super::{COARSE_MTIME_TICK, mtime_predates_scan};
    use super::*;
    use crate::repair::scan::ScanReport;
    use crate::report::Summary;
    use crate::retry::BackoffPolicy;
    use crate::test_support::stamp_ago;
    use girpr_repair_plan::LocalChunkMap;
    use girpr_repair_plan::RepairPlan;

    /// R-4: a failed delete in discard mode must be reported as the delete,
    /// not ignored and then blamed on the re-open. A directory at the target
    /// is the portable way to make `remove_file` fail while the path exists.
    #[test]
    fn a_refused_discard_names_the_delete() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_inplace", "discard_refused");
        let target = dir.join("foo.dat");
        std::fs::create_dir_all(&target).unwrap();
        let err = discard_target(&target).expect_err("a directory cannot be removed as a file");
        let chain = format!("{err:#}");
        assert!(
            chain.contains("discard") && chain.contains(&target.display().to_string()),
            "the delete is the failing step and must be named: {chain}"
        );
        assert!(target.exists(), "a refused delete leaves the path alone");
    }

    /// A target that disappeared between the open and the delete still gives
    /// discard mode what it asked for.
    #[test]
    fn an_already_absent_discard_target_is_not_an_error() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_inplace", "discard_absent");
        discard_target(&dir.join("gone.dat")).expect("absent is the desired state");
    }

    /// P-2: the two in-place stat sites used a bare `handle.metadata()?`, so
    /// the failure arm could only be read. The classifier keeps the error
    /// itself, and `handle_len` hands it back to the caller's `?`, which still
    /// fails the file (exit `3`) with the site's path context.
    #[test]
    fn a_failed_inplace_stat_preserves_the_error() {
        let denied = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        match classify_len(Err(denied)) {
            InplaceLen::Failed(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
                assert_eq!(e.to_string(), "denied");
            }
            other => panic!("an ACL failure is a stat failure, not a length: {other:?}"),
        }
    }

    /// The length half of the same classifier: the handle's metadata is what
    /// the trim and the salvage order consume.
    #[test]
    fn an_inplace_stat_reports_the_handle_length() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_inplace", "stat_len");
        let path = dir.join("foo.dat");
        std::fs::write(&path, b"12345").unwrap();
        let handle = std::fs::File::open(&path).unwrap();
        assert_eq!(handle_len(&handle).unwrap(), 5);
    }

    fn chunk(id: &str, size: i64) -> SophonChunk {
        chunk_at(id, &format!("md5-{id}"), 0, size)
    }

    fn chunk_at(id: &str, md5: &str, offset: i64, size: i64) -> SophonChunk {
        SophonChunk {
            id: id.into(),
            uncompressed_md5: md5.into(),
            offset,
            compressed_size: size,
            uncompressed_size: size,
            unknown_field6: 0,
            compressed_md5: String::new(),
        }
    }

    /// S-2: the payload is the one server-size sum that did not saturate. Three
    /// chunks at `i64::MAX` overflow a `u64` (`2*MAX = u64::MAX - 1`), so the
    /// old `.sum()` panicked here in debug and wrapped to `0` in release —
    /// selecting the staged tier with no RAM reservation.
    #[test]
    fn an_overflowing_staged_payload_saturates() {
        let chunks = vec![
            chunk("a", i64::MAX),
            chunk("b", i64::MAX),
            chunk("c", i64::MAX),
        ];
        let sources = vec![Some(0), Some(0), Some(0)];
        let ok = vec![true, true, true];
        let bad = vec![0, 1, 2];
        assert_eq!(staged_payload(&chunks, &sources, &ok, &bad), u64::MAX);
    }

    /// The exact boundary: `2 * i64::MAX + 1 == u64::MAX`, and one more byte
    /// saturates rather than wrapping or panicking.
    #[test]
    fn the_staged_payload_boundary_saturates() {
        let mut chunks = vec![chunk("a", i64::MAX), chunk("b", i64::MAX)];
        let mut sources = vec![Some(0), Some(0)];
        let mut ok = vec![true, true];

        chunks.push(chunk("c", 1));
        sources.push(Some(0));
        ok.push(true);
        let bad = vec![0, 1, 2];
        assert_eq!(
            staged_payload(&chunks, &sources, &ok, &bad),
            u64::MAX,
            "the sum reaches the u64 ceiling exactly"
        );

        chunks.push(chunk("d", 1));
        sources.push(Some(0));
        ok.push(true);
        let bad = vec![0, 1, 2, 3];
        assert_eq!(
            staged_payload(&chunks, &sources, &ok, &bad),
            u64::MAX,
            "anything past the ceiling saturates"
        );
    }

    /// A chunk without a salvage source is downloaded, so it is not staged and
    /// must not be in the reservation.
    #[test]
    fn the_staged_payload_counts_only_salvageable_chunks() {
        let chunks = vec![chunk("a", 10), chunk("b", 20), chunk("c", 40)];
        let sources = vec![Some(0), None, Some(0)];
        let ok = vec![true, true, false];
        let bad = vec![0, 1, 2];
        assert_eq!(staged_payload(&chunks, &sources, &ok, &bad), 10);
    }

    /// The tier decision, as a table: the RAM limit and the payload together
    /// pick the tier, and "stage nothing" is a mode (`Some(0)`), not a
    /// shortage.
    ///
    /// The payload here is 10 for `bad = [0, 1]` (only chunk 0 has an intact
    /// source) and 0 for `bad = [1]` (chunk 1 has no source at all).
    #[test]
    fn the_tier_decision_table_is_pinned() {
        fn tier_for(limit: Option<u64>, bad: &[usize]) -> (Tier, u64, bool) {
            let mut ctx = test_ctx(None);
            Arc::get_mut(&mut ctx.summary)
                .expect("the test context owns its summary")
                .set_ram_limit(limit);
            let chunks = vec![chunk("a", 10), chunk("b", 20)];
            let sources = vec![Some(0), None];
            let source_ok = vec![true, true];
            let file = PlannedFile {
                rel: "f.dat".into(),
                size: 30,
                md5: "m".into(),
                url_prefix: Arc::from(""),
                chunks,
            };
            let (tier, payload, reservation) = choose_tier(&ctx, &file, &sources, &source_ok, bad);
            let reserved = reservation.is_some();
            drop(reservation);
            (tier, payload, reserved)
        }

        // Nothing to stage: the tier is staged, but there is nothing to reserve.
        assert_eq!(tier_for(None, &[1]), (Tier::Staged, 0, false));
        // `--max-ram-bytes 0` is a mode: ordered even when there is nothing to
        // stage, and regardless of the payload.
        assert_eq!(tier_for(Some(0), &[1]), (Tier::Ordered, 0, false));
        assert_eq!(tier_for(Some(0), &[0, 1]), (Tier::Ordered, 10, false));
        // Exactly at the limit reserves; one byte short falls back.
        assert_eq!(tier_for(Some(10), &[0, 1]), (Tier::Staged, 10, true));
        assert_eq!(tier_for(Some(9), &[0, 1]), (Tier::Ordered, 10, false));
        // Without a limit the reservation always succeeds.
        assert_eq!(tier_for(None, &[0, 1]), (Tier::Staged, 10, true));
    }

    /// The bad-chunk plan, as a table: the list is the chunks that are not
    /// correct where they belong, the kept count is the complement, and the
    /// per-range source verdicts are folded back onto chunks (chunk 1 has no
    /// source range, so it can never be reused).
    #[test]
    fn the_bad_chunk_plan_is_the_wrong_chunks_with_the_kept_count() {
        let chunks = vec![chunk("a", 10), chunk("b", 20), chunk("c", 30)];
        let file = PlannedFile {
            rel: "f.dat".into(),
            size: 60,
            md5: "m".into(),
            url_prefix: Arc::from(""),
            chunks,
        };
        let mut stats = ChunkStats {
            chunks_total: 3,
            ..Default::default()
        };
        let mut scan = util::Scan {
            file_md5: Md5::EMPTY,
            file_len: 60,
            dest_ok: vec![true, false, true],
            source_ok: vec![true, false],
        };
        let bad = plan_bad_chunks(&file, &mut scan, &[0, 2], &mut stats);
        assert_eq!(bad, vec![1], "only the wrong chunk is patched");
        assert_eq!(stats.chunks_kept, 2, "the correct chunks are kept");
        assert_eq!(
            scan.source_ok,
            vec![true, false, false],
            "range 0 belongs to chunk 0; chunk 2's range was not intact and \
             chunk 1 has no range"
        );
    }

    /// B3: the in-place scan must not invent a source at offset 0 for a chunk
    /// that has none. The same content sits at the head of the file under a
    /// different layout, so a coerced source range would verify there and the
    /// writer would believe it can copy the chunk instead of downloading it.
    #[test]
    fn an_inplace_scan_never_checks_a_source_less_chunk_at_offset_zero() {
        let x = vec![0x58u8; 10];
        let y = vec![0x59u8; 10];
        let md5_x = util::md5::md5_digest(&x).to_hex();
        let mut bytes = x.clone();
        bytes.extend_from_slice(&y);
        let dir = girpr_tempdir::TempDir::new("girpr_test_inplace", "no_source_offset");
        std::fs::write(dir.join("a.dat"), &bytes).unwrap();
        let chunks = vec![chunk_at("c0", &md5_x, 10, 10)];

        let (dest, source, owner) = scan_ranges(&chunks, &[None]);
        assert!(
            source.is_empty(),
            "a chunk with no source contributes no source range"
        );
        assert!(owner.is_empty());
        let scan = util::scan_file(&dir.join("a.dat"), &dest, &source).unwrap();
        assert_eq!(
            scan.dest_ok,
            vec![false],
            "the destination is the other copy"
        );
        assert_eq!(
            source_ok_by_chunk(chunks.len(), &owner, &scan.source_ok),
            vec![false],
            "the in-place path must not invent an intact source"
        );
    }

    /// B3: `--repair-mode discard` replaces a real scan with `Scan::absent`
    /// over the range lists `scan_ranges` built, so its `source_ok` is per
    /// **range**, not per chunk. The remap has to land each verdict on the
    /// chunk that owns the range, and a source-less chunk stays false.
    #[test]
    fn the_discard_scan_remaps_its_absent_source_verdicts_by_chunk() {
        let chunks = vec![
            chunk_at("c0", "md5-c0", 0, 10),
            chunk_at("c1", "md5-c1", 10, 20),
        ];
        let (dest, source, owner) = scan_ranges(&chunks, &[None, Some(30)]);
        assert_eq!(owner, vec![1], "only the second chunk has a source range");
        let scan = util::Scan::absent(dest.len(), source.len());
        assert_eq!(
            scan.source_ok.len(),
            1,
            "absent is per range, not per chunk"
        );
        assert_eq!(
            source_ok_by_chunk(chunks.len(), &owner, &scan.source_ok),
            vec![false, false],
            "the deleted target has no intact source, and chunk 0 has none anyway"
        );

        // The same remap must carry a verdict to the chunk that owns it, so a
        // later per-chunk leak cannot pass by simply keeping everything false.
        assert_eq!(
            source_ok_by_chunk(chunks.len(), &owner, &[true]),
            vec![false, true]
        );
    }

    /// A whole-file digest mismatch with every chunk range already correct is
    /// unretryable: the writer marks it so the file loop answers `Fatal`
    /// instead of running the same scan, the same zero-fill, and the same
    /// digest again.
    #[tokio::test]
    async fn a_digest_mismatch_outside_the_chunks_is_unretryable() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_inplace", "unretryable_digest");
        let path = dir.join("f.dat");
        std::fs::write(&path, b"").unwrap();
        let file = PlannedFile {
            rel: "f.dat".into(),
            size: 0,
            // Not the digest of the empty file: no byte string has this one.
            md5: "00000000000000000000000000000001".into(),
            url_prefix: Arc::from(""),
            chunks: Vec::new(),
        };
        let ctx = test_ctx(None);
        let mut handle = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let mut writes = FileWrites::default();
        let mut stats = ChunkStats::default();

        let err = verify_and_retry(&ctx, &mut writes, &mut handle, &[], &file, "", &mut stats)
            .await
            .err()
            .expect("the empty file cannot carry a non-empty digest");
        assert!(
            err.downcast_ref::<FinalDigestMismatch>().is_some(),
            "the file loop must see the deterministic mismatch as terminal: {err:#}"
        );
        let chain = format!("{err:#}");
        assert!(
            chain.contains("the bytes outside the chunks are wrong"),
            "{chain}"
        );
    }

    /// A post-redo mismatch is only final when the redo's writes cannot be
    /// undone by the next attempt. A gap between chunks does not matter — it
    /// is zero-filled and no chunk write touches it — so the layout alone
    /// settles a plan with no overlap, without asking for range verdicts.
    ///
    /// An overlap does need the bytes: two identical chunks at one offset both
    /// match after the redo, so no write a retry could make would change a
    /// byte, while two genuinely different chunks cannot both match and must
    /// keep their retries. The same pass that produces the digest carries
    /// those verdicts, so the overlap path reads the file once instead of
    /// digesting and then rescanning.
    #[test]
    fn a_redo_mismatch_settles_in_one_read_and_rescans_only_an_overlap() {
        let overlapping = PlannedFile {
            rel: "f.dat".into(),
            size: 2,
            md5: "00000000000000000000000000000001".into(),
            url_prefix: Arc::from(""),
            chunks: vec![
                chunk_at("a", "md5-a", 0, 2),
                chunk_at("b", "md5-b", 0, 2),
                chunk_at("c", "md5-c", 1, 1),
            ],
        };

        // No overlap: the layout is the proof, so the single pass is given no
        // ranges and its digest settles the file.
        let gapped = PlannedFile {
            rel: "f.dat".into(),
            size: 3,
            md5: "00000000000000000000000000000001".into(),
            url_prefix: Arc::from(""),
            chunks: vec![chunk_at("a", "md5-a", 0, 1), chunk_at("b", "md5-b", 2, 1)],
        };
        let gapped_dest = [(0u64, 1u64, "md5-a"), (2, 1, "md5-b")];
        let mut reads = 0;
        let (scan, checked) = read_after_redo(&gapped, &gapped_dest, |ranges| {
            reads += 1;
            assert!(
                ranges.is_empty(),
                "a plan with no overlap needs no per-range verdicts"
            );
            Ok(util::Scan {
                file_md5: Md5::EMPTY,
                file_len: 3,
                dest_ok: Vec::new(),
                source_ok: Vec::new(),
            })
        })
        .unwrap();
        assert_eq!(reads, 1, "the post-redo tail reads the file exactly once");
        assert_eq!(checked, 0, "no range was asked for, so no verdict is read");
        assert!(
            redo_mismatch_is_terminal(&gapped, &scan, checked),
            "a gap is zero-filled and does not make the digest changeable"
        );

        // Duplicate identical chunks: every range matches after the redo, so
        // the next attempt would write nothing and repeat the same bytes.
        let overlap_dest = [(0u64, 2u64, "md5-a"), (0, 2, "md5-b"), (1, 1, "md5-c")];
        let mut reads = 0;
        let (scan, checked) = read_after_redo(&overlapping, &overlap_dest, |ranges| {
            reads += 1;
            assert_eq!(
                ranges.len(),
                3,
                "an overlap needs a verdict for every overlapping range"
            );
            Ok(util::Scan {
                file_md5: Md5::EMPTY,
                file_len: 2,
                dest_ok: vec![true, true, true],
                source_ok: Vec::new(),
            })
        })
        .unwrap();
        assert_eq!(
            reads, 1,
            "the whole-file digest and the range verdicts are one pass, not a \
             digest followed by a rescan"
        );
        assert_eq!(checked, 3, "every overlapping range was asked for");
        assert!(
            redo_mismatch_is_terminal(&overlapping, &scan, checked),
            "identical duplicates cannot make a retry assemble different bytes"
        );
        // This is the branch that builds `FinalDigestMismatch`, so the file
        // loop settles the file here instead of spending another attempt on
        // the same bytes.

        // A genuine overlap: at least one range cannot match, so the next
        // attempt may assemble different bytes.
        let mut reads = 0;
        let (scan, checked) = read_after_redo(&overlapping, &overlap_dest, |_ranges| {
            reads += 1;
            Ok(util::Scan {
                file_md5: Md5::EMPTY,
                file_len: 2,
                dest_ok: vec![true, false, true],
                source_ok: Vec::new(),
            })
        })
        .unwrap();
        assert_eq!(reads, 1, "one pass decides either way");
        assert!(
            !redo_mismatch_is_terminal(&overlapping, &scan, checked),
            "an overlapping plan whose ranges cannot all match keeps its retries"
        );
    }

    /// The post-redo pass reads the whole file for the digest, but the
    /// verdicts it needs are only the ones the layout says overlap.
    ///
    /// A chunk no other chunk claims cannot change when the redo writes: the
    /// redo only moves the bytes of the chunk it rewrites, and a chunk that
    /// claims the same bytes as none of them keeps its verdict — it is still
    /// the chunk the verify pass found, and the redo wrote it correctly when
    /// it was wrong. Its verdict is known, so the pass does not hash it again.
    ///
    /// The chunks that *do* overlap are the ones a rewrite can undo, and they
    /// are the ones the terminal decision reads.
    #[test]
    fn the_post_redo_pass_asks_only_for_the_overlapping_ranges() {
        // `a` and `b` claim the same bytes; `c` claims its own, past a gap.
        let file = PlannedFile {
            rel: "f.dat".into(),
            size: 12,
            md5: "00000000000000000000000000000001".into(),
            url_prefix: Arc::from(""),
            chunks: vec![
                chunk_at("a", "md5-a", 0, 4),
                chunk_at("b", "md5-b", 2, 4),
                chunk_at("c", "md5-c", 8, 4),
            ],
        };
        let dest = [(0u64, 4u64, "md5-a"), (2, 4, "md5-b"), (8, 4, "md5-c")];

        let mut reads = 0;
        let (scan, checked) = read_after_redo(&file, &dest, |ranges| {
            reads += 1;
            assert_eq!(
                ranges.len(),
                2,
                "only the chunks that overlap need a verdict"
            );
            assert!(
                ranges.iter().all(|(offset, _, _)| *offset != 8),
                "the chunk no other chunk claims is not asked for"
            );
            Ok(util::Scan {
                file_md5: Md5::EMPTY,
                file_len: 12,
                dest_ok: vec![true, true],
                source_ok: Vec::new(),
            })
        })
        .unwrap();
        assert_eq!(reads, 1, "the narrowing costs no extra pass");
        assert_eq!(checked, 2, "the decision reads two verdicts");
        assert!(
            redo_mismatch_is_terminal(&file, &scan, checked),
            "the overlapping chunks match, so the mismatch is final"
        );

        // The chunk that claims its own bytes cannot be wrong after the redo,
        // so the decision is the same one the whole chunk list gave: the two
        // overlapping chunks match and the third one cannot have changed.
        assert_eq!(
            file.layout().overlaps,
            vec![(0, 2)],
            "the layout names the two offsets of the overlap"
        );
    }

    /// The narrowing is only sound while a chunk the layout does not name is
    /// one **no other chunk claims**: the redo moves the bytes of the chunk
    /// it rewrites and nothing else, so a chunk nobody else claims keeps the
    /// verdict the verify pass gave it.
    ///
    /// The layout names both sides of every overlap, so the ranges it leaves
    /// out are exactly the ones whose verdict cannot change — and therefore
    /// the ones the terminal decision does not have to read again.
    #[test]
    fn the_ranges_the_layout_leaves_out_are_claimed_by_no_other_chunk() {
        // `a` and `b` claim the same bytes; `c` follows `b` and `d` follows a
        // gap. The layout names the first two and leaves the last two out.
        let file = PlannedFile {
            rel: "f.dat".into(),
            size: 14,
            md5: "00000000000000000000000000000001".into(),
            url_prefix: Arc::from(""),
            chunks: vec![
                chunk_at("a", "md5-a", 0, 4),
                chunk_at("b", "md5-b", 2, 4),
                chunk_at("c", "md5-c", 6, 2),
                chunk_at("d", "md5-d", 10, 4),
            ],
        };
        let dest = [
            (0u64, 4u64, "md5-a"),
            (2, 4, "md5-b"),
            (6, 2, "md5-c"),
            (10, 4, "md5-d"),
        ];

        let named = overlapping_ranges(&file, &dest);
        assert_eq!(
            named.len(),
            2,
            "the layout names the two chunks that claim the same bytes"
        );

        // The ranges the layout leaves out claim bytes no other range claims.
        for (i, a) in dest.iter().enumerate() {
            if named.contains(a) {
                continue;
            }
            for (j, b) in dest.iter().enumerate() {
                if i == j {
                    continue;
                }
                assert!(
                    disjoint(a, b),
                    "the range {a:?} is claimed by {b:?}, so the layout has to \
                     name it and the decision has to read it"
                );
            }
        }
    }

    /// Do two byte ranges share a byte?
    fn disjoint(a: &util::ScanRange<'_>, b: &util::ScanRange<'_>) -> bool {
        let (a_off, a_len, _) = *a;
        let (b_off, b_len, _) = *b;
        a_off.saturating_add(a_len) <= b_off || b_off.saturating_add(b_len) <= a_off
    }

    /// A verdict list that is shorter than the ranges the decision needs is
    /// missing answers, not correct ones.
    ///
    /// The scan answers one verdict per range it was given, so a shorter list
    /// means the scan did not say what the decision asked. Treating the
    /// missing verdicts as passes would mark a mismatch final — a file the
    /// next attempt could still repair — so the decision reads them the way
    /// [`verify_in_place`] reads the chunks it has to redo: a missing verdict
    /// is a wrong one.
    #[test]
    fn a_short_verdict_list_is_never_a_pass() {
        let overlapping = PlannedFile {
            rel: "f.dat".into(),
            size: 2,
            md5: "00000000000000000000000000000001".into(),
            url_prefix: Arc::from(""),
            chunks: vec![
                chunk_at("a", "md5-a", 0, 2),
                chunk_at("b", "md5-b", 0, 2),
                chunk_at("c", "md5-c", 1, 1),
            ],
        };
        let overlap_dest = [(0u64, 2u64, "md5-a"), (0, 2, "md5-b"), (1, 1, "md5-c")];

        // The scan answers one verdict for one of the three overlapping
        // ranges. The two it did not answer are not passes.
        let (scan, checked) = read_after_redo(&overlapping, &overlap_dest, |_ranges| {
            Ok(util::Scan {
                file_md5: Md5::EMPTY,
                file_len: 2,
                dest_ok: vec![true],
                source_ok: Vec::new(),
            })
        })
        .unwrap();
        assert_eq!(checked, 3, "the overlap needs a verdict for every range");
        assert!(
            !redo_mismatch_is_terminal(&overlapping, &scan, checked),
            "a missing verdict is a missing answer, never a pass"
        );

        // And the same for the empty list: no verdict at all decides nothing.
        let (scan, checked) = read_after_redo(&overlapping, &overlap_dest, |_ranges| {
            Ok(util::Scan {
                file_md5: Md5::EMPTY,
                file_len: 2,
                dest_ok: Vec::new(),
                source_ok: Vec::new(),
            })
        })
        .unwrap();
        assert!(
            !redo_mismatch_is_terminal(&overlapping, &scan, checked),
            "no verdict at all decides nothing"
        );

        // The complete list is the only one that decides.
        let complete = util::Scan {
            file_md5: Md5::EMPTY,
            file_len: 2,
            dest_ok: vec![true, true, true],
            source_ok: Vec::new(),
        };
        assert!(
            redo_mismatch_is_terminal(&overlapping, &complete, checked),
            "every overlapping range matches, so the mismatch is final"
        );
    }

    /// The no-overlap proof is the layout's, not the caller's: an overlapping
    /// layout handed a scan built from an empty range list is missing answers,
    /// never evidence that a retry cannot change the bytes. A scan that
    /// answers more ranges than were asked is not trusted either, because the
    /// caller cannot tell which verdict belongs to which range.
    #[test]
    fn an_overlapping_layout_needs_exactly_the_verdicts_it_asked_for() {
        // `a` and `b` claim the same bytes, so the layout has an overlap.
        let overlapping = PlannedFile {
            rel: "f.dat".into(),
            size: 2,
            md5: "00000000000000000000000000000001".into(),
            url_prefix: Arc::from(""),
            chunks: vec![chunk_at("a", "md5-a", 0, 2), chunk_at("b", "md5-b", 0, 2)],
        };
        let empty = util::Scan {
            file_md5: Md5::EMPTY,
            file_len: 2,
            dest_ok: Vec::new(),
            source_ok: Vec::new(),
        };

        // The wrong caller asks for no ranges for an overlapping layout. The
        // empty verdict list must not be read as "nothing can have changed".
        assert!(
            !redo_mismatch_is_terminal(&overlapping, &empty, 0),
            "no overlap is the layout's answer, not the empty range list's"
        );

        // One verdict was asked for, but the scan answered two.
        let surplus = util::Scan {
            dest_ok: vec![true, true],
            ..empty.clone()
        };
        assert!(
            !redo_mismatch_is_terminal(&overlapping, &surplus, 1),
            "a surplus verdict means the answers cannot be mapped to the ranges"
        );

        // Exactly the verdicts the caller asked for, all correct.
        let exact = util::Scan {
            dest_ok: vec![true],
            ..empty.clone()
        };
        assert!(
            redo_mismatch_is_terminal(&overlapping, &exact, 1),
            "the asked-for verdicts all match, so the mismatch is final"
        );

        // A layout with no overlap needs no verdicts at all: the layout alone
        // settles it, whatever the scan answered.
        let gapped = PlannedFile {
            rel: "f.dat".into(),
            size: 3,
            md5: "00000000000000000000000000000001".into(),
            url_prefix: Arc::from(""),
            chunks: vec![chunk_at("a", "md5-a", 0, 1), chunk_at("b", "md5-b", 2, 1)],
        };
        assert!(
            redo_mismatch_is_terminal(&gapped, &empty, 0),
            "no overlap is proof by itself"
        );
    }

    /// The coarse tick containing `t`, used to model a store that truncates
    /// modification times to 2 s.
    ///
    /// The tick stays on the Unix epoch's 2 s grid, where a real coarse store
    /// truncates, but it is found by subtracting the offset into the tick
    /// rather than by rebuilding a time from epoch seconds. A clock on either
    /// side of the epoch therefore gets the tick at or before it, and a
    /// subtraction the platform cannot represent leaves `t` unchanged instead
    /// of panicking.
    fn coarse_tick_of(t: SystemTime) -> SystemTime {
        let tick = COARSE_MTIME_TICK.as_nanos();
        let since_epoch = match t.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(since) => since.as_nanos() as i128,
            Err(before) => -(before.duration().as_nanos() as i128),
        };
        // The non-negative remainder is the distance back to the tick, whether
        // the stamp is before or after the epoch.
        let into_tick = since_epoch.rem_euclid(tick as i128) as u64;
        t.checked_sub(Duration::from_nanos(into_tick)).unwrap_or(t)
    }

    /// The coarse tick is the epoch-aligned 2 s boundary at or before the
    /// stamp, on either side of the epoch, and truncating it again is a no-op.
    #[test]
    fn coarse_tick_is_at_or_before_the_stamp_on_either_side_of_the_epoch() {
        let cases = [
            SystemTime::UNIX_EPOCH + Duration::from_millis(1500),
            SystemTime::UNIX_EPOCH + Duration::from_secs(10),
            SystemTime::UNIX_EPOCH - Duration::from_millis(1500),
            SystemTime::UNIX_EPOCH - Duration::from_secs(10),
        ];
        for t in cases {
            let tick = coarse_tick_of(t);
            let into_tick = t
                .duration_since(tick)
                .expect("the tick is at or before the stamp");
            assert!(
                into_tick < COARSE_MTIME_TICK,
                "the tick starts less than one tick before {t:?}"
            );
            assert_eq!(
                coarse_tick_of(tick),
                tick,
                "a tick is stable under truncation"
            );
        }
        assert_eq!(
            coarse_tick_of(SystemTime::UNIX_EPOCH + Duration::from_millis(1500)),
            SystemTime::UNIX_EPOCH
        );
        assert_eq!(
            coarse_tick_of(SystemTime::UNIX_EPOCH - Duration::from_millis(1500)),
            SystemTime::UNIX_EPOCH - COARSE_MTIME_TICK
        );
        for boundary in [
            SystemTime::UNIX_EPOCH + COARSE_MTIME_TICK * 4,
            SystemTime::UNIX_EPOCH - COARSE_MTIME_TICK * 4,
        ] {
            assert_eq!(
                coarse_tick_of(boundary),
                boundary,
                "a boundary is its own tick"
            );
        }
    }

    fn set_modified(path: &std::path::Path, stamp: SystemTime) {
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(stamp)
            .unwrap();
    }

    /// A stamp a given age before the current clock, or `None` after
    /// reporting the skip when this host cannot represent that age.
    ///
    /// The fixture asserts the age the test needs instead of silently running
    /// with a younger stamp: on a host that cannot represent it, the caller
    /// returns without having exercised the age rule, using the clock-skip
    /// reason the shared helper prints.
    fn ago(age: Duration) -> Option<SystemTime> {
        let stamp = stamp_ago(age);
        if stamp.is_none() {
            crate::clock_support::skip_pre_epoch("the fixture's age, a day before the clock");
        }
        stamp
    }

    /// The natural coarse-timestamp window: the recorded mtime must share a
    /// tick with the scan, and a same-length rewrite right after the scan can
    /// then present the same recorded mtime. When the plan scan found the file
    /// **intact**, the digest is the skip check, so accepting that verdict
    /// would report the rewritten file skipped and leave it unrepaired. The
    /// age guard refuses a verdict recorded that recently and re-reads instead.
    #[test]
    fn a_same_tick_rewrite_of_an_intact_file_is_rescanned() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_inplace", "same_tick_intact");
        let path = dir.join("f.dat");
        let good: Vec<u8> = (0..8u8).map(|b| b'a' + b).collect();
        let md5_0 = util::md5::md5_digest(&good[..4]).to_hex();
        let md5_1 = util::md5::md5_digest(&good[4..]).to_hex();

        // The scan starts inside the tick that holds the file's mtime.
        let scanned_at = SystemTime::now();
        let recorded = coarse_tick_of(scanned_at);
        std::fs::write(&path, &good).unwrap();
        set_modified(&path, recorded);
        let recorded = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert!(
            scanned_at.duration_since(recorded).unwrap() <= COARSE_MTIME_TICK,
            "the setup must put the scan in the recorded tick"
        );

        let file = PlannedFile {
            rel: "f.dat".into(),
            size: good.len() as i64,
            md5: util::md5::md5_digest(&good).to_hex(),
            url_prefix: Arc::from(""),
            chunks: vec![chunk_at("c0", &md5_0, 0, 4), chunk_at("c1", &md5_1, 4, 4)],
        };
        let verdict = Verdict {
            file_md5: util::md5::md5_digest(&good),
            dest_ok: vec![true, true],
            source_ok: vec![true, true],
            file_len: good.len() as u64,
            modified: Some(recorded),
        };
        let plan = RepairPlan::from_files("", vec![file.clone()]);
        let report = ScanReport::for_test_at(&plan, vec![Some(verdict)], scanned_at);
        let mut ctx = test_ctx_with_plan(plan, Some(Arc::new(report)));
        ctx.dry_run = false;

        // Same length, and the coarse store truncates the rewrite back onto
        // the recorded tick.
        let mut changed = good.clone();
        changed[1] ^= 0xff;
        std::fs::write(&path, &changed).unwrap();
        set_modified(&path, recorded);

        let sources = [None, None];
        let (dest, src, owner) = scan_ranges(&file.chunks, &sources);
        let OpenScan::Ready { scan, .. } =
            open_and_scan(&ctx, FileId::new(0), &file, &path, &dest, &src, &owner).unwrap()
        else {
            panic!("a same-tick rewrite of an intact file must not be reported skipped");
        };
        assert!(
            !scan.file_md5.matches_hex(&file.md5),
            "the writer must have taken a fresh scan and seen the rewrite"
        );
        assert_ne!(std::fs::read(&path).unwrap(), good);
    }

    /// Even when a store preserves the recorded mtime across a write (the
    /// residual limit on [`reusable_verdict`]), a stale verdict that found the
    /// file **damaged** cannot make the writer skip: its digest is not the
    /// manifest's, so the patch runs and the whole-file digest stays the gate.
    /// It fails, and the extra range scan names the chunks to redo.
    #[test]
    fn a_stale_damaged_verdict_never_makes_the_writer_skip() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_inplace", "stale_damaged");
        let path = dir.join("f.dat");
        let good: Vec<u8> = (0..8u8).map(|b| b'a' + b).collect();
        let md5_0 = util::md5::md5_digest(&good[..4]).to_hex();
        let md5_1 = util::md5::md5_digest(&good[4..]).to_hex();

        // What the plan scan saw: chunk 1 already wrong, and the file's mtime.
        let mut damaged = good.clone();
        damaged[5] ^= 0xff;
        std::fs::write(&path, &damaged).unwrap();
        let Some(stamp) = ago(Duration::from_secs(24 * 60 * 60)) else {
            return;
        };
        set_modified(&path, stamp);
        let recorded = std::fs::metadata(&path).unwrap().modified().unwrap();
        let scanned_at = SystemTime::now();
        assert!(
            mtime_predates_scan(recorded, Some(scanned_at)),
            "this store is the one that keeps an old mtime across a write"
        );

        let file = PlannedFile {
            rel: "f.dat".into(),
            size: good.len() as i64,
            md5: util::md5::md5_digest(&good).to_hex(),
            url_prefix: Arc::from(""),
            chunks: vec![chunk_at("c0", &md5_0, 0, 4), chunk_at("c1", &md5_1, 4, 4)],
        };
        let verdict = Verdict {
            file_md5: util::md5::md5_digest(&damaged),
            dest_ok: vec![true, false],
            source_ok: vec![true, true],
            file_len: good.len() as u64,
            modified: Some(recorded),
        };
        let plan = RepairPlan::from_files("", vec![file.clone()]);
        let report = ScanReport::for_test_at(&plan, vec![Some(verdict)], scanned_at);
        let mut ctx = test_ctx_with_plan(plan, Some(Arc::new(report)));
        ctx.dry_run = false;

        // Both chunks change, the length stays the same, and the mtime is
        // restored the way this store preserves it.
        let mut changed = good.clone();
        changed[1] ^= 0xff;
        changed[5] ^= 0xff;
        std::fs::write(&path, &changed).unwrap();
        set_modified(&path, recorded);

        let sources = [None, None];
        let (dest, src, owner) = scan_ranges(&file.chunks, &sources);
        let OpenScan::Ready { mut handle, scan } =
            open_and_scan(&ctx, FileId::new(0), &file, &path, &dest, &src, &owner).unwrap()
        else {
            panic!("a stale damaged verdict must not make the writer skip");
        };
        assert!(
            !scan.file_md5.matches_hex(&file.md5),
            "the guard reused a verdict whose digest is not the manifest's"
        );
        assert_ne!(std::fs::read(&path).unwrap(), good);

        let dest: Vec<util::ScanRange<'_>> = vec![(0, 4, &md5_0), (4, 4, &md5_1)];
        match verify_in_place(
            &dest,
            |ranges| util::scan_reader(&mut handle, ranges, &[]),
            2,
            &file.md5,
        )
        .unwrap()
        {
            Verified::Mismatch { still_bad, .. } => assert_eq!(
                still_bad,
                vec![0, 1],
                "the digest gate names both rewritten chunks for the redo"
            ),
            Verified::Matches(md5) => panic!("a rewritten file must not match, got {md5}"),
        }
    }

    /// A verdict that does not describe this file's chunk layout is not
    /// reusable, even when length and mtime still match: the per-chunk lists
    /// are positional, so a differently shaped verdict would be indexed with
    /// the wrong meaning. A verdict with no recorded mtime is not reusable
    /// either.
    #[test]
    fn the_guard_rejects_a_verdict_that_does_not_describe_the_file() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_inplace", "guard_shape");
        let path = dir.join("a.dat");
        let bytes = b"abcdef";
        std::fs::write(&path, bytes).unwrap();
        let Some(stamp) = ago(Duration::from_secs(24 * 60 * 60)) else {
            return;
        };
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(stamp)
            .unwrap();
        let recorded = std::fs::metadata(&path).unwrap().modified().unwrap();

        let file = PlannedFile {
            rel: "a.dat".into(),
            size: bytes.len() as i64,
            md5: util::md5::md5_digest(bytes).to_hex(),
            url_prefix: Arc::from(""),
            chunks: vec![chunk_at("c0", "m0", 0, bytes.len() as i64)],
        };
        let id = FileId::new(0);
        let handle = std::fs::File::open(&path).unwrap();

        let wrong_shape = Verdict {
            file_md5: Md5::parse_hex(&file.md5).unwrap(),
            dest_ok: vec![true, true],
            source_ok: vec![true, true],
            file_len: bytes.len() as u64,
            modified: Some(recorded),
        };
        let plan = RepairPlan::from_files("", vec![file.clone()]);
        let report = ScanReport::for_test(&plan, vec![Some(wrong_shape)]);
        let ctx = test_ctx_with_plan(plan, Some(Arc::new(report)));
        assert!(
            reusable_verdict(&ctx, id, &file, handle.metadata()).is_none(),
            "a verdict for another chunk layout is not this file's verdict"
        );

        let no_time = Verdict {
            file_md5: Md5::parse_hex(&file.md5).unwrap(),
            dest_ok: vec![true],
            source_ok: vec![true],
            file_len: bytes.len() as u64,
            modified: None,
        };
        let plan = RepairPlan::from_files("", vec![file.clone()]);
        let report = ScanReport::for_test(&plan, vec![Some(no_time)]);
        let ctx = test_ctx_with_plan(plan, Some(Arc::new(report)));
        assert!(
            reusable_verdict(&ctx, id, &file, handle.metadata()).is_none(),
            "without a recorded mtime the guard cannot prove the file unchanged"
        );
    }

    fn test_ctx(scan: Option<Arc<ScanReport>>) -> FileCtx {
        test_ctx_with_plan(RepairPlan::from_files("", Vec::new()), scan)
    }

    fn test_ctx_with_plan(plan: RepairPlan, scan: Option<Arc<ScanReport>>) -> FileCtx {
        FileCtx {
            game_dir: std::path::PathBuf::new(),
            root: None,
            http: reqwest::Client::new(),
            plan: Arc::new(plan),
            local_map: Arc::new(LocalChunkMap::new()),
            scan,
            summary: Arc::new(Summary::new()),
            dry_run: true,
            repair_mode: RepairMode::Inplace,
            backoff: BackoffPolicy::none(),
        }
    }

    /// The plan scan is reusable only when the open file is still the one the
    /// scan saw: the same length and an exactly equal modification time. Any
    /// uncertainty falls back to a fresh read.
    #[test]
    fn the_guard_reuses_only_an_unchanged_plan_scan() {
        use std::time::{Duration, SystemTime};

        let dir = girpr_tempdir::TempDir::new("girpr_test_inplace", "reuse_guard");
        let path = dir.join("a.dat");
        let bytes = b"abcdef";
        std::fs::write(&path, bytes).unwrap();
        let Some(stamp) = ago(Duration::from_secs(24 * 60 * 60)) else {
            return;
        };
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(stamp)
            .unwrap();
        let recorded = std::fs::metadata(&path).unwrap().modified().unwrap();

        let file = PlannedFile {
            rel: "a.dat".into(),
            size: bytes.len() as i64,
            md5: util::md5::md5_digest(bytes).to_hex(),
            url_prefix: Arc::from(""),
            chunks: Vec::new(),
        };
        let verdict = Verdict {
            file_md5: Md5::parse_hex(&file.md5).unwrap(),
            dest_ok: Vec::new(),
            source_ok: Vec::new(),
            file_len: bytes.len() as u64,
            modified: Some(recorded),
        };
        let plan = RepairPlan::from_files("", vec![file.clone()]);
        let report = ScanReport::for_test_at(&plan, vec![Some(verdict)], SystemTime::now());
        let ctx = test_ctx_with_plan(plan, Some(Arc::new(report)));
        let id = FileId::new(0);

        let handle = std::fs::File::open(&path).unwrap();
        assert!(
            reusable_verdict(&ctx, id, &file, handle.metadata()).is_some(),
            "an old mtime with the same length is the one reusable shape"
        );

        // A changed modification time, with the length held fixed.
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(recorded + Duration::from_secs(1))
            .unwrap();
        let handle = std::fs::File::open(&path).unwrap();
        assert!(
            reusable_verdict(&ctx, id, &file, handle.metadata()).is_none(),
            "a different mtime means a different snapshot"
        );

        // A changed length, with the recorded mtime restored.
        std::fs::write(&path, b"abcdefg").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(recorded)
            .unwrap();
        let handle = std::fs::File::open(&path).unwrap();
        assert!(
            reusable_verdict(&ctx, id, &file, handle.metadata()).is_none(),
            "a different length is never reused"
        );

        // The same file again, but the report has no verdict for it.
        std::fs::write(&path, bytes).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(recorded)
            .unwrap();
        let handle = std::fs::File::open(&path).unwrap();
        let no_verdict = test_ctx(Some(Arc::new(ScanReport::default())));
        assert!(
            reusable_verdict(&no_verdict, id, &file, handle.metadata()).is_none(),
            "a missing verdict is not reusable"
        );

        // And without a scan there is nothing to reuse.
        let no_scan = test_ctx(None);
        assert!(reusable_verdict(&no_scan, id, &file, handle.metadata()).is_none());
    }

    /// The age rule, as a table: only a recorded mtime more than one coarse
    /// tick before the scan is trusted. A missing scan time, a future mtime,
    /// and the exact boundary all mean "read it again".
    #[test]
    fn the_reuse_age_rule_requires_more_than_one_coarse_tick() {
        let Some(recorded) = ago(Duration::from_secs(24 * 60 * 60)) else {
            return;
        };
        assert!(
            !mtime_predates_scan(recorded, None),
            "a report with no scan time cannot prove the window is closed"
        );
        assert!(!mtime_predates_scan(recorded, Some(recorded)));
        assert!(
            !mtime_predates_scan(recorded, Some(recorded + COARSE_MTIME_TICK)),
            "the boundary itself is still within the coarse tick"
        );
        assert!(mtime_predates_scan(
            recorded,
            Some(recorded + COARSE_MTIME_TICK + Duration::from_millis(1))
        ));
        assert!(mtime_predates_scan(
            recorded,
            Some(recorded + COARSE_MTIME_TICK * 2)
        ));
        assert!(
            !mtime_predates_scan(recorded + COARSE_MTIME_TICK, Some(recorded)),
            "a recorded mtime after the scan is uncertain, not reusable"
        );
    }

    /// An overflowing candidate end is unrepresentable, so it cannot be
    /// inside the target: the eager trim is deferred and the source survives.
    /// The non-overflow boundaries keep their old answers.
    #[test]
    fn an_overflowing_trim_candidate_is_not_free() {
        let small = chunk("c0", 10);
        let chunks = vec![small.clone()];
        let file = PlannedFile {
            rel: "f.dat".into(),
            size: 10,
            md5: "m".into(),
            url_prefix: Arc::from(""),
            chunks: chunks.clone(),
        };
        assert!(
            trim_is_free(&file, &chunks, &[Some(0)]),
            "the end is exactly the target's"
        );
        assert!(
            !trim_is_free(&file, &chunks, &[Some(1)]),
            "one past the target is not free"
        );
        assert!(
            trim_is_free(&file, &chunks, &[None]),
            "no candidate loses nothing"
        );

        let huge = vec![chunk("c0", i64::MAX)];
        let huge_file = PlannedFile {
            rel: "f.dat".into(),
            size: 10,
            ..file
        };
        assert!(
            !trim_is_free(&huge_file, &huge, &[Some(u64::MAX)]),
            "a source that overflows r + s is outside the target, never inside"
        );
    }

    /// The bounded window caps the number of futures alive at once and yields
    /// their outputs in input order. Those are the two properties the download
    /// window relies on: bounded network/decompression overlap, and
    /// deterministic per-file accounting and first-error reporting even though
    /// the fetches complete out of order.
    #[tokio::test]
    async fn the_chunk_window_bounds_in_flight_work_and_keeps_order() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let count = 12;
        let futures: Vec<_> = (0..count)
            .map(|i| {
                let live = Arc::clone(&live);
                let peak = Arc::clone(&peak);
                async move {
                    let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    live.fetch_sub(1, Ordering::SeqCst);
                    Ok::<_, ()>(i)
                }
            })
            .collect();

        let out = bounded_window(3, futures).await;
        assert_eq!(out, (0..count).map(Ok).collect::<Vec<_>>());
        assert!(
            peak.load(Ordering::SeqCst) <= 3,
            "more than three futures were in flight at once"
        );
        assert_eq!(live.load(Ordering::SeqCst), 0, "every future was drained");
    }

    /// The window drives every task to completion even when one of them fails,
    /// so a chunk that reserved download bytes before a sibling failed still
    /// settles its reservation. The first error is reported in input order.
    #[tokio::test]
    async fn the_chunk_window_drains_every_task_and_reports_the_first_error() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let finished = Arc::new(AtomicUsize::new(0));
        let futures: Vec<_> = (0..6)
            .map(|i| {
                let finished = Arc::clone(&finished);
                async move {
                    tokio::task::yield_now().await;
                    finished.fetch_add(1, Ordering::SeqCst);
                    if i == 2 { Err(i) } else { Ok(i) }
                }
            })
            .collect();

        let out = bounded_window(2, futures).await;
        assert_eq!(out[2], Err(2), "the failing chunk is reported in its slot");
        assert_eq!(
            finished.load(Ordering::SeqCst),
            6,
            "no task was dropped with an unsettled reservation"
        );
    }
}
