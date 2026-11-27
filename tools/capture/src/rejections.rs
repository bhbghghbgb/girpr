//! Values the capture refused, recorded so the refusal is evidence rather than
//! only an exit code.
//!
//! **F6/G1.** The write rules in [`crate::paths`] keep a hostile or broken
//! manifest from naming a file outside the bundle. The rule is right; throwing
//! the value away is not. A manifest that says
//! `GenshinImpact_Data/../../../etc/passwd` is a fact about the API — the most
//! interesting fact a capture could learn that day — and a log line that scrolls
//! away is a poor place to keep it.
//!
//! # What is recorded
//!
//! One row per refused manifest entry `file` path or chunk id: where it came
//! from, the value itself, and the rule it failed. Only
//! [`crate::manifest`]'s decode check writes here; a flag, a flag-supplied tag
//! or an audio field is refused before a bundle exists and is reported only in
//! the error text. [`Rejections`] is a run-wide store so the values survive the
//! point where the run refuses and can be written into
//! `diagnostics/rejections.json` when a bundle is still produced (or into the
//! would-be bundle directory when the refusal is the last thing that happens).
//!
//! # Two caps, because the values are untrusted
//!
//! - [`MAX_RETAINED_VALUE_BYTES`] bounds one value. A manifest field is a
//!   filename; a multi-megabyte "path" is an attack on the document, not
//!   evidence worth keeping whole.
//! - [`MAX_ROWS`] bounds the document. A manifest can hold 113 610 chunks, and a
//!   hostile one can make every one of them invalid; the document keeps the
//!   [`MAX_ROWS`] smallest rows by key and counts the rest in `omitted`, so the
//!   size is bounded, the sample is deterministic, and the loss is still visible.
//!
//! `serde_json` escapes the value on the way out: a NUL, a newline or a lone
//! quote in a path cannot break the document. The value is text rather than a
//! path, and nothing here ever joins it to a filesystem path.

use crate::http::bounded_store::BoundedStore;
use serde::{Deserialize, Serialize};

/// Longest value kept in a rejection row, in bytes.
///
/// A generous filename, with room for a path that is long but legitimate. The
/// cap exists to bound the document, not to second-guess the rule that already
/// refused the value.
pub const MAX_RETAINED_VALUE_BYTES: usize = 512;

/// Most rows one run records.
///
/// Far more than a healthy run can produce and far below what a hostile
/// 113 610-chunk manifest could ask for. What does not fit is counted in
/// [`RejectionsDocument::omitted`].
pub const MAX_ROWS: usize = 4096;

/// One refused value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejection {
    /// Where the value came from: a manifest entry's `file` path or one of its
    /// chunk ids.
    ///
    /// Includes enough context to find it again — the tag, field, entry index
    /// and chunk index for a manifest row — but never the refused value itself,
    /// which is in [`Self::value`].
    pub source: String,
    /// The value exactly as it arrived, escaped by the JSON writer.
    pub value: String,
    /// The rule it failed, as a stable identifier such as `path.device`.
    pub rule: String,
    /// Whether [`Self::value`] was cut at [`MAX_RETAINED_VALUE_BYTES`].
    pub truncated: bool,
}

/// `diagnostics/rejections.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectionsDocument {
    /// The artifact schema version.
    pub schema_version: u32,
    /// The per-value cap this document was written under.
    pub cap_bytes: u64,
    /// How many rows [`Self::rejections`] holds.
    pub total: u64,
    /// How many refusals were not retained: rows past the cap and repeats of a
    /// key already kept.
    pub omitted: u64,
    /// The retained rows, in `(source, value, rule)` order — the smallest keys
    /// seen, so two captures of one input write the same document regardless of
    /// which manifest finished first.
    pub rejections: Vec<Rejection>,
}

/// The run-wide store of refused values, shared by every fetch task.
///
/// The mutex, row storage and refusal counter are the shared mechanics; this
/// store injects the keyed retention policy below.
#[derive(Debug, Default)]
pub struct Rejections {
    inner: BoundedStore<Rejection>,
}

impl Rejections {
    /// A store with the documented caps.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one refused value.
    ///
    /// `source` and `rule` are chosen by the call site and are trusted: they are
    /// the tool's own words. `value` is not, and is capped here rather than at
    /// every call site.
    ///
    /// The store keeps the [`MAX_ROWS`] smallest rows by `(source, value, rule)`
    /// rather than the first arrivals, so the sample cannot depend on which
    /// manifest finished first. A key already retained, or a key that is not
    /// smaller than the largest retained one once the store is full, is counted
    /// in `omitted` instead of being kept.
    pub fn record(
        &self,
        source: impl Into<String>,
        value: &str,
        rule: impl Into<String>,
    ) -> Rejection {
        let mut state = self.inner.lock();
        let (value, truncated) = cap_value(value);
        let row = Rejection {
            source: source.into(),
            value,
            rule: rule.into(),
            truncated,
        };
        // The retention key is explicit: a future field on `Rejection` must not
        // silently change which rows are kept.
        let key = (&row.source, &row.value, &row.rule);
        match state
            .rows
            .binary_search_by(|other| (&other.source, &other.value, &other.rule).cmp(&key))
        {
            Ok(_) => {
                // Identical rows are interchangeable, so the one already kept
                // stands for this refusal too.
                state.omitted = state.omitted.saturating_add(1);
            }
            Err(pos) => {
                if state.rows.len() < MAX_ROWS {
                    // Room while not full: insert in key order.
                    state.rows.insert(pos, row.clone());
                } else if pos < state.rows.len() {
                    // The new key sorts before the current largest, so drop
                    // the largest to make room. `rows` stays key-sorted.
                    state.rows.pop();
                    state.omitted = state.omitted.saturating_add(1);
                    state.rows.insert(pos, row.clone());
                } else {
                    state.omitted = state.omitted.saturating_add(1);
                }
            }
        }
        row
    }

    /// Every retained row, in the order the document writes them.
    pub fn snapshot(&self) -> Vec<Rejection> {
        // Inserts keep `rows` in key order; no sort is needed on the way out.
        self.inner.snapshot(|_| {})
    }

    /// How many rows have been recorded, including rows the cap dropped.
    pub fn len(&self) -> usize {
        let state = self.inner.lock();
        state.rows.len() + usize::try_from(state.omitted).unwrap_or(usize::MAX)
    }

    /// Whether nothing has been refused.
    pub fn is_empty(&self) -> bool {
        let state = self.inner.lock();
        state.rows.is_empty() && state.omitted == 0
    }

    /// The document to write, built from the rows recorded so far.
    pub fn document(&self) -> RejectionsDocument {
        let state = self.inner.lock();
        RejectionsDocument {
            schema_version: crate::SCHEMA_VERSION,
            cap_bytes: MAX_RETAINED_VALUE_BYTES as u64,
            total: state.rows.len() as u64,
            omitted: state.omitted,
            rejections: state.rows.clone(),
        }
    }
}

/// Cut `value` at [`MAX_RETAINED_VALUE_BYTES`] without splitting a character.
fn cap_value(value: &str) -> (String, bool) {
    if value.len() <= MAX_RETAINED_VALUE_BYTES {
        return (value.to_string(), false);
    }
    let mut end = MAX_RETAINED_VALUE_BYTES;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_carries_the_value_its_source_and_the_rule() {
        let store = Rejections::new();
        store.record("manifest 7.1.0/game entry file", "a/../b", "path.parent");
        let rows = store.snapshot();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, "manifest 7.1.0/game entry file");
        assert_eq!(rows[0].value, "a/../b");
        assert_eq!(rows[0].rule, "path.parent");
        assert!(!rows[0].truncated);
        assert_eq!(store.len(), 1);
        assert!(!store.is_empty());
    }

    #[test]
    fn a_long_value_is_capped_on_a_character_boundary_and_says_so() {
        let store = Rejections::new();
        // Three-byte characters, so `MAX_RETAINED_VALUE_BYTES` can land inside
        // one. The cap must step back rather than panic on a bad boundary.
        let value = "\u{4e2d}".repeat(MAX_RETAINED_VALUE_BYTES);
        store.record("--files", &value, "path.control");
        let row = &store.snapshot()[0];
        assert!(row.truncated);
        assert!(row.value.len() <= MAX_RETAINED_VALUE_BYTES);
        assert_eq!(row.value.len() % 3, 0);
        assert!(value.starts_with(&row.value));
    }

    #[test]
    fn the_document_is_sorted_so_two_runs_agree() {
        let a = Rejections::new();
        a.record("z", "2", "path.device");
        a.record("a", "9", "path.device");
        a.record("a", "1", "path.device");
        let b = Rejections::new();
        b.record("a", "1", "path.device");
        b.record("z", "2", "path.device");
        b.record("a", "9", "path.device");
        assert_eq!(a.document(), b.document());
        let rows = a.snapshot();
        let values: Vec<&str> = rows.iter().map(|r| r.value.as_str()).collect();
        assert_eq!(values, vec!["1", "9", "2"]);
    }

    #[test]
    fn rows_past_the_cap_are_counted_rather_than_silently_dropped() {
        let store = Rejections::new();
        for i in 0..(MAX_ROWS + 7) {
            store.record("source", &i.to_string(), "rule");
        }
        let doc = store.document();
        assert_eq!(doc.total, MAX_ROWS as u64);
        assert_eq!(doc.omitted, 7);
        assert_eq!(store.len(), MAX_ROWS + 7);
    }

    #[test]
    fn the_document_declares_the_caps_and_the_schema_version() {
        let doc = Rejections::new().document();
        assert_eq!(doc.schema_version, crate::SCHEMA_VERSION);
        assert_eq!(doc.cap_bytes, MAX_RETAINED_VALUE_BYTES as u64);
        assert_eq!(doc.total, 0);
        assert_eq!(doc.omitted, 0);
        assert!(doc.rejections.is_empty());
    }

    #[test]
    fn arrival_order_does_not_change_the_sample() {
        const EXTRA: usize = 9;
        let keys: Vec<String> = (0..(MAX_ROWS + EXTRA))
            .map(|i| format!("value {:06}", i))
            .collect();
        let forward = Rejections::new();
        let reverse = Rejections::new();
        for key in &keys {
            forward.record("source", key, "rule");
        }
        for key in keys.iter().rev() {
            reverse.record("source", key, "rule");
        }

        let a = forward.document();
        let b = reverse.document();
        assert_eq!(a, b);
        assert_eq!(a.rejections.len(), MAX_ROWS);
        let retained: Vec<&str> = a.rejections.iter().map(|r| r.value.as_str()).collect();
        let smallest: Vec<&str> = keys.iter().take(MAX_ROWS).map(String::as_str).collect();
        assert_eq!(retained, smallest);
        assert_eq!(a.total, MAX_ROWS as u64);
        assert_eq!(a.omitted, EXTRA as u64);
        assert_eq!(forward.len(), MAX_ROWS + EXTRA);
        assert_eq!(reverse.len(), MAX_ROWS + EXTRA);
    }

    #[test]
    fn a_smaller_key_displaces_the_largest_and_a_larger_key_is_omitted() {
        let store = Rejections::new();
        // Fill past the point where `k000000` and `k999999` sit, so one is
        // smaller than the current maximum and the other is larger.
        for i in 0..MAX_ROWS {
            store.record("source", &format!("k{:06}", i + 1000), "rule");
        }
        let full = store.document();
        let largest = full.rejections.last().unwrap().value.clone();

        store.record("source", "k000000", "rule");
        let smaller = store.document();
        assert_eq!(smaller.rejections.len(), MAX_ROWS);
        assert_eq!(smaller.rejections.first().unwrap().value, "k000000");
        assert!(!smaller.rejections.iter().any(|r| r.value == largest));
        assert_eq!(smaller.omitted, 1);

        store.record("source", "k999999", "rule");
        let larger = store.document();
        assert_eq!(larger.rejections, smaller.rejections);
        assert_eq!(larger.omitted, 2);
        assert_eq!(store.len(), MAX_ROWS + 2);
    }

    #[test]
    fn duplicate_keys_are_counted_without_changing_the_sample() {
        let store = Rejections::new();
        for i in 0..MAX_ROWS {
            store.record("source", &format!("k{:06}", i), "rule");
        }
        let full = store.document();
        // A repeat of a key already kept, even one smaller than the largest,
        // must not displace a distinct row.
        for _ in 0..3 {
            store.record("source", "k000000", "rule");
        }
        let after = store.document();
        assert_eq!(after.rejections, full.rejections);
        assert_eq!(after.total, MAX_ROWS as u64);
        assert_eq!(after.omitted, 3);
        assert_eq!(store.len(), MAX_ROWS + 3);
    }

    #[test]
    fn a_repeat_of_a_key_is_not_kept_twice_below_the_cap() {
        let store = Rejections::new();
        store.record("a", "1", "path.device");
        store.record("a", "1", "path.device");
        let doc = store.document();
        assert_eq!(doc.rejections.len(), 1);
        assert_eq!(doc.total, 1);
        assert_eq!(doc.omitted, 1);
        assert_eq!(store.len(), 2);
    }
}
