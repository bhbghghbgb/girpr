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
//! ## Retries
//!
//! Retried: connection and timeout errors, `408`, `429`, and any `5xx`.
//! **Not** retried: every other `4xx` — a `404` on a chunk means the blob is
//! gone, and retrying it three times only makes the failure take longer and look
//! like a network problem. Backoff is exponential from `--retries`' base and
//! capped; there is no jitter, because a capture that adds randomness to its own
//! request log is harder to diff against a previous one.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::exit::ExitCode;

pub mod probe;

pub use probe::TagProbe;

/// What a request was for. Recorded so a reader can ask about one class of
/// request without pattern-matching a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// One of the JSON endpoints.
    Api,
    /// A manifest archive blob.
    Manifest,
    /// A chunk blob.
    Chunk,
    /// A request made to *measure* the API rather than to consume data: a
    /// `getBuild?tag=` tag probe, or a negative probe for `api/error_shapes.json`.
    /// Costs no blob bytes, only JSON — or, for the negative blob probes, only a
    /// `404`.
    Probe,
}

impl Kind {
    /// The wire spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::Api => "api",
            Kind::Manifest => "manifest",
            Kind::Chunk => "chunk",
            Kind::Probe => "probe",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

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

    /// The code every API failure maps to.
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

/// When to retry, and how long to wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Attempts in total, including the first. `1` means never retry.
    pub attempts: u32,
    /// Delay before the second attempt; doubles each time after that.
    pub base_delay: Duration,
    /// Ceiling on the doubling, so `--retries 10` cannot sleep for an hour.
    pub max_delay: Duration,
}

impl RetryPolicy {
    /// The default for a `--retries` value.
    pub fn new(attempts: u32) -> Self {
        RetryPolicy {
            attempts: attempts.max(1),
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(10),
        }
    }

    /// No waiting at all — for tests, where a backoff turns a fast failure into a
    /// slow one and hides the cause.
    pub const fn immediate() -> Self {
        RetryPolicy {
            attempts: 1,
            base_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
        }
    }

    /// The delay before attempt number `attempt` (1-based; `1` never waits).
    ///
    /// Pure and off by construction: `base * 2^(attempt - 2)` is the delay
    /// *before* attempt 2, and the exponent is clamped before it can overflow a
    /// shift, which `--retries 10` would otherwise do at attempt 34.
    pub fn delay_before(&self, attempt: u32) -> Duration {
        if attempt <= 1 || self.base_delay.is_zero() {
            return Duration::ZERO;
        }
        let shift = (attempt - 2).min(16);
        let scaled = self
            .base_delay
            .checked_mul(1u32 << shift)
            .unwrap_or(self.max_delay);
        scaled.min(self.max_delay)
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

/// Something that can be waited on, so tests can count delays instead of
/// spending them.
pub trait Sleep: Send + Sync + fmt::Debug {
    /// Wait for `d`.
    fn sleep(&self, d: Duration);
}

/// Real wall-clock waiting.
#[derive(Debug, Default)]
pub struct RealSleep;

impl Sleep for RealSleep {
    fn sleep(&self, d: Duration) {
        if !d.is_zero() {
            std::thread::sleep(d);
        }
    }
}

/// Records what it was asked to wait for and returns immediately.
#[derive(Debug, Default)]
pub struct RecordingSleep(Mutex<Vec<Duration>>);

impl RecordingSleep {
    /// Every delay requested so far.
    pub fn delays(&self) -> Vec<Duration> {
        self.0.lock().expect("sleep log is not poisoned").clone()
    }
}

impl Sleep for RecordingSleep {
    fn sleep(&self, d: Duration) {
        self.0.lock().expect("sleep log is not poisoned").push(d);
    }
}

/// How much concurrency and patience a fetcher has.
#[derive(Debug, Clone, Copy)]
pub struct Transport {
    /// Concurrent chunk downloads. Advisory here; enforced by the caller.
    pub jobs: usize,
    /// Attempts per request, including the first.
    pub retries: u32,
    /// Per-request timeout.
    pub timeout: Duration,
}

impl Transport {
    /// For a local mock, where a retry or a long timeout would turn a fast
    /// failure into a slow one.
    pub const fn local() -> Self {
        Transport {
            jobs: 4,
            retries: 1,
            timeout: Duration::from_secs(10),
        }
    }
}

/// One row of `api/requests.ndjson`.
#[derive(Debug, Clone, Serialize)]
pub struct RequestRecord {
    /// Issue order, starting at 1. Assigned *before* the request goes out, so
    /// concurrent chunk downloads keep their issue order in the log rather than
    /// their completion order.
    pub seq: u64,
    /// RFC 3339, UTC, millisecond precision.
    pub ts: String,
    /// What the request was for.
    pub kind: Kind,
    /// Always `GET`; recorded so a future method is a visible schema change.
    pub method: &'static str,
    /// The URL requested.
    pub url: String,
    /// The literal `Range` header sent, if any.
    pub range: Option<String>,
    /// The status received, or `0` when no response arrived.
    pub status: u16,
    /// Where the response actually came from.
    pub final_url: String,
    /// Whether the client followed a redirect to get there.
    pub redirected: bool,
    /// Wall time across **all** attempts, so a slow retry reads as slowness.
    pub elapsed_ms: u64,
    /// What was sent. Keys lowercased.
    pub req_headers: BTreeMap<String, String>,
    /// What came back. Keys lowercased; values exactly as received.
    pub resp_headers: BTreeMap<String, String>,
    /// Body length in bytes.
    pub bytes_in: u64,
    /// Where the body was stored, filled in by the writer once it has one.
    pub stored_as: Option<String>,
    /// Attempts made, including the first.
    pub attempts: u32,
}

impl RequestRecord {
    /// Headers we always send, lowercased, for the record.
    fn default_req_headers(user_agent: &str, range: Option<&str>) -> BTreeMap<String, String> {
        let mut h = BTreeMap::new();
        h.insert("accept".to_string(), "*/*".to_string());
        h.insert("accept-encoding".to_string(), "gzip".to_string());
        h.insert("user-agent".to_string(), user_agent.to_string());
        if let Some(r) = range {
            h.insert("range".to_string(), r.to_string());
        }
        h
    }
}

/// The accumulated request log, shared by every fetch in a run.
///
/// Cheap to clone; all clones are the same log.
#[derive(Debug, Clone, Default)]
pub struct RequestLog(Arc<Mutex<LogState>>);

#[derive(Debug, Default)]
struct LogState {
    next_seq: u64,
    records: Vec<RequestRecord>,
}

impl RequestLog {
    /// An empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserve the next sequence number, at **issue** time.
    ///
    /// Reserved before the request goes out rather than assigned when the record
    /// lands, so `seq` is the order the requests were made in. A capture with
    /// `--jobs 8` would otherwise number its chunk fetches by which download
    /// happened to finish first, which is a function of the network.
    pub fn reserve(&self) -> u64 {
        let mut s = self.0.lock().expect("request log is not poisoned");
        s.next_seq += 1;
        s.next_seq
    }

    /// Append a record. Its `seq` is already assigned.
    pub fn push(&self, record: RequestRecord) {
        self.0
            .lock()
            .expect("request log is not poisoned")
            .records
            .push(record);
    }

    /// Every record, in `seq` order.
    pub fn records(&self) -> Vec<RequestRecord> {
        let mut v = self
            .0
            .lock()
            .expect("request log is not poisoned")
            .records
            .clone();
        // Sorted here, not appended in order. `reserve()` assigns `seq` at issue
        // time, so a `--jobs 8` capture reserves 1..N and then *lands* them in
        // whatever order the network finished; appending would leave the file's
        // rows in completion order while its `seq` column claimed otherwise,
        // and a reader who trusted the row order would reconstruct the wrong
        // provenance. Sorted on read rather than on push because `push` is
        // called once per chunk - a 113 610-chunk capture would spend O(n²)
        // keeping a vector ordered - and `records()` is read a handful of times.
        //
        // `reserve()` makes `seq` unique, so this is a total order.
        v.sort_by_key(|r| r.seq);
        v
    }

    /// Total bytes received across the run.
    pub fn bytes_in(&self) -> u64 {
        self.0
            .lock()
            .expect("request log is not poisoned")
            .records
            .iter()
            .map(|r| r.bytes_in)
            .sum()
    }

    /// How many requests were made.
    pub fn len(&self) -> usize {
        self.0
            .lock()
            .expect("request log is not poisoned")
            .records
            .len()
    }

    /// Whether nothing was requested.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Record where a body ended up, after the writer stored it.
    ///
    /// By `seq` rather than by URL, because a chunk id can legitimately be
    /// requested twice and the two requests are not the same request.
    pub fn set_stored_as(&self, seq: u64, path: impl Into<String>) {
        let mut s = self.0.lock().expect("request log is not poisoned");
        if let Some(r) = s.records.iter_mut().find(|r| r.seq == seq) {
            r.stored_as = Some(path.into());
        }
    }

    /// The log as newline-delimited JSON, one object per line.
    pub fn to_ndjson(&self) -> String {
        let mut out = String::new();
        for r in self.records() {
            match serde_json::to_string(&r) {
                Ok(line) => {
                    out.push_str(&line);
                    out.push('\n');
                }
                // A record that cannot serialize would otherwise vanish silently,
                // and a log with a hole in it is worse than one that says it has
                // one. `RequestRecord` is all primitives, so this is unreachable;
                // the line says so rather than being dropped.
                Err(e) => {
                    out.push_str(&format!(
                        "{{\"seq\":{},\"error\":\"unserializable record: {e}\"}}\n",
                        r.seq
                    ));
                }
            }
        }
        out
    }

    /// Write the log to `path`, creating parent directories.
    pub fn write_ndjson(&self, path: &Path) -> std::io::Result<usize> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let text = self.to_ndjson();
        std::fs::write(path, text.as_bytes())?;
        Ok(self.len())
    }
}

/// A fetched body and the record of how it arrived.
#[derive(Debug)]
pub struct Fetched {
    /// The body, byte-exact.
    pub body: Vec<u8>,
    /// The provenance row.
    pub record: RequestRecord,
}

impl Fetched {
    /// The body as UTF-8, for a response that is asserted to be JSON.
    pub fn text(&self) -> Result<&str, ApiError> {
        std::str::from_utf8(&self.body).map_err(|e| ApiError::shape("response", e))
    }

    /// The sequence number of this request, for `set_stored_as`.
    pub fn seq(&self) -> u64 {
        self.record.seq
    }
}

/// One response as observed, whatever its status.
///
/// A [`Fetched`] is only ever a success; this is the other half — the shape a
/// response has when the origin is saying *no*, kept as data rather than as an
/// [`ApiError`] because for a negative probe it **is** the data.
#[derive(Debug)]
pub struct Observation {
    /// The status as received. Not filtered, not normalised.
    pub status: u16,
    /// `Content-Type` as received, absent when the server sent none.
    pub content_type: Option<String>,
    /// The body, byte-exact.
    pub body: Vec<u8>,
    /// The provenance row.
    pub record: RequestRecord,
}

impl Observation {
    /// The body as UTF-8. Lossy is deliberate and is the only lossy step here:
    /// these bodies are XML or JSON error envelopes, and replacing an
    /// undecodable byte would be a worse record than an explicit U+FFFD.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The body's `retcode`, when it is a JSON API envelope.
    pub fn retcode(&self) -> Option<i64> {
        serde_json::from_slice::<serde_json::Value>(&self.body)
            .ok()?
            .get("retcode")
            .and_then(|r| r.as_i64())
    }
}

/// Issues requests, records them, and retries the ones worth retrying.
///
/// Cloneable, and for one reason: M6 fetches chunk blobs concurrently. A clone
/// shares the pool and the log — `reqwest::Client` is an `Arc` internally and
/// [`RequestLog`] is already one — so N clones are N handles on one fetcher
/// rather than N fetches with N logs. The log assigns each record its `seq` at
/// issue time from shared state, so concurrency does not make the numbering a
/// function of which download happened to finish first.
#[derive(Debug, Clone)]
pub struct Fetcher {
    client: reqwest::Client,
    policy: RetryPolicy,
    sleeper: Arc<dyn Sleep>,
    log: RequestLog,
    user_agent: String,
}

impl Fetcher {
    /// Build a fetcher with the real clock.
    pub fn new(transport: Transport, log: RequestLog) -> Result<Self, ApiError> {
        Self::with_sleeper(transport, log, Arc::new(RealSleep))
    }

    /// Build a fetcher with an injected sleeper, for tests that assert on the
    /// backoff schedule rather than spending it.
    pub fn with_sleeper(
        transport: Transport,
        log: RequestLog,
        sleeper: Arc<dyn Sleep>,
    ) -> Result<Self, ApiError> {
        let ua = crate::logging::user_agent();
        let client = reqwest::Client::builder()
            .timeout(transport.timeout)
            .user_agent(ua.clone())
            .build()
            .map_err(|e| ApiError::shape("http client", e))?;
        Ok(Fetcher {
            client,
            policy: RetryPolicy::new(transport.retries),
            sleeper,
            log,
            user_agent: ua,
        })
    }

    /// The log every fetch through this fetcher is recorded in.
    pub fn log(&self) -> &RequestLog {
        &self.log
    }

    /// The retry policy in force.
    pub fn policy(&self) -> &RetryPolicy {
        &self.policy
    }

    /// Fetch one URL **once** and report whatever came back, status and body
    /// included.
    ///
    /// **Never retries and never turns a non-2xx into an error.** The caller is
    /// asking what the origin *does* with a bad request, so a `404` here is the
    /// measurement, not a failure — and a second identical request would only buy
    /// another `404` while making the probe indistinguishable from a fetch that
    /// went wrong. This is the same reason [`probe_tags`](crate::http::probe)
    /// treats a `retcode -202` as an answer.
    ///
    /// Only a transport failure is an error: there is nothing to record, and a gap
    /// in the document would read as "this shape was never observed".
    ///
    /// Recorded in the request log like any other request, so the probe's cost is
    /// visible in the same place every other request's is.
    pub async fn observe(&self, url: &str, kind: Kind) -> Result<Observation, ApiError> {
        let started = Instant::now();
        let seq = self.log.reserve();
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| ApiError::Transport {
                url: url.to_string(),
                attempts: 1,
                detail: e.to_string(),
            })?;
        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();
        let redirected = !final_url.eq_ignore_ascii_case(url);
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let resp_headers = header_map(resp.headers());
        // A body that will not read is an empty body, not a failure: the status
        // and the content-type are the parts a consumer can rely on, and they
        // have already been captured by this point.
        let body = resp.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
        let bytes_in = body.len() as u64;
        let record = self.record(
            seq,
            kind,
            url,
            None,
            status,
            resp_headers,
            &final_url,
            redirected,
            bytes_in,
            1,
            started,
        );
        self.log.push(record.clone());
        Ok(Observation {
            status,
            content_type,
            body,
            record,
        })
    }

    /// Fetch one URL into memory.
    ///
    /// `range` is a literal `Range` header value — `bytes=10-99` — and is
    /// recorded verbatim when sent. The CDNs honour it (measured: `206` +
    /// `Content-Range`), which is how a manifest can be inspected without
    /// downloading it whole.
    pub async fn get(
        &self,
        url: &str,
        kind: Kind,
        range: Option<&str>,
    ) -> Result<Fetched, ApiError> {
        let started = Instant::now();
        let mut attempts: u32 = 0;
        let mut last: Option<ApiError> = None;

        while attempts < self.policy.attempts {
            attempts += 1;
            let waiting = self.policy.delay_before(attempts);
            if !waiting.is_zero() {
                tracing::debug!(url, ?waiting, attempts, "retry backoff");
                self.sleeper.sleep(waiting);
            }

            match self.attempt(url, kind, range, attempts, started).await {
                Ok((body, record)) => {
                    self.log.push(record.clone());
                    return Ok(Fetched { body, record });
                }
                Err((err, record)) => {
                    // The failed attempt is logged too: a run that hit a CDN
                    // throttle and recovered must show the 429, or `attempts`
                    // is a claim nobody can check.
                    self.log.push(record);
                    let retryable = match &err {
                        ApiError::Transport { .. } => true,
                        ApiError::Status { status, .. } => {
                            RetryPolicy::should_retry_status(*status)
                        }
                        ApiError::Shape { .. } => false,
                    };
                    if !retryable {
                        return Err(err);
                    }
                    tracing::debug!(url, attempts, error = %err, "retryable failure");
                    last = Some(err);
                }
            }
        }

        Err(last.unwrap_or_else(|| ApiError::shape(url, "no attempt was made")))
    }

    /// Fetch and parse as JSON, checking that the wrapper says `retcode == 0`.
    ///
    /// The `retcode` check is here rather than at each call site so that no
    /// endpoint can be read without it. A `200` carrying `retcode -202` is the
    /// documented way this API says *no such tag*.
    pub async fn get_json(
        &self,
        url: &str,
        kind: Kind,
    ) -> Result<(Vec<u8>, serde_json::Value), ApiError> {
        let f = self.get(url, kind, None).await?;
        let v: serde_json::Value = serde_json::from_slice(&f.body).map_err(|e| {
            // A CDN error page lands here. Naming the wrapper it should have been
            // is more use to whoever reads the log than the parser's column
            // number, which only says where the `<` was.
            ApiError::shape(
                url,
                format!("not a JSON API wrapper (expected `retcode`): {e}"),
            )
        })?;
        let retcode = v.get("retcode").and_then(|r| r.as_i64()).ok_or_else(|| {
            ApiError::shape(url, "response has no `retcode`; this is not an API wrapper")
        })?;
        if retcode != 0 {
            return Err(ApiError::Shape {
                subject: url.to_string(),
                detail: format!(
                    "retcode {retcode} ({})",
                    v.get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("no message")
                ),
            });
        }
        Ok((f.body, v))
    }

    /// One attempt. Returns the record even on failure so the caller can log it.
    ///
    /// The sequence number is taken **here**, at issue time, once per HTTP
    /// request. A logical fetch that retries three times writes three rows, and
    /// numbering them at the logical level would put the same number on all
    /// three — which is what `seq` exists to prevent.
    ///
    /// The error arm carries a full record by design: a failure that is not
    /// recorded is a failure nobody can debug. Boxing it would allocate on every
    /// failed attempt to save a stack copy of something consumed immediately.
    #[allow(clippy::result_large_err)]
    async fn attempt(
        &self,
        url: &str,
        kind: Kind,
        range: Option<&str>,
        attempts: u32,
        started: Instant,
    ) -> Result<(Vec<u8>, RequestRecord), (ApiError, RequestRecord)> {
        let seq = self.log.reserve();
        let mut req = self.client.get(url);
        if let Some(r) = range {
            req = req.header(reqwest::header::RANGE, r);
        }
        tracing::debug!(seq, url, kind = kind.as_str(), ?range, attempts, "GET");

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let err = ApiError::Transport {
                    url: url.to_string(),
                    attempts,
                    detail: e.to_string(),
                };
                let record = self.record(
                    seq,
                    kind,
                    url,
                    range,
                    0,
                    BTreeMap::new(),
                    url,
                    false,
                    0,
                    attempts,
                    started,
                );
                return Err((err, record));
            }
        };

        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();
        // reqwest does not expose a redirect count, so "did anything redirect"
        // is answered by comparing where the request went with where it landed.
        // It is a comparison, not a hop count, and is recorded as such.
        let redirected = !final_url.eq_ignore_ascii_case(url);
        let resp_headers = header_map(resp.headers());

        // `2xx`, which includes `206` — a `Range` response is a success, not a
        // partial failure.
        if !resp.status().is_success() {
            let err = ApiError::Status {
                url: url.to_string(),
                status,
                attempts,
            };
            let record = self.record(
                seq,
                kind,
                url,
                range,
                status,
                resp_headers,
                &final_url,
                redirected,
                0,
                attempts,
                started,
            );
            return Err((err, record));
        }

        match resp.bytes().await {
            Ok(b) => {
                let bytes_in = b.len() as u64;
                let record = self.record(
                    seq,
                    kind,
                    url,
                    range,
                    status,
                    resp_headers,
                    &final_url,
                    redirected,
                    bytes_in,
                    attempts,
                    started,
                );
                Ok((b.to_vec(), record))
            }
            Err(e) => {
                let err = ApiError::Transport {
                    url: url.to_string(),
                    attempts,
                    detail: format!("reading the body: {e}"),
                };
                let record = self.record(
                    seq,
                    kind,
                    url,
                    range,
                    status,
                    resp_headers,
                    &final_url,
                    redirected,
                    0,
                    attempts,
                    started,
                );
                Err((err, record))
            }
        }
    }

    /// Assemble a row. Every path through [`Fetcher::attempt`] builds one, which
    /// is why no branch can log a request without recording it.
    #[allow(clippy::too_many_arguments)]
    fn record(
        &self,
        seq: u64,
        kind: Kind,
        url: &str,
        range: Option<&str>,
        status: u16,
        resp_headers: BTreeMap<String, String>,
        final_url: &str,
        redirected: bool,
        bytes_in: u64,
        attempts: u32,
        started: Instant,
    ) -> RequestRecord {
        RequestRecord {
            seq,
            ts: now_rfc3339(),
            kind,
            method: "GET",
            url: url.to_string(),
            range: range.map(str::to_string),
            status,
            final_url: final_url.to_string(),
            redirected,
            elapsed_ms: started.elapsed().as_millis() as u64,
            req_headers: RequestRecord::default_req_headers(&self.user_agent, range),
            resp_headers,
            bytes_in,
            stored_as: None,
            attempts,
        }
    }
}

/// Lowercase every header key, keeping the value exactly as received.
///
/// Sorted, because a `BTreeMap` makes two captures of the same request diff
/// cleanly — and an `ETag` is *not* a string this tool parses, so nothing is
/// lost to the normalisation.
fn header_map(h: &reqwest::header::HeaderMap) -> BTreeMap<String, String> {
    h.iter()
        .map(|(k, v)| {
            (
                k.as_str().to_ascii_lowercase(),
                v.to_str()
                    .map(str::to_string)
                    // A header with non-UTF-8 bytes is recorded as its hex, not
                    // dropped: losing it would make the record incomplete in a
                    // way nobody could see.
                    .unwrap_or_else(|_| v.as_bytes().iter().map(|b| format!("{b:02x}")).collect()),
            )
        })
        .collect()
}

/// RFC 3339 in UTC, millisecond precision.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biz::{Biz, BranchParams};
    use crate::cli::Biz as BizId;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicU32, Ordering};

    // ---------------------------------------------------------------- URLs --

    #[test]
    fn the_measured_global_urls_are_reproduced_exactly() {
        let b = Biz::resolve(BizId::Hk4eGlobal);
        assert_eq!(
            b.bases()
                .hyp_url(&b, crate::biz::HypEndpoint::GameConfigs, "en-us"),
            "https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api/getGameConfigs\
             ?launcher_id=VYTpXlbWo8&language=en-us&game_ids[]=gopR6Cufr3"
        );
        let p = BranchParams {
            package_id: "ScSYQBFhu9".into(),
            password: "bDL4JUHL625x".into(),
            branch: "main".into(),
            tag: "7.1.0".into(),
            diff_tags: vec![],
        };
        assert_eq!(
            p.get_build_url(b.sophon_base(), Some("7.0.0")),
            "https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api/getBuild\
             ?branch=main&package_id=ScSYQBFhu9&password=bDL4JUHL625x&tag=7.0.0"
        );
    }

    // --------------------------------------------------------------- retry --

    #[test]
    fn the_backoff_is_exponential_from_the_second_attempt_and_capped() {
        let p = RetryPolicy {
            attempts: 10,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(10),
        };
        assert_eq!(p.delay_before(1), Duration::ZERO);
        assert_eq!(p.delay_before(2), Duration::from_millis(500));
        assert_eq!(p.delay_before(3), Duration::from_millis(1000));
        assert_eq!(p.delay_before(4), Duration::from_millis(2000));
        assert_eq!(p.delay_before(5), Duration::from_millis(4000));
        assert_eq!(p.delay_before(6), Duration::from_millis(8000));
        assert_eq!(p.delay_before(7), Duration::from_secs(10), "capped");
        // A shift overflow would panic or wrap; it must clamp instead.
        assert_eq!(p.delay_before(64), Duration::from_secs(10));
    }

    #[test]
    fn an_immediate_policy_never_waits() {
        let p = RetryPolicy::immediate();
        assert_eq!(p.attempts, 1);
        for a in 1..=10 {
            assert_eq!(p.delay_before(a), Duration::ZERO);
        }
    }

    #[test]
    fn at_least_one_attempt_is_always_made() {
        assert_eq!(RetryPolicy::new(0).attempts, 1);
        assert_eq!(RetryPolicy::new(1).attempts, 1);
        assert_eq!(RetryPolicy::new(5).attempts, 5);
    }

    /// The whole retry policy in one test: a `404` is not worth retrying, a
    /// `503` is, and a timeout is.
    #[test]
    fn only_overload_and_server_faults_are_retryable() {
        for s in [408, 429, 500, 502, 503, 504, 599] {
            assert!(RetryPolicy::should_retry_status(s), "{s} should retry");
        }
        for s in [200, 204, 206, 301, 302, 400, 401, 403, 404, 410, 451] {
            assert!(!RetryPolicy::should_retry_status(s), "{s} must not retry");
        }
    }

    // ----------------------------------------------------------------- log --

    #[test]
    fn sequence_numbers_are_reserved_at_issue_time_and_stay_unique() {
        let log = RequestLog::new();
        let a = log.reserve();
        let b = log.reserve();
        assert_eq!((a, b), (1, 2));
        // Reserving before pushing is what keeps concurrent fetches in issue
        // order; the log stays sorted even if records arrive out of order.
        let mk = |seq, url: &str| RequestRecord {
            seq,
            ts: now_rfc3339(),
            kind: Kind::Chunk,
            method: "GET",
            url: url.to_string(),
            range: None,
            status: 200,
            final_url: url.to_string(),
            redirected: false,
            elapsed_ms: 1,
            req_headers: BTreeMap::new(),
            resp_headers: BTreeMap::new(),
            bytes_in: 10,
            stored_as: None,
            attempts: 1,
        };
        log.push(mk(b, "/second"));
        log.push(mk(a, "/first"));
        // Pushed out of order, read back in **seq** order.
        //
        // This assertion used to be `vec![2, 1]` - "records keep the order they
        // were pushed" - which is what `records()` used to do and which is why
        // `requests.ndjson` came out in completion order while its `seq` column
        // claimed otherwise. A `--jobs 8` capture lands its chunk fetches
        // whenever the network finishes, so the two were different in exactly
        // the run that had the most of them.
        let records = log.records();
        let seqs: Vec<u64> = records.iter().map(|r| r.seq).collect();
        assert_eq!(seqs, vec![a, b], "records come back in seq order");
        let urls: Vec<&str> = records.iter().map(|r| r.url.as_str()).collect();
        assert_eq!(
            urls,
            vec!["/first", "/second"],
            "so issue order is recoverable"
        );
        assert_eq!(log.bytes_in(), 20);
        assert_eq!(log.len(), 2);
        assert!(!log.is_empty());
    }

    #[test]
    fn stored_as_is_filled_in_by_sequence_number() {
        let log = RequestLog::new();
        let seq = log.reserve();
        log.push(RequestRecord {
            seq,
            ts: now_rfc3339(),
            kind: Kind::Manifest,
            method: "GET",
            url: "https://x/m".into(),
            range: None,
            status: 200,
            final_url: "https://x/m".into(),
            redirected: false,
            elapsed_ms: 1,
            req_headers: BTreeMap::new(),
            resp_headers: BTreeMap::new(),
            bytes_in: 5,
            stored_as: None,
            attempts: 1,
        });
        log.set_stored_as(seq, "manifests/7.1.0/game.manifest.zst");
        assert_eq!(
            log.records()[0].stored_as.as_deref(),
            Some("manifests/7.1.0/game.manifest.zst")
        );
        // An unknown seq is ignored rather than appended: a typo must not create
        // a phantom record.
        log.set_stored_as(999, "nowhere");
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn the_log_renders_as_ndjson_with_every_field_present() {
        let log = RequestLog::new();
        let seq = log.reserve();
        log.push(RequestRecord {
            seq,
            ts: "2026-10-14T10:15:02.117Z".into(),
            kind: Kind::Api,
            method: "GET",
            url: "https://x/getGameConfigs".into(),
            range: Some("bytes=0-9".into()),
            status: 200,
            final_url: "https://x/getGameConfigs".into(),
            redirected: false,
            elapsed_ms: 418,
            req_headers: BTreeMap::from([("user-agent".to_string(), "gircap/0.1.0".to_string())]),
            resp_headers: BTreeMap::from([("etag".to_string(), "\"313EA8\"".to_string())]),
            bytes_in: 1529,
            stored_as: Some("api/getGameConfigs.json".into()),
            attempts: 2,
        });
        let text = log.to_ndjson();
        assert_eq!(text.lines().count(), 1);
        let v: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        for key in [
            "seq",
            "ts",
            "kind",
            "method",
            "url",
            "range",
            "status",
            "final_url",
            "redirected",
            "elapsed_ms",
            "req_headers",
            "resp_headers",
            "bytes_in",
            "stored_as",
            "attempts",
        ] {
            assert!(v.get(key).is_some(), "missing {key} in {text}");
        }
        assert_eq!(v["kind"], "api");
        assert_eq!(v["seq"], 1);
        assert_eq!(v["attempts"], 2);
        assert_eq!(v["resp_headers"]["etag"], "\"313EA8\"");
        assert!(text.ends_with('\n'), "ndjson must be newline-terminated");
    }

    #[test]
    fn an_empty_log_is_an_empty_file() {
        let log = RequestLog::new();
        assert!(log.is_empty());
        assert_eq!(log.to_ndjson(), "");
        assert_eq!(log.bytes_in(), 0);
    }

    // ------------------------------------------------------------- sleeper --

    /// Exercises the retry loop against a real socket, with the waiting removed.
    ///
    /// These bind `127.0.0.1:0` and are therefore local-only; nothing here
    /// reaches a CDN.
    mod local_server {
        use super::*;
        use std::io::{BufRead, BufReader, Write};
        use std::net::{TcpListener, TcpStream};

        /// A throwaway HTTP server that answers with a scripted sequence and
        /// records the request lines it saw.
        pub struct Mock {
            pub port: u16,
            pub hits: Arc<AtomicU32>,
            pub requests: Arc<Mutex<Vec<String>>>,
        }

        pub fn spawn(statuses: Vec<(u16, &'static [u8])>, extra_body: &'static str) -> Mock {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().unwrap().port();
            let hits = Arc::new(AtomicU32::new(0));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let (h, r) = (hits.clone(), requests.clone());
            std::thread::spawn(move || {
                for (status, body) in statuses {
                    let Ok((stream, _)) = listener.accept() else {
                        return;
                    };
                    h.fetch_add(1, Ordering::SeqCst);
                    serve_one(stream, status, body, extra_body, &r);
                }
            });
            Mock {
                port,
                hits,
                requests,
            }
        }

        fn serve_one(
            mut stream: TcpStream,
            status: u16,
            body: &'static [u8],
            extra_body: &'static str,
            requests: &Arc<Mutex<Vec<String>>>,
        ) {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).ok();
            // Drain headers; record the ones that matter.
            let mut headers = Vec::new();
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap_or(0) == 0 {
                    break;
                }
                if h.trim().is_empty() {
                    break;
                }
                headers.push(h);
            }
            requests.lock().unwrap().push(format!(
                "{line}{}",
                headers
                    .iter()
                    .filter(|h| h.to_ascii_lowercase().starts_with("range:"))
                    .map(String::as_str)
                    .collect::<String>()
            ));

            let head = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nETag: \"abc\"\r\nServer: MockOSS\r\n{extra_body}Connection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
            let _ = stream.flush();
        }

        pub fn url(mock: &Mock, path: &str) -> String {
            format!("http://127.0.0.1:{}{path}", mock.port)
        }
    }

    use local_server::*;

    fn fetcher(retries: u32, sleeper: Arc<dyn Sleep>) -> Fetcher {
        Fetcher::with_sleeper(
            Transport {
                jobs: 1,
                retries,
                timeout: Duration::from_secs(5),
            },
            RequestLog::new(),
            sleeper,
        )
        .expect("client builds")
    }

    #[tokio::test]
    async fn a_first_attempt_success_records_attempts_one_and_no_waiting() {
        let mock = spawn(vec![(200, b"hello")], "");
        let sleeper = Arc::new(RecordingSleep::default());
        let f = fetcher(3, sleeper.clone());
        let got = f
            .get(&url(&mock, "/x"), Kind::Chunk, None)
            .await
            .expect("200");

        assert_eq!(got.body, b"hello");
        assert_eq!(got.record.attempts, 1);
        assert_eq!(got.record.status, 200);
        assert_eq!(got.record.bytes_in, 5);
        assert_eq!(got.record.method, "GET");
        assert_eq!(got.record.kind, Kind::Chunk);
        assert!(!got.record.redirected);
        assert_eq!(
            got.record.req_headers["user-agent"],
            crate::logging::user_agent()
        );
        assert_eq!(
            sleeper.delays(),
            Vec::<Duration>::new(),
            "no backoff on success"
        );
        assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_server_fault_is_retried_and_the_recovered_record_says_so() {
        let mock = spawn(
            vec![(503, b"busy"), (503, b"busy"), (200, b"recovered")],
            "",
        );
        let sleeper = Arc::new(RecordingSleep::default());
        let f = fetcher(3, sleeper.clone());
        let got = f
            .get(&url(&mock, "/x"), Kind::Api, None)
            .await
            .expect("third attempt succeeds");

        assert_eq!(got.body, b"recovered");
        assert_eq!(
            got.record.attempts, 3,
            "the provenance row must show the retries"
        );
        assert_eq!(mock.hits.load(Ordering::SeqCst), 3);
        assert_eq!(
            sleeper.delays(),
            vec![
                RetryPolicy::new(3).delay_before(2),
                RetryPolicy::new(3).delay_before(3),
            ],
            "exactly one wait between attempts, exponential, none before the first"
        );

        // Every attempt is logged, including the two that failed. A capture that
        // hit a throttle and recovered must show the 429s.
        let records = f.log().records();
        assert_eq!(records.len(), 3, "the failed attempts are part of the log");
        assert_eq!(
            records.iter().map(|r| r.status).collect::<Vec<_>>(),
            vec![503, 503, 200]
        );
        assert_eq!(
            records.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "seq is reserved at issue time, so the log stays in order"
        );
    }

    #[tokio::test]
    async fn a_404_is_not_retried_because_retrying_it_changes_nothing() {
        let mock = spawn(vec![(404, b"gone"), (200, b"never reached")], "");
        let sleeper = Arc::new(RecordingSleep::default());
        let f = fetcher(5, sleeper.clone());
        let err = f
            .get(&url(&mock, "/x"), Kind::Chunk, None)
            .await
            .expect_err("404 is fatal");

        match &err {
            ApiError::Status {
                status, attempts, ..
            } => {
                assert_eq!(*status, 404);
                assert_eq!(*attempts, 1);
            }
            other => panic!("expected a status error, got {other:?}"),
        }
        assert_eq!(
            mock.hits.load(Ordering::SeqCst),
            1,
            "the second answer is unused"
        );
        assert!(sleeper.delays().is_empty());
        assert_eq!(err.exit_code(), ExitCode::Network);
        assert_eq!(f.log().len(), 1, "the failure is still recorded");
    }

    #[tokio::test]
    async fn exhausting_the_attempts_fails_and_logs_every_one() {
        let mock = spawn(vec![(503, b"a"), (503, b"b"), (503, b"c")], "");
        let sleeper = Arc::new(RecordingSleep::default());
        let f = fetcher(3, sleeper.clone());
        let err = f
            .get(&url(&mock, "/x"), Kind::Api, None)
            .await
            .expect_err("all three failed");

        match &err {
            ApiError::Status { attempts, .. } => assert_eq!(*attempts, 3),
            other => panic!("expected a status error, got {other:?}"),
        }
        assert_eq!(mock.hits.load(Ordering::SeqCst), 3);
        assert_eq!(sleeper.delays().len(), 2, "no wait after the last attempt");
        assert_eq!(f.log().len(), 3);
    }

    #[tokio::test]
    async fn a_transport_failure_is_retried_and_reported_as_transport() {
        // A port nothing is listening on: connection refused on every attempt.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let sleeper = Arc::new(RecordingSleep::default());
        let f = fetcher(2, sleeper.clone());
        let err = f
            .get(&format!("http://127.0.0.1:{port}/x"), Kind::Api, None)
            .await
            .expect_err("nothing is listening");

        match &err {
            ApiError::Transport { attempts, .. } => {
                assert_eq!(*attempts, 2);
            }
            other => panic!("expected a transport error, got {other:?}"),
        }
        assert_eq!(sleeper.delays().len(), 1);
        let records = f.log().records();
        assert_eq!(records.len(), 2);
        assert!(
            records.iter().all(|r| r.status == 0),
            "no response means no status: {:?}",
            records.iter().map(|r| r.status).collect::<Vec<_>>()
        );
        assert!(err.url().unwrap().contains(&format!("127.0.0.1:{port}")));
    }

    #[tokio::test]
    async fn a_range_request_sends_the_literal_header_and_records_it() {
        let mock = spawn(
            vec![(206, b"0123456789")],
            "Content-Range: bytes 0-9/100\r\n",
        );
        let sleeper = Arc::new(RecordingSleep::default());
        let f = fetcher(1, sleeper.clone());
        let got = f
            .get(&url(&mock, "/m"), Kind::Manifest, Some("bytes=0-9"))
            .await
            .expect("206");

        assert_eq!(got.body, b"0123456789");
        assert_eq!(got.record.status, 206, "a partial response is a success");
        assert_eq!(got.record.range.as_deref(), Some("bytes=0-9"));
        assert_eq!(got.record.req_headers["range"], "bytes=0-9");
        assert_eq!(got.record.resp_headers["content-range"], "bytes 0-9/100");
        assert_eq!(got.record.bytes_in, 10);

        let seen = mock.requests.lock().unwrap().clone();
        assert!(
            seen[0].to_ascii_lowercase().contains("range: bytes=0-9"),
            "the header never reached the server: {seen:?}"
        );
    }

    #[tokio::test]
    async fn a_response_without_a_range_sends_no_range_header() {
        let mock = spawn(vec![(200, b"whole")], "");
        let f = fetcher(1, Arc::new(RecordingSleep::default()));
        let got = f
            .get(&url(&mock, "/m"), Kind::Manifest, None)
            .await
            .unwrap();
        assert_eq!(got.record.range, None);
        assert!(!got.record.req_headers.contains_key("range"));
        assert!(
            !mock.requests.lock().unwrap()[0]
                .to_ascii_lowercase()
                .contains("range:"),
            "an absent --range must not be sent as an empty header"
        );
    }

    #[tokio::test]
    async fn response_headers_are_lowercased_and_kept_verbatim() {
        let mock = spawn(vec![(200, b"x")], "");
        let f = fetcher(1, Arc::new(RecordingSleep::default()));
        let got = f.get(&url(&mock, "/x"), Kind::Api, None).await.unwrap();
        let keys: Vec<&String> = got.record.resp_headers.keys().collect();
        assert!(
            keys.iter()
                .all(|k| k.chars().all(|c| !c.is_ascii_uppercase()))
        );
        assert_eq!(got.record.resp_headers["server"], "MockOSS");
        assert_eq!(
            got.record.resp_headers["etag"], "\"abc\"",
            "quotes are kept"
        );
    }

    #[tokio::test]
    async fn a_zero_retcode_is_accepted_and_a_nonzero_one_is_exit_two() {
        let mock = spawn(vec![(200, br#"{"retcode":0,"data":{"tag":"7.1.0"}}"#)], "");
        let f = fetcher(1, Arc::new(RecordingSleep::default()));
        let (body, v) = f.get_json(&url(&mock, "/g"), Kind::Api).await.unwrap();
        assert_eq!(v["data"]["tag"], "7.1.0");
        assert!(body.starts_with(b"{"));

        // The documented way this API says "no such tag": HTTP 200, retcode -202.
        let mock2 = spawn(
            vec![(
                200,
                br#"{"data":null,"message":"not found","retcode":-202}"#,
            )],
            "",
        );
        let f2 = fetcher(1, Arc::new(RecordingSleep::default()));
        let err = f2
            .get_json(&url(&mock2, "/g?tag=0.0.1"), Kind::Api)
            .await
            .expect_err("retcode -202");
        assert_eq!(err.exit_code(), ExitCode::Network);
        assert!(err.to_string().contains("-202"), "{err}");
        assert!(err.to_string().contains("not found"), "{err}");
    }

    /// A response that is not a wrapper fails, and is **not** retried: a `200` that
    /// is an HTML error page is a wrong answer, and asking again makes it a
    /// slower wrong answer.
    #[tokio::test]
    async fn a_body_that_is_not_a_wrapper_is_an_error_not_a_silent_pass() {
        let mock = spawn(vec![(200, b"<html>404</html>")], "");
        let f = fetcher(1, Arc::new(RecordingSleep::default()));
        let err = f.get_json(&url(&mock, "/g"), Kind::Api).await.unwrap_err();
        assert!(err.to_string().contains("retcode"), "{err}");
        assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_log_is_written_to_disk_as_ndjson() {
        let dir = std::env::temp_dir().join(format!("gircap-http-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("api").join("requests.ndjson");

        let mock = spawn(vec![(200, b"body")], "");
        let f = fetcher(1, Arc::new(RecordingSleep::default()));
        f.get(&url(&mock, "/x"), Kind::Api, None).await.unwrap();
        let n = f.log().write_ndjson(&path).unwrap();

        assert_eq!(n, 1);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 1);
        let v: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(v["kind"], "api");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kinds_are_wire_spelled() {
        assert_eq!(Kind::Api.as_str(), "api");
        assert_eq!(Kind::Manifest.to_string(), "manifest");
        assert_eq!(Kind::Chunk.to_string(), "chunk");
        assert_eq!(Kind::Probe.to_string(), "probe");
        assert_eq!(serde_json::to_string(&Kind::Probe).unwrap(), "\"probe\"");
    }

    #[test]
    fn every_api_error_maps_to_exit_two() {
        for e in [
            ApiError::Transport {
                url: "u".into(),
                attempts: 1,
                detail: "d".into(),
            },
            ApiError::Status {
                url: "u".into(),
                status: 500,
                attempts: 1,
            },
            ApiError::shape("s", "d"),
        ] {
            assert_eq!(e.exit_code(), ExitCode::Network);
            assert!(!e.to_string().is_empty());
        }
    }
}
