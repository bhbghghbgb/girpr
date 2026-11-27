//! The download-projection fold shared by the repair binary and the capture
//! tool.
//!
//! A program decides *which* chunks it will fetch — a local-reuse map, a scan
//! verdict, a write mode, a retention set — and hands this leaf one entry per
//! candidate chunk: `(compressed_size, downloads)`. The leaf answers exactly one
//! question: how many transfer bytes does that selection cost? It chooses no
//! set, keeps no index, compares no identity and makes no reuse decision; the
//! boolean is the caller's decision and the leaf only respects it.
//!
//! ## The one figure, one arithmetic
//!
//! The returned number is simultaneously the repair `PLAN` total, the
//! `SUMMARY plan_download_bytes` key and the capture bundle's projected fetch
//! bytes, and it is also the input to the `--max-download-bytes` ceiling and
//! the artifact ranking. Keeping the arithmetic here is what prevents those
//! surfaces from drifting: both programs must sum the same declarations the
//! same way. Nothing here emits a key or fixes a byte order — that stays with
//! each program's report plane.
//!
//! ## Why saturate, and why clamp negatives
//!
//! `compressed_size` is server-controlled `i64` data. A corrupt manifest can
//! declare `i64::MAX` on several chunks; the honest answer is "more than this
//! program can count", not a debug abort or a release wrap. Sizes below zero
//! are not transfers, so they contribute zero. The result is therefore an
//! **estimate and a lower bound, not a promise**: a failed local reuse can
//! turn a zero estimate into a full download, and both ceiling checks remain
//! the runtime enforcement, not this figure.

/// Widen a signed byte quantity to `u64`, treating a negative declaration as
/// no bytes at all.
///
/// Manifest sizes and offsets are server-controlled `i64`. A negative value is
/// not a transfer and not a length, so casting it straight to `u64` would turn
/// `-1` into eighteen quintillion bytes. This is the clamp-then-widen step every
/// byte fold in the two programs has to agree on.
///
/// ```
/// use girpr_chunk_cost::nonneg;
///
/// assert_eq!(nonneg(7), 7);
/// assert_eq!(nonneg(0), 0);
/// assert_eq!(nonneg(-1), 0);
/// assert_eq!(nonneg(i64::MIN), 0);
/// ```
pub fn nonneg(value: i64) -> u64 {
    value.max(0) as u64
}

/// Fold signed byte quantities into one saturating `u64` total.
///
/// Each term is clamped with [`nonneg`] before it is added, so a negative
/// declaration contributes nothing and a hostile declaration cannot overflow
/// the accumulator. Adding non-negative terms in any order yields the same
/// result, so callers may stream their own order.
///
/// The terms are `i64` because that is the type the wire declarations use; a
/// total that is already `u64` is none of this leaf's business.
///
/// ```
/// use girpr_chunk_cost::saturating_nonneg_sum;
///
/// assert_eq!(saturating_nonneg_sum([10, -5, 7, 0]), 17);
/// assert_eq!(saturating_nonneg_sum([i64::MAX, i64::MAX, 1]), u64::MAX);
/// ```
pub fn saturating_nonneg_sum<I>(values: I) -> u64
where
    I: IntoIterator<Item = i64>,
{
    values
        .into_iter()
        .fold(0u64, |total, value| total.saturating_add(nonneg(value)))
}

/// The exclusive end of a signed `[offset, offset + len)` range as a `u64`.
///
/// This is for *signed cursors*: the addition is checked in `i64` first, so an
/// end the signed type cannot represent is reported as `None` instead of being
/// saturated or wrapped into a plausible-looking `u64`. A negative result
/// clamps to zero, the same as [`nonneg`]. Byte totals that merely sum
/// declarations should use [`saturating_nonneg_sum`] instead — saturation is
/// their honest answer, but it would hide a broken cursor.
///
/// ```
/// use girpr_chunk_cost::checked_end;
///
/// assert_eq!(checked_end(10, 5), Some(15));
/// assert_eq!(checked_end(-10, 5), Some(0));
/// assert_eq!(checked_end(i64::MAX, 1), None);
/// ```
pub fn checked_end(offset: i64, len: i64) -> Option<u64> {
    offset.checked_add(len).map(nonneg)
}

/// Sum the transfer cost of the chunks a caller marked as downloads.
///
/// Each entry is `(compressed_size, downloads)`: the first half is the
/// manifest's signed declaration, the second is the caller's own decision for
/// that chunk. A chunk not marked as a download contributes nothing; a marked
/// chunk contributes `compressed_size.max(0)`, added with saturation so a
/// hostile manifest cannot overflow the accumulator. Adding positive terms in
/// any order yields the same result, so callers may stream entries in their
/// own order.
///
/// ```
/// use girpr_chunk_cost::projected_download_bytes;
///
/// // Three chunks, one reused locally: only the other two cross the wire.
/// let entries = [(10_i64, false), (20, true), (-5, true), (7, true)];
/// assert_eq!(projected_download_bytes(entries), 27);
/// ```
pub fn projected_download_bytes<I>(entries: I) -> u64
where
    I: IntoIterator<Item = (i64, bool)>,
{
    entries.into_iter().fold(0u64, |total, (size, downloads)| {
        if downloads {
            total.saturating_add(nonneg(size))
        } else {
            total
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The clamp is exact at zero and at both signed extremes: a negative
    /// declaration is no bytes, not a huge `u64` through a sign cast.
    #[test]
    fn nonneg_clamps_every_negative_and_widens_the_rest() {
        assert_eq!(nonneg(0), 0);
        assert_eq!(nonneg(1), 1);
        assert_eq!(nonneg(10), 10);
        assert_eq!(nonneg(-1), 0);
        assert_eq!(nonneg(-10), 0);
        assert_eq!(nonneg(i64::MIN), 0);
        assert_eq!(nonneg(i64::MAX), i64::MAX as u64);
    }

    /// The sum skips negatives, keeps zeros, saturates on overflow, and is
    /// empty-fold-zero. Order cannot change the answer: every term that reaches
    /// the accumulator is non-negative.
    #[test]
    fn saturating_nonneg_sum_folds_signed_terms_without_wrapping() {
        assert_eq!(saturating_nonneg_sum([]), 0);
        assert_eq!(saturating_nonneg_sum([3, -4, 0, 5]), 8);
        assert_eq!(saturating_nonneg_sum([-1, i64::MIN]), 0);
        assert_eq!(
            saturating_nonneg_sum([i64::MAX, 1, -7]),
            i64::MAX as u64 + 1
        );
        assert_eq!(
            saturating_nonneg_sum([i64::MAX, i64::MAX]),
            (i64::MAX as u64) * 2
        );
        assert_eq!(saturating_nonneg_sum([i64::MAX, i64::MAX, 1]), u64::MAX);
        assert_eq!(
            saturating_nonneg_sum([7, i64::MAX, 9]),
            saturating_nonneg_sum([9, 7, i64::MAX])
        );
    }

    /// A signed cursor end is checked before widening: a negative offset
    /// clamps, a negative result is zero, and a signed overflow is refused
    /// rather than saturated into a believable `u64`.
    #[test]
    fn checked_end_reports_signed_overflow_and_clamps_negatives() {
        assert_eq!(checked_end(0, 0), Some(0));
        assert_eq!(checked_end(10, 5), Some(15));
        assert_eq!(checked_end(-10, 5), Some(0));
        assert_eq!(checked_end(i64::MIN, 5), Some(0));
        assert_eq!(checked_end(i64::MIN, i64::MAX), Some(0));
        assert_eq!(checked_end(-1, 1), Some(0));
        assert_eq!(checked_end(i64::MAX - 1, 1), Some(i64::MAX as u64));
        assert_eq!(checked_end(i64::MAX, 1), None);
        assert_eq!(checked_end(i64::MAX, i64::MAX), None);
        assert_eq!(checked_end(i64::MIN, -1), None);
    }

    /// Only the entries the caller marks as downloads are charged.
    #[test]
    fn only_marked_chunks_are_counted() {
        let entries = [(10_i64, false), (20, true), (7, false), (5, true)];
        assert_eq!(projected_download_bytes(entries), 25);
    }

    /// A negative declaration is not a transfer: it contributes zero rather
    /// than wrapping a large `u64` through the sign cast.
    #[test]
    fn negative_sizes_contribute_zero() {
        assert_eq!(projected_download_bytes([(-1_i64, true), (9, true)]), 9);
        assert_eq!(projected_download_bytes([(i64::MIN, true)]), 0);
    }

    /// Several hostile declarations saturate instead of panicking in debug or
    /// wrapping in release; the answer is the largest `u64`.
    #[test]
    fn hostile_sizes_saturate() {
        assert_eq!(
            projected_download_bytes([(i64::MAX, true), (i64::MAX, true), (1, true)]),
            u64::MAX
        );
    }

    /// Nothing marked means no transfer, whatever the declarations say.
    #[test]
    fn nothing_downloads_costs_nothing() {
        assert_eq!(
            projected_download_bytes([(i64::MAX, false), (42, false)]),
            0
        );
    }

    /// The fold is order-independent over non-negative terms, so a caller may
    /// stream entries in whatever order its own selection produces.
    #[test]
    fn the_fold_ignores_entry_order() {
        let a = [(3_i64, true), (0, false), (5, true), (2, true)];
        let b = [(2_i64, true), (5, true), (3, true), (0, false)];
        assert_eq!(projected_download_bytes(a), projected_download_bytes(b));
    }
}
