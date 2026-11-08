//! The temp-path repair: `{path}_tmp` is written and renamed over the target.
//!
//! Moved byte-for-byte from `file.rs`. Uses `super::{ChunkStats, FileCtx,
//! FileWrites, Inplace, Mode, PlannedFile, COPY_BUF, note_write,
//! reserve_write}` and crate paths for `StoppedBy`, `sophon`, `util`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

use anyhow::{Context, Result};

use super::{
    COPY_BUF, ChunkStats, FileCtx, FileWrites, Inplace, Mode, PlannedFile, note_write,
    reserve_write, scratch_slice,
};
use crate::repair::scope::ledger::StoppedBy;
use crate::sophon::{self, FileReuse, SophonChunk};
use crate::util;

/// The temp path: the original size + MD5 check, then the attempt.
///
/// The repair is assembled in `{path}_tmp` and swapped over the target with
/// an atomic rename, so **the target file is never modified** and a repair
/// that fails leaves it exactly as it was. That is the property this mode
/// exists for; what it costs is the disk — the whole file has to exist
/// twice, for as long as the repair takes.
pub(super) async fn attempt_temp(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    tmp_path: &std::path::Path,
    final_path: &std::path::Path,
    writes: &mut FileWrites,
) -> Result<Inplace> {
    match util::file_len(final_path) {
        util::FileLen::Absent => {
            tracing::debug!(path = %final_path.display(), "check: missing local file");
        }
        util::FileLen::Unreadable(e) => tracing::warn!(
            path = %final_path.display(),
            error = %e,
            "check: cannot stat the local file -> needs repair"
        ),
        util::FileLen::Present(n) if n != file.size as u64 => {
            tracing::debug!(actual_size = n, "check: size mismatch -> needs repair");
        }
        util::FileLen::Present(_) => match util::md5::md5_file_digest(final_path) {
            Ok(h) if h.matches_hex(&file.md5) => {
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
    let stats = repair_attempt(ctx, file, prefix, tmp_path, final_path, writes).await?;
    Ok(Inplace::Repaired(stats))
}

/// Repair one file **through a temp file** — see [`attempt_temp`].
async fn repair_attempt(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    tmp_path: &std::path::Path,
    final_path: &std::path::Path,
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
    repair_attempt_inner(ctx, file, prefix, tmp_path, final_path, &mut stats, writes).await
}

/// One full pass over a file's chunks, then verify + promote. The caller
/// ([`repair_attempt`]) owns the accounting of a failed attempt.
async fn repair_attempt_inner(
    ctx: &FileCtx,
    file: &PlannedFile,
    prefix: &str,
    tmp_path: &std::path::Path,
    final_path: &std::path::Path,
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
    let have = f
        .metadata()
        .with_context(|| format!("stat tmp {}", tmp_path.display()))?
        .len();
    let mut tmp_len = have;
    if have > file.size as u64 {
        tracing::debug!(tmp_bytes = have, "tmp oversize -> truncate");
        f.set_len(file.size as u64)?;
        tmp_len = file.size as u64;
    }
    tracing::debug!(
        tmp_bytes = tmp_len,
        expect_size = file.size,
        "tmp resume point"
    );
    let reuse_entries = ctx.local_map.get(&file.rel);
    // One scratch buffer for this file: every reuse hash and copy reuses it
    // instead of allocating a fresh 512 KiB buffer per chunk.
    let mut scratch = Vec::new();
    // The reuse source is the same file for every chunk, so one lazily opened
    // handle serves them all. A failed read drops it, and the next chunk may
    // open the path once more instead of failing the file.
    let mut src_handle: Option<File> = None;
    for c in &file.chunks {
        let end = chunk_end(c)?;
        let outcome = try_reuse_slice(&mut ReuseCall {
            ctx,
            src: final_path,
            c,
            end,
            reuse_entries,
            out: &mut f,
            scratch: &mut scratch,
            src_handle: &mut src_handle,
            tmp_len: &mut tmp_len,
            writes: &mut *writes,
        })?;
        match outcome {
            ChunkOutcome::Resumed => stats.chunks_resumed += 1,
            ChunkOutcome::Reused => {
                stats.chunks_reused += 1;
                tmp_len = tmp_len.max(end);
            }
            ChunkOutcome::Download => {
                if let Err(e) = write_downloaded_chunk(DownloadedChunk {
                    ctx,
                    prefix,
                    c,
                    out: &mut f,
                    stats: &mut *stats,
                    writes: &mut *writes,
                })
                .await
                {
                    // The chunk's bytes may have landed in the `_tmp` before
                    // its digest was known, and the temp mode resumes by
                    // **length**: a `_tmp` that grew past this chunk would be
                    // taken as "already complete" by the next run, and the
                    // corrupt bytes would be kept. Truncating back to the
                    // chunk's offset makes the resume honest — the chunks
                    // before it were each digest-checked when they were
                    // written, and everything from it on is gone.
                    if let Err(rollback) = rollback_failed_chunk(&mut f, tmp_path, c, &mut tmp_len)
                    {
                        return Err(chunk_failure_after_rollback(tmp_path, rollback, e));
                    }
                    return Err(e);
                }
                tmp_len = tmp_len.max(end);
            }
        }
    }
    f.flush()?;
    drop(f);
    // The reuse source is the same file the promote renames over. On Windows
    // `std` opens it with `FILE_SHARE_DELETE` among its default share flags
    // (this code never narrows them through `OpenOptionsExt`), so this handle
    // alone would not block the rename. Release it before promoting anyway:
    // it costs nothing and keeps the code independent of that default instead
    // of leaning on it.
    drop(src_handle);
    promote_tmp(file, tmp_path, final_path, stats, writes)
}

/// What one chunk needed. `Resumed` (already complete) and `Reused` (copied
/// from a verified local slice) are both zero-network paths, but they are
/// counted separately because they mean different things in the log.
enum ChunkOutcome {
    Resumed,
    Reused,
    Download,
}

/// The exclusive end of a chunk in the target file, validated against the
/// manifest's signed fields.
///
/// The manifest controls both numbers; a corrupt pair must fail this file
/// normally rather than panic (release builds would wrap here).
fn chunk_end(c: &SophonChunk) -> Result<u64> {
    u64::try_from(c.offset)
        .ok()
        .zip(u64::try_from(c.uncompressed_size).ok())
        .and_then(|(offset, size)| offset.checked_add(size))
        .with_context(|| {
            format!(
                "chunk {} has an invalid offset/size: {} + {}",
                c.id, c.offset, c.uncompressed_size
            )
        })
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
fn rollback_tmp(f: &mut File, tmp_path: &std::path::Path, offset: u64) -> Result<()> {
    // The handle is already open, so a stat failure here is a real I/O error,
    // not "empty". Treating it as 0 would skip the truncate and let the next
    // attempt resume the failed chunk's partial bytes by length — a corrupt
    // `_tmp` that only the final MD5 gate catches, with nothing logged.
    let len = f
        .metadata()
        .with_context(|| format!("stat tmp {} before rollback", tmp_path.display()))?
        .len();
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

/// Roll a failed chunk out of the `_tmp` and move the tracked resume point
/// back with it.
///
/// The truncation and the counter update belong together: the next chunk's
/// resume check reads `tmp_len`, so the tracked point must never claim bytes
/// the rollback removed. The order matters — a failed rollback leaves both the
/// `_tmp` and the counter as they were, and the caller reports the combined
/// failure.
fn rollback_failed_chunk(
    f: &mut File,
    tmp_path: &std::path::Path,
    c: &SophonChunk,
    tmp_len: &mut u64,
) -> Result<()> {
    let offset = c.offset.max(0) as u64;
    rollback_tmp(f, tmp_path, offset)?;
    *tmp_len = offset;
    tracing::debug!(
        tmp_bytes = *tmp_len,
        chunk = %c.id,
        "tmp resume point after rollback"
    );
    Ok(())
}

/// Compose the two failures of one chunk attempt: the chunk error stays the
/// reported cause, and a rollback failure is attached as context.
///
/// The rollback is the urgent condition — a `_tmp` left past the failed chunk
/// would be trusted by the next length-based resume — but replacing the chunk
/// error with it hides which chunk failed and why. Keep both.
fn chunk_failure_after_rollback(
    tmp_path: &std::path::Path,
    rollback: anyhow::Error,
    chunk: anyhow::Error,
) -> anyhow::Error {
    chunk.context(format!(
        "rolling the tmp back to the failed chunk also failed ({}): {rollback:#}",
        tmp_path.display()
    ))
}

/// One chunk's reuse attempt and the per-file state it shares.
///
/// The chunk, its manifest pair, and its validated end are per chunk; `out`,
/// `scratch`, `src_handle`, `tmp_len`, and `writes` live across the file's
/// chunks. Naming them keeps the copy path's inputs readable at the call site.
struct ReuseCall<'a, W: Write + Seek> {
    ctx: &'a FileCtx,
    src: &'a std::path::Path,
    c: &'a SophonChunk,
    end: u64,
    reuse_entries: Option<&'a FileReuse>,
    out: &'a mut W,
    scratch: &'a mut Vec<u8>,
    src_handle: &'a mut Option<File>,
    tmp_len: &'a mut u64,
    writes: &'a mut FileWrites,
}

/// S2: copy `c` from a hash-verified slice of the same local file. Returns
/// [`ChunkOutcome::Reused`] when the copy lands, and
/// [`ChunkOutcome::Download`] when there is no candidate or the slice does not
/// verify, so the caller falls back to downloading.
///
/// V1-SIMPLIFICATION (S2, see README): same-file-only reuse. Starward maps each
/// chunk to any local file via `OriginalFileFullPath/Offset` (cross-file dedup);
/// we only reuse slices from the same path, which covers ~all Genshin wins at a
/// fraction of the bookkeeping.
/// See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallFile.cs#L100-L131
/// and https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L347-L360
fn try_reuse_slice<W: Write + Seek>(call: &mut ReuseCall<'_, W>) -> Result<ChunkOutcome> {
    let c = call.c;
    // Resume-by-length: the temp mode's resume property (see README). A
    // completed prefix is kept without per-chunk re-verification; the final
    // whole-file MD5 gates promotion, so a corrupt prefix only wastes work
    // before failing shut. Mirrors Starward's
    // `if (fs.Length < chunk.Offset + chunk.UncompressedSize)` skip.
    // See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L342-L343
    if *call.tmp_len >= call.end {
        tracing::trace!(chunk = %c.id, offset = c.offset, size = c.uncompressed_size, "chunk already complete -> keep");
        return Ok(ChunkOutcome::Resumed);
    }

    let candidate = call.reuse_entries.and_then(|reuse| reuse.get(c));
    let Some(entry) = candidate else {
        tracing::trace!(chunk = %c.id, offset = c.offset, size = c.uncompressed_size, "no local reuse candidate -> download");
        return Ok(ChunkOutcome::Download);
    };
    let ro = entry.offset;

    // Take the file out while it is in use. A failed read leaves the slot
    // empty, so the next chunk tries a fresh open once rather than holding on
    // to a handle that is known to be unusable.
    let mut sf = match call.src_handle.take() {
        Some(sf) => sf,
        None => {
            match File::open(call.src).with_context(|| format!("open {}", call.src.display())) {
                Ok(sf) => sf,
                Err(e) => {
                    tracing::debug!(chunk = %c.id, "reuse slice unreadable ({e:#}) -> download");
                    return Ok(ChunkOutcome::Download);
                }
            }
        }
    };
    let digest = match util::md5::md5_reader_slice_digest(
        &mut sf,
        ro as u64,
        c.uncompressed_size as u64,
        &mut *call.scratch,
    ) {
        Ok(h) => h,
        Err(e) => {
            tracing::debug!(chunk = %c.id, "reuse slice unreadable ({e:#}) -> download");
            return Ok(ChunkOutcome::Download);
        }
    };
    if !digest.matches_hex(&c.uncompressed_md5) {
        *call.src_handle = Some(sf);
        tracing::debug!(chunk = %c.id, slice_md5 = %digest, "reuse slice md5 mismatch -> download");
        return Ok(ChunkOutcome::Download);
    }
    tracing::trace!(chunk = %c.id, src_offset = ro, "reuse slice md5 ok -> copy");
    // The same handle: seek back to the slice start and copy the bytes that
    // were just verified, instead of opening `src` again.
    if let Err(e) = sf.seek(SeekFrom::Start(ro as u64)) {
        // Leave the handle slot empty, the way a failed read does: a seek that
        // failed means this handle cannot be trusted to position the next
        // slice, so the next chunk opens the path once more rather than
        // reusing a bad handle. Restoring it here would only change the retry
        // path, not the outcome — the chunk is downloaded either way.
        tracing::debug!(chunk = %c.id, "reuse slice unreadable ({e:#}) -> download");
        return Ok(ChunkOutcome::Download);
    }
    // Position the sink only on the copy path. The download path does not need
    // it: `write_downloaded_chunk` builds a `ChunkFileSink`, whose constructor
    // seeks this same file to the chunk's offset before the stream starts.
    call.out.seek(SeekFrom::Start(c.offset as u64))?;
    let mut remaining = c.uncompressed_size as u64;
    let buf = scratch_slice(&mut *call.scratch, COPY_BUF);
    while remaining > 0 {
        let want = (remaining as usize).min(buf.len());
        let n = match sf.read(&mut buf[..want]) {
            Ok(n) => n,
            Err(e) => {
                tracing::debug!(chunk = %c.id, "reuse slice unreadable ({e:#}) -> download");
                return Ok(ChunkOutcome::Download);
            }
        };
        if n == 0 {
            // Short slice: fall through to the download, which re-seeks
            // to `c.offset` and overwrites the partial copy.
            *call.src_handle = Some(sf);
            tracing::debug!(chunk = %c.id, "reuse slice truncated -> download");
            return Ok(ChunkOutcome::Download);
        }
        // The write ceiling counts *this* copy: it goes into `*_tmp` and
        // appears in neither `download_bytes` nor the plan's estimate.
        reserve_write(call.ctx, n as u64)?;
        call.out.write_all(&buf[..n])?;
        note_write(call.ctx, &mut *call.writes, n as u64)?;
        remaining -= n as u64;
    }
    *call.src_handle = Some(sf);
    Ok(ChunkOutcome::Reused)
}

/// The sink of a streamed chunk: the decoded bytes go straight into the
/// target file, at the chunk's offset.
///
/// **The write ceiling is charged per piece**, the way the reuse copy charges
/// it (`copy_within`): a chunk is written piece by piece, so a ceiling that
/// trips in the middle of a chunk stops the run in the middle of a chunk. The
/// README already documents that a runtime hard stop leaves the work done in
/// place, and the next run re-verifies and continues.
struct ChunkFileSink<'a, W: Write + Seek> {
    ctx: &'a FileCtx,
    file: &'a mut W,
    offset: u64,
    written: u64,
}

impl<'a, W: Write + Seek> ChunkFileSink<'a, W> {
    /// Position the file at the chunk's offset once. The sink then writes
    /// sequentially: it holds the only `&mut W` for the stream, so no other
    /// writer can move the cursor behind it.
    fn new(ctx: &'a FileCtx, file: &'a mut W, offset: u64) -> std::io::Result<Self> {
        file.seek(SeekFrom::Start(offset))?;
        Ok(Self {
            ctx,
            file,
            offset,
            written: 0,
        })
    }

    /// How many bytes have landed in the file.
    fn written(&self) -> u64 {
        self.written
    }
}

impl<W: Write + Seek> Write for ChunkFileSink<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // The write ceiling counts this piece: it goes into the install, and
        // the ceiling counts what lands there.
        reserve_write(self.ctx, buf.len() as u64).map_err(std::io::Error::other)?;
        // The cursor was placed at the chunk's offset in `new`, and this sink
        // is the only writer for the stream, so the piece lands where the
        // previous one ended. `restart` is what repositions it.
        self.file.write_all(buf)?;
        self.written += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl<W: Write + Seek> sophon::ChunkSink for ChunkFileSink<'_, W> {
    fn restart(&mut self) -> std::io::Result<()> {
        // A retry starts the chunk over. The failed attempt's bytes are in the
        // file, and they are about to be overwritten by the same chunk's
        // bytes — the region was wrong before the write and it is wrong now.
        self.written = 0;
        self.file.seek(SeekFrom::Start(self.offset))?;
        Ok(())
    }
}

/// One chunk to download into a file: where it comes from, where its decoded
/// bytes land, and the per-file counters the write updates.
pub(super) struct DownloadedChunk<'a, W: Write + Seek> {
    pub(super) ctx: &'a FileCtx,
    pub(super) prefix: &'a str,
    pub(super) c: &'a SophonChunk,
    pub(super) out: &'a mut W,
    pub(super) stats: &'a mut ChunkStats,
    pub(super) writes: &'a mut FileWrites,
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
pub(super) async fn write_downloaded_chunk<W: Write + Seek>(
    chunk: DownloadedChunk<'_, W>,
) -> Result<()> {
    let DownloadedChunk {
        ctx,
        prefix,
        c,
        out,
        stats,
        writes,
    } = chunk;
    // Reserve before fetching, not after: the pre-flight estimate is a *lower
    // bound* (a reuse slice whose MD5 fails falls back to a download), so this
    // compare-and-swap is what actually bounds the run. Reserving the manifest's
    // `compressed_size` keeps the stop exact under `--jobs N` instead of
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
    // The seek is the only thing that can fail before the fetch starts. It
    // happens after the reservation, so a failure refunds it: no byte crossed
    // the wire, and a leaked reservation would count against the ceiling for
    // the rest of the run.
    let mut sink = match ChunkFileSink::new(ctx, &mut *out, c.offset.max(0) as u64) {
        Ok(sink) => sink,
        Err(e) => return Err(settle_seek_failure(ctx, reserved, &c.id, e)),
    };
    // What the fetch really spent, across every attempt: the checks below run
    // after the stream ends, so on failure the bytes of the attempt that died
    // mid-body would otherwise be lost. They crossed the wire, and the
    // ceiling is a ceiling: a settle that pushes the live figure past it trips
    // the stop right there, even if this was the run's last chunk.
    let mut consumed = 0u64;
    let digest = match sophon::stream_chunk_to(sophon::ChunkFetch {
        client: &ctx.http,
        sleep: ctx.sleep.as_ref(),
        url_prefix: prefix,
        id: &c.id,
        expected,
        max_compressed: reserved,
        consumed: &mut consumed,
        sink: &mut sink,
    })
    .await
    {
        Ok(d) => d,
        Err(e) => {
            settle_reservation(ctx, reserved, consumed);
            return Err(e).with_context(|| format!("chunk {}", c.id));
        }
    };
    // The body may not exceed its reservation (`stream_chunk_to` refuses one
    // that does), and a failed attempt's partial body is in `consumed` too, so
    // the live figure is what crossed the wire rather than the manifest's claim.
    settle_reservation(ctx, reserved, consumed);
    let actual = digest.compressed_bytes;
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
    if !digest.md5.matches_hex(&c.uncompressed_md5) {
        tracing::debug!(chunk = %c.id, expect_md5 = %c.uncompressed_md5, actual_md5 = %digest.md5, "chunk md5 mismatch");
        anyhow::bail!("chunk {} md5 mismatch", c.id);
    }
    note_write(ctx, writes, sink.written())?;
    stats.chunks_downloaded += 1;
    Ok(())
}

/// Settle a chunk whose sink could not be positioned, then report the seek
/// failure.
///
/// The reservation is taken before the seek, so this is the one failure that
/// must give it back whole: no byte crossed the wire. Keeping the refund and
/// the error together gives the test a seam to drive the path with a
/// synthetic seek error; production still returns the same context chain.
fn settle_seek_failure(
    ctx: &FileCtx,
    reserved: u64,
    chunk: &str,
    error: std::io::Error,
) -> anyhow::Error {
    settle_reservation(ctx, reserved, 0);
    anyhow::Error::new(error).context(format!("seek to chunk {chunk} offset"))
}

/// Fold the bytes a fetch really spent into the live download meter.
///
/// The reservation was the manifest's `compressed_size`; `consumed` is what
/// crossed the wire for this chunk in this call. `stream_chunk_to` retries up
/// to `ATTEMPTS` times and each attempt can deliver a partial body before it
/// fails, so a failed chunk is charged for those bodies and refunded only the
/// part of the reservation that never arrived. A chunk can therefore spend up
/// to `ATTEMPTS × reserved` in one call, and — because a file is attempted up
/// to `MAX_ATTEMPTS` times — up to `ATTEMPTS × MAX_ATTEMPTS` partial bodies per
/// chunk per run.
///
/// The reservation before the *next* fetch is what normally stops the run, but
/// a partial body can push the live figure over the line between two
/// reservations — including on the last chunk of the run. The bytes are
/// already spent, so the stop is tripped here, as soon as the real figure is
/// known. The overrun is bounded by the body that was already in flight: it is
/// the bytes of the piece that crossed the line, not a whole new reservation.
fn settle_reservation(ctx: &FileCtx, reserved: u64, consumed: u64) {
    use std::cmp::Ordering;

    let delta = match consumed.cmp(&reserved) {
        Ordering::Greater => i64::try_from(consumed - reserved).unwrap_or(i64::MAX),
        Ordering::Less => -i64::try_from(reserved - consumed).unwrap_or(i64::MAX),
        Ordering::Equal => 0,
    };
    ctx.summary.adjust_download_bytes(delta);
    if let Some(limit) = ctx.summary.max_download_bytes()
        && ctx.summary.download.live() > limit
    {
        ctx.summary.stop(StoppedBy::Download);
        tracing::warn!(
            limit,
            have = ctx.summary.download.live(),
            "download-bytes ceiling crossed by a body already in flight -> stopping the run"
        );
    }
}

/// Final gate: length + whole-file MD5, then the atomic promote. On mismatch the
/// tmp is deleted so the next attempt starts from a clean state and a bad file
/// can never be promoted partially.
///
/// NOTE(starward-parity): final MD5 gates promotion; on mismatch the tmp is
/// deleted and the file is retried/failed, never promoted partially.
/// See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L443-L459
fn promote_tmp(
    file: &PlannedFile,
    tmp_path: &std::path::Path,
    final_path: &std::path::Path,
    stats: &ChunkStats,
    writes: &FileWrites,
) -> Result<ChunkStats> {
    let len = std::fs::metadata(tmp_path)
        .with_context(|| format!("stat tmp {}", tmp_path.display()))?
        .len();
    if len != file.size as u64 {
        anyhow::bail!("tmp length {} != expected {}", len, file.size);
    }
    let h = util::md5::md5_file_digest(tmp_path)?;
    tracing::debug!(tmp_size = len, tmp_md5 = %h, "after: tmp ready for promote");
    if !h.matches_hex(&file.md5) {
        if let Err(e) = std::fs::remove_file(tmp_path) {
            // The next attempt resumes by length, so a `_tmp` that cannot be
            // deleted only costs work: the final MD5 gate refuses it again.
            tracing::warn!(
                tmp = %tmp_path.display(),
                error = %e,
                "cannot delete the corrupt temp file; the next attempt will re-check it"
            );
        }
        anyhow::bail!(
            "final md5 mismatch for {}: expect {} got {}",
            file.rel,
            file.md5,
            h
        );
    }
    std::fs::rename(tmp_path, final_path).with_context(|| format!("promote {}", file.rel))?;
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
    use std::sync::Arc;

    use crate::config::RepairMode;
    use crate::repair::phases::manifest::LocalChunkMap;
    use crate::repair::phases::plan::RepairPlan;
    use crate::report::Summary;
    use crate::sleep::NoSleep;
    use crate::sophon::{ChunkSink, ReuseEntry};
    use crate::util::Md5;

    /// A context with no network activity: the tests below only reach the
    /// reuse/no-candidate branches, where nothing is fetched.
    fn test_ctx() -> FileCtx {
        FileCtx {
            game_dir: std::path::PathBuf::new(),
            root: None,
            http: reqwest::Client::new(),
            plan: Arc::new(RepairPlan::from_files("", Vec::new())),
            local_map: Arc::new(LocalChunkMap::new()),
            scan: None,
            summary: Arc::new(Summary::new()),
            dry_run: false,
            repair_mode: RepairMode::Temp,
            sleep: Arc::new(NoSleep),
        }
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

    fn reuse_at(md5: &str, size: i64, offset: i64) -> FileReuse {
        FileReuse::new(vec![ReuseEntry {
            md5: Md5::parse_hex(md5).unwrap(),
            size,
            offset,
        }])
    }

    fn temp_out(path: &std::path::Path) -> File {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .unwrap()
    }

    /// A sink that records every seek it is asked for, so a test can assert
    /// that a path did not position the file instead of inferring it from the
    /// final cursor value.
    struct SeekRecording<'a> {
        file: &'a mut File,
        seeks: Vec<SeekFrom>,
        bytes: usize,
    }

    impl<'a> SeekRecording<'a> {
        fn new(file: &'a mut File) -> Self {
            Self {
                file,
                seeks: Vec::new(),
                bytes: 0,
            }
        }
    }

    impl Write for SeekRecording<'_> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let n = self.file.write(buf)?;
            self.bytes += n;
            Ok(n)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.file.flush()
        }
    }

    impl Seek for SeekRecording<'_> {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.seeks.push(pos);
            self.file.seek(pos)
        }
    }

    /// A sink that can be written but not positioned: every seek is refused,
    /// the way a stream that cannot be rewound presents itself. It records
    /// what the writer attempted, so a test can tell a refused seek from a
    /// path that never ran.
    #[derive(Default)]
    struct RefusingSeek {
        bytes: Vec<u8>,
        seeks: usize,
    }

    impl Write for RefusingSeek {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Seek for RefusingSeek {
        fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
            self.seeks += 1;
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "seek refused",
            ))
        }
    }

    /// A stat failure on the `_tmp` must name the stat and the path:
    /// `foo.dat_tmp` under a **regular file** is `ENOTDIR` on every platform,
    /// and that `io::Error` must surface instead of a zero length.
    #[test]
    fn a_tmp_that_cannot_be_statted_is_reported_as_a_stat_failure() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "promote_stat");
        let blocker = dir.join("game");
        std::fs::write(&blocker, b"not a directory").unwrap();
        let tmp = blocker.join("foo.dat_tmp");
        let final_path = dir.join("foo.dat");
        let file = PlannedFile {
            rel: "game/foo.dat".into(),
            size: 64,
            md5: String::new(),
            url_prefix: Arc::from(""),
            chunks: Vec::new(),
        };

        let err = promote_tmp(
            &file,
            &tmp,
            &final_path,
            &ChunkStats::default(),
            &FileWrites::default(),
        )
        .expect_err("the tmp path cannot be statted");
        let chain = format!("{err:#}");
        assert!(
            chain.contains("stat tmp") && chain.contains(&tmp.display().to_string()),
            "the real cause must be named, not `tmp length 0`: {chain}"
        );
    }

    /// R-3: when the rollback fails as well, the report must name the chunk
    /// failure *and* the rollback. The old `?` returned the rollback error and
    /// dropped the chunk error entirely.
    ///
    /// The seam is a read-only handle: truncation needs write access, so
    /// `rollback_tmp` fails while the original chunk error — the fault the run
    /// was cleaning up after — is still in hand.
    #[test]
    fn a_failed_rollback_keeps_the_chunk_error_as_the_cause() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "rollback_failure");
        let tmp = dir.join("foo.dat_tmp");
        std::fs::write(&tmp, b"0123456789").unwrap();
        let mut read_only = File::open(&tmp).unwrap();
        let rollback =
            rollback_tmp(&mut read_only, &tmp, 0).expect_err("a read-only handle cannot truncate");
        let err = chunk_failure_after_rollback(
            &tmp,
            rollback,
            anyhow::anyhow!("chunk 7: digest mismatch"),
        );
        let chain = format!("{err:#}");
        assert!(
            chain.contains("chunk 7: digest mismatch"),
            "the original chunk failure must survive: {chain}"
        );
        assert!(
            chain.contains(&tmp.display().to_string()) && chain.contains("rolling the tmp back"),
            "the rollback failure must be attached with the path: {chain}"
        );
    }

    /// The source is one file for the whole attempt: the first reuse opens it,
    /// and later chunks are served from the same handle. Removing the path is
    /// allowed to fail while the handle is open; either way the bytes come out
    /// the same.
    #[test]
    fn a_held_source_handle_serves_later_chunks() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "reuse_handle");
        let src = dir.join("source.bin");
        std::fs::write(&src, b"0123456789abcdef").unwrap();
        let md5 = util::md5::md5_file_slice(&src, 8, 4).unwrap();
        let reuse = reuse_at(&md5, 4, 8);
        let tmp_path = dir.join("out.bin");
        let mut out = temp_out(&tmp_path);
        let ctx = test_ctx();
        let mut scratch = Vec::new();
        let mut writes = FileWrites::default();
        let mut tmp_len = 0u64;
        let mut handle = None;

        let first = chunk_at("c0", &md5, 0, 4);
        let outcome = try_reuse_slice(&mut ReuseCall {
            ctx: &ctx,
            src: &src,
            c: &first,
            end: chunk_end(&first).unwrap(),
            reuse_entries: Some(&reuse),
            out: &mut out,
            scratch: &mut scratch,
            src_handle: &mut handle,
            tmp_len: &mut tmp_len,
            writes: &mut writes,
        })
        .unwrap();
        assert!(matches!(outcome, ChunkOutcome::Reused));
        assert!(handle.is_some(), "the first reuse keeps the source handle");

        // The removal is allowed to fail on a platform that will not unlink an
        // open file; the held handle is what the next chunk must use either way.
        let _ = std::fs::remove_file(&src);
        let second = chunk_at("c1", &md5, 4, 4);
        let outcome = try_reuse_slice(&mut ReuseCall {
            ctx: &ctx,
            src: &src,
            c: &second,
            end: chunk_end(&second).unwrap(),
            reuse_entries: Some(&reuse),
            out: &mut out,
            scratch: &mut scratch,
            src_handle: &mut handle,
            tmp_len: &mut tmp_len,
            writes: &mut writes,
        })
        .unwrap();
        assert!(
            matches!(outcome, ChunkOutcome::Reused),
            "the held handle serves the later chunk"
        );
        out.flush().unwrap();
        assert_eq!(std::fs::read(&tmp_path).unwrap(), b"89ab89ab");
    }

    /// A source that cannot be opened is not a file failure: the chunk is
    /// downloaded, and the empty slot lets the next chunk try a fresh open.
    #[test]
    fn a_missing_reuse_source_falls_back_to_download() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "reuse_missing");
        let src = dir.join("never-written.bin");
        let md5 = "d41d8cd98f00b204e9800998ecf8427e";
        let reuse = reuse_at(md5, 4, 8);
        let c = chunk_at("c0", md5, 4, 4);
        let mut file = temp_out(&dir.join("out.bin"));
        let mut out = SeekRecording::new(&mut file);
        let ctx = test_ctx();
        let mut scratch = Vec::new();
        let mut writes = FileWrites::default();
        let mut tmp_len = 0u64;
        let mut handle = None;

        let outcome = try_reuse_slice(&mut ReuseCall {
            ctx: &ctx,
            src: &src,
            c: &c,
            end: chunk_end(&c).unwrap(),
            reuse_entries: Some(&reuse),
            out: &mut out,
            scratch: &mut scratch,
            src_handle: &mut handle,
            tmp_len: &mut tmp_len,
            writes: &mut writes,
        })
        .unwrap();
        assert!(matches!(outcome, ChunkOutcome::Download));
        assert!(handle.is_none(), "a failed open leaves no handle behind");
        assert!(
            out.seeks.is_empty(),
            "the download path must not position the sink; the streaming sink places it"
        );
        assert_eq!(
            out.bytes, 0,
            "the download path must not write into the sink"
        );
    }

    /// A chunk with no local reuse candidate is downloaded, and the failed
    /// lookup must not touch the sink: the download path builds its own
    /// streaming sink, which positions the file when it starts.
    #[test]
    fn a_chunk_with_no_reuse_candidate_falls_back_without_touching_the_sink() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "reuse_none");
        let md5 = "d41d8cd98f00b204e9800998ecf8427e";
        // An entry for a different chunk identity: the lookup finds nothing.
        let reuse = reuse_at("00000000000000000000000000000000", 4, 0);
        let c = chunk_at("c0", md5, 0, 4);
        let out_path = dir.join("out.bin");
        let mut file = temp_out(&out_path);
        let mut out = SeekRecording::new(&mut file);
        let ctx = test_ctx();
        let mut scratch = Vec::new();
        let mut writes = FileWrites::default();
        let mut tmp_len = 0u64;
        let mut handle = None;

        let outcome = try_reuse_slice(&mut ReuseCall {
            ctx: &ctx,
            src: &dir.join("never-written.bin"),
            c: &c,
            end: chunk_end(&c).unwrap(),
            reuse_entries: Some(&reuse),
            out: &mut out,
            scratch: &mut scratch,
            src_handle: &mut handle,
            tmp_len: &mut tmp_len,
            writes: &mut writes,
        })
        .unwrap();

        assert!(matches!(outcome, ChunkOutcome::Download));
        assert!(handle.is_none(), "there is no source to hold open");
        assert!(
            out.seeks.is_empty(),
            "the download path must not position the sink for a missing candidate"
        );
        assert_eq!(
            out.bytes, 0,
            "the download path must not write into the sink"
        );
        assert_eq!(
            std::fs::metadata(&out_path).unwrap().len(),
            0,
            "nothing may land in the sink before the download starts"
        );
    }

    /// A reuse candidate whose local slice does not hash to the chunk's digest
    /// is downloaded. The mismatch is found before the sink is positioned, so
    /// the sink must not move or receive bytes — and the verified handle is
    /// kept for the next chunk, since the file itself is readable.
    #[test]
    fn a_reuse_slice_digest_mismatch_falls_back_without_touching_the_sink() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "reuse_mismatch");
        let src = dir.join("source.bin");
        std::fs::write(&src, b"0123456789abcdef").unwrap();
        // The chunk claims the first slice's digest, but the candidate points
        // at the fourth slice, which hashes differently.
        let md5 = util::md5::md5_file_slice(&src, 0, 4).unwrap();
        let reuse = reuse_at(&md5, 4, 8);
        let c = chunk_at("c0", &md5, 0, 4);
        let out_path = dir.join("out.bin");
        let mut file = temp_out(&out_path);
        let mut out = SeekRecording::new(&mut file);
        let ctx = test_ctx();
        let mut scratch = Vec::new();
        let mut writes = FileWrites::default();
        let mut tmp_len = 0u64;
        let mut handle = None;

        let outcome = try_reuse_slice(&mut ReuseCall {
            ctx: &ctx,
            src: &src,
            c: &c,
            end: chunk_end(&c).unwrap(),
            reuse_entries: Some(&reuse),
            out: &mut out,
            scratch: &mut scratch,
            src_handle: &mut handle,
            tmp_len: &mut tmp_len,
            writes: &mut writes,
        })
        .unwrap();

        assert!(matches!(outcome, ChunkOutcome::Download));
        assert!(
            handle.is_some(),
            "a readable source stays open for the next chunk when only the slice digest failed"
        );
        assert!(
            out.seeks.is_empty(),
            "the mismatch must be found before the sink is positioned"
        );
        assert_eq!(out.bytes, 0, "a mismatching slice must not be written");
        assert_eq!(
            std::fs::metadata(&out_path).unwrap().len(),
            0,
            "nothing may land in the sink when the slice does not verify"
        );
    }

    /// Resume-by-length reads the tracked counter, so a chunk the failed
    /// download rolled back out of the `_tmp` must be downloaded again rather
    /// than kept as complete.
    #[test]
    fn a_rollback_moves_the_resume_point_back_to_the_failed_chunk() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "rollback_resume");
        let tmp = dir.join("foo.dat_tmp");
        std::fs::write(&tmp, b"01234567").unwrap();
        let mut out = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&tmp)
            .unwrap();
        let ctx = test_ctx();
        let md5 = "d41d8cd98f00b204e9800998ecf8427e";
        let c = chunk_at("c1", md5, 4, 4);
        let missing = dir.join("never-written.bin");
        let mut scratch = Vec::new();
        let mut writes = FileWrites::default();
        let mut handle = None;
        let mut tmp_len = 8u64;

        let outcome = try_reuse_slice(&mut ReuseCall {
            ctx: &ctx,
            src: &missing,
            c: &c,
            end: chunk_end(&c).unwrap(),
            reuse_entries: None,
            out: &mut out,
            scratch: &mut scratch,
            src_handle: &mut handle,
            tmp_len: &mut tmp_len,
            writes: &mut writes,
        })
        .unwrap();
        assert!(
            matches!(outcome, ChunkOutcome::Resumed),
            "the chunk's end is inside the tmp"
        );

        // The rollback and the tracked resume point move together; the update
        // is the caller's, not a value the test sets by hand.
        rollback_failed_chunk(&mut out, &tmp, &c, &mut tmp_len).unwrap();
        assert_eq!(tmp_len, 4, "the tracked point follows the truncation");
        let outcome = try_reuse_slice(&mut ReuseCall {
            ctx: &ctx,
            src: &missing,
            c: &c,
            end: chunk_end(&c).unwrap(),
            reuse_entries: None,
            out: &mut out,
            scratch: &mut scratch,
            src_handle: &mut handle,
            tmp_len: &mut tmp_len,
            writes: &mut writes,
        })
        .unwrap();
        assert!(
            matches!(outcome, ChunkOutcome::Download),
            "the rolled-back chunk must not be resumed"
        );
        assert_eq!(std::fs::metadata(&tmp).unwrap().len(), 4);
    }

    /// The seek is the only failure between the download reservation and the
    /// fetch, so it must give the whole reservation back: no byte crossed the
    /// wire. The seam is the production helper, driven with a synthetic seek
    /// error instead of a file whose cursor cannot move.
    #[test]
    fn a_sink_that_cannot_be_positioned_refunds_its_reservation() {
        let mut summary = Summary::new();
        summary.set_download_limit(Some(1_000));
        let mut ctx = test_ctx();
        ctx.summary = Arc::new(summary);
        assert!(ctx.summary.reserve_download_bytes(400));
        assert_eq!(ctx.summary.download.live(), 400);

        let err = settle_seek_failure(
            &ctx,
            400,
            "c7",
            std::io::Error::new(std::io::ErrorKind::Unsupported, "seek refused"),
        );

        assert_eq!(
            ctx.summary.download.live(),
            0,
            "no byte crossed the wire, so the whole reservation comes back"
        );
        let chain = format!("{err:#}");
        assert!(
            chain.contains("seek to chunk c7 offset") && chain.contains("seek refused"),
            "the seek failure is still named with its chunk: {chain}"
        );
    }

    /// The whole download path over a sink that refuses to be positioned: the
    /// reservation is taken, the seek fails, and no byte is fetched, written,
    /// or left reserved. The seek failure is named with its chunk.
    #[tokio::test]
    async fn a_download_into_an_unseekable_sink_refunds_and_writes_nothing() {
        let mut summary = Summary::new();
        summary.set_download_limit(Some(1_000));
        let mut ctx = test_ctx();
        ctx.summary = Arc::new(summary);
        let c = chunk_at("c9", "d41d8cd98f00b204e9800998ecf8427e", 8, 16);
        let mut out = RefusingSeek::default();
        let mut stats = ChunkStats::default();
        let mut writes = FileWrites::default();

        let err = write_downloaded_chunk(DownloadedChunk {
            ctx: &ctx,
            prefix: "",
            c: &c,
            out: &mut out,
            stats: &mut stats,
            writes: &mut writes,
        })
        .await
        .expect_err("the sink refuses to be positioned");

        assert_eq!(out.seeks, 1, "the writer asks to position the sink once");
        assert!(
            out.bytes.is_empty(),
            "no byte may be written after the seek fails"
        );
        assert_eq!(
            ctx.summary.download.live(),
            0,
            "no byte crossed the wire, so the whole reservation comes back"
        );
        assert_eq!(stats.chunks_downloaded, 0);
        assert_eq!(stats.download_bytes, 0);
        assert_eq!(writes.settled, 0);
        let chain = format!("{err:#}");
        assert!(
            chain.contains("seek to chunk c9 offset") && chain.contains("seek refused"),
            "the seek failure is still named with its chunk: {chain}"
        );
    }

    /// The sink positions the file once and then writes sequentially: two
    /// pieces land at `offset..offset+len`, and `written` reports their total.
    /// A retry restarts the chunk at its offset instead of appending to what
    /// the failed attempt left behind.
    #[test]
    fn the_sink_writes_from_its_offset_and_restarts_over_it() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "sink_offset");
        let path = dir.join("out.bin");
        std::fs::write(&path, b"0123456789abcdef").unwrap();
        let mut out = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let ctx = test_ctx();

        let mut sink = ChunkFileSink::new(&ctx, &mut out, 4).expect("seek to the chunk offset");
        sink.write_all(b"ab").unwrap();
        sink.write_all(b"cd").unwrap();
        assert_eq!(sink.written(), 4, "the count is the bytes that landed");
        sink.flush().unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"0123abcd89abcdef",
            "the two pieces are contiguous from the chunk offset"
        );

        sink.restart().unwrap();
        assert_eq!(sink.written(), 0, "a retry starts the chunk over");
        sink.write_all(b"XY").unwrap();
        assert_eq!(sink.written(), 2);
        sink.flush().unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"0123XYcd89abcdef",
            "the retry overwrites from the offset rather than appending"
        );
    }
}
