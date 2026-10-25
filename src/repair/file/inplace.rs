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
use super::temp::write_downloaded_chunk;
use super::{
    ChunkStats, FileCtx, FileWrites, Inplace, PlannedFile, copy_within, write_at, zero_fill,
};
use crate::config::RepairMode;
use crate::repair::reuse;
use crate::sophon::SophonChunk;
use crate::util;
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
pub(super) async fn attempt_inplace(
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
