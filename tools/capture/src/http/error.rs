use std::fmt;

use crate::exit::ExitCode;

/// Everything wrong with a fetch, all of which is exit `2`.
///
/// One type rather than four, because there is one answer to give for all of
/// them: the capture did not get a usable response. The distinction survives in
/// the message and in the logged record, which is where a reader needs it.
#[derive(Debug)]
pub enum ApiError {
    /// The request never completed: DNS, TLS, connect, or timeout.
    Transport {
        /// The URL that failed.
        url: String,
        /// Attempts made, including the first.
        attempts: u32,
        /// What went wrong.
        detail: String,
    },
    /// The server answered with a status this fetcher will not accept.
    Status {
        /// The URL that failed.
        url: String,
        /// The status received.
        status: u16,
        /// Attempts made.
        attempts: u32,
    },
    /// A `200` whose body is not a usable API answer.
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

    /// The default exit code for a failure that has not been classified yet.
    ///
    /// **This is not the tool's exit-code policy.** Two things stop it from being
    /// one:
    ///
    /// - [`ApiError::Shape`] covers around fifty conditions with nothing to do
    ///   with the network: local invariants, writer errors, budget ceilings,
    ///   assembly steps. A `2` is a network code, and it would be wrong for most of
    ///   them.
    /// - The same error value is legitimately reported as exit `2` or exit `3`
    ///   depending on **where it happened**, which `ApiError` cannot express.
    ///   [`crate::failure::apierr`] and [`crate::failure::assertion`] take the same
    ///   `&ApiError` and differ only in the code they choose, because the run
    ///   knows whether the request failed or the answer came back unusable.
    ///
    /// Production picks at the call site; this is what a caller uses when it has
    /// not picked yet, and today only tests do. Splitting `ApiError` so the code
    /// could travel with the error is a design change, deliberately not made.
    ///
    /// The real contract is pinned by
    /// `failure::tests::every_failure_constructor_agrees_on_the_summary_shape`,
    /// which feeds one `ApiError::Shape` to both constructors and requires two
    /// different codes from it.
    pub const fn exit_code(&self) -> ExitCode {
        ExitCode::Network
    }

    /// The URL this failure is about, when it is about one.
    pub fn url(&self) -> Option<&str> {
        match self {
            ApiError::Transport { url, .. } | ApiError::Status { url, .. } => Some(url),
            ApiError::Shape { subject, .. } => Some(subject),
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Transport {
                url,
                attempts,
                detail,
            } => write!(
                f,
                "{url}: transport failed after {attempts} attempt(s): {detail}"
            ),
            ApiError::Status {
                url,
                status,
                attempts,
            } => {
                write!(f, "{url}: HTTP {status} after {attempts} attempt(s)")
            }
            ApiError::Shape { subject, detail } => write!(f, "{subject}: {detail}"),
        }
    }
}

impl std::error::Error for ApiError {}
