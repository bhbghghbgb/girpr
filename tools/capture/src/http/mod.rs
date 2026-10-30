//! The HTTP fetcher and the request log that makes a capture auditable.
//!
//! **M1.** [`Fetcher`] owns retries, the timeout, the `User-Agent`, and the
//! provenance record that becomes `api/requests.ndjson`. A bundle that records
//! *what it fetched* is a bundle someone can re-fetch and diff; one that records
//! only its *results* is a claim.
//!
//! [`probe`] answers a different question — which tags will the API serve at
//! all — and the difference is the fact `diff_tags` does not carry.
//!
//! ## What is recorded, and why each field earns its place
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

#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::{Arc, Mutex};
#[cfg(test)]
use std::time::{Duration, Instant};

#[cfg(test)]
use crate::exit::ExitCode;

pub mod error;
pub mod error_bodies;
pub mod fetcher;
pub mod log;
pub mod negative;
pub mod probe;
pub mod record;
pub mod transport;

pub use error::ApiError;
pub use error_bodies::{
    ERROR_BODY_BUDGET_BYTES, ErrorBodies, ErrorBody, ErrorResponseRow, ErrorResponses,
};
pub(crate) use fetcher::shared_req_headers;
pub use fetcher::{Destination, Fetched, Fetcher, Observation, Streamed};

#[cfg(test)]
use fetcher::{body_failure, client_default_headers};
pub use log::{RequestLog, now_rfc3339};
pub use probe::TagProbe;
pub(crate) use record::header_map;
pub use record::{ACCEPT, ACCEPT_ENCODING, Kind, RequestRecord};
pub use transport::{
    DEFAULT_MAX_RESPONSE_BYTES, RealSleep, RecordingSleep, RetryPolicy, Sleep, SleepFuture,
    Transport,
};

#[cfg(test)]
mod tests;
