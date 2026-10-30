//! The bodies of incidental non-2xx responses, kept as bundle content.
//!
//! **F5.** A `404` from a CDN or a `429` from a throttle carries a body worth
//! keeping: it is the origin's own words about why it would not serve, and the
//! alternative — the status alone — tells a later reader what happened but not
//! what the API said. [`drain_error_body`](super::fetcher) already paid for those
//! bytes on the wire; this module is where they stop being thrown away.
//!
//! # Two budgets, because an error body is still untrusted
//!
//! - `--max-response-bytes` bounds **reading**, exactly as it does for a success.
//! - [`ERROR_BODY_BUDGET_BYTES`] bounds **retention** for the whole run. A
//!   throttled run can fail thousands of chunk requests, and keeping every
//!   `429` page would make the bundle larger than the data it exists to
//!   describe.
//!
//! A response whose body did not fit is still recorded, with `truncated: true`
//! and the bytes that were kept. Nothing is silently dropped: the row is the
//! fact, the body is the evidence, and when the evidence had to be cut the row
//! says so.
//!
//! # Why the rows are sorted
//!
//! Chunk fetches run concurrently, so the order rows are pushed is the order
//! responses happened to finish. `seq` is assigned at issue time and
//! [`ErrorBodies::snapshot`] sorts by it, so two captures of one fixture write
//! the same document regardless of which download won a race.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// How many bytes of incidental error bodies one run keeps.
///
/// Sized after the observed reference bodies (~300–400 B of XML) multiplied by
/// far more failures than a healthy run has, and deliberately far below
/// `--max-response-bytes`: an error body is evidence, not the artifact.
pub const ERROR_BODY_BUDGET_BYTES: u64 = 8 * 1024 * 1024;

/// One non-2xx response whose body was seen, with the bytes that were kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorBody {
    /// The `seq` of the request in `api/requests.ndjson`.
    pub seq: u64,
    /// The URL requested.
    pub url: String,
    /// Where the response actually came from.
    pub final_url: String,
    /// The status as received.
    pub status: u16,
    /// This attempt's retry index: `1` for the first try, `2` for the second, …
    pub attempts: u32,
    /// `Content-Type` as received, when the origin sent one.
    pub content_type: Option<String>,
    /// Bytes that crossed the wire, whether or not they were retained. This is
    /// the same number the request row prices.
    pub bytes_received: u64,
    /// The bytes kept, at most `--max-response-bytes` and at most what the run's
    /// budget still allowed.
    pub body: Vec<u8>,
    /// Whether `body` is the whole received body. True for a read that was cut by
    /// the ceiling, a read that failed mid-way, and a body the retention budget
    /// could not fully hold.
    pub truncated: bool,
}

impl ErrorBody {
    /// How many bytes were kept.
    pub fn retained_bytes(&self) -> u64 {
        self.body.len() as u64
    }
}

/// One row of `api/error_responses.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponseRow {
    /// The `seq` of the request in `api/requests.ndjson`.
    pub seq: u64,
    /// The URL requested.
    pub url: String,
    /// Where the response actually came from.
    pub final_url: String,
    /// The status as received.
    pub status: u16,
    /// This attempt's retry index: `1` for the first try, `2` for the second, …
    pub attempts: u32,
    /// `Content-Type` as received, when the origin sent one.
    pub content_type: Option<String>,
    /// Bytes that crossed the wire, as `api/requests.ndjson` prices them.
    pub bytes_received: u64,
    /// Bytes kept in `stored_path`.
    pub retained_bytes: u64,
    /// Whether the kept bytes are the whole received body.
    pub truncated: bool,
    /// Where the kept bytes are, artifact-relative; `None` when none were kept.
    pub stored_path: Option<String>,
}

/// `api/error_responses.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponses {
    /// The artifact schema version.
    pub schema_version: u32,
    /// The run-wide retention budget this document was written under.
    pub budget_bytes: u64,
    /// `Σ retained_bytes`.
    pub total_retained_bytes: u64,
    /// One row per non-2xx attempt seen by `super::Fetcher::run`, in `seq`
    /// order.
    pub responses: Vec<ErrorResponseRow>,
}

/// The run-wide error-body store, shared by every clone of a [`Fetcher`].
///
/// [`Fetcher`]: super::Fetcher
#[derive(Debug)]
pub struct ErrorBodies {
    budget: u64,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    /// Bytes already promised to rows in `rows`.
    used: u64,
    rows: Vec<ErrorBody>,
}

impl ErrorBodies {
    /// A store with an explicit budget, for tests and for a future flag.
    pub fn new(budget: u64) -> Self {
        ErrorBodies {
            budget,
            state: Mutex::new(State::default()),
        }
    }

    /// A store with [`ERROR_BODY_BUDGET_BYTES`].
    pub fn with_default_budget() -> Self {
        Self::new(ERROR_BODY_BUDGET_BYTES)
    }

    /// How many bytes the run can still keep.
    ///
    /// Read *before* a body is buffered, so the buffer does not grow past what
    /// could possibly be retained. It is not a reservation: two concurrent reads
    /// may both see the same remainder, and [`Self::push`] is what makes the
    /// total exact by truncating whichever row arrives later.
    pub fn remaining(&self) -> u64 {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.budget.saturating_sub(state.used)
    }

    /// The budget this store enforces.
    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// Record one response, truncating its body to what the budget still allows.
    ///
    /// A truncated body still produces a row, with `truncated: true`: losing the
    /// observation because the evidence did not fit would be the same mistake as
    /// throwing the body away in the first place.
    pub fn push(&self, mut row: ErrorBody) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let room = self.budget.saturating_sub(state.used);
        if row.body.len() as u64 > room {
            row.body
                .truncate(usize::try_from(room).unwrap_or(usize::MAX));
            row.truncated = true;
        }
        state.used = state.used.saturating_add(row.body.len() as u64);
        state.rows.push(row);
    }

    /// Every row, in `seq` order.
    pub fn snapshot(&self) -> Vec<ErrorBody> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut rows = state.rows.clone();
        rows.sort_by_key(|r| r.seq);
        rows
    }

    /// How many rows have been recorded.
    pub fn len(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .rows
            .len()
    }

    /// Whether nothing has been recorded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for ErrorBodies {
    fn default() -> Self {
        Self::with_default_budget()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(seq: u64, body: &[u8]) -> ErrorBody {
        ErrorBody {
            seq,
            url: format!("https://example.test/{seq}"),
            final_url: format!("https://example.test/{seq}"),
            status: 404,
            attempts: 1,
            content_type: Some("text/xml".to_string()),
            bytes_received: body.len() as u64,
            body: body.to_vec(),
            truncated: false,
        }
    }

    #[test]
    fn the_whole_body_is_kept_when_the_budget_allows_it() {
        let store = ErrorBodies::new(1024);
        store.push(row(1, b"<Error/>"));
        let rows = store.snapshot();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, b"<Error/>");
        assert_eq!(rows[0].retained_bytes(), 8);
        assert!(!rows[0].truncated);
        assert_eq!(store.remaining(), 1024 - 8);
    }

    #[test]
    fn a_body_over_the_budget_is_truncated_and_says_so() {
        let store = ErrorBodies::new(4);
        store.push(row(2, b"123456"));
        let rows = store.snapshot();
        assert_eq!(rows[0].body, b"1234");
        assert_eq!(rows[0].bytes_received, 6);
        assert_eq!(rows[0].retained_bytes(), 4);
        assert!(
            rows[0].truncated,
            "the row must not claim it kept everything"
        );
        assert_eq!(store.remaining(), 0);
    }

    #[test]
    fn a_later_row_gets_what_the_budget_has_left() {
        let store = ErrorBodies::new(6);
        store.push(row(1, b"1234"));
        store.push(row(2, b"4567"));
        let rows = store.snapshot();
        assert_eq!(rows[0].body, b"1234");
        assert_eq!(rows[1].body, b"45");
        assert!(rows[1].truncated);
        assert_eq!(store.remaining(), 0);
    }

    #[test]
    fn rows_are_returned_in_seq_order_not_push_order() {
        // Chunk fetches finish out of order; the document must not.
        let store = ErrorBodies::new(1024);
        store.push(row(9, b"nine"));
        store.push(row(2, b"two"));
        store.push(row(5, b"five"));
        let seqs: Vec<u64> = store.snapshot().iter().map(|r| r.seq).collect();
        assert_eq!(seqs, vec![2, 5, 9]);
    }

    #[test]
    fn a_body_larger_than_the_ceiling_is_cut_at_the_ceiling() {
        // The fetcher passes `remaining().min(max_response_bytes)` as the
        // retention cap, so this is the store's share of that promise.
        let store = ErrorBodies::new(2);
        let mut r = row(1, b"abcdef");
        r.body.truncate(2);
        r.truncated = true;
        store.push(r);
        assert_eq!(store.snapshot()[0].body, b"ab");
    }
}
