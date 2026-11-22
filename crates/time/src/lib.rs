//! The named UTC timestamp formats shared across the workspace.
//!
//! A stamp is a byte contract: file names, capture ids, manifest ids, request
//! logs and lock bodies all carry one of these spellings and are compared or
//! parsed verbatim. The formats are deliberately distinct — six fractional
//! digits and a `Z` suffix for the sortable stamp, millisecond precision and a
//! `Z` suffix for request logs, whole seconds for the manifest, and chrono's
//! default RFC 3339 for the lock body. Each has one definition here and one
//! name; none is a parameter of another.
//!
//! Every formatter takes the instant as an argument (so a caller can stamp the
//! moment a run *started* rather than the moment it asks) and has a `_now`
//! companion for the common case. The leaf owns chrono so its callers do not.

// Unit tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use chrono::{DateTime, SecondsFormat, Utc};

/// The compact, sortable stamp: `20261014T101500.000000Z`.
///
/// Six fractional digits, a literal `T` separator and a literal `Z` suffix —
/// the spelling is pinned, so none of those may change. Sortable as text, so a
/// `logs/` directory or a bundle root reads oldest-first when listed by name.
pub fn compact_stamp(now: &DateTime<Utc>) -> String {
    now.format("%Y%m%dT%H%M%S%.6fZ").to_string()
}

/// The compact stamp for the current instant.
pub fn compact_stamp_now() -> String {
    compact_stamp(&Utc::now())
}

/// RFC 3339 in UTC, millisecond precision, with a `Z` suffix.
pub fn rfc3339_millis_z(now: &DateTime<Utc>) -> String {
    now.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The millisecond RFC 3339 spelling for the current instant.
pub fn rfc3339_millis_z_now() -> String {
    rfc3339_millis_z(&Utc::now())
}

/// RFC 3339 in UTC, whole-second precision, with a `Z` suffix.
pub fn rfc3339_secs_z(now: &DateTime<Utc>) -> String {
    now.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// RFC 3339 as chrono renders it by default, used by the lock body's `started`
/// field. Unlike the two spellings above this carries chrono's own offset
/// rendering and automatic fractional precision; it is its own named format.
pub fn rfc3339(now: &DateTime<Utc>) -> String {
    now.to_rfc3339()
}

/// The default RFC 3339 spelling for the current instant.
pub fn rfc3339_now() -> String {
    rfc3339(&Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instant() -> DateTime<Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-10-14T10:15:00Z")
            .expect("a valid instant")
            .with_timezone(&Utc)
    }

    /// The four names stay distinct and each keeps its exact spelling.
    #[test]
    fn each_named_format_keeps_its_own_spelling() {
        let t = instant();
        assert_eq!(compact_stamp(&t), "20261014T101500.000000Z");
        assert_eq!(rfc3339_millis_z(&t), "2026-10-14T10:15:00.000Z");
        assert_eq!(rfc3339_secs_z(&t), "2026-10-14T10:15:00Z");
        assert_eq!(rfc3339(&t), "2026-10-14T10:15:00+00:00");
    }

    /// Each formatter is still the exact expression its caller used before the
    /// move, including for an instant with a fractional part.
    #[test]
    fn formatters_match_the_original_expressions() {
        let t = instant() + chrono::Duration::milliseconds(123);
        assert_eq!(
            compact_stamp(&t),
            t.format("%Y%m%dT%H%M%S%.6fZ").to_string()
        );
        assert_eq!(
            rfc3339_millis_z(&t),
            t.to_rfc3339_opts(SecondsFormat::Millis, true)
        );
        assert_eq!(
            rfc3339_secs_z(&t),
            t.to_rfc3339_opts(SecondsFormat::Secs, true)
        );
        assert_eq!(rfc3339(&t), t.to_rfc3339());
    }

    /// The compact stamp keeps the shape its old home pinned.
    #[test]
    fn the_compact_stamp_keeps_its_shape() {
        let stamp = compact_stamp_now();
        assert_eq!(stamp.len(), 23, "{stamp}");
        assert_eq!(stamp.as_bytes()[8], b'T');
        assert_eq!(stamp.as_bytes()[15], b'.');
        assert_eq!(stamp.as_bytes()[22], b'Z');
    }

    /// A supplied instant still orders the way a named file needs it to.
    #[test]
    fn a_supplied_instant_orders_as_before() {
        let t = instant();
        let later = t + chrono::Duration::seconds(1);
        assert!(compact_stamp(&later) > compact_stamp(&t));
    }
}
