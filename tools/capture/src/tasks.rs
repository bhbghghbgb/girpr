//! The ordered-join loop and the panic-preserving `JoinError` rule.
//!
//! The mechanics now live in `gircap-http`, where the fetcher uses them. This
//! module is the thin shim the rest of gircap still resolves through: it keeps
//! the `crate::tasks::*` paths used by the blob fill pass, the request-log write
//! and the run summary unchanged while the compiled code lives in one crate.

pub(crate) use gircap_http::tasks::{JoinOutcome, join_blocking, join_ordered};
