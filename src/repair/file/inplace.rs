//! The in-place repair path: scan → stage → patch → verify.
//!
//! Moved byte-for-byte from `file.rs`. Imports from `super`:
//! `ChunkStats`, `FileCtx`, `FileWrites`, `Inplace`, `PlannedFile`,
//! `write_at`, `copy_within`, `zero_fill`, plus `super::mode::{open_inplace,
//! open_rw}` and `super::temp::write_downloaded_chunk`. Crate paths for
//! `RepairMode`, `reuse`, `SophonChunk`, `util`, and
//! `crate::repair::phases::plan::{PlannedFile, RepairPlan}`.

use std::io::Write;

use super::mode::{open_inplace, open_rw};
use super::temp::{DownloadedChunk, write_downloaded_chunk};
use super::{
    ChunkStats, FileCtx, FileWrites, Inplace, PlannedFile, copy_within, write_at, zero_fill,
};
use crate::config::RepairMode;
use crate::repair::phases::plan::FileId;
use crate::repair::reuse;
use crate::repair::scan::{Verdict, scan_ranges, source_ok_by_chunk, source_ok_by_range};
use crate::sophon::SophonChunk;
use crate::util::{self, Md5};
use anyhow::{Context, Result};

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
/// 5. **verify** — the whole-file MD5 is computed from one read and is the
///    gate. Only when it does not match does a second pass read the chunk
///    ranges, so the per-chunk verdicts can name the culprit; only those
///    chunks are redone.
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
    // candidate get a source range (see `scan::scan_ranges`); `source_owner`
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
        file,
        prefix,
        final_path,
        &sources,
        &scan.source_ok,
        &bad,
        &captured,
        tier,
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
    let scan = match reusable_verdict(ctx, id, file, final_path, &handle) {
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
            util::Scan {
                file_md5: v.file_md5.clone(),
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
    if Md5::parse_hex(&scan.file_md5).is_ok_and(|actual| actual.matches_hex(&file.md5)) {
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

/// Step 4: write the wrong chunks, fill the holes, and end at the target size.
#[allow(clippy::too_many_arguments)] // one parameter per piece of the patch decision
async fn patch_chunks(
    ctx: &FileCtx,
    writes: &mut FileWrites,
    handle: &mut std::fs::File,
    file: &PlannedFile,
    prefix: &str,
    final_path: &std::path::Path,
    sources: &[Option<u64>],
    source_ok: &[bool],
    bad: &[usize],
    captured: &[Option<Vec<u8>>],
    tier: Tier,
    stats: &mut ChunkStats,
) -> Result<()> {
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
            Some(o)
        }
    };

    // Ascending in the staged tier (forward writes), in the salvage order in
    // the ordered tier. One scratch buffer serves the whole patch: the chunks
    // are written one at a time, so the copy pieces reuse it.
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
    let layout = file.layout();
    for (offset, len) in &layout.gaps {
        zero_fill(ctx, writes, handle, *offset, *len)?;
    }

    // The file must end at its manifest size: shorter means the tail is
    // missing, longer means it is dead weight.
    handle.set_len(file.size.max(0) as u64)?;
    handle.flush()?;
    Ok(())
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
    // One whole-file digest is the gate. Only when it fails does the range
    // scan run, and its per-chunk verdicts name the culprit.
    let still_bad = match verify_in_place(handle, dest_ranges, file.chunks.len(), &file.md5)? {
        Verified::Matches(md5) => return Ok(repaired_in_place(stats, md5)),
        Verified::Mismatch { md5, still_bad } => {
            if still_bad.is_empty() {
                anyhow::bail!(
                    "final md5 mismatch for {}: expect {} got {} (the bytes outside the chunks are wrong)",
                    file.rel,
                    file.md5,
                    md5
                );
            }
            still_bad
        }
    };

    // Redo only the chunks that are still wrong. A chunk that fails here
    // passed its own MD5, so the damage is elsewhere: a second pass names
    // it instead of re-fetching the whole file.
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
    let final_md5 = util::md5::md5_reader_digest(handle)?;
    if !Md5::parse_hex(&file.md5).is_ok_and(|expected| expected == final_md5) {
        anyhow::bail!(
            "final md5 mismatch for {}: expect {} got {}",
            file.rel,
            file.md5,
            final_md5
        );
    }
    Ok(repaired_in_place(stats, final_md5))
}

/// What one verification pass over the in-place target answered.
enum Verified {
    /// The whole-file digest is the manifest's; nothing else had to be read.
    Matches(Md5),
    /// The digest did not match. `still_bad` is one entry per chunk, in plan
    /// order, from the extra scan that names what to redo.
    Mismatch { md5: Md5, still_bad: Vec<usize> },
}

/// Verify the target with one whole-file digest, and scan the ranges only
/// when that digest fails.
///
/// The success path only needs the whole-file MD5, so it costs one digest over
/// the file's bytes. The per-chunk verdicts exist to say which chunks to redo;
/// they are needed after a mismatch and never on success, so the scan that
/// produces them runs only then.
fn verify_in_place(
    handle: &mut std::fs::File,
    dest_ranges: &[util::ScanRange<'_>],
    chunks_len: usize,
    expect: &str,
) -> Result<Verified> {
    let after = util::md5::md5_reader_digest(handle)?;
    if Md5::parse_hex(expect).is_ok_and(|expected| expected == after) {
        return Ok(Verified::Matches(after));
    }
    let scan = util::scan_reader(handle, dest_ranges, &[])?;
    let still_bad = (0..chunks_len)
        .filter(|&i| !scan.dest_ok.get(i).copied().unwrap_or(false))
        .collect();
    Ok(Verified::Mismatch {
        md5: after,
        still_bad,
    })
}

/// The plan-phase verdict for this file, when a cheap stat proves the file has
/// not changed since it was scanned.
///
/// Reuse is deliberately conservative: a missing report, a failed stat, a
/// length mismatch, or any modification-time difference all mean "read it
/// again". A stale verdict describes a layout that is no longer on disk, so
/// every uncertainty costs the read rather than risking a wrong write.
///
/// # The coarse-timestamp limit
///
/// The check is exact only where the filesystem records timestamps with
/// enough resolution. It can be evaded on a **coarse** one - FAT/exFAT (2 s),
/// HFS+, and some SMB/NFS servers (1-2 s), or any store that truncates a
/// `set_modified` - by a rewrite that keeps the length and lands inside the
/// same timestamp tick as the scan. The recorded and current mtimes are then
/// equal and the guard accepts a verdict that describes the previous bytes.
///
/// That cannot produce a wrong final file, which is why the guard does not pay
/// for a second read or a digest here: the verdict only chooses which chunks
/// are skipped and which sources are trusted, while [`verify_in_place`] always
/// digests the finished file. A stale verdict makes that digest fail, its range
/// scan names the chunks that are still wrong, and [`verify_and_retry`]
/// downloads them. The cost is one extra verify pass and a redo, never a
/// silently wrong byte.
fn reusable_verdict<'a>(
    ctx: &'a FileCtx,
    id: FileId,
    file: &PlannedFile,
    path: &std::path::Path,
    handle: &std::fs::File,
) -> Option<&'a Verdict> {
    let verdict = ctx.scan.as_deref()?.get(id)?;
    if verdict.dest_ok.len() != file.chunks.len() || verdict.source_ok.len() != file.chunks.len() {
        return None;
    }
    let recorded = verdict.modified?;
    if handle.metadata().ok()?.len() != verdict.file_len {
        return None;
    }
    if std::fs::metadata(path).ok()?.modified().ok()? != recorded {
        return None;
    }
    Some(verdict)
}

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
        r.checked_add(s).is_none_or(|end| end <= size)
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
    use std::io::{Seek, SeekFrom};
    use std::sync::Arc;

    use super::*;
    use crate::repair::phases::manifest::LocalChunkMap;
    use crate::repair::phases::plan::RepairPlan;
    use crate::repair::scan::ScanReport;
    use crate::report::Summary;
    use crate::sleep::NoSleep;

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
            unknown: 0,
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

    /// The verify pass pins the whole-file digest on success and names the
    /// chunks to redo on a mismatch. The corrupt byte is in the chunk the
    /// patch did not write, so the digest has to fail first and the extra
    /// scan then has to report that chunk — not the one the patch rewrote.
    #[test]
    fn the_in_place_verify_pins_the_digest_and_names_the_still_bad_chunks() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_inplace", "verify_digest");
        let path = dir.join("f.dat");
        let mut good = vec![0x41u8; 64];
        good.extend_from_slice(&[0x42u8; 64]);
        std::fs::write(&path, &good).unwrap();

        let md5_0 = util::md5::md5_digest(&good[..64]).to_hex();
        let md5_1 = util::md5::md5_digest(&good[64..]).to_hex();
        let dest: Vec<util::ScanRange<'_>> = vec![(0, 64, &md5_0), (64, 64, &md5_1)];
        let expect = util::md5::md5_digest(&good).to_hex();

        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        // The verify starts at the beginning of the file even when the last
        // write left the handle at EOF.
        f.seek(SeekFrom::End(0)).unwrap();
        match verify_in_place(&mut f, &dest, 2, &expect).unwrap() {
            Verified::Matches(md5) => assert_eq!(md5.to_hex(), expect),
            Verified::Mismatch { md5, .. } => {
                panic!("an intact file must match, got {md5}")
            }
        }

        // Only the second chunk is damaged: chunk 0 is the one a patch would
        // have written, chunk 1 is outside it.
        let mut bad = good.clone();
        bad[70] ^= 0xff;
        std::fs::write(&path, &bad).unwrap();
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        f.seek(SeekFrom::End(0)).unwrap();
        match verify_in_place(&mut f, &dest, 2, &expect).unwrap() {
            Verified::Mismatch { md5, still_bad } => {
                assert_eq!(
                    still_bad,
                    vec![1],
                    "the extra scan names the chunk that is still wrong"
                );
                assert_eq!(md5, util::md5::md5_digest(&bad));
            }
            Verified::Matches(md5) => panic!("a damaged file must not match, got {md5}"),
        }
    }

    /// A filesystem with coarse timestamps can give two same-length writes the
    /// same mtime, so the length+mtime guard accepts a verdict that describes
    /// the previous bytes. That is safe because the whole-file digest is the
    /// real gate: it fails, and the extra range scan names the chunks to redo.
    #[test]
    fn a_coarse_mtime_rewrite_evades_the_guard_but_not_the_verify() {
        use std::time::{Duration, SystemTime};

        let dir = girpr_tempdir::TempDir::new("girpr_test_inplace", "coarse_mtime");
        let path = dir.join("f.dat");
        let good: Vec<u8> = (0..8u8).map(|b| b'a' + b).collect();
        std::fs::write(&path, &good).unwrap();
        let stamp = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(stamp)
            .unwrap();
        let recorded = std::fs::metadata(&path).unwrap().modified().unwrap();

        let md5_0 = util::md5::md5_digest(&good[..4]).to_hex();
        let md5_1 = util::md5::md5_digest(&good[4..]).to_hex();
        let file = PlannedFile {
            rel: "f.dat".into(),
            size: good.len() as i64,
            md5: util::md5::md5_digest(&good).to_hex(),
            url_prefix: Arc::from(""),
            chunks: vec![chunk_at("c0", &md5_0, 0, 4), chunk_at("c1", &md5_1, 4, 4)],
        };
        let verdict = Verdict {
            file_md5: file.md5.clone(),
            dest_ok: vec![true, true],
            source_ok: vec![true, true],
            file_len: good.len() as u64,
            modified: Some(recorded),
        };
        let ctx = test_ctx(Some(Arc::new(ScanReport {
            files: vec![Some(verdict)],
            bytes: good.len() as u64,
            damaged_chunks: 0,
            scanned_at: None,
        })));

        // Both chunks change, the length stays the same, and the mtime is
        // restored: exactly what a coarse-granularity store can hide.
        let mut changed = good.clone();
        changed[1] ^= 0xff;
        changed[5] ^= 0xff;
        std::fs::write(&path, &changed).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(recorded)
            .unwrap();
        let mut handle = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        assert!(
            reusable_verdict(&ctx, FileId::new(0), &file, &path, &handle).is_some(),
            "the guard really cannot see a same-length rewrite inside one tick"
        );

        let dest: Vec<util::ScanRange<'_>> = vec![(0, 4, &md5_0), (4, 4, &md5_1)];
        match verify_in_place(&mut handle, &dest, 2, &file.md5).unwrap() {
            Verified::Mismatch { still_bad, .. } => assert_eq!(
                still_bad,
                vec![0, 1],
                "the digest gate names both rewritten chunks for the redo"
            ),
            Verified::Matches(md5) => panic!("a rewritten file must not match, got {md5}"),
        }
    }

    fn test_ctx(scan: Option<Arc<ScanReport>>) -> FileCtx {
        FileCtx {
            game_dir: std::path::PathBuf::new(),
            root: None,
            http: reqwest::Client::new(),
            plan: Arc::new(RepairPlan {
                latest: String::new(),
                files: Vec::new(),
            }),
            local_map: Arc::new(LocalChunkMap::new()),
            scan,
            summary: Arc::new(Summary::new()),
            dry_run: true,
            repair_mode: RepairMode::Inplace,
            sleep: Arc::new(NoSleep),
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
        let stamp = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
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
            file_md5: file.md5.clone(),
            dest_ok: Vec::new(),
            source_ok: Vec::new(),
            file_len: bytes.len() as u64,
            modified: Some(recorded),
        };
        let report = ScanReport {
            files: vec![Some(verdict)],
            bytes: bytes.len() as u64,
            damaged_chunks: 0,
            scanned_at: None,
        };
        let ctx = test_ctx(Some(Arc::new(report)));
        let id = FileId::new(0);

        let handle = std::fs::File::open(&path).unwrap();
        assert!(
            reusable_verdict(&ctx, id, &file, &path, &handle).is_some(),
            "the same length and mtime is the one reusable shape"
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
            reusable_verdict(&ctx, id, &file, &path, &handle).is_none(),
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
            reusable_verdict(&ctx, id, &file, &path, &handle).is_none(),
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
            reusable_verdict(&no_verdict, id, &file, &path, &handle).is_none(),
            "a missing verdict is not reusable"
        );

        // And without a scan there is nothing to reuse.
        let no_scan = test_ctx(None);
        assert!(reusable_verdict(&no_scan, id, &file, &path, &handle).is_none());
    }
}
