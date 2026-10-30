use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MD5_BUF: usize = 512 * 1024;

/// The MD5 of the empty file — what a missing (or empty) file hashes to.
///
/// Named rather than inlined so the "the file is not there" answer and the
/// "the file is there and empty" answer cannot drift apart.
pub const EMPTY_MD5: &str = "d41d8cd98f00b204e9800998ecf8427e";

/// A byte range of a file, with the MD5 the manifest expects for its content.
///
/// `(offset, len, expected_md5)`. The MD5 is the *uncompressed* chunk MD5 out
/// of the Sophon manifest, so a range is "correct" exactly when the bytes at
/// `offset..offset+len` hash to it.
pub type ScanRange<'a> = (u64, u64, &'a str);

/// What one read pass over a file answers.
///
/// The scan exists so the repair loop needs **one** read per file instead of
/// three: the whole-file MD5 (the skip check), the per-chunk verdict ("is
/// this chunk already correct where it belongs?") and the per-source verdict
/// ("is this reuse source intact?") all come out of the same pass over the
/// bytes. Every answer is derived from bytes that were read anyway, so asking
/// for them costs hashing and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    /// Whole-file MD5 of what was on disk. The MD5 of the empty file when the
    /// file was absent.
    pub file_md5: String,
    /// Bytes on disk (`0` when the file was absent).
    pub file_len: u64,
    /// Per destination range, **in the order given**: is the range already
    /// correct where it belongs?
    pub dest_ok: Vec<bool>,
    /// Per candidate source range, **in the order given**: is the source
    /// intact?
    pub source_ok: Vec<bool>,
}

impl Scan {
    /// The scan of a file that is not there.
    ///
    /// "Nothing is correct" is the *answer*, not a failure: a file that does
    /// not exist has no correct chunk and no intact reuse source, and the
    /// repair of it is a full write. A missing file therefore never fails the
    /// scan.
    pub fn absent(dest_len: usize, source_len: usize) -> Self {
        Self {
            file_md5: EMPTY_MD5.to_string(),
            file_len: 0,
            dest_ok: vec![false; dest_len],
            source_ok: vec![false; source_len],
        }
    }
}

/// One byte range being hashed, with the state of its digest.
struct RangeState {
    offset: u64,
    len: u64,
    /// Lowercased expected MD5 — the manifest's spelling is not canonical.
    expect: String,
    ctx: md5::Context,
    /// Bytes of this range actually read. A range that runs past the end of
    /// the file is short, and a short range is never correct.
    seen: u64,
}

impl RangeState {
    /// Is the range exactly what the manifest says it should be?
    ///
    /// Three things have to hold: every byte of the range was on disk, the
    /// range hashes to the expected MD5, and the expectation was not empty
    /// (an empty MD5 is "unknown", not "matches").
    fn is_ok(&self) -> bool {
        self.seen == self.len
            && !self.expect.is_empty()
            && format!("{:x}", self.ctx.clone().compute()) == self.expect
    }
}

/// One read pass: the whole-file MD5 plus a verdict per byte range.
///
/// `dest` are the ranges the file *should* have (the manifest's chunk
/// destinations); `source` are the ranges a chunk could be *reused* from
/// (the local reuse map's offsets). The two lists are independent — the same
/// range can be in both, and a source is usually somewhere else in the file
/// — and the verdicts are returned in the order the ranges were given.
///
/// A file that is not there is **not** an error: the scan says nothing is
/// correct (see [`Scan::absent`]). A file that is shorter than a range makes
/// that range incorrect, because a short range cannot hash to anything.
pub fn scan_file(path: &Path, dest: &[ScanRange<'_>], source: &[ScanRange<'_>]) -> Result<Scan> {
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Scan::absent(dest.len(), source.len()));
        }
        Err(e) => return Err(e).with_context(|| format!("open {}", path.display())),
    };
    scan_reader(&mut f, dest, source)
}

/// [`scan_file`], over a handle the caller already holds.
///
/// The in-place repair opens its target `read+write` and then has to write to
/// it, so the scan cannot own the handle: it borrows it, and the caller seeks
/// before every write afterwards.
///
/// The scan always starts at the beginning of the file: the handle's current
/// position is whatever the caller's last read or write left it at, and a
/// scan that started there would hash half a file.
pub fn scan_reader(f: &mut File, dest: &[ScanRange<'_>], source: &[ScanRange<'_>]) -> Result<Scan> {
    f.seek(SeekFrom::Start(0))?;
    let mut ranges: Vec<RangeState> = Vec::new();
    let mut index: HashMap<(u64, u64, String), usize> = HashMap::new();
    let mut dest_idx = Vec::with_capacity(dest.len());
    let mut source_idx = Vec::with_capacity(source.len());
    // The two lists are interned separately, so a range that appears in both
    // shares one digest and each list's verdict lands on the right range.
    for (offset, len, md5) in dest.iter() {
        dest_idx.push(intern(&mut ranges, &mut index, *offset, *len, md5));
    }
    for (offset, len, md5) in source.iter() {
        source_idx.push(intern(&mut ranges, &mut index, *offset, *len, md5));
    }

    // The sweep walks the file once, in 512 KiB blocks, and feeds every range
    // the bytes that fall inside it. The ranges are sorted by offset so the
    // block's overlap with them is a contiguous walk, not a scan of the list.
    let mut order: Vec<usize> = (0..ranges.len()).collect();
    order.sort_by_key(|&i| (ranges[i].offset, ranges[i].len));

    let mut whole = md5::Context::new();
    let mut buf = vec![0u8; MD5_BUF];
    let mut pos: u64 = 0;
    let mut total: u64 = 0;
    let mut first = 0usize;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        whole.consume(&buf[..n]);
        let block_end = pos + n as u64;
        // Ranges that ended before this block are done with; the rest are
        // visited in order and take the part of themselves inside the block.
        while first < order.len() {
            let r = &ranges[order[first]];
            if r.offset + r.len <= pos {
                first += 1;
            } else {
                break;
            }
        }
        let mut j = first;
        while j < order.len() {
            let ri = order[j];
            let r = &ranges[ri];
            if r.offset >= block_end {
                break;
            }
            let start = r.offset.max(pos);
            let end = (r.offset + r.len).min(block_end);
            if end > start {
                let a = (start - pos) as usize;
                let b = (end - pos) as usize;
                ranges[ri].ctx.consume(&buf[a..b]);
                ranges[ri].seen += end - start;
            }
            j += 1;
        }
        pos = block_end;
        total += n as u64;
    }

    Ok(Scan {
        file_md5: format!("{:x}", whole.compute()),
        file_len: total,
        dest_ok: dest_idx.iter().map(|&i| ranges[i].is_ok()).collect(),
        source_ok: source_idx.iter().map(|&i| ranges[i].is_ok()).collect(),
    })
}

/// Find or create the range state for `(offset, len, md5)`.
///
/// Two ranges are the same range when they cover the same bytes **and**
/// expect the same content; the same bytes with a different expectation are
/// two different questions, so they get two digests.
fn intern(
    ranges: &mut Vec<RangeState>,
    index: &mut HashMap<(u64, u64, String), usize>,
    offset: u64,
    len: u64,
    md5: &str,
) -> usize {
    let expect = md5.to_ascii_lowercase();
    let key = (offset, len, expect.clone());
    if let Some(&i) = index.get(&key) {
        return i;
    }
    let i = ranges.len();
    ranges.push(RangeState {
        offset,
        len,
        expect,
        ctx: md5::Context::new(),
        seen: 0,
    });
    index.insert(key, i);
    i
}

/// Read `len` bytes at `offset` into a buffer.
///
/// The in-place repair's capture pass: the bytes of a reuse source, read
/// before anything overwrites them. A short read is an error rather than a
/// silent truncation — the caller has to know it did not get what it asked
/// for.
pub fn read_range(f: &mut File, offset: u64, len: u64) -> Result<Vec<u8>> {
    f.seek(SeekFrom::Start(offset))?;
    // `len` comes from the server's `uncompressed_size`, so convert it instead
    // of truncating, and reserve fallibly: a corrupt manifest must fail this
    // read, not abort the process with an allocation failure.
    let len = usize::try_from(len).with_context(|| format!("range of {len} bytes is too large"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .with_context(|| format!("reserve {len} bytes for a file range"))?;
    out.resize(len, 0);
    f.read_exact(&mut out)?;
    Ok(out)
}

/// Lowercase hex MD5 of a file.
pub fn md5_file(path: &Path) -> Result<String> {
    let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut buf = vec![0u8; MD5_BUF];
    let mut ctx = md5::Context::new();
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ctx.consume(&buf[..n]);
    }
    Ok(format!("{:x}", ctx.compute()))
}

/// MD5 of a byte slice range of a file [offset, offset+len).
///
/// Allocates its own read buffer; callers that hash several ranges of one
/// file use [`md5_file_slice_with_buf`] to reuse one.
pub fn md5_file_slice(path: &Path, offset: u64, len: u64) -> Result<String> {
    let mut buf = vec![0u8; MD5_BUF];
    md5_file_slice_with_buf(path, offset, len, &mut buf)
}

/// MD5 of a byte slice range of a file [offset, offset+len), reusing `buf`.
///
/// `buf` is grown to at least [`MD5_BUF`] and is never shrunk, so one buffer
/// serves every range of a file instead of one allocation per range.
pub fn md5_file_slice_with_buf(
    path: &Path,
    offset: u64,
    len: u64,
    buf: &mut Vec<u8>,
) -> Result<String> {
    let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    md5_reader_slice(&mut f, offset, len, buf)
}

/// MD5 of a byte slice range of a file the caller already has open, reusing
/// `buf`.
///
/// Seeks `f` to `offset` first and leaves it at the end of the range (or at
/// end of file, for a short read), so a caller that wants to copy the same
/// bytes afterwards seeks back once instead of opening the file again.
pub fn md5_reader_slice(f: &mut File, offset: u64, len: u64, buf: &mut Vec<u8>) -> Result<String> {
    f.seek(SeekFrom::Start(offset))?;
    if buf.len() < MD5_BUF {
        buf.resize(MD5_BUF, 0);
    }
    let mut ctx = md5::Context::new();
    let mut remaining = len;
    while remaining > 0 {
        let want = (remaining as usize).min(buf.len());
        let n = f.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        ctx.consume(&buf[..n]);
        remaining -= n as u64;
    }
    Ok(format!("{:x}", ctx.compute()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(data: &[u8]) -> String {
        format!("{:x}", md5::compute(data))
    }

    /// Ranges with **owned** MD5s, so a test can build a list without
    /// borrowing a temporary. [`Ranges::view`] hands out the borrowed form the
    /// scan takes.
    #[derive(Default)]
    struct Ranges {
        owned: Vec<(u64, u64, String)>,
    }

    impl Ranges {
        fn of(list: &[(u64, u64, String)]) -> Self {
            Self {
                owned: list.to_vec(),
            }
        }

        fn view(&self) -> Vec<ScanRange<'_>> {
            self.owned
                .iter()
                .map(|(o, l, m)| (*o, *l, m.as_str()))
                .collect()
        }
    }

    /// A file of four blocks, each with its own content, so a per-range
    /// verdict can be checked one block at a time.
    fn four_blocks() -> Vec<u8> {
        let mut v = Vec::new();
        for (i, b) in b"ABCD".iter().enumerate() {
            v.extend(std::iter::repeat_n(*b, 64 + i));
        }
        v
    }

    #[test]
    fn the_empty_md5_constant_is_the_empty_files_md5() {
        assert_eq!(hex(b""), EMPTY_MD5);
    }

    #[test]
    fn an_intact_file_answers_every_range_correctly() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "intact");
        let data = four_blocks();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        let dest = Ranges::of(&[(0, 64, hex(&data[0..64])), (64, 65, hex(&data[64..129]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.file_len, data.len() as u64);
        assert_eq!(scan.file_md5, hex(&data));
        assert_eq!(scan.dest_ok, vec![true, true]);
        assert!(scan.source_ok.is_empty());
    }

    #[test]
    fn one_corrupt_chunk_leaves_the_other_chunk_correct() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "one_bad");
        let good = four_blocks();
        let mut bad = good.clone();
        // Flip a byte inside the second block only.
        bad[70] ^= 0xFF;
        std::fs::write(dir.join("f.dat"), &bad).unwrap();
        let dest = Ranges::of(&[(0, 64, hex(&good[0..64])), (64, 65, hex(&good[64..129]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(
            scan.dest_ok,
            vec![true, false],
            "only the block that was actually damaged is wrong"
        );
        assert_ne!(scan.file_md5, hex(&good), "the whole file is wrong too");
    }

    #[test]
    fn a_truncated_file_makes_every_range_past_its_end_wrong() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "truncated");
        let good = four_blocks();
        // Only the first 100 bytes are on disk.
        std::fs::write(dir.join("f.dat"), &good[..100]).unwrap();
        let dest = Ranges::of(&[(0, 64, hex(&good[0..64])), (64, 65, hex(&good[64..129]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.file_len, 100);
        assert_eq!(scan.dest_ok, vec![true, false]);
    }

    #[test]
    fn an_absent_file_is_not_an_error_and_nothing_is_correct() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "absent");
        let dest = Ranges::of(&[(0, 64, EMPTY_MD5.to_string())]);
        let source = Ranges::of(&[(0, 64, EMPTY_MD5.to_string())]);
        let scan = scan_file(&dir.join("nope.dat"), &dest.view(), &source.view()).unwrap();
        assert_eq!(scan.file_md5, EMPTY_MD5);
        assert_eq!(scan.file_len, 0);
        assert_eq!(scan.dest_ok, vec![false]);
        assert_eq!(scan.source_ok, vec![false]);
    }

    #[test]
    fn ranges_may_be_given_in_any_order() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "unsorted");
        let data = four_blocks();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        // Descending by offset: the scan must not care.
        let dest = Ranges::of(&[(64, 65, hex(&data[64..129])), (0, 64, hex(&data[0..64]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.dest_ok, vec![true, true]);
    }

    #[test]
    fn a_range_can_be_both_a_destination_and_a_source() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "both");
        let data = four_blocks();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        let dest = Ranges::of(&[(0, 64, hex(&data[0..64]))]);
        let source = Ranges::of(&[(0, 64, hex(&data[0..64]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &source.view()).unwrap();
        assert_eq!(scan.dest_ok, vec![true]);
        assert_eq!(scan.source_ok, vec![true]);
    }

    #[test]
    fn a_source_elsewhere_in_the_file_is_verdicted_on_its_own_bytes() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "source");
        let data = four_blocks();
        // The second block is damaged, so it cannot serve as a source, and the
        // third block is intact and can.
        let mut bad = data.clone();
        bad[70] ^= 0xFF;
        std::fs::write(dir.join("f.dat"), &bad).unwrap();
        let source = Ranges::of(&[
            (64, 65, hex(&data[64..129])),
            (128, 66, hex(&data[128..194])),
        ]);
        let scan = scan_file(&dir.join("f.dat"), &[], &source.view()).unwrap();
        assert_eq!(scan.source_ok, vec![false, true]);
    }

    #[test]
    fn a_range_past_the_end_of_the_file_is_never_correct() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "past_end");
        let data = four_blocks();
        std::fs::write(dir.join("f.dat"), &data[..100]).unwrap();
        let dest = Ranges::of(&[(200, 32, hex(&data[200..232]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.dest_ok, vec![false]);
    }

    #[test]
    fn an_empty_expectation_is_never_correct() {
        // An empty MD5 is "unknown", not "matches" — a manifest that carries
        // one must not be read as a free pass.
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "empty_expect");
        std::fs::write(dir.join("f.dat"), b"abc").unwrap();
        let dest = Ranges::of(&[(0, 3, String::new())]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.dest_ok, vec![false]);
    }

    #[test]
    fn the_expected_md5_is_compared_case_insensitively() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "case");
        let data = four_blocks();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        let dest = Ranges::of(&[(0, 64, hex(&data[0..64]).to_ascii_uppercase())]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.dest_ok, vec![true]);
    }

    #[test]
    fn a_file_larger_than_one_block_is_scanned_across_the_boundary() {
        // The sweep is what makes "one read, every answer" true; a range that
        // straddles the 512 KiB block boundary is where an off-by-one would
        // show up.
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "boundary");
        let data: Vec<u8> = (0..(MD5_BUF + 1000)).map(|i| (i % 251) as u8).collect();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        let start = MD5_BUF as u64 - 10;
        let dest = Ranges::of(&[(start, 20, hex(&data[start as usize..start as usize + 20]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.dest_ok, vec![true]);
        assert_eq!(scan.file_len, data.len() as u64);
    }

    #[test]
    fn read_range_returns_the_bytes_at_the_offset() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "read_range");
        let data = four_blocks();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        let mut f = File::open(dir.join("f.dat")).unwrap();
        let got = read_range(&mut f, 64, 8).unwrap();
        assert_eq!(got, &data[64..72]);
    }

    #[test]
    fn read_range_refuses_a_length_it_cannot_reserve() {
        // A server-supplied size must become an error, not an abort: neither
        // the `usize` conversion nor the fallible reservation may panic.
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "read_range_big");
        std::fs::write(dir.join("f.dat"), b"abc").unwrap();
        let mut f = File::open(dir.join("f.dat")).unwrap();
        assert!(read_range(&mut f, 0, u64::MAX).is_err());
    }
}
