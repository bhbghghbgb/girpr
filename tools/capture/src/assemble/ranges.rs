//! Range algebra: offsets, lengths, and the gap/overlap checks.
//!
//! **M7.** Chunks are placed at absolute offsets in a target file, and a REUSE
//! reads them from an absolute offset in a *different* file, so a moved chunk is
//! two ranges that happen to have equal length. Representing a range by
//! `(start, end)` rather than `(start, length)` is what makes an overlap or a gap
//! a comparison instead of an arithmetic coincidence.
//!
//! ## Half-open, always
//!
//! `[start, end)`. The alternative — inclusive ends — makes the length of a
//! one-byte chunk `end - start + 1` and every caller has to remember the `+1`.
//! It also makes "adjacent" (`a.end == b.start`) a fact about the numbers rather
//! than an accident of convention, which is the property A11 leans on: a file
//! whose chunks tile it exactly is one where every `a.end` equals the next
//! `b.start` and the last equals the declared size.
//!
//! ## Violations are data
//!
//! [`Layout::of`] never returns an error. It returns the ranges *and* the list of
//! things wrong with them, because the artifact is supposed to record a manifest
//! that violates A11 rather than refuse to describe it — the same rule M6's
//! `measure` follows, and for the same reason: the finding is the deliverable.

use serde::{Deserialize, Serialize};

/// A half-open byte range `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Range {
    /// First byte included.
    pub start: i64,
    /// One past the last byte included.
    pub end: i64,
}

impl Range {
    /// A range from explicit bounds.
    ///
    /// Accepts `end < start` rather than panicking: a manifest with an inverted
    /// chunk offset is a finding to record, not a crash. [`Range::is_inverted`]
    /// names it and the layout check reports it.
    pub const fn new(start: i64, end: i64) -> Range {
        Range { start, end }
    }

    /// The range covering `len` bytes at `offset`.
    ///
    /// Saturating rather than `+`: an index or manifest can declare an offset
    /// near `i64::MAX`, and a range algebra that panics on the document it
    /// describes is the class `decode_counts` already refuses. [`Layout::of`]
    /// records the overflow as an A11 violation; this method stays total for
    /// every caller, including the public ones that never build a `Layout`.
    pub const fn at(offset: i64, len: i64) -> Range {
        Range::new(offset, offset.saturating_add(len))
    }

    /// The empty range at `offset`.
    pub const fn empty_at(offset: i64) -> Range {
        Range::new(offset, offset)
    }

    /// How many bytes this covers. Negative when inverted.
    ///
    /// Saturating for the same reason as [`Range::at`]: the bounds are
    /// untrusted, and the gap/overlap diagnostic must not abort on them.
    pub const fn len(&self) -> i64 {
        self.end.saturating_sub(self.start)
    }

    /// Whether the range covers no bytes.
    pub const fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    /// Whether the bounds are the wrong way round.
    pub const fn is_inverted(&self) -> bool {
        self.end < self.start
    }

    /// Whether `other` lies entirely within this range.
    pub const fn contains(&self, other: Range) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    /// Whether the two share at least one byte.
    ///
    /// Empty ranges never overlap anything, including each other: two zero-length
    /// ranges at the same offset share no bytes, and counting them as an overlap
    /// would report a conflict in a file full of zero-length chunks.
    pub const fn overlaps(&self, other: Range) -> bool {
        !self.is_empty() && !other.is_empty() && self.start < other.end && other.start < self.end
    }

    /// The bytes between this range's end and `next`'s start, if they do not meet.
    ///
    /// Positive for a gap, negative for an overlap, zero when they are adjacent.
    /// Saturating, so an `i64::MIN` start next to an `i64::MAX` end is a
    /// diagnosable violation rather than an abort.
    pub const fn to(&self, next: Range) -> i64 {
        next.start.saturating_sub(self.end)
    }

    /// The slice of `bytes` this range names, if it names a valid one.
    pub fn slice<'a>(&self, bytes: &'a [u8]) -> Option<&'a [u8]> {
        if self.is_inverted() || self.start < 0 {
            return None;
        }
        let end = self.end.max(self.start) as usize;
        // A zero-length range at exactly the end of the buffer is legitimate:
        // the last chunk of a file that ends in an empty chunk.
        bytes.get(self.start as usize..end)
    }
}

impl std::fmt::Display for Range {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}, {})", self.start, self.end)
    }
}

/// Where a file's chunks sit, and what is wrong with that if anything is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    /// The ranges, in the order the manifest listed the chunks.
    pub ranges: Vec<Range>,
    /// `Σ len`, which is what the file would be if the chunks were concatenated.
    pub total: i64,
    /// The size the manifest declared for the file.
    pub declared_size: i64,
    /// Every A11 rule this layout breaks, in a readable form.
    pub violations: Vec<String>,
}

impl Layout {
    /// Build a layout from `(offset, uncompressed_size)` pairs.
    ///
    /// The checks are exactly A11: offsets ascending and gapless from 0, and
    /// `Σ uncompressed_size == size`.
    ///
    /// Takes an iterator of `(offset, length)` rather than a slice, so a caller
    /// can stream its chunks straight in. It used to take `&[(i64, i64)]`, which
    /// meant every caller built a `Vec` of tuples only for this to convert it
    /// into a `Vec<Range>` and drop the first one — two vectors where one would
    /// do, for every file assembled.
    pub fn of(parts: impl IntoIterator<Item = (i64, i64)>, declared_size: i64) -> Layout {
        let mut violations = Vec::new();
        let mut ranges: Vec<Range> = Vec::new();
        // Each end is `checked_add`, because the offset and length come from a
        // document. An overflow is a finding about that document, not a reason
        // to abort the tool that records it — the same rule `decode_counts`
        // applies to a manifest it is only measuring.
        for (i, (offset, len)) in parts.into_iter().enumerate() {
            let range = match offset.checked_add(len) {
                Some(end) => Range::new(offset, end),
                None => {
                    violations.push(format!(
                        "chunk {i}: offset {offset} + length {len} overflows i64"
                    ));
                    Range::new(offset, i64::MAX)
                }
            };
            ranges.push(range);
        }
        // `Σ len` is what the file would weigh if the chunks were concatenated.
        // A sum that does not fit `i64` is a violation, with the running total
        // named, and one row is enough: every later term is not a byte count.
        let mut total = 0_i64;
        let mut total_overflowed = false;
        for r in &ranges {
            if total_overflowed {
                break;
            }
            total = match total.checked_add(r.len()) {
                Some(next) => next,
                None => {
                    violations.push(format!(
                        "the chunk lengths sum overflows i64 at range {r} (running total {total} B)"
                    ));
                    total_overflowed = true;
                    i64::MAX
                }
            };
        }

        for (i, r) in ranges.iter().enumerate() {
            if r.is_inverted() {
                violations.push(format!(
                    "chunk {i} has offset {} and length {} — an inverted range",
                    r.start,
                    r.len()
                ));
                continue;
            }
            if r.is_empty() {
                violations.push(format!("chunk {i} is zero-length at offset {}", r.start));
            }
            if i == 0 && r.start != 0 {
                violations.push(format!(
                    "the first chunk starts at {}, not 0 — the file does not tile from its \
                     beginning",
                    r.start
                ));
            }
            if i > 0 {
                let prev = ranges[i - 1];
                match prev.to(*r) {
                    0 => {}
                    gap if gap > 0 => violations.push(format!(
                        "a gap of {gap} B between chunk {} (ends {}) and chunk {} (starts {})",
                        i - 1,
                        prev.end,
                        i,
                        r.start
                    )),
                    over => violations.push(format!(
                        "chunk {} starts {}(-overlap bytes) before chunk {} ends",
                        i,
                        over.saturating_neg(),
                        i - 1
                    )),
                }
            }
        }
        if total != declared_size {
            violations.push(format!(
                "the chunks total {total} B but the manifest declares {declared_size} B"
            ));
        }
        Layout {
            ranges,
            total,
            declared_size,
            violations,
        }
    }

    /// Whether A11 holds.
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_is_half_open_and_its_length_never_needs_an_adjustment() {
        let r = Range::at(10, 5);
        assert_eq!(r, Range::new(10, 15));
        assert_eq!(r.len(), 5);
        assert_eq!(r.to_string(), "[10, 15)");
        assert!(!r.is_empty());
        let one = Range::at(0, 1);
        assert_eq!(one.len(), 1, "a one-byte range is one byte, not zero");
        assert!(Range::at(3, 0).is_empty());
        assert_eq!(Range::empty_at(7).len(), 0);
    }

    /// Adjacency is a fact about the numbers, which is the property A11 rests
    /// on: a file that tiles exactly has every `end` equal to the next `start`.
    #[test]
    fn adjacent_ranges_meet_at_zero_and_gaps_and_overlaps_are_signed() {
        let a = Range::at(0, 10);
        assert_eq!(a.to(Range::at(10, 5)), 0, "adjacent");
        assert_eq!(a.to(Range::at(15, 5)), 5, "a 5 B gap");
        assert_eq!(a.to(Range::at(8, 5)), -2, "a 2 B overlap");
    }

    #[test]
    fn containment_and_overlap_are_different_questions() {
        let outer = Range::at(0, 100);
        let inner = Range::at(10, 20);
        assert!(outer.contains(inner));
        assert!(!inner.contains(outer));
        assert!(outer.overlaps(inner));
        assert!(!inner.overlaps(Range::at(200, 10)), "disjoint");
        // Touching without sharing is not overlapping.
        assert!(!outer.overlaps(Range::at(100, 10)));
        // Two empty ranges at the same offset share no bytes.
        assert!(!Range::empty_at(5).overlaps(Range::empty_at(5)));
    }

    #[test]
    fn an_inverted_range_is_kept_and_named_rather_than_panicking() {
        let r = Range::new(10, 4);
        assert!(r.is_inverted());
        // It covers no bytes, so `is_empty` is true — the two questions are
        // different but not contradictory.
        assert!(r.is_empty());
        assert_eq!(r.len(), -6);
        assert_eq!(r.slice(b"0123456789"), None);
    }

    #[test]
    fn slicing_a_range_that_does_not_fit_the_buffer_gives_none() {
        let b = b"0123456789";
        assert_eq!(Range::at(2, 3).slice(b), Some(&b[2..5]));
        assert_eq!(Range::empty_at(0).slice(b), Some(&b[0..0]));
        // A zero-length range at exactly the end is legitimate: the last chunk
        // of a file that ends in an empty chunk.
        assert_eq!(Range::empty_at(10).slice(b), Some(&b[10..10]));
        assert_eq!(Range::empty_at(11).slice(b), None);
        assert_eq!(Range::at(5, 100).slice(b), None);
        assert_eq!(Range::at(-1, 2).slice(b), None);
    }

    /// A11, the happy path: three chunks that tile a 300 B file from 0.
    #[test]
    fn a_gapless_ascending_layout_is_clean() {
        let l = Layout::of([(0, 100), (100, 100), (200, 100)], 300);
        assert!(l.is_clean(), "{:?}", l.violations);
        assert_eq!(l.total, 300);
        assert_eq!(l.ranges.len(), 3);
        assert_eq!(
            l.ranges[2].end, l.declared_size,
            "and it ends where the file ends"
        );
    }

    #[test]
    fn a_gap_is_named_with_its_size_and_the_chunks_it_sits_between() {
        // The declared size matches the sum of the chunk lengths, so the gap is
        // the *only* violation and the message can be about the gap alone. A11
        // counts `Σ uncompressed_size`, not `last chunk's end`.
        let l = Layout::of([(0, 100), (150, 100)], 200);
        assert!(!l.is_clean());
        assert_eq!(l.violations.len(), 1, "{:?}", l.violations);
        assert!(
            l.violations[0].contains("gap of 50 B"),
            "{:?}",
            l.violations
        );
        assert!(l.violations[0].contains("chunk 0") && l.violations[0].contains("chunk 1"));
    }

    /// Two independent rules can break at once, and both get a row — a manifest
    /// with a hole *and* a wrong total is two findings, and reporting only the
    /// first would leave the second undiscovered until it was fixed.
    #[test]
    fn two_independent_violations_are_both_reported() {
        let l = Layout::of([(0, 100), (150, 100)], 999);
        assert_eq!(l.violations.len(), 2, "{:?}", l.violations);
        assert!(l.violations[0].contains("gap of 50 B"));
        assert!(l.violations[1].contains("total 200 B"));
    }

    #[test]
    fn an_overlap_is_named_as_an_overlap_and_not_as_a_negative_gap() {
        let l = Layout::of([(0, 100), (80, 100)], 200);
        assert_eq!(l.violations.len(), 1, "{:?}", l.violations);
        assert!(
            l.violations[0].contains("20-overlap bytes") || l.violations[0].contains("overlap"),
            "{:?}",
            l.violations
        );
        assert!(!l.violations[0].contains("gap"), "{:?}", l.violations);
    }

    #[test]
    fn a_layout_that_does_not_start_at_zero_is_named_first() {
        let l = Layout::of([(10, 100), (110, 100)], 210);
        assert!(
            l.violations[0].contains("first chunk starts at 10"),
            "{:?}",
            l.violations
        );
    }

    #[test]
    fn a_total_that_disagrees_with_the_declared_size_is_its_own_violation() {
        let l = Layout::of([(0, 100), (100, 100)], 999);
        assert_eq!(l.violations.len(), 1, "{:?}", l.violations);
        assert!(
            l.violations[0].contains("total 200 B"),
            "{:?}",
            l.violations
        );
        assert!(
            l.violations[0].contains("declares 999 B"),
            "{:?}",
            l.violations
        );
    }

    /// Out-of-order chunk lists are legal to describe and illegal to use, and
    /// the difference has to show up.
    #[test]
    fn out_of_order_chunks_are_reported_but_still_usable() {
        let l = Layout::of([(200, 100), (0, 100), (100, 100)], 300);
        assert!(!l.is_clean());
        assert!(
            l.violations
                .iter()
                .any(|v| v.contains("100 bytes before") || v.contains("overlap")),
            "{:?}",
            l.violations
        );
        // And the ranges are still there, in offset order, because assembly needs them.
        let s: Vec<Range> = {
            let mut s = l.ranges.clone();
            s.sort();
            s
        };
        assert_eq!(s[0], Range::at(0, 100));
        assert_eq!(s[2], Range::at(200, 100));
    }

    #[test]
    fn a_zero_length_chunk_is_reported_rather_than_silently_allowed() {
        let l = Layout::of([(0, 100), (100, 0), (100, 100)], 200);
        assert!(
            l.violations.iter().any(|v| v.contains("zero-length")),
            "{:?}",
            l.violations
        );
    }

    #[test]
    fn an_empty_file_is_clean_and_empty() {
        let l = Layout::of([], 0);
        assert!(l.is_clean());
        assert!(l.ranges.is_empty());
        assert_eq!(l.total, 0);
    }

    #[test]
    fn ranges_serialise_with_start_and_end_the_schema_doc_names() {
        let j = serde_json::to_value(Range::at(690719, 1626019)).unwrap();
        assert_eq!(j["start"], 690719);
        assert_eq!(j["end"], 690719 + 1626019);
        assert!(j.get("len").is_none(), "length is derived, not stored");
    }

    /// **K1.** The crafted index's `offset: i64::MAX, size: 1` is a violation,
    /// not an abort. This is the exact arithmetic that panicked in debug before
    /// the fix, and wrapped into an inverted range in release.
    #[test]
    fn an_offset_that_overflows_is_a_violation_not_a_panic() {
        let l = Layout::of([(i64::MAX, 1)], 1);
        assert!(!l.is_clean());
        assert!(
            l.violations
                .iter()
                .any(|v| v.contains("overflow") && v.contains("9223372036854775807")),
            "{:?}",
            l.violations
        );
        assert_eq!(Range::at(i64::MAX, 1), Range::new(i64::MAX, i64::MAX));
    }

    /// **K1.** Two maximum-length ranges overflow `Σ len`; the sum is a named
    /// violation rather than a debug abort or a release wrap.
    #[test]
    fn a_total_that_overflows_is_a_violation_not_a_panic() {
        let l = Layout::of([(i64::MIN, i64::MAX), (i64::MIN, i64::MAX)], 0);
        assert!(!l.is_clean());
        assert!(
            l.violations.iter().any(|v| v.contains("sum overflows i64")),
            "{:?}",
            l.violations
        );
    }

    /// **K1.** The gap/overlap diagnostic reads `i64::MIN - i64::MAX` for this
    /// pair. It saturates, so the layout still describes the defect.
    #[test]
    fn a_gap_between_extreme_bounds_saturates_instead_of_panicking() {
        let l = Layout::of([(0, i64::MAX), (i64::MIN, 0)], 0);
        assert!(!l.is_clean());
        assert_eq!(
            Range::new(0, i64::MAX).to(Range::new(i64::MIN, 0)),
            i64::MIN
        );
        assert_eq!(
            Range::new(i64::MIN, 0).len(),
            i64::MAX,
            "the length of an inverted extreme range saturates too"
        );
    }
}
