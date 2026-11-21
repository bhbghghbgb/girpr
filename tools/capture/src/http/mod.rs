//! The HTTP fetcher and the request log that makes a capture auditable.
//!
//! The mechanics (transport, retrying fetcher, request log, error-body budget)
//! live in the logic-free `gircap-http` crate and are re-exported here so every
//! existing `crate::http::*` path keeps resolving. The two modules that carry
//! gircap policy — [`probe`] (which tags the API serves) and [`negative`] (which
//! inputs prove a refusal) — stay here, since neither belongs in a mechanics
//! crate and both depend on other gircap subsystems.
//!
//! **M1.** [`Fetcher`] owns retries, the timeout, the `User-Agent`, and the
//! provenance record that becomes `api/requests.ndjson`. A bundle that records
//! *what it fetched* is a bundle someone can re-fetch and diff; one that records
//! only its *results* is a claim.
//!
//! [`probe`] answers a different question — which tags will the API serve at
//! all — and the difference is the fact `diff_tags` does not carry.

#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::time::{Duration, Instant};

#[cfg(test)]
use crate::exit::ExitCode;

// Every public mechanics item is re-exported at the path callers already use.
// The crate owns no program policy, so this is a thin alias, not a facade over
// two halves of the same rebuild set: the heavy reqwest code compiles once in
// gircap-http and stays cached across edits here.
pub use gircap_http::*;

pub mod negative;
pub mod probe;

#[cfg(test)]
mod tests;

/// Test-only seam for the exit-code mapping that deliberately stays out of the
/// mechanics crate. `ApiError` may be reported as exit 2 or exit 3 depending on
/// where it happened, so only gircap owns that decision; the shipped method used
/// to live on the error type and returned the network code for callers that had
/// not classified it yet, which is what these assertions still check.
#[cfg(test)]
pub(crate) trait ApiErrorExitCode {
    fn exit_code(&self) -> ExitCode;
}

#[cfg(test)]
impl ApiErrorExitCode for ApiError {
    fn exit_code(&self) -> ExitCode {
        ExitCode::Network
    }
}

pub use probe::TagProbe;
