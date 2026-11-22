//! Pure range facts for numeric command-line values, shared by the three
//! programs.
//!
//! Three programs check numeric flags against a range and each turns a
//! violation into its own published wording and process status: the repair
//! binary uses `bail!`, the capture tool raises a `UsageError` that exits `1`,
//! and girsync deliberately refuses a zero outside clap so it exits `3` rather
//! than a parse error's `2`. What is common is only the question — is this
//! value below a minimum, above an optional maximum, or inside both? — and the
//! answer is a fact, not a decision.
//!
//! This crate owns no error type, no message, no exit code and no range
//! constant. Each caller passes the bounds it owns and maps the returned
//! [`RangeFact`] to its own wording and exit class, so no published byte and no
//! process-status contract can be changed from here. It is an acyclic leaf with
//! no dependencies.

/// Which bound an out-of-range value crossed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeBound {
    /// The value was below the caller's inclusive minimum.
    Minimum,
    /// The value was above the caller's inclusive maximum.
    Maximum,
}

/// The fact produced by checking one value against an inclusive range.
///
/// Pure and program-free. The caller decides what [`RangeFact::OutOfRange`]
/// means in its own error type, wording and exit class; the fact records only
/// the offending value, which bound it crossed and that bound's limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeFact<T> {
    /// The value satisfies the minimum and, when present, the maximum.
    InRange,
    /// The value crossed one of the two bounds.
    OutOfRange {
        /// The value the caller supplied.
        value: T,
        /// The bound it crossed.
        bound: RangeBound,
        /// The limit of the crossed bound.
        limit: T,
    },
}

/// Check `value` against an inclusive `[min, max]` range.
///
/// `max` is optional so a caller can express "at least `min`" without inventing
/// a sentinel ceiling. The minimum is checked first: only a value that passes
/// it can be reported as crossing the maximum, so a caller with a coherent
/// range gets exactly one answer and knows which bound its own wording should
/// name.
///
/// ```
/// use girpr_value_ranges::{RangeBound, RangeFact, check_in_range};
///
/// assert_eq!(check_in_range(4_usize, 1, Some(64)), RangeFact::InRange);
/// assert_eq!(
///     check_in_range(0_usize, 1, Some(64)),
///     RangeFact::OutOfRange { value: 0, bound: RangeBound::Minimum, limit: 1 }
/// );
/// assert_eq!(
///     check_in_range(100_u32, 1, Some(64)),
///     RangeFact::OutOfRange { value: 100, bound: RangeBound::Maximum, limit: 64 }
/// );
/// // No maximum: any value at or above the minimum is in range.
/// assert_eq!(check_in_range(1_u64, 1, None), RangeFact::InRange);
/// ```
pub fn check_in_range<T: PartialOrd + Copy>(value: T, min: T, max: Option<T>) -> RangeFact<T> {
    if value < min {
        return RangeFact::OutOfRange {
            value,
            bound: RangeBound::Minimum,
            limit: min,
        };
    }
    if let Some(max) = max
        && value > max
    {
        return RangeFact::OutOfRange {
            value,
            bound: RangeBound::Maximum,
            limit: max,
        };
    }
    RangeFact::InRange
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both inclusive bounds are accepted, and just outside each is not.
    #[test]
    fn checks_both_inclusive_bounds() {
        assert_eq!(check_in_range(1_usize, 1, Some(10)), RangeFact::InRange);
        assert_eq!(check_in_range(10_usize, 1, Some(10)), RangeFact::InRange);
        assert_eq!(
            check_in_range(0_usize, 1, Some(10)),
            RangeFact::OutOfRange {
                value: 0,
                bound: RangeBound::Minimum,
                limit: 1
            }
        );
        assert_eq!(
            check_in_range(11_usize, 1, Some(10)),
            RangeFact::OutOfRange {
                value: 11,
                bound: RangeBound::Maximum,
                limit: 10
            }
        );
    }

    /// Without a maximum, any value at or above the minimum is in range.
    #[test]
    fn open_upper_bound_accepts_everything_at_or_above_min() {
        assert_eq!(check_in_range(1_usize, 1, None), RangeFact::InRange);
        assert_eq!(check_in_range(usize::MAX, 1, None), RangeFact::InRange);
        assert_eq!(
            check_in_range(0_usize, 1, None),
            RangeFact::OutOfRange {
                value: 0,
                bound: RangeBound::Minimum,
                limit: 1
            }
        );
    }

    /// The fact carries the offending value and the crossed bound's limit, so
    /// a caller can spell the number without re-deriving it.
    #[test]
    fn fact_carries_value_bound_and_limit() {
        match check_in_range(1_025_u64, 1, Some(1_024)) {
            RangeFact::OutOfRange {
                value,
                bound: RangeBound::Maximum,
                limit,
            } => {
                assert_eq!(value, 1_025);
                assert_eq!(limit, 1_024);
            }
            other => panic!("unexpected fact: {other:?}"),
        }
    }

    /// The predicate works for every unsigned width the three callers use.
    #[test]
    fn works_for_each_caller_width() {
        assert!(matches!(
            check_in_range(0_u32, 1, None),
            RangeFact::OutOfRange { .. }
        ));
        assert_eq!(check_in_range(2_u32, 1, None), RangeFact::InRange);
        assert_eq!(check_in_range(7_usize, 1, Some(64)), RangeFact::InRange);
        assert!(matches!(
            check_in_range(65_usize, 1, Some(64)),
            RangeFact::OutOfRange { .. }
        ));
        assert_eq!(check_in_range(1_u64, 1, Some(10)), RangeFact::InRange);
    }
}
