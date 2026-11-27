//! The temp-path repair: `{path}_tmp` is written and renamed over the target.
//!
//! Moved byte-for-byte from `file.rs`. Uses `super::{ChunkStats, FileCtx,
//! FileWrites, Inplace, Mode, PlannedFile, COPY_BUF, note_write,
//! reserve_write}` and crate paths for `StoppedBy`, `sophon`, `util`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

use anyhow::{Context, Result};

use super::{
    COPY_BUF, ChunkStats, FileCtx, FileId, FileWrites, Inplace, Mode, PlannedFile, note_write,
    reserve_write, reusable_verdict, scratch_slice,
};
use girpr_chunk_verify::{WholeFileVerdict, digest_matches, verify_whole_file};

use crate::repair::diagnostics;
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
    file_id: FileId,
    file: &PlannedFile,
    prefix: &str,
    tmp_path: &std::path::Path,
    final_path: &std::path::Path,
    writes: &mut FileWrites,
) -> Result<Inplace> {
    // Under --scan an intact target was already read in the plan phase. Reuse
    // that verdict when the same length/mtime guard the in-place path applies
    // proves the target is still the scanned one, so the whole-file digest is
    // not paid twice for the same bytes. Temp has no handle on the target at
    // this point, so it passes a path stat; that follows symlinks just like the
    // scan's own `modified` stat, so both paths agree on a symlinked target.
    // A file written earlier in this run has an mtime at or after the scan
    // start and is refused by the guard, so it is read again rather than
    // skipped from a stale verdict.
    if let Some(verdict) = reusable_verdict(ctx, file_id, file, std::fs::metadata(final_path)) {
        if digest_matches(&file.md5, &verdict.file_md5.to_hex()) {
            tracing::debug!(actual_md5 = %verdict.file_md5, "check: md5 match -> skip");
            diagnostics::record_plan_scan_reused();
            return Ok(Inplace::Skipped);
        }
        tracing::debug!(actual_md5 = %verdict.file_md5, "check: md5 mismatch -> needs repair");
    } else {
        // An unreadable stat is neither "there" nor "not there" and already has
        // its own warning; it is not handed to a gate that has no place for it.
        let observed_size = match util::file_len(final_path) {
            util::FileLen::Absent => {
                tracing::debug!(path = %final_path.display(), "check: missing local file");
                None
            }
            util::FileLen::Unreadable(e) => {
                tracing::warn!(
                    path = %final_path.display(),
                    error = %e,
                    "check: cannot stat the local file -> needs repair"
                );
                None
            }
            util::FileLen::Present(n) => Some(n),
        };
        let verdict = verify_whole_file(file.size as u64, &file.md5, observed_size, || {
            util::md5::md5_file_digest(final_path).map(|h| h.to_hex())
        });
        match verdict {
            WholeFileVerdict::Match { observed } => {
                tracing::debug!(actual_md5 = %observed, "check: md5 match -> skip");
                return Ok(Inplace::Skipped);
            }
            WholeFileVerdict::LengthMismatch { actual, .. } => {
                tracing::debug!(actual_size = actual, "check: size mismatch -> needs repair");
            }
            WholeFileVerdict::DigestMismatch { actual, .. } => {
                tracing::debug!(actual_md5 = %actual, "check: md5 mismatch -> needs repair");
            }
            WholeFileVerdict::DigestUnavailable(e) => {
                tracing::debug!("check: unreadable ({e:#}) -> needs repair");
            }
            // Both "not there" shapes were logged above, with their own wording.
            WholeFileVerdict::Absent => {}
        }
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
    // One scratch buffer for this file: every reuse copy reuses it instead of
    // allocating a fresh 512 KiB buffer per chunk.
    let mut scratch = Vec::new();
    // One capture buffer for this file, grown to one chunk's length at a time
    // and reused across chunks. A reuse slice is read once into this buffer;
    // its digest and the copy that follows both consume the bytes already in
    // memory. The bound is a single chunk, held only until its copy lands, so
    // the staging RAM is one slice rather than one slice per chunk.
    let mut slice_buf = Vec::new();
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
            slice_buf: &mut slice_buf,
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
    /// The whole-slice staging buffer, reused across chunks.
    slice_buf: &'a mut Vec<u8>,
    src_handle: &'a mut Option<File>,
    tmp_len: &'a mut u64,
    writes: &'a mut FileWrites,
}

/// How the single capture read of a reuse slice ended.
enum SliceCapture {
    /// The whole slice was read into the staging buffer.
    Complete,
    /// The source ended before the slice did. The bytes read are still in the
    /// buffer; the caller must not treat them as a verified slice.
    Short { have: u64, want: u64 },
    /// A seek or read failed. The concrete `io::Error` is kept so the caller
    /// can surface it instead of collapsing it to "no candidate".
    Failed(std::io::Error),
}

/// Read a reuse slice once, into `buf`, and report how it ended.
///
/// The old path hashed the slice with `md5_reader_slice_digest` and then sought
/// back and read the same bytes again for the copy. Here one read fills the
/// staging buffer; the digest and the copy both consume those bytes, so a
/// reused slice costs one source read. The buffer is grown at most to one
/// chunk's declared length, so the extra RAM is bounded to a single slice and
/// is released for the next chunk by `clear`.
fn capture_slice<R: Read + Seek>(
    sf: &mut R,
    offset: u64,
    len: u64,
    buf: &mut Vec<u8>,
) -> SliceCapture {
    if let Err(e) = sf.seek(SeekFrom::Start(offset)) {
        return SliceCapture::Failed(e);
    }
    let want = match usize::try_from(len) {
        Ok(n) => n,
        // An unrepresentable length is a corrupt manifest, not an I/O error:
        // report it as a short slice so the caller falls back to a download
        // whose reservation is bounded by the run's ceilings.
        Err(_) => {
            return SliceCapture::Short { have: 0, want: len };
        }
    };
    if buf.try_reserve_exact(want).is_err() {
        return SliceCapture::Short { have: 0, want: len };
    }
    buf.clear();
    let mut remaining = len;
    while remaining > 0 {
        let piece = (remaining as usize).min(COPY_BUF);
        let start = buf.len();
        // `read_to_end` over a bounded `Take` appends into the spare capacity
        // reserved above: only bytes actually read are admitted to `buf`, with
        // no zero-fill of the piece before the read overwrites it.
        let n = match (&mut *sf).take(piece as u64).read_to_end(buf) {
            Ok(n) => n,
            Err(e) => {
                buf.truncate(start);
                return SliceCapture::Failed(e);
            }
        };
        if n == 0 {
            return SliceCapture::Short {
                have: len - remaining,
                want: len,
            };
        }
        remaining -= n as u64;
    }
    SliceCapture::Complete
}

/// S2: copy `c` from a hash-verified slice of the same local file. Returns
/// [`ChunkOutcome::Reused`] when the copy lands, and
/// [`ChunkOutcome::Download`] when there is no candidate or the slice does not
/// verify, so the caller falls back to downloading.
///
/// V1-SIMPLIFICATION (S2, see README): same-file-only reuse. Starward maps each
/// chunk to any local file via `OriginalFileFullPath/Offset` (cross-file dedup);
/// we only reuse slices from the same path, which covers ~all GI wins at a
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
                    // An unreadable source is not "no candidate": it is an I/O
                    // failure worth surfacing. Keep the concrete error text and
                    // the download fallback, but do not hide the cause at debug.
                    tracing::warn!(
                        chunk = %c.id,
                        path = %call.src.display(),
                        error = %format!("{e:#}"),
                        "reuse source cannot be opened -> download"
                    );
                    diagnostics::record_reuse_io_fallback();
                    return Ok(ChunkOutcome::Download);
                }
            }
        }
    };
    // One source read for both the digest and the copy. The old code hashed the
    // slice, sought back and read it a second time; here the bytes land in a
    // per-file staging buffer that both passes consume.
    let digest = match capture_slice(
        &mut sf,
        ro as u64,
        c.uncompressed_size as u64,
        &mut *call.slice_buf,
    ) {
        SliceCapture::Complete => util::md5::md5_digest(call.slice_buf),
        SliceCapture::Short { have, want } => {
            // A truncated source is still readable, so keep the handle for the
            // next chunk; only a failed read or seek drops it.
            *call.src_handle = Some(sf);
            tracing::info!(
                chunk = %c.id,
                src_offset = ro,
                have,
                want,
                "reuse slice is short -> download"
            );
            diagnostics::record_reuse_io_fallback();
            return Ok(ChunkOutcome::Download);
        }
        SliceCapture::Failed(e) => {
            tracing::warn!(
                chunk = %c.id,
                src_offset = ro,
                error = %format!("{e:#}"),
                "reuse slice cannot be read -> download"
            );
            diagnostics::record_reuse_io_fallback();
            return Ok(ChunkOutcome::Download);
        }
    };
    if !digest.matches_hex(&c.uncompressed_md5) {
        *call.src_handle = Some(sf);
        // The expected outcome: this candidate simply does not hold the
        // chunk's bytes. Debug is right here, unlike the I/O cases above.
        tracing::debug!(chunk = %c.id, slice_md5 = %digest, "reuse slice md5 mismatch -> download");
        return Ok(ChunkOutcome::Download);
    }
    tracing::trace!(chunk = %c.id, src_offset = ro, "reuse slice md5 ok -> copy");
    // Position the sink only on the copy path. The download path does not need
    // it: `write_downloaded_chunk` builds a `ChunkFileSink`, whose constructor
    // seeks this same file to the chunk's offset before the stream starts. The
    // bytes come from the staging buffer now, so there is no seek back on the
    // source and no second read of the slice.
    call.out.seek(SeekFrom::Start(c.offset as u64))?;
    match copy_reuse_pieces(
        call.ctx,
        &mut *call.writes,
        &mut &call.slice_buf[..],
        &mut *call.out,
        &mut *call.scratch,
        c,
    )? {
        ReuseCopy::Complete => {
            *call.src_handle = Some(sf);
            Ok(ChunkOutcome::Reused)
        }
        // A short slice falls through to the download, which re-seeks to
        // `c.offset` and overwrites the partial copy. The handle is still
        // usable, so the next chunk keeps it.
        ReuseCopy::Truncated => {
            *call.src_handle = Some(sf);
            Ok(ChunkOutcome::Download)
        }
        // A failed read leaves the slot empty, so the next chunk tries a fresh
        // open once rather than holding on to a handle known to be unusable.
        ReuseCopy::ReadFailed => Ok(ChunkOutcome::Download),
    }
}

/// How far the copy of a verified reuse slice got.
enum ReuseCopy {
    /// Every byte of the slice landed in the sink.
    Complete,
    /// The source ended before the slice did; the caller falls back to the
    /// download with the handle still usable.
    Truncated,
    /// A read failed; the caller falls back to the download and drops the
    /// handle so the next chunk opens the path once more.
    ReadFailed,
}

/// Copy a verified reuse slice into the sink, one piece at a time.
///
/// Each piece is reserved before it lands, so the write ceiling is charged at
/// the same granularity as the download sink.
fn copy_reuse_pieces<R: Read, W: Write>(
    ctx: &FileCtx,
    writes: &mut FileWrites,
    src: &mut R,
    out: &mut W,
    scratch: &mut Vec<u8>,
    c: &SophonChunk,
) -> Result<ReuseCopy> {
    let mut remaining = c.uncompressed_size as u64;
    let buf = scratch_slice(scratch, COPY_BUF);
    while remaining > 0 {
        let want = (remaining as usize).min(buf.len());
        let n = match src.read(&mut buf[..want]) {
            Ok(n) => n,
            Err(e) => {
                tracing::debug!(chunk = %c.id, "reuse slice unreadable ({e:#}) -> download");
                return Ok(ReuseCopy::ReadFailed);
            }
        };
        if n == 0 {
            tracing::debug!(chunk = %c.id, "reuse slice truncated -> download");
            return Ok(ReuseCopy::Truncated);
        }
        // The write ceiling counts *this* copy: it goes into `*_tmp` and
        // appears in neither `download_bytes` nor the plan's estimate.
        reserve_write(ctx, n as u64)?;
        out.write_all(&buf[..n])?;
        note_write(ctx, writes, n as u64)?;
        remaining -= n as u64;
    }
    Ok(ReuseCopy::Complete)
}

/// The sink of a streamed chunk: the decoded bytes go straight into the
/// target file, at the chunk's offset.
///
/// Both repair modes download through this sink: the temp path writes the
/// `_tmp`, and the in-place path writes the target itself.
///
/// **The write ceiling is charged per piece**, the way each mode's reuse copy
/// charges it: the temp path copies from a verified local slice in
/// [`try_reuse_slice`], while the in-place path copies inside the file in
/// [`copy_within`](super::copy_within). Both reserve every piece before it
/// lands, so a chunk is written piece by piece and a ceiling that trips in the
/// middle of a chunk stops the run in the middle of a chunk. The README
/// already documents that a runtime hard stop leaves the work done in place,
/// and the next run re-verifies and continues.
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
///
/// # Threading
///
/// This future is polled by the file task that owns it, and that task is
/// driven with `Handle::block_on` on Tokio's blocking pool (see
/// `join_file_tasks` in the file module). The zstd decode and the
/// `ChunkFileSink` writes therefore already run on a blocking thread. Keep the
/// stream call there: an extra `spawn_blocking` hop would be redundant, and
/// moving it back onto a runtime worker would put the disk work where it
/// starves timers and network polling. `stream_chunk_to` needs no wrapper here
/// because its blocking-context contract is already met.
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
    // Polled on the file task's blocking-pool thread; the decode and sink
    // writes below are synchronous. No spawn_blocking wrapper here — that
    // would duplicate the context the file task already provides.
    let digest = match sophon::stream_chunk_to(sophon::ChunkFetch {
        client: &ctx.http,
        backoff: &ctx.backoff,
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
/// The reservation is taken before the seek, so a failed seek must give it
/// back whole: no byte crossed the wire, and a reservation left behind would
/// count against the ceiling for the rest of the run. Keeping the refund and
/// the error together is what makes that invariant hold on this path.
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

    use crate::report::Summary;
    use crate::retry::BackoffPolicy;
    use crate::sophon::{ChunkSink, ReuseEntry};
    use crate::util::Md5;
    use girpr_repair_plan::LocalChunkMap;
    use girpr_repair_plan::RepairPlan;
    use girpr_repair_policy::RepairMode;

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
            backoff: BackoffPolicy::none(),
        }
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

    /// The per-file state a reuse test shares across its calls: one scratch
    /// buffer, the lazily opened source handle, the tracked resume point, and
    /// the write counter. Bundling it keeps each call down to the pieces the
    /// test varies.
    struct ReuseState<'a> {
        ctx: &'a FileCtx,
        scratch: Vec<u8>,
        slice_buf: Vec<u8>,
        src_handle: Option<File>,
        tmp_len: u64,
        writes: FileWrites,
    }

    impl<'a> ReuseState<'a> {
        fn new(ctx: &'a FileCtx) -> Self {
            Self {
                ctx,
                scratch: Vec::new(),
                slice_buf: Vec::new(),
                src_handle: None,
                tmp_len: 0,
                writes: FileWrites::default(),
            }
        }

        /// A call for `c` against `src` and `out`. The chunk's end is derived
        /// from its offset and size, and there is no reuse entry until a test
        /// adds one with [`ReuseCall::with_reuse`].
        fn call<'b, W: Write + Seek>(
            &'b mut self,
            src: &'b std::path::Path,
            c: &'b SophonChunk,
            out: &'b mut W,
        ) -> ReuseCall<'b, W> {
            ReuseCall {
                ctx: self.ctx,
                src,
                c,
                end: chunk_end(c).expect("test chunk has a valid offset and size"),
                reuse_entries: None,
                out,
                scratch: &mut self.scratch,
                slice_buf: &mut self.slice_buf,
                src_handle: &mut self.src_handle,
                tmp_len: &mut self.tmp_len,
                writes: &mut self.writes,
            }
        }
    }

    impl<'a, W: Write + Seek> ReuseCall<'a, W> {
        /// Point the lookup at `entries`.
        fn with_reuse(mut self, entries: &'a FileReuse) -> Self {
            self.reuse_entries = Some(entries);
            self
        }
    }

    /// A sink that records every seek it is asked for, so a test can assert
    /// that a path did not position the file instead of inferring it from the
    /// final cursor value.
    struct SeekRecording<'a> {
        file: &'a mut File,
        seeks: Vec<SeekFrom>,
        write_calls: usize,
        bytes: usize,
    }

    impl<'a> SeekRecording<'a> {
        fn new(file: &'a mut File) -> Self {
            Self {
                file,
                seeks: Vec::new(),
                write_calls: 0,
                bytes: 0,
            }
        }
    }

    impl Write for SeekRecording<'_> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let n = self.file.write(buf)?;
            self.write_calls += 1;
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

    /// A reader that hands back exactly what the caller asked for, up to what
    /// is left. The copy loop's piece boundaries then come from the loop
    /// itself, not from how the OS chose to split a regular-file read.
    struct ExactReads {
        data: Vec<u8>,
        pos: usize,
        reads: usize,
    }

    impl ExactReads {
        fn new(data: Vec<u8>) -> Self {
            Self {
                data,
                pos: 0,
                reads: 0,
            }
        }
    }

    impl Read for ExactReads {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            let n = buf.len().min(self.data.len() - self.pos);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    /// A reader that records every seek and every byte handed back, so a test
    /// can tell a single capture read from the old hash-then-seek-back pair.
    struct CountingReader<'a> {
        file: &'a mut File,
        seeks: Vec<SeekFrom>,
        bytes_read: u64,
    }

    impl<'a> CountingReader<'a> {
        fn new(file: &'a mut File) -> Self {
            Self {
                file,
                seeks: Vec::new(),
                bytes_read: 0,
            }
        }
    }

    impl Read for CountingReader<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.file.read(buf)?;
            self.bytes_read += n as u64;
            Ok(n)
        }
    }

    impl Seek for CountingReader<'_> {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.seeks.push(pos);
            self.file.seek(pos)
        }
    }

    /// A reader that fails every read, standing in for an I/O error in the
    /// middle of a reuse copy.
    struct FailingReads;

    impl Read for FailingReads {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("read failed"))
        }
    }

    /// A sink that can be written but not positioned: every seek is refused,
    /// the way a stream that cannot be rewound presents itself. It records
    /// what the writer attempted, so a test can tell a refused seek from a
    /// path that never ran.
    ///
    /// Each refused seek also records the live download figure it saw. That
    /// pins the order: the figure after the failure is zero whether the chunk
    /// was reserved and then refunded or never reserved at all, so only the
    /// value *while the seek runs* tells the two apart.
    struct RefusingSeek {
        bytes: Vec<u8>,
        seeks: usize,
        summary: Arc<Summary>,
        live_at_seek: Vec<u64>,
    }

    impl RefusingSeek {
        /// Refuse every seek while watching `summary`'s download figure.
        fn watching(summary: Arc<Summary>) -> Self {
            Self {
                bytes: Vec::new(),
                seeks: 0,
                summary,
                live_at_seek: Vec::new(),
            }
        }
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
            self.live_at_seek.push(self.summary.download.live());
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

    /// The recorder's byte count is the bytes that landed, not the writes that
    /// carried them: a chunk larger than one copy piece is written in two
    /// pieces, and the count is still the chunk's size.
    #[test]
    fn a_reused_slice_is_counted_by_the_bytes_it_writes() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "reuse_bytes");
        let src = dir.join("source.bin");
        let size = COPY_BUF + 4;
        let source: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        std::fs::write(&src, &source).unwrap();
        let md5 = util::md5::md5_file_slice(&src, 0, size as u64).unwrap();
        let reuse = reuse_at(&md5, size as i64, 0);
        let c = chunk_at("c0", &md5, 0, size as i64);
        let out_path = dir.join("out.bin");
        let mut file = temp_out(&out_path);
        let mut out = SeekRecording::new(&mut file);
        let ctx = test_ctx();
        let mut state = ReuseState::new(&ctx);

        let outcome =
            try_reuse_slice(&mut state.call(&src, &c, &mut out).with_reuse(&reuse)).unwrap();

        assert!(matches!(outcome, ChunkOutcome::Reused));
        assert!(
            out.write_calls >= 2,
            "a chunk larger than one copy piece takes more than one write"
        );
        assert_eq!(
            out.bytes, size,
            "the count is the bytes written, not the number of writes"
        );
        out.flush().unwrap();
        assert_eq!(std::fs::read(&out_path).unwrap(), source);
    }

    /// The copy loop asks for one piece at a time and writes each piece it
    /// reads. This source returns exactly the requested length, so the pieces
    /// come from the loop's `COPY_BUF` boundary rather than from how a
    /// regular-file read happened to split.
    #[test]
    fn a_reuse_copy_writes_one_piece_per_read() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "reuse_pieces");
        let size = COPY_BUF + 4;
        let source: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let mut src = ExactReads::new(source.clone());
        let out_path = dir.join("out.bin");
        let mut file = temp_out(&out_path);
        let mut out = SeekRecording::new(&mut file);
        let c = chunk_at("c0", "", 0, size as i64);
        let ctx = test_ctx();
        let mut scratch = Vec::new();
        let mut writes = FileWrites::default();

        let copied =
            copy_reuse_pieces(&ctx, &mut writes, &mut src, &mut out, &mut scratch, &c).unwrap();

        assert!(
            matches!(copied, ReuseCopy::Complete),
            "the whole slice is available"
        );
        assert_eq!(
            src.reads, 2,
            "the loop asks for COPY_BUF and then the remainder"
        );
        assert_eq!(out.write_calls, 2, "one write per read piece");
        assert_eq!(
            out.bytes, size,
            "the count is the bytes written, not the number of writes"
        );
        out.flush().unwrap();
        assert_eq!(std::fs::read(&out_path).unwrap(), source);
    }

    /// IO-02: a reuse slice is captured in one read. The old path hashed the
    /// slice, sought back and read it again to copy it; the capture helper now
    /// seeks once to the slice start and hands back exactly the slice's bytes,
    /// which both the digest and the copy consume.
    #[test]
    fn a_reuse_slice_is_captured_in_one_read() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "capture_once");
        let src = dir.join("source.bin");
        let size = COPY_BUF + 4;
        let source: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        std::fs::write(&src, &source).unwrap();
        let mut file = File::open(&src).unwrap();
        let mut reader = CountingReader::new(&mut file);
        let mut buf = Vec::new();

        let outcome = capture_slice(&mut reader, 0, size as u64, &mut buf);

        assert!(matches!(outcome, SliceCapture::Complete));
        assert_eq!(
            reader.seeks,
            vec![SeekFrom::Start(0)],
            "one seek to the slice start, no seek-back for a second pass"
        );
        assert_eq!(
            reader.bytes_read, size as u64,
            "the slice is read exactly once"
        );
        assert_eq!(buf, source, "the single read captured the whole slice");
    }

    /// A short source is a failure condition, not a digest to trust: it is
    /// reported with how much was actually available, and it still costs only
    /// the one read.
    #[test]
    fn a_short_reuse_source_is_reported_without_a_second_read() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "capture_short");
        let src = dir.join("source.bin");
        std::fs::write(&src, b"0123").unwrap();
        let mut file = File::open(&src).unwrap();
        let mut reader = CountingReader::new(&mut file);
        let mut buf = Vec::new();

        let outcome = capture_slice(&mut reader, 0, 8, &mut buf);

        assert!(matches!(outcome, SliceCapture::Short { have: 4, want: 8 }));
        assert_eq!(reader.seeks.len(), 1, "a short slice does not seek back");
        assert_eq!(reader.bytes_read, 4, "only the available bytes were read");
    }

    /// The spare-capacity capture returns exactly the slice bytes with no
    /// zero-fill and no stale tail, and reuses one buffer across calls.
    #[test]
    fn a_captured_slice_contains_only_the_bytes_that_were_read() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "capture_spare");
        let src = dir.join("source.bin");
        let data: Vec<u8> = (0..COPY_BUF + 16).map(|i| (i % 251) as u8).collect();
        std::fs::write(&src, &data).unwrap();
        let mut f = File::open(&src).unwrap();
        // Pre-filled with garbage: `clear` resets the length and the read must
        // admit only initialized bytes, never a padding or stale byte.
        let mut buf = vec![0xAAu8; 8];

        let outcome = capture_slice(&mut f, 3, (COPY_BUF + 4) as u64, &mut buf);
        assert!(matches!(outcome, SliceCapture::Complete));
        assert_eq!(buf, &data[3..3 + COPY_BUF + 4]);

        let outcome = capture_slice(&mut f, 0, 5, &mut buf);
        assert!(matches!(outcome, SliceCapture::Complete));
        assert_eq!(buf, &data[..5], "no stale bytes from the longer slice");
    }

    /// A local manifest can lie about `f.size` and let a reuse offset past the
    /// real EOF pass the map's declared-size range check (warn + skip, no
    /// clamp). This extends that same scenario to the temp path: the real-length
    /// re-check in `capture_slice` must refuse to capture the bad offset, so the
    /// chunk falls back to a download instead of earning an intact/copy verdict.
    /// The identical entry against a source whose real length *does* cover the
    /// offset verifies — showing it is the real length, not the declared one,
    /// that decides. Neither the map (T12/COR-03) nor the downstream guards
    /// change.
    #[test]
    fn a_lying_declared_size_cannot_make_a_bad_offset_an_intact_temp_reuse() {
        use crate::hyp::ChunkManifestMeta;
        use crate::sophon::{SophonChunkFile, WantedManifest, build_local_chunk_map};

        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "lying_size_temp");
        let real_len = 16u64;
        let declared_size = 4096i64; // the local manifest lies
        let bad_offset = 2048i64; // past the real EOF, inside the declared size
        let chunk_len = 8i64;

        // Genuine bytes that verify `target` if they were present at the
        // declared offset. They sit at 0 in the short file so the digest is real
        // and the only thing that can reject the reuse is the real length.
        let good: Vec<u8> = (0..chunk_len as usize)
            .map(|i| (i as u8).wrapping_mul(7).wrapping_add(1))
            .collect();
        let md5 = util::md5::md5_digest(&good).to_hex();
        let target = chunk_at("target", &md5, 0, chunk_len);

        let local = WantedManifest {
            meta: ChunkManifestMeta {
                category_id: "game".into(),
                category_name: "game".into(),
                matching_field: "game".into(),
                manifest: Default::default(),
                chunk_download: Default::default(),
                manifest_download: Default::default(),
            },
            files: vec![SophonChunkFile {
                file: "game/foo.dat".into(),
                chunks: vec![chunk_at("stale", &md5, bad_offset, chunk_len)],
                is_folder: false,
                size: declared_size,
                md5: "m1".into(),
            }],
        };
        let map = build_local_chunk_map(&[local]);
        let reuse = map
            .get("game/foo.dat")
            .expect("the declared size admits the bad range");
        assert_eq!(
            reuse.get(&target).map(|entry| entry.offset),
            Some(bad_offset),
            "the map keeps the bad offset: warn + skip, no clamp"
        );

        // Short real source: the offset is past EOF, so the real-length re-check
        // reports a short capture, never a complete one.
        let short_path = dir.join("short.bin");
        std::fs::write(&short_path, vec![0u8; real_len as usize]).unwrap();
        let mut short_file = File::open(&short_path).unwrap();
        let mut buf = Vec::new();
        match capture_slice(
            &mut short_file,
            bad_offset as u64,
            chunk_len as u64,
            &mut buf,
        ) {
            SliceCapture::Short { have, want } => {
                assert_eq!(have, 0, "no bytes exist past the real EOF");
                assert_eq!(want, chunk_len as u64);
            }
            SliceCapture::Complete => panic!("a past-EOF offset cannot be a complete capture"),
            SliceCapture::Failed(e) => panic!("seeking past EOF is not an I/O failure: {e}"),
        }

        // End to end on the temp path: the bad offset earns a download and
        // nothing is copied into the sink.
        let short_out = dir.join("short.out");
        let mut file = temp_out(&short_out);
        let mut out = SeekRecording::new(&mut file);
        let ctx = test_ctx();
        let mut state = ReuseState::new(&ctx);
        let outcome =
            try_reuse_slice(&mut state.call(&short_path, &target, &mut out).with_reuse(reuse))
                .unwrap();
        assert!(
            matches!(outcome, ChunkOutcome::Download),
            "a lying declared size cannot turn a past-EOF reuse into an intact copy"
        );
        assert_eq!(
            out.bytes, 0,
            "no bytes were copied from the past-EOF offset"
        );

        // Contrast: the same entry against a source whose real length covers
        // the offset captures and verifies, so the real length is the guard.
        let long_len = (bad_offset + chunk_len) as usize;
        let mut long_data = vec![0u8; long_len];
        long_data[bad_offset as usize..].copy_from_slice(&good);
        let long_path = dir.join("long.bin");
        std::fs::write(&long_path, &long_data).unwrap();
        let mut long_file = File::open(&long_path).unwrap();
        let mut long_buf = Vec::new();
        let captured = capture_slice(
            &mut long_file,
            bad_offset as u64,
            chunk_len as u64,
            &mut long_buf,
        );
        assert!(matches!(captured, SliceCapture::Complete));
        assert_eq!(long_buf, good);

        let long_out = dir.join("long.out");
        let mut file = temp_out(&long_out);
        let mut out = SeekRecording::new(&mut file);
        let mut state = ReuseState::new(&ctx);
        let outcome =
            try_reuse_slice(&mut state.call(&long_path, &target, &mut out).with_reuse(reuse))
                .unwrap();
        assert!(
            matches!(outcome, ChunkOutcome::Reused),
            "with real bytes present the same entry verifies"
        );
        out.flush().unwrap();
        assert_eq!(std::fs::read(&long_out).unwrap(), good);
    }

    /// COR-04: an unreadable reuse source is an unexpected I/O failure and is
    /// recorded in the structured diagnostics; the ordinary MD5 mismatch is
    /// the expected "candidate did not verify" outcome and is not counted.
    #[test]
    fn unexpected_reuse_io_fallbacks_are_recorded_but_digest_mismatches_are_not() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "reuse_diag");
        let src = dir.join("source.bin");
        std::fs::write(&src, b"0123456789abcdef").unwrap();
        let md5 = util::md5::md5_file_slice(&src, 0, 4).unwrap();
        let reuse = reuse_at(&md5, 4, 0);
        let out_path = dir.join("out.bin");
        let ctx = test_ctx();
        let _ = diagnostics::take();

        {
            let mut file = temp_out(&out_path);
            let mut out = SeekRecording::new(&mut file);
            let mismatching = chunk_at("c0", "00000000000000000000000000000000", 0, 4);
            let mut state = ReuseState::new(&ctx);
            let outcome =
                try_reuse_slice(&mut state.call(&src, &mismatching, &mut out).with_reuse(&reuse))
                    .unwrap();
            assert!(matches!(outcome, ChunkOutcome::Download));
        }
        assert_eq!(
            diagnostics::take().reuse_io_fallbacks,
            0,
            "an MD5 mismatch is expected and stays at debug"
        );

        {
            let mut file = temp_out(&out_path);
            let mut out = SeekRecording::new(&mut file);
            let c = chunk_at("c1", &md5, 0, 4);
            let mut state = ReuseState::new(&ctx);
            let outcome = try_reuse_slice(
                &mut state
                    .call(&dir.join("missing.bin"), &c, &mut out)
                    .with_reuse(&reuse),
            )
            .unwrap();
            assert!(matches!(outcome, ChunkOutcome::Download));
        }
        assert_eq!(
            diagnostics::take().reuse_io_fallbacks,
            1,
            "an unreadable source is surfaced"
        );
    }

    /// A read error and a short slice both send the chunk to the download, but
    /// only the short slice leaves a handle the caller can keep. The copy
    /// helper reports them apart so the caller can drop a handle whose read
    /// failed.
    #[test]
    fn a_reuse_copy_tells_a_read_error_from_a_short_slice() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "reuse_copy_errors");
        let out_path = dir.join("out.bin");
        let mut file = temp_out(&out_path);
        let mut out = SeekRecording::new(&mut file);
        let c = chunk_at("c0", "", 0, 4);
        let ctx = test_ctx();
        let mut scratch = Vec::new();
        let mut writes = FileWrites::default();

        let mut failing = FailingReads;
        let failed =
            copy_reuse_pieces(&ctx, &mut writes, &mut failing, &mut out, &mut scratch, &c).unwrap();
        assert!(matches!(failed, ReuseCopy::ReadFailed));

        let mut short = ExactReads::new(vec![0; 2]);
        let truncated =
            copy_reuse_pieces(&ctx, &mut writes, &mut short, &mut out, &mut scratch, &c).unwrap();
        assert!(matches!(truncated, ReuseCopy::Truncated));
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
        let mut state = ReuseState::new(&ctx);

        let first = chunk_at("c0", &md5, 0, 4);
        let outcome =
            try_reuse_slice(&mut state.call(&src, &first, &mut out).with_reuse(&reuse)).unwrap();
        assert!(matches!(outcome, ChunkOutcome::Reused));
        assert!(
            state.src_handle.is_some(),
            "the first reuse keeps the source handle"
        );

        // The removal is allowed to fail on a platform that will not unlink an
        // open file; the held handle is what the next chunk must use either way.
        let _ = std::fs::remove_file(&src);
        let second = chunk_at("c1", &md5, 4, 4);
        let outcome =
            try_reuse_slice(&mut state.call(&src, &second, &mut out).with_reuse(&reuse)).unwrap();
        assert!(
            matches!(outcome, ChunkOutcome::Reused),
            "the held handle serves the later chunk"
        );
        out.flush().unwrap();
        assert_eq!(std::fs::read(&tmp_path).unwrap(), b"89ab89ab");
    }

    /// A read error while hashing the held source drops the handle, the same
    /// way one during the copy does: the chunk falls back to the download and
    /// the next chunk opens the readable path again instead of trusting a
    /// handle whose read already failed.
    ///
    /// The fault is a handle opened without read access: it seeks fine and
    /// refuses every read, which is a deterministic read error on a regular
    /// file on every host. The digest-mismatch case keeps the handle; this one
    /// must not.
    #[test]
    fn a_read_error_on_the_held_source_drops_it() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "reuse_read_error");
        let src = dir.join("source.bin");
        std::fs::write(&src, b"0123456789abcdef").unwrap();
        let md5 = util::md5::md5_file_slice(&src, 0, 4).unwrap();
        let reuse = reuse_at(&md5, 4, 0);
        let out_path = dir.join("out.bin");
        let mut file = temp_out(&out_path);
        let mut out = SeekRecording::new(&mut file);
        let ctx = test_ctx();
        let mut state = ReuseState::new(&ctx);
        state.src_handle = Some(
            std::fs::OpenOptions::new()
                .write(true)
                .open(&src)
                .expect("the source exists"),
        );
        let first = chunk_at("c0", &md5, 0, 4);

        let outcome =
            try_reuse_slice(&mut state.call(&src, &first, &mut out).with_reuse(&reuse)).unwrap();

        assert!(matches!(outcome, ChunkOutcome::Download));
        assert!(
            state.src_handle.is_none(),
            "a failed read leaves no handle for the next chunk"
        );
        assert!(
            out.seeks.is_empty(),
            "the read fails before the download path positions the sink"
        );
        assert_eq!(out.bytes, 0, "a failed read must not write to the sink");
        assert_eq!(
            std::fs::metadata(&out_path).unwrap().len(),
            0,
            "a failed read must not leave bytes behind in the sink"
        );

        // The dropped handle costs one open, not the file's reuse: the next
        // chunk takes the same readable path and copies from it.
        let second = chunk_at("c1", &md5, 4, 4);
        let outcome =
            try_reuse_slice(&mut state.call(&src, &second, &mut out).with_reuse(&reuse)).unwrap();
        assert!(matches!(outcome, ChunkOutcome::Reused));
        assert!(
            state.src_handle.is_some(),
            "the reopened handle is kept for later chunks"
        );
        out.flush().unwrap();
        let written = std::fs::read(&out_path).unwrap();
        assert_eq!(written.len(), 8);
        assert_eq!(&written[4..], b"0123");
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
        let mut state = ReuseState::new(&ctx);

        let outcome =
            try_reuse_slice(&mut state.call(&src, &c, &mut out).with_reuse(&reuse)).unwrap();
        assert!(matches!(outcome, ChunkOutcome::Download));
        assert!(
            state.src_handle.is_none(),
            "a failed open leaves no handle behind"
        );
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
        let mut state = ReuseState::new(&ctx);

        let outcome = try_reuse_slice(
            &mut state
                .call(&dir.join("never-written.bin"), &c, &mut out)
                .with_reuse(&reuse),
        )
        .unwrap();

        assert!(matches!(outcome, ChunkOutcome::Download));
        assert!(
            state.src_handle.is_none(),
            "there is no source to hold open"
        );
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
        let mut state = ReuseState::new(&ctx);

        let outcome =
            try_reuse_slice(&mut state.call(&src, &c, &mut out).with_reuse(&reuse)).unwrap();

        assert!(matches!(outcome, ChunkOutcome::Download));
        assert!(
            state.src_handle.is_some(),
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
        let mut state = ReuseState::new(&ctx);
        state.tmp_len = 8;

        let outcome = try_reuse_slice(&mut state.call(&missing, &c, &mut out)).unwrap();
        assert!(
            matches!(outcome, ChunkOutcome::Resumed),
            "the chunk's end is inside the tmp"
        );

        // The rollback and the tracked resume point move together; the update
        // is the caller's, not a value the test sets by hand.
        rollback_failed_chunk(&mut out, &tmp, &c, &mut state.tmp_len).unwrap();
        assert_eq!(state.tmp_len, 4, "the tracked point follows the truncation");
        let outcome = try_reuse_slice(&mut state.call(&missing, &c, &mut out)).unwrap();
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
    /// or left reserved. The sink reads the ledger at seek time, so the test
    /// asserts the reservation was live *before* the seek and gone after it —
    /// a final figure of zero alone would also fit a path that never reserved.
    /// The seek failure is named with its chunk.
    #[tokio::test]
    async fn a_download_into_an_unseekable_sink_refunds_and_writes_nothing() {
        let mut summary = Summary::new();
        summary.set_download_limit(Some(1_000));
        let mut ctx = test_ctx();
        ctx.summary = Arc::new(summary);
        let c = chunk_at("c9", "d41d8cd98f00b204e9800998ecf8427e", 8, 16);
        let mut out = RefusingSeek::watching(Arc::clone(&ctx.summary));
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
        assert_eq!(
            out.live_at_seek,
            vec![c.compressed_size as u64],
            "the chunk's reservation is live before the seek is attempted"
        );
        assert!(
            out.bytes.is_empty(),
            "no byte may be written after the seek fails"
        );
        assert_eq!(
            ctx.summary.download.live(),
            0,
            "the reservation is given back after the seek fails, with no byte fetched"
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

    /// An intact target with a guarded plan verdict is skipped without the
    /// temp path reading it at all — the digest the scan took is the skip
    /// check. This is the temp half of the verdict sharing; the guard itself is
    /// the one the in-place path applies.
    #[tokio::test]
    async fn a_guarded_plan_verdict_skips_the_temp_check() {
        use crate::repair::scan::ScanReport;
        use crate::test_support::stamp_ago;
        use girpr_chunk_verify::Verdict;

        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "verdict_skip");
        let path = dir.join("f.dat");
        let tmp = dir.join("f.dat_tmp");
        let data = b"abcdef";
        std::fs::write(&path, data).unwrap();
        let Some(stamp) = stamp_ago(std::time::Duration::from_secs(24 * 60 * 60)) else {
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
            rel: "f.dat".into(),
            size: data.len() as i64,
            md5: util::md5::md5_digest(data).to_hex(),
            url_prefix: Arc::from(""),
            chunks: Vec::new(),
        };
        let verdict = Verdict {
            file_md5: util::md5::md5_digest(data),
            dest_ok: Vec::new(),
            source_ok: Vec::new(),
            file_len: data.len() as u64,
            modified: Some(recorded),
        };
        let plan = RepairPlan::from_files("", vec![file.clone()]);
        let report =
            ScanReport::for_test_at(&plan, vec![Some(verdict)], std::time::SystemTime::now());
        let mut ctx = test_ctx();
        ctx.plan = Arc::new(plan);
        ctx.scan = Some(Arc::new(report));

        let outcome = attempt_temp(
            &ctx,
            FileId::new(0),
            &file,
            "",
            &tmp,
            &path,
            &mut FileWrites::default(),
        )
        .await
        .unwrap();
        assert!(matches!(outcome, Inplace::Skipped));
        assert!(!tmp.exists(), "a skipped file never opens a temp file");
    }

    /// The other half of assumption 3: a target written after the plan scan
    /// must not be skipped from that scan's digest. The shared guard sees the
    /// newer mtime, so the temp path falls through to its own check and repair.
    #[tokio::test]
    async fn a_file_written_after_the_scan_is_not_skipped_by_the_temp_path() {
        use crate::repair::scan::ScanReport;
        use crate::test_support::stamp_ago;
        use girpr_chunk_verify::Verdict;

        let dir = girpr_tempdir::TempDir::new("girpr_test_temp", "verdict_stale");
        let path = dir.join("f.dat");
        let tmp = dir.join("f.dat_tmp");
        let data = b"abcdef";
        std::fs::write(&path, data).unwrap();
        let Some(stamp) = stamp_ago(std::time::Duration::from_secs(24 * 60 * 60)) else {
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
            rel: "f.dat".into(),
            size: data.len() as i64,
            md5: util::md5::md5_digest(data).to_hex(),
            url_prefix: Arc::from(""),
            chunks: Vec::new(),
        };
        let verdict = Verdict {
            file_md5: util::md5::md5_digest(data),
            dest_ok: Vec::new(),
            source_ok: Vec::new(),
            file_len: data.len() as u64,
            modified: Some(recorded),
        };
        let plan = RepairPlan::from_files("", vec![file.clone()]);
        let report =
            ScanReport::for_test_at(&plan, vec![Some(verdict)], std::time::SystemTime::now());
        let mut ctx = test_ctx();
        ctx.plan = Arc::new(plan);
        ctx.scan = Some(Arc::new(report));

        // Same length, but written after the scan: the mtime no longer matches
        // the recorded one, so the verdict is stale.
        let mut changed = *data;
        changed[1] ^= 0xff;
        std::fs::write(&path, changed).unwrap();

        let outcome = attempt_temp(
            &ctx,
            FileId::new(0),
            &file,
            "",
            &tmp,
            &path,
            &mut FileWrites::default(),
        )
        .await;
        assert!(
            !matches!(outcome, Ok(Inplace::Skipped)),
            "a post-scan write must be repaired, not skipped from the stale verdict"
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
