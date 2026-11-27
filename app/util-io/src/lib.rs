//! Bounded byte I/O mechanics shared by the two repair writers.
//!
//! The in-place writer copies bytes inside one file and zero-fills its holes;
//! the temp writer copies a verified slice into the `_tmp`. Both assemble their
//! output in pieces of at most [`PIECE`] bytes, and both charge the write
//! ceiling at the same points. This crate owns those piece loops; the callers
//! keep the policy — the ceiling checks, the settled-byte accounting, and the
//! distinct "short / read failed / done" vocabularies each path reports.
//!
//! The reserved-capacity bounded read is *not* reimplemented here:
//! [`capture_bounded`] composes `girpr_util_hash::read_up_to`, so there is one implementation of
//! the "reserve spare capacity, read through a bounded `Take`" mechanics.

use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::OnceLock;

use anyhow::Result;

/// The piece both repair writers copy and zero in.
pub const PIECE: usize = 512 * 1024;

/// The shared zero block for a hole fill: [`PIECE`] zero bytes, allocated once.
pub fn zeros() -> &'static [u8] {
    static ZEROS: OnceLock<Vec<u8>> = OnceLock::new();
    ZEROS.get_or_init(|| vec![0u8; PIECE])
}

/// Grow a reusable scratch buffer to at least `want` bytes and hand back that
/// prefix. A file's pieces are written one at a time, so one buffer per file
/// is enough and is never shrunk between calls.
pub fn scratch(buf: &mut Vec<u8>, want: usize) -> &mut [u8] {
    if buf.len() < want {
        buf.resize(want, 0);
    }
    &mut buf[..want]
}

/// How a bounded copy ended. A caller maps these to its own vocabulary; the
/// leaf deliberately does not guess whether a short source or a read error
/// should fall back to a download.
#[derive(Debug)]
pub enum CopyEnd {
    /// Every byte was read and written.
    Complete,
    /// The source ended before `want` bytes were copied; `have` landed.
    Short { have: u64, want: u64 },
    /// A read failed. The concrete error is kept for the caller to surface.
    Failed(std::io::Error),
}

/// Copy `len` bytes through `rw` in pieces of at most `piece.len()`.
///
/// Each piece is read **before** it is charged, written and settled, so a
/// source that overlaps the destination in the same handle survives the write
/// of its own piece. `charge(n)` runs before the piece lands (the live write
/// ceiling), and `settle(n)` after it lands (the verified write ceiling);
/// both are the caller's policy. A short read or a read error is reported as
/// [`CopyEnd::Short`] / [`CopyEnd::Failed`] with no policy attached.
pub fn copy_bounded<RW>(
    rw: &mut RW,
    len: u64,
    piece: &mut [u8],
    mut charge: impl FnMut(u64) -> Result<()>,
    mut settle: impl FnMut(u64) -> Result<()>,
) -> Result<CopyEnd>
where
    RW: Read + Write,
{
    let mut done = 0u64;
    while done < len {
        let want = ((len - done) as usize).min(piece.len());
        let n = match rw.read(&mut piece[..want]) {
            Ok(n) => n,
            Err(e) => return Ok(CopyEnd::Failed(e)),
        };
        if n == 0 {
            return Ok(CopyEnd::Short {
                have: done,
                want: len,
            });
        }
        charge(n as u64)?;
        rw.write_all(&piece[..n])?;
        settle(n as u64)?;
        done += n as u64;
    }
    Ok(CopyEnd::Complete)
}

/// A single handle with independent read and write cursors. Each operation
/// seeks before touching the handle, so the two cursors cannot disturb one
/// another and an overlapping source is read in full before its piece is
/// written.
struct Positioned<'a, T> {
    inner: &'a mut T,
    read_at: u64,
    write_at: u64,
}

impl<T: Read + Seek> Read for Positioned<'_, T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.seek(SeekFrom::Start(self.read_at))?;
        // Read the whole piece, exactly as the in-file copy's `read_exact`
        // did: a short source is an error, not a partial piece.
        self.inner.read_exact(buf)?;
        self.read_at += buf.len() as u64;
        Ok(buf.len())
    }
}

impl<T: Write + Seek> Write for Positioned<'_, T> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.seek(SeekFrom::Start(self.write_at))?;
        let n = self.inner.write(buf)?;
        self.write_at += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Copy `len` bytes from `src` to `dst` inside one handle, in pieces.
///
/// This is the in-place writer's copy: the source and destination offsets live
/// in the same file, so every piece is read before it is written. A short
/// source or a read error is a hard error here, matching the old `read_exact`
/// loop; the caller keeps its own failure vocabulary for those errors.
pub fn copy_within_bounded<T>(
    rw: &mut T,
    src: u64,
    dst: u64,
    len: u64,
    piece: &mut [u8],
    charge: impl FnMut(u64) -> Result<()>,
    settle: impl FnMut(u64) -> Result<()>,
) -> Result<()>
where
    T: Read + Write + Seek,
{
    if len == 0 {
        return Ok(());
    }
    let mut positioned = Positioned {
        inner: rw,
        read_at: src,
        write_at: dst,
    };
    match copy_bounded(&mut positioned, len, piece, charge, settle)? {
        CopyEnd::Complete => Ok(()),
        CopyEnd::Short { have, want } => {
            anyhow::bail!("in-file copy ended after {have} of {want} bytes")
        }
        CopyEnd::Failed(e) => Err(e.into()),
    }
}

/// Write `len` zero bytes at `offset` in pieces from the shared zero block.
///
/// The whole length is charged once, before the first piece lands — a hole is
/// reserved as a whole, so a ceiling that trips cannot leave it half-filled.
/// Each piece is then written and settled at the call site's chosen points.
pub fn zero_bounded<W>(
    out: &mut W,
    offset: u64,
    len: u64,
    charge: impl FnOnce(u64) -> Result<()>,
    mut settle: impl FnMut(u64) -> Result<()>,
) -> Result<()>
where
    W: Write + Seek,
{
    if len == 0 {
        return Ok(());
    }
    charge(len)?;
    let zeros = zeros();
    let mut left = len;
    let mut pos = offset;
    while left > 0 {
        let n = (left as usize).min(zeros.len());
        out.seek(SeekFrom::Start(pos))?;
        out.write_all(&zeros[..n])?;
        settle(n as u64)?;
        left -= n as u64;
        pos += n as u64;
    }
    Ok(())
}

/// How a bounded capture read ended. A caller classifies a short source or a
/// failed seek/read in its own terms; the leaf only reports what happened.
#[derive(Debug)]
pub enum Capture {
    /// The whole range was read into `buf`.
    Complete,
    /// The source ended before the range did. Bytes read so far are in `buf`.
    Short { have: u64, want: u64 },
    /// A seek or read failed. The concrete error is kept for the caller.
    Failed(std::io::Error),
}

/// Read up to `len` bytes into reserved spare capacity, in [`PIECE`] pieces.
///
/// The reserved-capacity read itself is `girpr_util_hash::read_up_to`: each
/// piece is appended through a bounded `Take` so spare capacity is never
/// zero-filled before the read overwrites it. `buf` is cleared first, so a
/// reused staging buffer carries only the bytes of the current range. An
/// unrepresentable length or a failed reservation is a short capture, so the
/// caller falls back rather than aborting on a corrupt manifest.
pub fn capture_bounded<R: Read>(f: &mut R, len: u64, buf: &mut Vec<u8>) -> Capture {
    let want = match usize::try_from(len) {
        Ok(n) => n,
        Err(_) => return Capture::Short { have: 0, want: len },
    };
    if girpr_util_hash::reserve_exact(buf, want).is_err() {
        return Capture::Short { have: 0, want: len };
    }
    buf.clear();
    let mut remaining = len;
    while remaining > 0 {
        let piece = (remaining as usize).min(PIECE);
        let start = buf.len();
        let n = match girpr_util_hash::read_up_to(f, buf, piece) {
            Ok(n) => n,
            Err(e) => {
                buf.truncate(start);
                // A genuine read failure is the `io::Error` `read_to_end`
                // returned; keep it intact so this path's `Failed` variant is
                // still the concrete error the caller can surface.
                let e = match e.downcast::<std::io::Error>() {
                    Ok(e) => e,
                    Err(e) => std::io::Error::other(e.to_string()),
                };
                return Capture::Failed(e);
            }
        };
        if n == 0 {
            return Capture::Short {
                have: len - remaining,
                want: len,
            };
        }
        remaining -= n as u64;
    }
    Capture::Complete
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn noop(_: u64) -> Result<()> {
        Ok(())
    }

    #[test]
    fn a_bounded_copy_keeps_read_before_write_for_an_overlapping_piece() {
        // One piece, source and destination overlapping: the read must finish
        // before the write, or the destination would clobber the source.
        let original: Vec<u8> = (0..16).map(|i| i as u8).collect();
        let mut rw = Cursor::new(original.clone());
        let mut piece = [0u8; 8];
        let charged = std::cell::RefCell::new(Vec::new());
        let settled = std::cell::RefCell::new(Vec::new());
        copy_within_bounded(
            &mut rw,
            0,
            2,
            4,
            &mut piece,
            |n| {
                charged.borrow_mut().push(n);
                Ok(())
            },
            |n| {
                settled.borrow_mut().push(n);
                Ok(())
            },
        )
        .unwrap();
        let mut want = original.clone();
        want[2..6].copy_from_slice(&original[0..4]);
        assert_eq!(rw.into_inner(), want);
        assert_eq!(charged.into_inner(), vec![4]);
        assert_eq!(settled.into_inner(), vec![4]);
    }

    #[test]
    fn a_bounded_copy_reports_a_short_source_and_a_read_error_apart() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("read failed"))
            }
        }
        impl Write for Failing {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let mut short = Cursor::new(vec![1u8, 2, 3]);
        let mut piece = [0u8; 8];
        match copy_bounded(&mut short, 5, &mut piece, noop, noop).unwrap() {
            CopyEnd::Short { have, want } => {
                assert_eq!((have, want), (3, 5));
            }
            other => panic!("expected a short copy, got {other:?}"),
        }
        let mut failing = Failing;
        assert!(matches!(
            copy_bounded(&mut failing, 5, &mut piece, noop, noop).unwrap(),
            CopyEnd::Failed(_)
        ));
    }

    /// A writer that accepts only part of a piece still gets the whole copy:
    /// `write_all` drains it, while charge and settle stay once per read piece.
    /// A refused charge stops before the next piece lands, so the ceiling is
    /// charged at exactly the boundary the caller chose.
    #[test]
    fn a_bounded_copy_survives_short_writes_and_refuses_after_a_charge_failure() {
        struct ShortWrites {
            data: Vec<u8>,
            calls: usize,
        }
        impl Write for ShortWrites {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.calls += 1;
                let n = buf.len().min(2);
                self.data.extend_from_slice(&buf[..n]);
                Ok(n)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        struct Pair<'a, R: Read, W: Write> {
            r: &'a mut R,
            w: &'a mut W,
        }
        impl<R: Read, W: Write> Read for Pair<'_, R, W> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.r.read(buf)
            }
        }
        impl<R: Read, W: Write> Write for Pair<'_, R, W> {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.w.write(buf)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                self.w.flush()
            }
        }

        let source: Vec<u8> = (0..8).map(|i| i as u8).collect();
        let mut piece = [0u8; 4];
        let charged = std::cell::RefCell::new(Vec::new());
        let settled = std::cell::RefCell::new(Vec::new());
        {
            let mut src = Cursor::new(source.clone());
            let mut out = ShortWrites {
                data: Vec::new(),
                calls: 0,
            };
            let mut rw = Pair {
                r: &mut src,
                w: &mut out,
            };
            let end = copy_bounded(
                &mut rw,
                8,
                &mut piece,
                |n| {
                    charged.borrow_mut().push(n);
                    Ok(())
                },
                |n| {
                    settled.borrow_mut().push(n);
                    Ok(())
                },
            )
            .unwrap();
            assert!(matches!(end, CopyEnd::Complete));
            assert!(out.calls > 2, "write_all drains a partial writer");
            assert_eq!(out.data, source);
        }
        assert_eq!(charged.into_inner(), vec![4, 4]);
        assert_eq!(settled.into_inner(), vec![4, 4]);

        let mut src = Cursor::new(source.clone());
        let mut out = ShortWrites {
            data: Vec::new(),
            calls: 0,
        };
        let mut rw = Pair {
            r: &mut src,
            w: &mut out,
        };
        let mut charges = 0;
        let refused = copy_bounded(
            &mut rw,
            8,
            &mut piece,
            |_| {
                charges += 1;
                if charges == 2 {
                    Err(anyhow::anyhow!("write ceiling reached"))
                } else {
                    Ok(())
                }
            },
            noop,
        );
        assert!(refused.is_err());
        assert_eq!(
            out.data,
            source[..4],
            "the charged piece landed; the refused one did not"
        );
    }

    #[test]
    fn a_zero_fill_charges_once_and_settles_each_piece() {
        let mut out = Cursor::new(vec![0xAAu8; 16]);
        let charged = std::cell::RefCell::new(Vec::new());
        let settled = std::cell::RefCell::new(Vec::new());
        zero_bounded(
            &mut out,
            2,
            10,
            |n| {
                charged.borrow_mut().push(n);
                Ok(())
            },
            |n| {
                settled.borrow_mut().push(n);
                Ok(())
            },
        )
        .unwrap();
        let bytes = out.into_inner();
        assert_eq!(&bytes[2..12], &[0u8; 10]);
        assert_eq!(&bytes[..2], &[0xAA, 0xAA]);
        assert_eq!(charged.into_inner(), vec![10]);
        assert_eq!(settled.into_inner(), vec![10]);
    }

    #[test]
    fn a_capture_reads_only_the_range_through_reserved_capacity() {
        let data: Vec<u8> = (0..PIECE + 16).map(|i| (i % 251) as u8).collect();
        let mut f = Cursor::new(data.clone());
        let mut buf = vec![0xAAu8; 4];
        match capture_bounded(&mut f, (PIECE + 4) as u64, &mut buf) {
            Capture::Complete => {}
            other => panic!("expected a complete capture, got {other:?}"),
        }
        assert_eq!(buf, &data[..PIECE + 4]);
    }
}
