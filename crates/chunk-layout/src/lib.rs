//! Half-open byte ranges and the one chunk-tiling cursor.
//!
//! A chunk is a half-open byte range `[start, end)`. Three questions recur
//! wherever chunks are placed in a file: is this range well formed, do the
//! ranges tile `[0, size)` without a hole or an overlap, and what is the gap or
//! overlap between two neighbours? This leaf answers those questions once. It
//! holds no diagnostic wording, no file-size policy and no write decision: a
//! caller turns the typed problems it returns into its own error text, counters
//! or gap-fill records.
//!
//! ## Coordinates
//!
//! Ranges carry signed `i64` bounds because that is the wire type both programs
//! decode. An end is derived from a signed cursor with
//! [`checked_end`](girpr_chunk_cost::checked_end): a negative result clamps to
//! zero and an addition the signed type cannot represent is refused rather
//! than wrapped into a plausible-looking `u64`. Byte totals that merely sum
//! declarations use `girpr_chunk_cost::saturating_nonneg_sum` instead.
//!
//! ## The fold and its fast path
//!
//! [`TileCursor`] is the single incremental implementation of the tiling rule.
//! [`tile`] folds it over a sequence and collects the problems it finds; a
//! caller that only needs the yes/no verdict pushes its chunks into the same
//! cursor without allocating anything.

use serde::{Deserialize, Serialize};

pub use girpr_chunk_cost::checked_end;

/// A half-open byte range `[start, end)` in signed (wire) coordinates.
///
/// The field names are the published `start`/`end` keys; deriving serde here
/// keeps every caller's document byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ByteRange {
    /// First byte included.
    pub start: i64,
    /// One past the last byte included.
    pub end: i64,
}

impl ByteRange {
    /// A range from explicit bounds. Inverted bounds are accepted: a corrupt
    /// document is a problem to report, not a reason to abort the reader.
    pub const fn new(start: i64, end: i64) -> Self {
        Self { start, end }
    }

    /// The range covering `len` bytes at `offset`.
    ///
    /// Saturating, so an unrepresentable end stays a diagnosable bound instead
    /// of wrapping into a range that looks valid.
    pub const fn at(offset: i64, len: i64) -> Self {
        Self::new(offset, offset.saturating_add(len))
    }

    /// The empty range at `offset`.
    pub const fn empty_at(offset: i64) -> Self {
        Self::new(offset, offset)
    }

    /// The signed length: `end - start`, saturating, and negative when inverted.
    pub const fn len(&self) -> i64 {
        self.end.saturating_sub(self.start)
    }

    /// The length as a non-negative `u64`: negative declarations are no bytes.
    pub const fn len_sat(&self) -> u64 {
        let len = self.len();
        if len < 0 { 0 } else { len as u64 }
    }

    /// Whether the range covers no bytes (including when it is inverted).
    pub const fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    /// Whether the bounds are the wrong way round.
    pub const fn is_inverted(&self) -> bool {
        self.end < self.start
    }

    /// Whether `other` lies entirely within this range.
    pub const fn contains(&self, other: Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    /// Whether the two share at least one byte. Empty ranges share none.
    pub const fn overlaps(&self, other: Self) -> bool {
        !self.is_empty() && !other.is_empty() && self.start < other.end && other.start < self.end
    }

    /// The signed distance from this range's end to `next`'s start: positive
    /// for a gap, negative for an overlap, zero when the two meet exactly.
    /// Saturating, so extreme bounds stay diagnosable.
    pub const fn gap_to(&self, next: Self) -> i64 {
        next.start.saturating_sub(self.end)
    }

    /// The exclusive end of a signed range, or `None` when it cannot be
    /// represented by the signed cursor.
    pub fn checked_end(offset: i64, len: i64) -> Option<u64> {
        girpr_chunk_cost::checked_end(offset, len)
    }

    /// The slice of `bytes` this range names, if it names a valid one.
    pub fn slice<'a>(&self, bytes: &'a [u8]) -> Option<&'a [u8]> {
        if self.is_inverted() || self.start < 0 {
            return None;
        }
        let end = self.end.max(self.start) as usize;
        bytes.get(self.start as usize..end)
    }
}

impl std::fmt::Display for ByteRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}, {})", self.start, self.end)
    }
}

/// A typed range error, to be mapped to each caller's own wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeProblem {
    NegativeOffset(i64),
    NegativeLen(i64),
    Overflow { offset: i64, len: i64 },
    PastEnd { end: u64, bound: u64 },
}

/// Check one signed `(offset, len)` declaration, returning its exclusive end.
pub fn check_range(offset: i64, len: i64) -> Result<u64, RangeProblem> {
    if offset < 0 {
        return Err(RangeProblem::NegativeOffset(offset));
    }
    if len < 0 {
        return Err(RangeProblem::NegativeLen(len));
    }
    checked_end(offset, len).ok_or(RangeProblem::Overflow { offset, len })
}

/// Check one declaration against a caller-supplied end bound.
///
/// The bound is supplied by the caller: this leaf does not know a file's size
/// and never guesses one.
pub fn check_range_within(offset: i64, len: i64, bound: u64) -> Result<u64, RangeProblem> {
    let end = check_range(offset, len)?;
    if end > bound {
        return Err(RangeProblem::PastEnd { end, bound });
    }
    Ok(end)
}

/// Whether `[start, start + len)` fits within `bound` in unsigned coordinates.
pub fn fits(start: u64, len: u64, bound: u64) -> bool {
    start.checked_add(len).is_some_and(|end| end <= bound)
}

/// A span of the file, as carried by a typed problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: i64,
    pub end: i64,
}

/// One typed tiling problem, independent of any program's diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutProblem {
    /// A declaration has a negative offset or length.
    Negative { offset: i64, len: i64 },
    /// The end of a declaration does not fit the signed cursor.
    Overflow { offset: i64, len: i64 },
    /// Bytes `[from, to)` are claimed by no chunk.
    Gap { from: i64, to: i64 },
    /// A chunk starts before the coverage cursor: those bytes are claimed twice.
    Overlap { from: i64, to: i64 },
    /// A later chunk's offset does not exceed the previous one's.
    NotAscending { previous: i64, offset: i64 },
    /// Coverage stops short of the declared size.
    Short { from: i64, to: i64 },
    /// Coverage runs past the declared size.
    PastEnd { end: i64, size: i64 },
}

/// What one fold step found. Carries the full payload of each fact so the
/// collector needs no second pass and the verdict-only caller reads no more
/// than a boolean.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Step {
    pub gap: Option<Span>,
    pub overlap: Option<Span>,
    pub short: Option<Span>,
    pub negative: Option<(i64, i64)>,
    pub overflow: Option<(i64, i64)>,
    pub past_end: Option<(i64, i64)>,
    pub not_ascending: Option<(i64, i64)>,
}

impl Step {
    fn problems(self) -> Vec<LayoutProblem> {
        let mut out = Vec::new();
        if let Some((offset, len)) = self.negative {
            out.push(LayoutProblem::Negative { offset, len });
        }
        if let Some((offset, len)) = self.overflow {
            out.push(LayoutProblem::Overflow { offset, len });
        }
        if let Some(span) = self.gap {
            out.push(LayoutProblem::Gap {
                from: span.start,
                to: span.end,
            });
        }
        if let Some(span) = self.overlap {
            out.push(LayoutProblem::Overlap {
                from: span.start,
                to: span.end,
            });
        }
        if let Some((previous, offset)) = self.not_ascending {
            out.push(LayoutProblem::NotAscending { previous, offset });
        }
        if let Some(span) = self.short {
            out.push(LayoutProblem::Short {
                from: span.start,
                to: span.end,
            });
        }
        if let Some((end, size)) = self.past_end {
            out.push(LayoutProblem::PastEnd { end, size });
        }
        out
    }
}

/// The single incremental tiling rule.
///
/// `cursor` is the end reached by the last chunk, `covered` is the furthest end
/// reached by any chunk, and `prev_offset` is the offset of the chunk that last
/// extended that coverage. Gap and overlap records use those two values, so a
/// caller that needs the spans gets them without re-walking the list.
#[derive(Debug, Clone, Copy)]
pub struct TileCursor {
    cursor: i64,
    covered: i64,
    prev_offset: i64,
    last_offset: Option<i64>,
    gapless: bool,
    ascending: bool,
}

impl Default for TileCursor {
    fn default() -> Self {
        Self::new()
    }
}

impl TileCursor {
    /// An empty cursor over the start of a file.
    pub const fn new() -> Self {
        Self {
            cursor: 0,
            covered: 0,
            prev_offset: 0,
            last_offset: None,
            gapless: true,
            ascending: true,
        }
    }

    /// Fold one chunk's `(offset, len)` into the cursor.
    pub fn push(&mut self, offset: i64, len: i64) -> Step {
        let mut step = Step::default();
        if offset < 0 || len < 0 {
            step.negative = Some((offset, len));
        }
        // The end is checked first: an unrepresentable end is a tiling
        // violation by definition, and the furthest reachable bound is used as
        // the cursor so the walk stays total.
        let end = match offset.checked_add(len) {
            Some(end) => end,
            None => {
                step.overflow = Some((offset, len));
                self.gapless = false;
                i64::MAX
            }
        };
        if let Some(previous) = self.last_offset {
            if offset <= previous {
                step.not_ascending = Some((previous, offset));
                self.ascending = false;
            }
        }
        self.last_offset = Some(offset);
        if offset > self.covered {
            step.gap = Some(Span {
                start: self.covered,
                end: offset,
            });
            self.gapless = false;
        } else if offset < self.covered {
            step.overlap = Some(Span {
                start: self.prev_offset,
                end: offset,
            });
            self.gapless = false;
        }
        if end > self.covered {
            self.covered = end;
            self.prev_offset = offset;
        }
        self.cursor = end;
        step
    }

    /// Compare the finished coverage against a declared size. This does not
    /// change the gapless verdict: a short or overlong tail is a size question,
    /// not a hole between chunks.
    pub fn finish(&self, size: i64) -> Step {
        let mut step = Step::default();
        if self.covered < size {
            step.short = Some(Span {
                start: self.covered,
                end: size,
            });
        } else if self.covered > size {
            step.past_end = Some((self.covered, size));
        }
        step
    }

    /// The chunks pushed so far cover `[0, size)` with no hole or overlap.
    pub const fn is_tiled(&self, size: i64) -> bool {
        self.gapless && self.covered == size
    }

    /// Every pushed chunk started exactly where the previous one ended.
    pub const fn is_gapless(&self) -> bool {
        self.gapless
    }

    /// Every pushed chunk had an offset strictly greater than the previous one's.
    pub const fn is_ascending(&self) -> bool {
        self.ascending
    }

    /// The furthest end reached so far.
    pub const fn covered(&self) -> i64 {
        self.covered
    }
}

/// The full tiling result for one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tiling {
    pub tiled: bool,
    pub gapless: bool,
    pub ascending: bool,
    pub problems: Vec<LayoutProblem>,
}

/// Fold a sequence of `(offset, len)` declarations and compare their coverage
/// with the caller's declared size. The caller decides whether to sort or
/// filter the declarations first; this fold applies the same rule to whatever
/// order it is given.
pub fn tile(parts: impl IntoIterator<Item = (i64, i64)>, size: i64) -> Tiling {
    let mut cursor = TileCursor::new();
    let mut problems = Vec::new();
    for (offset, len) in parts {
        problems.extend(cursor.push(offset, len).problems());
    }
    problems.extend(cursor.finish(size).problems());

    Tiling {
        tiled: cursor.is_tiled(size),
        gapless: cursor.is_gapless(),
        ascending: cursor.is_ascending(),
        problems,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_is_half_open_and_its_length_never_needs_an_adjustment() {
        let r = ByteRange::at(10, 5);
        assert_eq!(r, ByteRange::new(10, 15));
        assert_eq!(r.len(), 5);
        assert_eq!(r.len_sat(), 5);
        assert_eq!(r.to_string(), "[10, 15)");
        assert!(!r.is_empty());
        assert_eq!(ByteRange::at(0, 1).len(), 1);
        assert!(ByteRange::at(3, 0).is_empty());
        assert_eq!(ByteRange::empty_at(7).len(), 0);
    }

    #[test]
    fn adjacency_gaps_and_overlaps_are_signed_and_saturating() {
        let a = ByteRange::at(0, 10);
        assert_eq!(a.gap_to(ByteRange::at(10, 5)), 0);
        assert_eq!(a.gap_to(ByteRange::at(15, 5)), 5);
        assert_eq!(a.gap_to(ByteRange::at(8, 5)), -2);
        assert_eq!(
            ByteRange::new(0, i64::MAX).gap_to(ByteRange::new(i64::MIN, 0)),
            i64::MIN
        );
    }

    #[test]
    fn containment_and_overlap_are_different_questions() {
        let outer = ByteRange::at(0, 100);
        let inner = ByteRange::at(10, 20);
        assert!(outer.contains(inner));
        assert!(!inner.contains(outer));
        assert!(outer.overlaps(inner));
        assert!(!inner.overlaps(ByteRange::at(200, 10)));
        assert!(!outer.overlaps(ByteRange::at(100, 10)));
        assert!(!ByteRange::empty_at(5).overlaps(ByteRange::empty_at(5)));
    }

    #[test]
    fn inverted_extreme_and_out_of_buffer_ranges_are_total() {
        let r = ByteRange::new(10, 4);
        assert!(r.is_inverted());
        assert!(r.is_empty());
        assert_eq!(r.len(), -6);
        assert_eq!(r.len_sat(), 0);
        assert_eq!(r.slice(b"0123456789"), None);
        assert_eq!(
            ByteRange::new(i64::MIN, 0).len(),
            i64::MAX,
            "an inverted extreme length saturates"
        );
        assert_eq!(
            ByteRange::at(i64::MAX, 1),
            ByteRange::new(i64::MAX, i64::MAX)
        );

        let b = b"0123456789";
        assert_eq!(ByteRange::at(2, 3).slice(b), Some(&b[2..5]));
        assert_eq!(ByteRange::empty_at(10).slice(b), Some(&b[10..10]));
        assert_eq!(ByteRange::empty_at(11).slice(b), None);
        assert_eq!(ByteRange::at(-1, 2).slice(b), None);
    }

    #[test]
    fn the_published_bounds_are_named_start_and_end() {
        let j = serde_json::to_value(ByteRange::at(690719, 1626019)).unwrap();
        assert_eq!(j["start"], 690719);
        assert_eq!(j["end"], 690719 + 1626019);
        assert!(j.get("len").is_none());
    }

    #[test]
    fn declaration_checks_distinguish_every_problem() {
        assert_eq!(check_range(10, 5), Ok(15));
        assert_eq!(check_range(-1, 5), Err(RangeProblem::NegativeOffset(-1)));
        assert_eq!(check_range(10, -1), Err(RangeProblem::NegativeLen(-1)));
        assert_eq!(
            check_range(i64::MAX, 1),
            Err(RangeProblem::Overflow {
                offset: i64::MAX,
                len: 1
            })
        );
        assert_eq!(
            check_range_within(32, 64, 64),
            Err(RangeProblem::PastEnd { end: 96, bound: 64 })
        );
        assert_eq!(check_range_within(32, 32, 64), Ok(64));
        assert!(!fits(u64::MAX, 1, u64::MAX));
        assert!(fits(10, 5, 15));
    }

    #[test]
    fn an_exact_cover_is_tiled_and_a_hole_is_a_gap() {
        let tiled = tile([(0, 64), (64, 64)], 128);
        assert!(tiled.tiled);
        assert!(tiled.gapless);
        assert!(tiled.ascending);
        assert!(tiled.problems.is_empty());

        let holed = tile([(0, 64), (128, 64)], 192);
        assert!(!holed.tiled);
        assert!(!holed.gapless);
        assert!(holed.ascending);
        assert_eq!(
            holed.problems,
            vec![LayoutProblem::Gap { from: 64, to: 128 }]
        );
    }

    #[test]
    fn an_overlap_names_the_last_extending_offset_and_the_later_one() {
        let t = tile([(0, 100), (10, 50), (20, 5)], 128);
        assert!(!t.tiled);
        assert_eq!(
            t.problems,
            vec![
                LayoutProblem::Overlap { from: 0, to: 10 },
                LayoutProblem::Overlap { from: 0, to: 20 },
                LayoutProblem::Short { from: 100, to: 128 },
            ]
        );
    }

    #[test]
    fn out_of_order_chunks_are_not_gapless_nor_ascending() {
        let t = tile([(100, 50), (0, 100)], 150);
        assert!(!t.tiled);
        assert!(!t.gapless);
        assert!(!t.ascending);
        assert!(
            t.problems
                .iter()
                .any(|p| matches!(p, LayoutProblem::NotAscending { .. }))
        );
    }

    #[test]
    fn a_tail_is_a_size_question_and_not_a_hole_between_chunks() {
        let short = tile([(0, 50)], 100);
        assert!(!short.tiled);
        assert!(short.gapless, "no interior hole");
        assert_eq!(
            short.problems,
            vec![LayoutProblem::Short { from: 50, to: 100 }]
        );

        let long = tile([(0, 150)], 100);
        assert!(!long.tiled);
        assert!(long.gapless);
        assert_eq!(
            long.problems,
            vec![LayoutProblem::PastEnd {
                end: 150,
                size: 100
            }]
        );
    }

    #[test]
    fn overflows_and_negative_declarations_are_typed_not_fatal() {
        let t = tile([(i64::MAX, 1)], 1);
        assert!(!t.gapless);
        assert!(
            t.problems
                .iter()
                .any(|p| matches!(p, LayoutProblem::Overflow { .. }))
        );

        let n = tile([(-1, 10)], 10);
        assert!(!n.gapless);
        assert!(
            n.problems
                .iter()
                .any(|p| matches!(p, LayoutProblem::Negative { .. }))
        );
    }

    #[test]
    fn the_cursor_agrees_with_the_collecting_fold() {
        let cases = [
            (128, vec![(0i64, 64i64), (64, 64)]),
            (192, vec![(0, 64), (128, 64)]),
            (96, vec![(0, 64), (32, 64)]),
            (200, vec![(0, 64), (64, 64)]),
            (64, vec![(0, 128)]),
            (0, vec![]),
            (150, vec![(100, 50), (0, 100)]),
        ];
        for (size, parts) in cases {
            let mut cursor = TileCursor::new();
            for (offset, len) in &parts {
                cursor.push(*offset, *len);
            }
            let fold = tile(parts, size);
            assert_eq!(cursor.is_tiled(size), fold.tiled, "size {size}");
            assert_eq!(cursor.is_gapless(), fold.gapless, "size {size}");
            assert_eq!(cursor.is_ascending(), fold.ascending, "size {size}");
        }
    }
}
