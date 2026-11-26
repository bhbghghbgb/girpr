use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use girpr_retry::{DelaySequence, Schedule};

// The sleep hook lives in the shared leaf so both tools inject the same three
// drivers. Re-exported here to keep the `http` module's public surface (and the
// in-tree imports that reach through it) unchanged.
pub use girpr_retry::{RealSleep, RecordingSleep, Sleep, ZeroSleep};

/// When to retry, and how long to wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Attempts in total, including the first. `1` means never retry.
    pub attempts: u32,
}

impl RetryPolicy {
    /// Delay before the second attempt; doubles each time after that.
    const BASE_DELAY: Duration = Duration::from_millis(500);
    /// Ceiling on the doubling, so `--retries 10` cannot sleep for an hour.
    const MAX_DELAY: Duration = Duration::from_secs(10);

    /// The default for a `--retries` value.
    pub fn new(attempts: u32) -> Self {
        RetryPolicy {
            attempts: attempts.max(1),
        }
    }

    /// A fresh, no-jitter schedule for one request's retry loop.
    ///
    /// The leaf's `Schedule` owns the doubling and the clamp: the delay before
    /// retry *k* is `BASE_DELAY * 2^(k-1)` capped at `MAX_DELAY`, with the
    /// overflow-safe clamp coming from `backoff` rather than a hand-rolled
    /// shift. A fresh `DelaySequence` is needed per request because the
    /// schedule is stateful: every retry loop advances its own delay counter.
    ///
    /// `jitter = 0.0` is **jitter: None**. `next_delay` still draws an internal
    /// random number, but with a zero-width range the drawn value cannot change
    /// the returned duration, so the schedule — and every artifact derived from
    /// a capture — stays deterministic and diffable. This is the opposite of
    /// girpr, which deliberately passes `0.5` for RNG de-synchronization.
    pub fn backoff(&self) -> DelaySequence {
        Schedule::exponential(Self::BASE_DELAY, 2.0, 0.0, Self::MAX_DELAY).build()
    }

    /// Whether a status is worth another attempt.
    ///
    /// The split is the whole retry policy in miniature: a server that is
    /// overloaded or briefly broken may answer again, and a server that has
    /// decided this URL does not exist will not.
    pub fn should_retry_status(status: u16) -> bool {
        matches!(status, 408 | 429) || (500..=599).contains(&status)
    }
}

/// The default ceiling on one response body: 64 MiB.
///
/// **Seven times the largest thing this tool fetches.** The reference bundle's
/// biggest single response is the 7.1.0 `game` manifest archive at 9 007 187 B,
/// and the biggest chunk blob is 2 162 366 B. The headroom is deliberate: a
/// ceiling that a legitimate future tag could brush against would be a worse
/// failure than the one it prevents, because it would turn a working capture into
/// a non-working one.
///
/// The point of the ceiling is not this number. It is that there is a number at
/// all: a broken or hostile origin that never stops sending would otherwise be
/// bounded by nothing but the machine's memory. See
/// [`Transport::max_response_bytes`].
pub const DEFAULT_MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

/// How much concurrency and patience a fetcher has.
#[derive(Debug, Clone, Copy)]
pub struct Transport {
    /// Attempts per request, including the first.
    pub retries: u32,
    /// Per-request timeout.
    pub timeout: Duration,
    /// The most bytes one response body may contribute, in total.
    ///
    /// **A timeout is not a size limit.** A server streaming a gigabyte steadily
    /// for ten minutes never trips one, so the ceiling is the only bound on the
    /// path between a socket and this tool's buffers.
    ///
    /// Checked on every body, successful or not, in two places: against the
    /// declared `Content-Length` before a byte is read (so a server claiming
    /// 10 GiB is refused immediately), and against the running total as bytes
    /// arrive (so a server that lies about or omits the header is still stopped).
    ///
    /// What exceeding it *means* depends on the response. A successful body over
    /// the ceiling is a [`super::ApiError::Shape`], so it is **not** retried: an
    /// origin that sent too much will send too much again. An error body is only
    /// counted: the `4xx`/`5xx` status is still the failure reported, because the
    /// body is not worth reading and the status is what the caller needs.
    pub max_response_bytes: u64,
}

impl Transport {
    /// For a local mock, where a retry or a long timeout would turn a fast
    /// failure into a slow one.
    pub const fn local() -> Self {
        Transport {
            retries: 1,
            timeout: Duration::from_secs(10),
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }
}

// ------------------------------------------------------------ http transport --

/// One outgoing GET, as an [`HttpTransport`] sees it.
///
/// `range` is a literal `Range` header value. The production transport puts it
/// on the wire; an in-memory transport may ignore it, exactly as the TCP mock
/// does. The fetcher still records it in the request row either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// The absolute URL to fetch.
    pub url: String,
    /// A literal `Range` header value, when one was requested.
    pub range: Option<String>,
}

/// Where an [`HttpResponse`]'s body comes from.
///
/// A concrete enum rather than another trait object: [`HttpResponse::chunk`]
/// stays an inherent `async fn`, which keeps `drain_capped`/`drain_error_body`
/// free of boxed futures and lets the two transports share every byte-accounting
/// path verbatim.
pub enum ResponseBody {
    /// A live body from a socket, drained chunk by chunk.
    Live(reqwest::Response),
    /// Pre-built chunks, used by an in-memory transport.
    Memory(VecDeque<Vec<u8>>),
}

impl fmt::Debug for ResponseBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResponseBody::Live(_) => f.write_str("Live"),
            ResponseBody::Memory(chunks) => f.debug_tuple("Memory").field(chunks).finish(),
        }
    }
}

/// One HTTP response, independent of how it was obtained.
///
/// The shape is chosen so the fetcher's byte-accounting, header recording and
/// redirect detection are written **once** and shared by the live (`reqwest`)
/// transport and any in-memory one. A response carries the header map its own
/// path received, so `resp_headers` in `api/requests.ndjson` reflects that path:
/// the in-memory helper's three headers, or the axum mock's set including the
/// `date` hyper adds. The two paths are not byte-identical.
pub struct HttpResponse {
    /// Status as received.
    pub status: u16,
    /// Where the request actually landed (after any redirect).
    pub final_url: String,
    /// The response headers, exactly as received (names lowercased by `http`).
    pub headers: reqwest::header::HeaderMap,
    /// The declared body length, when the origin declared one.
    pub content_length: Option<u64>,
    body: ResponseBody,
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("final_url", &self.final_url)
            .field("headers", &self.headers)
            .field("content_length", &self.content_length)
            .field("body", &self.body)
            .finish()
    }
}

impl HttpResponse {
    /// Adapt a live `reqwest` response.
    pub fn from_reqwest(resp: reqwest::Response) -> Self {
        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();
        let headers = resp.headers().clone();
        let content_length = resp.content_length();
        HttpResponse {
            status,
            final_url,
            headers,
            content_length,
            body: ResponseBody::Live(resp),
        }
    }

    /// Build a response in memory with the three headers this path supplies.
    ///
    /// `connection: keep-alive` and `content-type` mirror the axum mock's
    /// explicit headers, and `content-length` (the total of all chunks) matches
    /// the length hyper computes for the TCP path. Hyper also adds its own
    /// `date` there, so a capture through this in-memory transport records
    /// these three headers rather than the TCP mock's full `resp_headers`.
    pub fn in_memory(
        status: u16,
        final_url: String,
        content_type: Option<&str>,
        chunks: Vec<Vec<u8>>,
    ) -> Self {
        let total: usize = chunks.iter().map(Vec::len).sum();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONNECTION,
            reqwest::header::HeaderValue::from_static("keep-alive"),
        );
        headers.insert(
            reqwest::header::CONTENT_LENGTH,
            reqwest::header::HeaderValue::from_str(&total.to_string())
                .expect("a byte count is a valid header value"),
        );
        if let Some(ct) = content_type
            && let Ok(value) = reqwest::header::HeaderValue::from_str(ct)
        {
            headers.insert(reqwest::header::CONTENT_TYPE, value);
        }
        HttpResponse {
            status,
            final_url,
            headers,
            content_length: Some(total as u64),
            body: ResponseBody::Memory(chunks.into()),
        }
    }

    /// The next body chunk, as bytes, or `None` when the body is exhausted.
    ///
    /// `reqwest` yields a refcounted [`Bytes`], so the live path hands that
    /// buffer straight through — no per-chunk `Vec` allocation or byte copy.
    /// The in-memory path already owns a `Vec`, which [`Bytes::from`] adopts
    /// without copying. Both sink paths write through `&chunk`, so the bytes
    /// that reach the wire, the digests and the record are unchanged.
    pub async fn chunk(&mut self) -> Result<Option<Bytes>, String> {
        match &mut self.body {
            ResponseBody::Live(resp) => resp.chunk().await.map_err(|e| e.to_string()),
            ResponseBody::Memory(chunks) => Ok(chunks.pop_front().map(Bytes::from)),
        }
    }
}

/// The future [`HttpTransport::send`] hands back, boxed because this trait is a
/// trait object (an `async fn` or `impl Future` is not dyn-compatible).
pub type SendFuture = Pin<Box<dyn Future<Output = Result<HttpResponse, String>> + Send + 'static>>;

/// Something that can issue a GET and hand back a response.
///
/// The second injection seam on [`super::Fetcher`], alongside [`Sleep`]. `Send +
/// Sync` is required because a `Fetcher` is `Clone` and shared across `--jobs`
/// workers. The default implementation is [`ReqwestTransport`], which wraps the
/// same `reqwest::Client` the tool has always used, so a live capture stays
/// byte-identical; a test may inject an in-memory implementation instead.
pub trait HttpTransport: Send + Sync + fmt::Debug {
    /// Issue `request`. `Err` carries the transport-level detail; the fetcher
    /// wraps it in `ApiError::Transport` with the attempt number it knows.
    fn send(&self, request: HttpRequest) -> SendFuture;
}

/// The production [`HttpTransport`]: a real `reqwest` client.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    /// Wrap a configured client (timeout, user agent, default headers already set).
    pub fn new(client: reqwest::Client) -> Self {
        ReqwestTransport { client }
    }
}

impl HttpTransport for ReqwestTransport {
    fn send(&self, request: HttpRequest) -> SendFuture {
        let client = self.client.clone();
        Box::pin(async move {
            let mut builder = client.get(&request.url);
            if let Some(range) = request.range.as_deref() {
                builder = builder.header(reqwest::header::RANGE, range);
            }
            let resp = builder.send().await.map_err(|e| e.to_string())?;
            Ok(HttpResponse::from_reqwest(resp))
        })
    }
}
