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
    reserve_write,
};
use crate::repair::scope::ledger::StoppedBy;
use crate::sophon::{self, SophonChunk};
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
        util::FileLen::Present(_) => match util::md5_file(final_path) {
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
    if have > file.size as u64 {
        tracing::debug!(tmp_bytes = have, "tmp oversize -> truncate");
        f.set_len(file.size as u64)?;
    }
    tracing::debug!(
        tmp_bytes = f
            .metadata()
            .with_context(|| format!("stat tmp {}", tmp_path.display()))?
            .len(),
        expect_size = file.size,
        "tmp resume point"
    );
    let reuse_entries = ctx.local_map.get(&file.rel);
    for c in &file.chunks {
        match try_reuse_slice(ctx, final_path, c, reuse_entries, &mut f, tmp_path, writes)? {
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
                    if let Err(rollback) = rollback_tmp(&mut f, tmp_path, c.offset.max(0) as u64) {
                        return Err(chunk_failure_after_rollback(tmp_path, rollback, e));
                    }
                    return Err(e);
                }
            }
        }
    }
    f.flush()?;
    drop(f);
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
    src: &std::path::Path,
    c: &SophonChunk,
    reuse_entries: Option<&Vec<(String, i64, i64)>>,
    out: &mut File,
    tmp_path: &std::path::Path,
    writes: &mut FileWrites,
) -> Result<ChunkOutcome> {
    // The manifest controls both numbers; a corrupt pair must fail this file
    // normally rather than panic (release builds would wrap here).
    let end = u64::try_from(c.offset)
        .ok()
        .zip(u64::try_from(c.uncompressed_size).ok())
        .and_then(|(offset, size)| offset.checked_add(size))
        .with_context(|| {
            format!(
                "chunk {} has an invalid offset/size: {} + {}",
                c.id, c.offset, c.uncompressed_size
            )
        })?;
    let cur_len = out
        .metadata()
        .with_context(|| format!("stat tmp {} before reuse", tmp_path.display()))?
        .len();
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

    match util::md5_file_slice(src, *ro as u64, c.uncompressed_size as u64) {
        Ok(h) if h.eq_ignore_ascii_case(&c.uncompressed_md5) => {
            tracing::trace!(chunk = %c.id, src_offset = ro, "reuse slice md5 ok -> copy");
            let mut sf = File::open(src)?;
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
pub(super) async fn write_downloaded_chunk(
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
    // What the fetch really spent, across every attempt: the checks below run
    // after the stream ends, so on failure the bytes of the attempt that died
    // mid-body would otherwise be lost. They crossed the wire, and the
    // ceiling is a ceiling: a settle that pushes the live figure past it trips
    // the stop right there, even if this was the run's last chunk.
    let mut consumed = 0u64;
    let digest = match sophon::stream_chunk_to(
        &ctx.http,
        ctx.sleep.as_ref(),
        prefix,
        &c.id,
        expected,
        reserved,
        &mut consumed,
        &mut sink,
    )
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
    if !digest.md5.eq_ignore_ascii_case(&c.uncompressed_md5) {
        tracing::debug!(chunk = %c.id, expect_md5 = %c.uncompressed_md5, actual_md5 = %digest.md5, "chunk md5 mismatch");
        anyhow::bail!("chunk {} md5 mismatch", c.id);
    }
    note_write(ctx, writes, sink.written())?;
    stats.chunks_downloaded += 1;
    Ok(())
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
    let h = util::md5_file(tmp_path)?;
    tracing::debug!(tmp_size = len, tmp_md5 = %h, "after: tmp ready for promote");
    if !h.eq_ignore_ascii_case(&file.md5) {
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

    /// A stat failure on the `_tmp` used to be read as "0 bytes" by
    /// `util::file_len`, so `promote_tmp` answered "tmp length 0 != expected N"
    /// for a tmp that was not empty and discarded the real `io::Error`.
    ///
    /// The seam is a path that cannot be statted: `foo.dat_tmp` under a
    /// **regular file** is `ENOTDIR` on every platform, which is exactly the
    /// `io::Error` the old `unwrap_or(0)` threw away. The error must name the
    /// stat and the path.
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
}
