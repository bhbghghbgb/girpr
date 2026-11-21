use std::fmt;

/// Everything wrong with a fetch, all of which is exit `2`.
///
/// One type rather than four, because there is one answer to give for all of
/// them: the capture did not get a usable response. The distinction survives in
/// the message and in the logged record, which is where a reader needs it.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// The request never completed: DNS, TLS, connect, or timeout.
    #[error("{url}: transport failed after {attempts} attempt(s): {detail}")]
    Transport {
        /// The URL that failed.
        url: String,
        /// Attempts made, including the first.
        attempts: u32,
        /// What went wrong.
        detail: String,
    },
    /// The server answered with a status this fetcher will not accept.
    #[error("{url}: HTTP {status} after {attempts} attempt(s)")]
    Status {
        /// The URL that failed.
        url: String,
        /// The status received.
        status: u16,
        /// Attempts made.
        attempts: u32,
    },
    /// A `200` whose body is not a usable API answer.
    #[error("{subject}: {detail}")]
    Shape {
        /// Which endpoint, or what was being built.
        subject: String,
        /// What was wrong with it.
        detail: String,
    },
}

impl ApiError {
    /// A body that parsed but is not the shape this tool needs.
    pub fn shape(subject: impl Into<String>, detail: impl fmt::Display) -> Self {
        ApiError::Shape {
            subject: subject.into(),
            detail: detail.to_string(),
        }
    }

    // No exit-code mapping lives here. `ApiError::Shape` covers around fifty
    // conditions with nothing to do with the network, and the same value is
    // legitimately reported as exit 2 or exit 3 depending on where it happened;
    // only the caller knows which. gircap keeps that policy on its side of the
    // boundary, so this mechanics crate owns no program exit-code contract.

    /// The URL this failure is about, when it is about one.
    pub fn url(&self) -> Option<&str> {
        match self {
            ApiError::Transport { url, .. } | ApiError::Status { url, .. } => Some(url),
            ApiError::Shape { subject, .. } => Some(subject),
        }
    }
}
