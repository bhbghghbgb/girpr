//! The logic-free HTTP mechanics owned by `gircap`.
//!
//! This crate owns `reqwest` and the fetcher/transport/request-log mechanics:
//! retries, timeout, headers, the request provenance record and the error-body
//! retention budget. It holds no program policy — no exit codes, no version
//! pairs, no region resolution, no probe decisions, no artifact writing. Those
//! stay in the `gircap` package, which depends on this crate directly so that an
//! edit to `emit`/`blobs`/`manifest` leaves this crate (and its native `reqwest`
//! tree) cached.
//!
//! ## Seams to the parent package
//!
//! Three narrow seams keep this crate independent of `gircap`:
//!
//! - [`BodyFile`] — the caller's digesting file. The byte sink and its digests
//!   belong to the parent; this crate only writes bytes and calls `restart`.
//! - [`USER_AGENT`] / [`user_agent`] — the one header value, a fixed string.
//! - [`tasks`] — the seam for the ordered-join loop. The window and the panic
//!   rule live in the shared logic-free `girpr-async-join` leaf; this module
//!   keeps gircap's wording, and the parent re-exports it for its blob fill pass.
//!
//! ## What the request log records, and why each field earns its place
//!
//! | field | why |
//! |---|---|
//! | `kind` | so "what did the manifest fetch cost?" is a filter, not a string match on a URL |
//! | `elapsed_ms` | total across attempts, so a slow retry is visible as slowness |
//! | `attempts` | a 200 after three tries is a different fact from a 200 first time |
//! | `redirected` / `final_url` | a CDN that redirects is a CDN that can change what it serves |
//! | `resp_headers` | `etag`, `content-md5`, `x-cache` are how the CDN's claims get checked |
//! | `bytes_in` | the only way to price a manifest without re-downloading it |
//!
//! `bytes_in` counts **every** body received, including the body of a failed
//! request. A throttled run that was retried three times paid for four responses
//! and a summary that said `0` for three of them would be a cost figure that
//! flatters itself exactly when the cost was highest.
//!
//! ## Retries
//!
//! Retried: connection and timeout errors, `408`, `429`, and any `5xx`.
//! **Not** retried: every other `4xx` — a `404` on a chunk means the blob is
//! gone, and retrying it three times only makes the failure take longer and look
//! like a network problem. Backoff is exponential from `--retries`' base and
//! capped; there is no jitter, because a capture that adds randomness to its own
//! request log is harder to diff against a previous one.

pub mod error;
pub mod error_bodies;
pub mod fetcher;
pub mod log;
pub mod record;
pub mod tasks;
pub mod transport;

pub use error::ApiError;
pub use error_bodies::{
    ERROR_BODY_BUDGET_BYTES, ErrorBodies, ErrorBody, ErrorResponseRow, ErrorResponses,
};
pub use fetcher::{
    Destination, FetchFailure, Fetched, Fetcher, Observation, Streamed, body_failure,
    client_default_headers, shared_req_headers,
};
pub use log::{RequestLog, now_rfc3339};
pub use record::{ACCEPT, ACCEPT_ENCODING, Kind, RequestRecord, header_map};
pub use transport::{
    DEFAULT_MAX_RESPONSE_BYTES, HttpRequest, HttpResponse, HttpTransport, RealSleep,
    RecordingSleep, ReqwestTransport, ResponseBody, RetryPolicy, SendFuture, Sleep, Transport,
    ZeroSleep,
};

// The mechanics tests live with the mechanics: this crate's own test target
// links only reqwest/tokio and the fixtures it needs, so an edit to gircap's
// emit/blobs/manifest leaves these tests cached instead of re-linking them
// through the parent package's lib test target. The mock servers the tests use
// are this crate's own copies (one process per test under nextest).
#[cfg(test)]
mod tests;

/// The one `User-Agent` string sent on every request, shared with the repair
/// binary through the `girpr-hyp-wire` leaf. Re-exported here so the fetcher and
/// `gircap`'s `logging` module keep their existing names; the byte on the wire
/// and in the request log is unchanged.
pub use girpr_hyp_wire::{USER_AGENT, user_agent};

/// The narrow view of a caller-owned file that the fetcher streams a body into.
///
/// The trait is defined in the logic-free `girpr-digest` leaf, which both this
/// crate and the digesting-file owner already depend on. Keeping it there lets
/// the owner implement it without depending on this crate (and rebuilding when
/// this crate changes); it is re-exported here so fetcher call sites keep one
/// name. This crate writes through [`std::io::Write`] and calls
/// [`BodyFile::restart`] before a retry; the owner keeps the path, the digests
/// and the rename. The byte path is unchanged.
pub use girpr_digest::BodyFile;
