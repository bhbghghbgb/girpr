use anyhow::{Context, Result};
use md5::Digest;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MD5_BUF: usize = 512 * 1024;

// One read buffer per thread, allocated on first use and reused by every hash
// and scan on that thread. Repair file tasks run to completion on a blocking
// pool thread, so this is one allocation per worker rather than one per call.
// Callers that own a task's whole lifetime can pass their own buffer to the
// `*_with_buf` variants instead of going through this pool.
std::thread_local! {
    static MD5_SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Borrow this thread's reusable hash buffer, sized to the production block.
///
/// The buffer is filled with zeros **once**, when a thread first hashes; every
/// later call reuses those bytes and returns them to this pool, so there is no
/// per-call allocation or zero-fill. The `_with_buf` helpers only ever read the
/// first `n` bytes returned by `read`, so the untouched tail stays initialized
/// and unobserved.
fn with_md5_scratch<R>(f: impl FnOnce(&mut [u8]) -> R) -> R {
    MD5_SCRATCH.with(|cell| {
        let mut buf = cell.borrow_mut();
        if buf.len() < MD5_BUF {
            buf.resize(MD5_BUF, 0);
        }
        f(&mut buf)
    })
}

/// Read up to `want` bytes from `f` into `buf`'s spare capacity, without a
/// zero-fill pass first.
///
/// `Read::read_to_end` over a bounded `Take` appends straight into the `Vec`'s
/// spare capacity (it grows the length by only the bytes actually read), so a
/// reserved scratch buffer is refilled without ever memsetting bytes the read
/// immediately overwrites. `buf` keeps its allocation across calls. The `Take`
/// limit makes a short source end the read exactly as a failed `read_exact`
/// would; the caller checks the byte count.
fn read_into_vec(f: &mut File, buf: &mut Vec<u8>, want: usize) -> Result<usize> {
    let have = buf.capacity() - buf.len();
    if have < want {
        buf.try_reserve_exact(want - have)
            .with_context(|| format!("reserve {want} bytes for a file read"))?;
    }
    let n = (&mut *f).take(want as u64).read_to_end(buf)?;
    Ok(n)
}

// The strict 16-byte `Md5` newtype and the empty-file oracle come from the
// shared digest mechanics. girpr keeps the semantics — `EMPTY` means "unknown,
// not a match", a non-32-digit string is not a digest — and everything below
// that relies on them; only the hex spelling and the fixed-width type are
// sourced from the leaf crate.
pub use girpr_digest::{EMPTY_MD5, Md5};

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
    /// file was absent. Kept as a digest: callers compare bytes, and only a
    /// report or log turns it back into hex.
    pub file_md5: Md5,
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
            file_md5: Md5::EMPTY,
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
    /// The expected MD5, parsed once — the manifest's spelling is not
    /// canonical. `None` when the expectation is empty (or otherwise not a
    /// hex digest): "unknown", not "matches".
    expect: Option<Md5>,
    ctx: md5::Md5,
    /// Bytes of this range actually read. A range that runs past the end of
    /// the file is short, and a short range is never correct.
    seen: u64,
    /// The finished digest, stored once after the read loop. Keeping bytes
    /// rather than the context means a range interned into both verdict lists
    /// is finalized once, and `is_ok` never clones the whole per-range context
    /// just to answer a verdict.
    digest: Option<Md5>,
}

impl RangeState {
    /// Is the range exactly what the manifest says it should be?
    ///
    /// Three things have to hold: every byte of the range was on disk, there
    /// *is* an expectation (an empty MD5 is "unknown", not "matches"), and the
    /// range hashes to it.
    fn is_ok(&self) -> bool {
        let Some(expect) = self.expect else {
            return false;
        };
        self.seen == self.len && self.digest == Some(expect)
    }

    /// Finalize the digest once, after the last byte has been fed in.
    fn finalize(&mut self) {
        let ctx = std::mem::replace(&mut self.ctx, md5::Md5::new());
        self.digest = Some(Md5::from(ctx.finalize()));
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
    with_md5_scratch(|buf| scan_reader_with_buf(f, dest, source, buf))
}

/// [`scan_reader`] with a caller-supplied read buffer.
///
/// The sweep's arithmetic is driven by the bytes actually read, so a smaller
/// buffer crosses the same range boundaries as the production-sized one. A
/// caller that keeps one buffer for the life of a task can call this directly
/// instead of going through the per-thread pool [`scan_reader`] uses.
pub fn scan_reader_with_buf(
    f: &mut File,
    dest: &[ScanRange<'_>],
    source: &[ScanRange<'_>],
    buf: &mut [u8],
) -> Result<Scan> {
    f.seek(SeekFrom::Start(0))?;
    let mut ranges: Vec<RangeState> = Vec::new();
    let mut index: HashMap<(u64, u64, Option<Md5>), usize> = HashMap::new();
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
    // Ranges reach us in chunk order (already ascending by offset) on the hot
    // path, so detect that and skip the sort there; the public API still takes
    // ranges in any order, so the sort remains for that case.
    let mut order: Vec<usize> = (0..ranges.len()).collect();
    if !order.is_sorted_by_key(|&i| (ranges[i].offset, ranges[i].len)) {
        order.sort_by_key(|&i| (ranges[i].offset, ranges[i].len));
    }

    let mut whole = md5::Md5::new();
    let mut pos: u64 = 0;
    let mut total: u64 = 0;
    let mut first = 0usize;
    loop {
        let n = f.read(&mut buf[..])?;
        if n == 0 {
            break;
        }
        whole.update(&buf[..n]);
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
                ranges[ri].ctx.update(&buf[a..b]);
                ranges[ri].seen += end - start;
            }
            j += 1;
        }
        pos = block_end;
        total += n as u64;
    }

    // Every range is done now, including ones that ended before the last block
    // and ones that ran past EOF. Finalize each digest exactly once here rather
    // than cloning the whole context inside `is_ok` for every verdict.
    for r in &mut ranges {
        r.finalize();
    }

    Ok(Scan {
        file_md5: Md5::from(whole.finalize()),
        file_len: total,
        dest_ok: dest_idx.iter().map(|&i| ranges[i].is_ok()).collect(),
        source_ok: source_idx.iter().map(|&i| ranges[i].is_ok()).collect(),
    })
}

/// Find or create the range state for `(offset, len, md5)`.
///
/// Two ranges are the same range when they cover the same bytes **and** expect
/// the same digest; the same bytes with a different expectation are
/// two different questions, so they get two digests. An expectation that is
/// not a digest is "unknown", and every unknown is the same question.
fn intern(
    ranges: &mut Vec<RangeState>,
    index: &mut HashMap<(u64, u64, Option<Md5>), usize>,
    offset: u64,
    len: u64,
    md5: &str,
) -> usize {
    let expect = Md5::try_parse_hex(md5);
    let key = (offset, len, expect);
    if let Some(&i) = index.get(&key) {
        return i;
    }
    let i = ranges.len();
    ranges.push(RangeState {
        offset,
        len,
        expect,
        ctx: md5::Md5::new(),
        seen: 0,
        digest: None,
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
    // Read straight into reserved spare capacity instead of zeroing `len`
    // bytes and overwriting all of them. `read_to_end` over a `Take(len)` stops
    // at the range end (or at EOF), exactly as `read_exact` would; a short
    // source still ends in an error, as before.
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .with_context(|| format!("reserve {len} bytes for a file range"))?;
    let n = read_into_vec(f, &mut out, len)?;
    if n != len {
        anyhow::bail!("short read: wanted {len} bytes, got {n}");
    }
    Ok(out)
}

/// The MD5 of a byte slice, as a digest.
pub fn md5_digest(data: &[u8]) -> Md5 {
    Md5::from(md5::Md5::digest(data))
}

/// Lowercase hex MD5 of a file.
///
/// The text form callers that report a digest use; the internal gates compare
/// [`Md5`] bytes through [`md5_file_digest`]. Kept as the public hex wrapper
/// even though the crate's own gates no longer need to go through it.
#[allow(dead_code)]
pub fn md5_file(path: &Path) -> Result<String> {
    Ok(md5_file_digest(path)?.to_hex())
}

/// [`md5_file`] as a digest, for a gate that compares bytes.
pub fn md5_file_digest(path: &Path) -> Result<Md5> {
    let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    md5_reader_digest(&mut f)
}

/// The whole-file MD5 of a handle the caller already holds.
///
/// The handle's position is whatever the caller's last read or write left it
/// at, so the digest always starts at the beginning of the file — the same
/// rule [`scan_reader`] follows. This is the verify gate that only needs the
/// whole-file answer: it does not build a digest per range.
pub(crate) fn md5_reader_digest(f: &mut File) -> Result<Md5> {
    with_md5_scratch(|buf| md5_reader_digest_with_buf(f, buf))
}

/// `md5_reader_digest` with a caller-supplied read buffer.
///
/// The seek is this function's reset contract — the digest always starts at
/// the beginning of the file — and stays here. The shared reader sink owns
/// only the read loop and the MD5 context; the buffer size is the caller's.
/// A caller with a task-owned buffer can call this directly to avoid the
/// per-thread pool `md5_reader_digest` uses.
pub fn md5_reader_digest_with_buf(f: &mut File, buf: &mut [u8]) -> Result<Md5> {
    f.seek(SeekFrom::Start(0))?;
    let raw = girpr_digest::hash_reader(f, girpr_digest::Algorithms::MD5, buf)?;
    let md5 = raw
        .md5
        .ok_or_else(|| anyhow::anyhow!("the MD5 hasher was not requested"))?;
    Ok(Md5::from(md5))
}

/// MD5 of a byte slice range of a file [offset, offset+len).
///
/// Opens the file and hashes the range through the shared reader helper; the
/// read buffer is internal. A caller that already holds the file and wants to
/// reuse its buffer uses `md5_reader_slice` instead.
#[cfg(any(test, feature = "test-support"))]
pub fn md5_file_slice(path: &Path, offset: u64, len: u64) -> Result<String> {
    let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut buf = Vec::new();
    md5_reader_slice(&mut f, offset, len, &mut buf)
}

/// MD5 of a byte slice range of a file the caller already has open, reusing
/// `buf`.
///
/// Seeks `f` to `offset` first and leaves it at the end of the range (or at
/// end of file, for a short read), so a caller that wants to copy the same
/// bytes afterwards seeks back once instead of opening the file again.
#[cfg(any(test, feature = "test-support"))]
pub fn md5_reader_slice(f: &mut File, offset: u64, len: u64, buf: &mut Vec<u8>) -> Result<String> {
    Ok(md5_reader_slice_digest(f, offset, len, buf)?.to_hex())
}

/// `md5_reader_slice` as a digest, for a gate that compares bytes.
pub fn md5_reader_slice_digest(
    f: &mut File,
    offset: u64,
    len: u64,
    buf: &mut Vec<u8>,
) -> Result<Md5> {
    f.seek(SeekFrom::Start(offset))?;
    let mut ctx = md5::Md5::new();
    let mut remaining = len;
    while remaining > 0 {
        let want = (remaining as usize).min(MD5_BUF);
        // `buf` is a reusable scratch: clear it so each read fills the same
        // capacity from index 0, and let `read_into_vec` append only the bytes
        // that were actually initialized. No zero-fill, no per-call allocation.
        buf.clear();
        let n = read_into_vec(f, buf, want)?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
        remaining -= n as u64;
    }
    Ok(Md5::from(ctx.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(data: &[u8]) -> String {
        md5_digest(data).to_hex()
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
        assert_eq!(Md5::EMPTY.to_hex(), EMPTY_MD5);
    }

    #[test]
    fn the_rfc_1321_vectors_hold() {
        // The expected values are shared with the other two programs; `hex`
        // still exercises this crate's own digest path.
        girpr_digest::vectors::assert_md5_vectors(hex);
    }

    #[test]
    fn a_digest_parses_in_either_case_and_prints_lowercase() {
        let upper = Md5::parse_hex("D41D8CD98F00B204E9800998ECF8427E").unwrap();
        let lower = Md5::parse_hex("d41d8cd98f00b204e9800998ecf8427e").unwrap();
        assert_eq!(upper, lower);
        assert_eq!(upper.to_hex(), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(upper.to_string(), lower.to_hex());
        assert_eq!(format!("{upper:?}"), format!("Md5({})", upper.to_hex()));
        assert!(upper.matches_hex("d41d8cd98f00b204e9800998ecf8427e"));
        assert!(upper.matches_hex("D41D8CD98F00B204E9800998ECF8427E"));
        assert!(!upper.matches_hex(""));
        assert!(!upper.matches_hex("md5-a"));
    }

    #[test]
    fn a_non_digest_is_not_an_md5() {
        // The hand-built manifests in tests use short labels; they are
        // "unknown", not a digest, and must never compare equal to anything.
        assert!(Md5::parse_hex("").is_err());
        assert!(Md5::parse_hex("md5-a").is_err());
        assert!(Md5::parse_hex(&"g".repeat(32)).is_err());
        assert!(Md5::parse_hex(&"a".repeat(31)).is_err());
    }

    #[test]
    fn an_intact_file_answers_every_range_correctly() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "intact");
        let data = four_blocks();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        let dest = Ranges::of(&[(0, 64, hex(&data[0..64])), (64, 65, hex(&data[64..129]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.file_len, data.len() as u64);
        assert_eq!(scan.file_md5, md5_digest(&data));
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
        assert_ne!(
            scan.file_md5,
            md5_digest(&good),
            "the whole file is wrong too"
        );
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
        assert_eq!(scan.file_md5, Md5::EMPTY);
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
        // straddles the read-buffer boundary is where an off-by-one would
        // show up. A 64-byte buffer over a 100-byte file crosses it with the
        // same `block_end`/`start`/`end` arithmetic as the production buffer.
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "boundary");
        let data: Vec<u8> = (0..100).map(|i| (i % 251) as u8).collect();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        let mut buf = vec![0u8; 64];
        let start = buf.len() as u64 - 10;
        let dest = Ranges::of(&[(start, 20, hex(&data[start as usize..start as usize + 20]))]);
        let mut f = File::open(dir.join("f.dat")).unwrap();
        let scan = scan_reader_with_buf(&mut f, &dest.view(), &[], &mut buf).unwrap();
        assert_eq!(scan.dest_ok, vec![true]);
        assert_eq!(scan.file_len, data.len() as u64);
    }

    #[test]
    fn a_handle_digest_agrees_with_the_file_digest_at_any_position() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_md5", "reader_digest");
        let mut buf = vec![0u8; 64];
        let large: Vec<u8> = (0..100).map(|i| (i % 251) as u8).collect();
        for (name, data) in [
            ("empty", Vec::new()),
            ("small", b"small file".to_vec()),
            ("large", large),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, &data).unwrap();
            let want = md5_digest(&data);
            let mut f = File::open(&path).unwrap();
            assert_eq!(
                md5_reader_digest_with_buf(&mut f, &mut buf).unwrap(),
                want,
                "{name}"
            );
            // The handle is left at EOF by the read; the next digest still
            // starts at the beginning of the file.
            f.seek(SeekFrom::End(0)).unwrap();
            assert_eq!(
                md5_reader_digest_with_buf(&mut f, &mut buf).unwrap(),
                want,
                "{name} at EOF"
            );
            // The production path still uses the full buffer; the small-buffer
            // digest must agree with it byte for byte.
            assert_eq!(
                md5_reader_digest_with_buf(&mut f, &mut buf).unwrap(),
                md5_file_digest(&path).unwrap(),
                "{name}: the small-buffer digest and the path digest are one answer"
            );
        }
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

    /// The spare-capacity read returns exactly the bytes the zero-filling
    /// `resize` + `read_exact` path did, at every offset and length, including
    /// one that crosses the reserve boundary. A short source is still an error.
    #[test]
    fn read_range_matches_the_zero_filled_reference_at_any_offset() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "read_range_equiv");
        let data: Vec<u8> = (0..MD5_BUF + 4096).map(|i| (i % 251) as u8).collect();
        let path = dir.join("f.dat");
        std::fs::write(&path, &data).unwrap();
        let mut f = File::open(&path).unwrap();
        for &(offset, len) in &[
            (0u64, 0u64),
            (0, 1),
            (7, 64),
            (64, MD5_BUF as u64 + 123),
            (MD5_BUF as u64 - 10, 20),
        ] {
            let got = read_range(&mut f, offset, len).unwrap();
            let start = offset as usize;
            let end = (start + len as usize).min(data.len());
            let want = if start >= data.len() {
                &[][..]
            } else {
                &data[start..end]
            };
            assert_eq!(got, want, "offset {offset} len {len}");
        }
        let mut f = File::open(&path).unwrap();
        assert!(
            read_range(&mut f, 0, data.len() as u64 + 1).is_err(),
            "a short read is still an error"
        );
    }

    /// The reusable slice scratch agrees byte for byte with the one-shot digest
    /// for every buffer size, and never grows past one read block (it is not
    /// charged with the whole requested range).
    #[test]
    fn slice_digest_agrees_across_scratch_buffer_sizes() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "slice_equiv");
        let data: Vec<u8> = (0..MD5_BUF * 2 + 37).map(|i| (i % 251) as u8).collect();
        let path = dir.join("f.dat");
        std::fs::write(&path, &data).unwrap();
        let cases = [
            (0u64, 0u64),
            (0, 3),
            (5, MD5_BUF as u64 + 9),
            (MD5_BUF as u64 + 1, 100),
            (data.len() as u64 - 5, 50),
        ];
        for &(offset, len) in &cases {
            let start = offset as usize;
            let end = (start + len as usize).min(data.len());
            let want = if start >= data.len() {
                md5_digest(b"")
            } else {
                md5_digest(&data[start..end])
            };
            for cap in [1usize, 7, 64, MD5_BUF] {
                let mut f = File::open(&path).unwrap();
                let mut buf = Vec::with_capacity(cap);
                let got = md5_reader_slice_digest(&mut f, offset, len, &mut buf).unwrap();
                assert_eq!(got, want, "offset {offset} len {len} cap {cap}");
                assert!(buf.len() <= MD5_BUF, "scratch stays one block");
            }
        }
    }

    /// Reusing the per-thread buffer (and interleaving other hash calls through
    /// it) changes no scan answer, and a caller-owned small buffer agrees with
    /// the pool byte for byte. The shared range at offset 0 is interned once and
    /// lands in both verdict lists, which also pins the finalize-once change.
    #[test]
    fn repeated_scans_through_the_thread_pool_are_identical() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "reuse_pool");
        let data = four_blocks();
        let path = dir.join("f.dat");
        std::fs::write(&path, &data).unwrap();
        let dest = Ranges::of(&[
            (0, 64, hex(&data[0..64])),
            (64, 65, hex(&data[64..129])),
            (129, 66, hex(&data[129..195])),
        ]);
        let source = Ranges::of(&[(64, 65, hex(&data[64..129])), (0, 64, hex(&data[0..64]))]);
        let first = scan_file(&path, &dest.view(), &source.view()).unwrap();
        for _ in 0..3 {
            let _ = md5_file_digest(&path).unwrap();
            let mut f = File::open(&path).unwrap();
            let mut buf = vec![0u8; 64];
            let with_buf =
                scan_reader_with_buf(&mut f, &dest.view(), &source.view(), &mut buf).unwrap();
            assert_eq!(with_buf, first, "a caller buffer and the pool agree");
            let again = scan_file(&path, &dest.view(), &source.view()).unwrap();
            assert_eq!(again, first, "reusing the thread buffer changes no answer");
        }
        assert_eq!(first.dest_ok, vec![true, true, true]);
        assert_eq!(first.source_ok, vec![true, true]);
    }

    /// Two verdicts over the same bytes with different expectations stay
    /// independent: finalizing each interned range once must not mix them up.
    #[test]
    fn the_same_bytes_with_two_expectations_get_independent_verdicts() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "two_expect");
        std::fs::write(dir.join("f.dat"), b"abcdef").unwrap();
        let good = hex(b"abcdef");
        let wrong = hex(b"xyzxyz");
        let dest = Ranges::of(&[(0, 6, good), (0, 6, wrong), (0, 6, hex(b"abcdef"))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.dest_ok, vec![true, false, true]);
    }

    /// Whole-file MD5 throughput, on the path the repair gate uses:
    /// [`md5_file`] → [`md5_reader_digest`], with the 512 KiB read buffer.
    ///
    /// This is a measurement, not a property, so it is `#[ignore]`d and is
    /// never part of the normal suite or CI. It writes a file of
    /// `GIRPR_MD5_BENCH_MIB` MiB (default 64), hashes it `GIRPR_MD5_BENCH_RUNS`
    /// times (default 5), and prints the throughput. Run it by hand:
    ///
    /// ```text
    /// cargo nextest run --release -p girpr --lib --run-ignored only -E 'test(md5_throughput)' --no-capture
    /// ```
    #[test]
    #[ignore = "throughput measurement, run by hand"]
    fn md5_throughput() {
        let mib: usize = std::env::var("GIRPR_MD5_BENCH_MIB")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(64);
        let runs: usize = std::env::var("GIRPR_MD5_BENCH_RUNS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(5);
        assert!(
            mib > 0 && runs > 0,
            "the benchmark needs a non-zero size and run count"
        );
        let dir = girpr_tempdir::TempDir::new("girpr_test_md5", "throughput");
        let path = dir.join("throughput.bin");
        let block: Vec<u8> = (0..1024 * 1024).map(|i| (i % 251) as u8).collect();
        let mut file = File::create(&path).expect("create the benchmark file");
        for _ in 0..mib {
            std::io::Write::write_all(&mut file, &block).expect("write the benchmark file");
        }
        file.sync_all().expect("flush the benchmark file");
        drop(file);

        // Warm the page cache and the hasher before taking the clock, so the
        // number is throughput and not first-touch I/O.
        let _ = md5_file(&path).expect("warm-up hash");
        let start = std::time::Instant::now();
        let mut digest = String::new();
        for _ in 0..runs {
            digest = md5_file(&path).expect("hash the benchmark file");
        }
        let elapsed = start.elapsed().as_secs_f64();
        let bytes = (mib * 1024 * 1024) as f64 * runs as f64;
        println!(
            "md5_file: {mib} MiB x {runs} runs in {elapsed:.3} s = {:.1} MiB/s (digest {digest})",
            bytes / (1024.0 * 1024.0) / elapsed,
        );
        assert_eq!(digest.len(), 32, "the benchmark hashed a real file");
    }

    /// Scan throughput, on the repair path that dwarfs the whole-file digest:
    /// [`scan_file`] and [`scan_reader`], with one destination range per MiB
    /// and a half-chunk source range in every chunk.
    ///
    /// This is a measurement, not a property, so it is `#[ignore]`d and is
    /// never part of the normal suite or CI. It writes a file of
    /// `GIRPR_SCAN_BENCH_MIB` MiB (default 64), scans it
    /// `GIRPR_SCAN_BENCH_RUNS` times (default 5), and prints the throughput.
    /// Run it by hand:
    ///
    /// ```text
    /// cargo nextest run --release -p girpr --lib --run-ignored only -E 'test(scan_throughput)' --no-capture
    /// ```
    #[test]
    #[ignore = "throughput measurement, run by hand"]
    fn scan_throughput() {
        let mib: usize = std::env::var("GIRPR_SCAN_BENCH_MIB")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(64);
        let runs: usize = std::env::var("GIRPR_SCAN_BENCH_RUNS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(5);
        assert!(
            mib > 0 && runs > 0,
            "the benchmark needs a non-zero size and run count"
        );
        const MIB: usize = 1024 * 1024;

        let dir = girpr_tempdir::TempDir::new("girpr_test_md5", "scan_throughput");
        let path = dir.join("scan.bin");
        let block: Vec<u8> = (0..MIB).map(|i| (i % 251) as u8).collect();
        let mut file = File::create(&path).expect("create the benchmark file");
        for _ in 0..mib {
            std::io::Write::write_all(&mut file, &block).expect("write the benchmark file");
        }
        file.sync_all().expect("flush the benchmark file");
        drop(file);

        let chunk_md5 = md5_digest(&block).to_hex();
        let dest_owned: Vec<(u64, u64, String)> = (0..mib)
            .map(|i| ((i * MIB) as u64, MIB as u64, chunk_md5.clone()))
            .collect();
        let half = MIB / 2;
        let half_md5 = md5_digest(&block[half..]).to_hex();
        let source_owned: Vec<(u64, u64, String)> = (0..mib)
            .map(|i| ((i * MIB + half) as u64, half as u64, half_md5.clone()))
            .collect();
        let dest = Ranges::of(&dest_owned);
        let source = Ranges::of(&source_owned);
        let dest = dest.view();
        let source = source.view();

        // Warm the page cache before taking the clock, so the number is
        // throughput and not first-touch I/O.
        let _ = scan_file(&path, &dest, &source).expect("warm-up scan");
        let start = std::time::Instant::now();
        let mut dest_ok = 0;
        for _ in 0..runs {
            let scan = scan_file(&path, &dest, &source).expect("scan the benchmark file");
            dest_ok = scan.dest_ok.iter().filter(|&&ok| ok).count();
        }
        let elapsed = start.elapsed().as_secs_f64();
        let bytes = (mib * MIB) as f64 * runs as f64;
        // A scan that stops finding intact ranges would otherwise still print
        // a plausible throughput figure. The file is generated by this test,
        // so every range must be correct on every run.
        assert_eq!(
            dest_ok, mib,
            "the scan must find every destination range intact"
        );
        println!(
            "scan_file: {mib} MiB x {runs} runs in {elapsed:.3} s = {:.1} MiB/s (dest_ok {dest_ok}/{mib})",
            bytes / (1024.0 * 1024.0) / elapsed,
        );

        let mut handle = File::open(&path).expect("open the benchmark file");
        let start = std::time::Instant::now();
        let mut source_ok = 0;
        for _ in 0..runs {
            let scan = scan_reader(&mut handle, &dest, &source).expect("scan the benchmark handle");
            source_ok = scan.source_ok.iter().filter(|&&ok| ok).count();
        }
        let elapsed = start.elapsed().as_secs_f64();
        assert_eq!(
            source_ok, mib,
            "the scan must find every source range intact"
        );
        println!(
            "scan_reader: {mib} MiB x {runs} runs in {elapsed:.3} s = {:.1} MiB/s (source_ok {source_ok}/{mib})",
            bytes / (1024.0 * 1024.0) / elapsed,
        );
    }
}
