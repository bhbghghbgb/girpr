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
            total.saturating_add(size.max(0) as u64)
        } else {
            total
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
