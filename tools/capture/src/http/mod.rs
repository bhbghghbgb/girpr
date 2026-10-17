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

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::exit::ExitCode;

pub mod negative;
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
    ///
    /// **Two paths share this kind, and they differ in retry behaviour. §5.5 row
    /// 16.** The tag probe goes through [`Fetcher::get`] and so through
    /// [`RetryPolicy`]; the negative probes go through [`Fetcher::observe`], which
    /// sends once and records `attempts: 1` unconditionally. A reader of
    /// `api/requests.ndjson` therefore cannot tell them apart from the row alone,
    /// and the `kind` was deliberately not split to fix that — see the note on
    /// [`Kind`]'s wire spelling below.
    Probe,
}

impl Kind {
    /// The wire spelling.
    ///
    /// **This is a closed vocabulary and `docs/01-artifact-schema.md:153` says
    /// so**, which is why §5.5 row 16 was answered with documentation rather than
    /// distinct kinds. Splitting `probe` into `tag_probe` and `shape_probe` would
    /// not be additive: it would *remove* the value `probe`, so any reader matching
    /// on it — which is the natural way to ask "what did the metadata cost?" —
    /// would silently start seeing nothing. Widening a closed vocabulary by
    /// subtraction is a breaking change to a recorded field, which is a larger
    /// decision than an unfalsifiable log row deserves.
    ///
    /// The distinction is recorded instead, on [`Kind::Probe`] and in the schema
    /// doc, and the behaviour it describes is pinned by
    /// `observe_records_a_server_error_once_and_never_retries`.
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

    /// The default exit code for a failure that has not been classified yet.
    ///
    /// **This is not the tool's exit-code policy, and the old doc claiming it was
    /// — "the code every API failure maps to" — was false.** §6.6 named two
    /// reasons, and both hold:
    ///
    /// - [`ApiError::Shape`] covers roughly 55 conditions with nothing to do with
    ///   the network: local invariants, writer errors, budget ceilings, assembly
    ///   steps. A `2` is a network code, and it would be wrong for most of them.
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

/// The future [`Sleep::sleep`] hands back.
///
/// **Boxed on purpose.** [`Fetcher`] holds an `Arc<dyn Sleep>`, and a trait
/// method returning `impl Future` or declared `async fn` is not dyn-compatible.
/// One `Box` per *retry backoff* is nothing beside the 55.5 s of waiting it may
/// be standing in for. `'static` because neither implementor borrows `self`.
pub type SleepFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>;

/// Something that can be waited on, so tests can count delays instead of
/// spending them.
///
/// **Async on purpose.** This was `fn sleep(&self, d: Duration)` and the real
/// implementation was [`std::thread::sleep`], called from inside
/// [`Fetcher::get`] — an `async fn` whose tasks are spawned onto the runtime.
/// So a retry backoff *blocked a tokio worker thread* rather than yielding it:
/// with `--retries 10` the schedule is 0.5+1+2+4+8+10+10+10+10 = 55.5 s of
/// thread-blocked time per failing request, and on a 4-core box at `--jobs 64`
/// all four workers can be asleep at once with nothing able to make progress.
///
/// [`tokio::time::sleep`] yields the worker instead, so the requests that *are*
/// in flight keep going while this one waits.
pub trait Sleep: Send + Sync + fmt::Debug {
    /// Wait for `d`.
    fn sleep(&self, d: Duration) -> SleepFuture;
}

/// Real wall-clock waiting, without taking the worker with it.
#[derive(Debug, Default)]
pub struct RealSleep;

impl Sleep for RealSleep {
    fn sleep(&self, d: Duration) -> SleepFuture {
        Box::pin(async move {
            if !d.is_zero() {
                tokio::time::sleep(d).await;
            }
        })
    }
}

/// Records what it was asked to wait for and returns immediately.
#[derive(Debug, Default)]
pub struct RecordingSleep(Mutex<Vec<Duration>>);

impl RecordingSleep {
    /// Every delay requested so far.
    pub fn delays(&self) -> Vec<Duration> {
        self.lock().clone()
    }

    /// Take the lock, recovering from poisoning — the same reasoning as
    /// [`RequestLog::lock`]. A recorded delay is evidence about a retry
    /// schedule, and losing it to a panic is a worse outcome than reading a
    /// half-pushed vector.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Duration>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Sleep for RecordingSleep {
    fn sleep(&self, d: Duration) -> SleepFuture {
        // Recorded before the future is built rather than inside it: this
        // sleeper never waits, so the only observable difference would be when
        // the push happens, and doing it here keeps the future free of a
        // borrow and therefore `'static`.
        self.lock().push(d);
        Box::pin(async {})
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
/// The point of the ceiling is not this number. It is that there is a number: a
/// broken or hostile origin that never stops sending used to be able to fill RAM
/// until the process died, with no bound anywhere on the path. See
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
    /// **§5.5 row 11.** The client had only `timeout` and `user_agent`, and every
    /// body read was unbounded, so an origin that kept sending was bounded by
    /// nothing but the machine's memory. A timeout does not help: a server
    /// streaming a gigabyte steadily for ten minutes never trips one.
    ///
    /// Checked in two places, because one is not enough: against the declared
    /// `Content-Length` before a byte is read (so a server claiming 10 GiB is
    /// refused immediately), and against the running total as bytes arrive (so a
    /// server that lies about or omits the header is still stopped).
    ///
    /// Exceeding it is a [`ApiError::Shape`], so it is **not** retried: an origin
    /// that sent too much will send too much again.
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
    /// The headers every request carries, lowercased, for the record.
    ///
    /// Built once per `Fetcher` and cloned per record; see
    /// [`Fetcher::base_req_headers`].
    ///
    /// **`accept-encoding` is `identity`, and that is the §6.2 finding.** It used
    /// to read `gzip` here while the client was configured with no
    /// `accept-encoding` at all — reqwest only sends one when its `gzip` feature
    /// is on, and it is not. So the bundle claimed a header the tool never sent.
    ///
    /// Claiming `gzip` rather than fixing it would have been the worse of the
    /// two. Adding the header *without* the feature is not a no-op: the CDN
    /// would answer with a gzip body and reqwest, unable to decompress it, would
    /// hand back the compressed bytes — and every manifest digest in the bundle
    /// would be a digest of compressed JSON. So the record states what the wire
    /// does, which is identity, and the client is configured to match.
    fn default_req_headers(user_agent: &str, range: Option<&str>) -> BTreeMap<String, String> {
        let mut h = shared_req_headers(user_agent);
        if let Some(r) = range {
            h.insert("range".to_string(), r.to_string());
        }
        h
    }

    /// The fetcher's shared headers, plus this request's `Range` when it sent one.
    ///
    /// The only per-request part of [`RequestRecord::default_req_headers`] is
    /// `Range`, so that is the only copy made.
    fn req_headers(
        base: &BTreeMap<String, String>,
        range: Option<&str>,
    ) -> BTreeMap<String, String> {
        match range {
            None => base.clone(),
            Some(r) => {
                let mut h = base.clone();
                h.insert("range".to_string(), r.to_string());
                h
            }
        }
    }
}

/// The accumulated request log, shared by every fetch in a run.
///
/// Cheap to clone; all clones are the same log.
#[derive(Debug, Clone, Default)]
pub struct RequestLog(Arc<Mutex<LogState>>);

impl RequestLog {
    /// Take the lock, **recovering from poisoning**.
    ///
    /// This was `.lock().expect("request log is not poisoned")` at eight sites,
    /// and poisoning is genuinely reachable rather than theoretical: the lock
    /// sits inside the concurrent chunk fetcher, so one task panicking while it
    /// holds this lock poisons it for every other task — and a poisoned lock
    /// makes every subsequent `.expect` panic too, so one panic anywhere turns
    /// into a run that cannot even record its own failure.
    ///
    /// The crate already *tolerates* panicking fetch tasks (`blobs::Store::one`
    /// records "a chunk fetch task panicked" and carries on), so the two
    /// policies contradicted each other: a task was allowed to panic, and the
    /// log it poisoned was then allowed to take the whole run down.
    ///
    /// `PoisonError::into_inner` hands back the guard anyway, and a half-updated
    /// log is still a perfectly good log for provenance — which is the only job
    /// this has. `Record` and `reserve` are the only things that touch
    /// `LogState`, and both are single field writes.
    fn lock(&self) -> std::sync::MutexGuard<'_, LogState> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Debug, Default)]
struct LogState {
    next_seq: u64,
    /// In **completion** order, which under `--jobs > 1` is not issue order.
    records: Vec<RequestRecord>,
    /// `seq -> index into records`.
    ///
    /// The index is what makes [`RequestLog::set_stored_as`] O(log n) instead of
    /// O(n). It used to hold the lock and then walk `records` looking for a
    /// matching `seq` — once per chunk, over a vector that reaches 113 610
    /// entries. That is a ~23 MB memory walk *while holding the one
    /// process-wide lock every other request needs*, which is why `--jobs 8`
    /// did not scale: the eight workers serialised on exactly this line and
    /// gave back the concurrency the chunk fetcher had bought.
    ///
    /// A `BTreeMap` rather than a `Vec<Option<usize>>`: `reserve()` hands out
    /// dense `1..N` ids, so a vector would do — but `push` is told the `seq` and
    /// `reserve` is a separate call, and a map stays correct if that ever
    /// changes, whereas a vector would silently misplace a record.
    by_seq: BTreeMap<u64, usize>,
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
        let mut s = self.lock();
        s.next_seq += 1;
        s.next_seq
    }

    /// Append a record. Its `seq` is already assigned.
    pub fn push(&self, record: RequestRecord) {
        let mut s = self.lock();
        let at = s.records.len();
        let seq = record.seq;
        s.records.push(record);
        s.by_seq.insert(seq, at);
    }

    /// Every record, in `seq` order.
    pub fn records(&self) -> Vec<RequestRecord> {
        let mut v = self
            .lock()
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
        // **Deliberately not read through `by_seq`.** That index would make the
        // ordering structural and drop the sort, but a `seq` pushed twice would
        // then silently *lose* a row from the file this tool exists to produce,
        // and an audit log that quietly drops entries is worse than one that
        // spends n log n to put them in order. `reserve()` is the only allocator
        // and `attempt()` calls it once per record, so the duplicate cannot
        // happen today; the sort is what keeps that from being load-bearing.
        //
        // `reserve()` makes `seq` unique, so this is a total order.
        v.sort_by_key(|r| r.seq);
        v
    }

    /// Total bytes received across the run.
    pub fn bytes_in(&self) -> u64 {
        self.lock()
            .records
            .iter()
            .map(|r| r.bytes_in)
            .sum()
    }

    /// How many requests were made.
    pub fn len(&self) -> usize {
        self.lock()
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
    ///
    /// **O(log n), holding the lock only long enough to look up an index.** This
    /// used to walk `records` under the lock; see [`LogState::by_seq`] for what
    /// that cost.
    pub fn set_stored_as(&self, seq: u64, path: impl Into<String>) {
        let mut s = self.lock();
        if let Some(&i) = s.by_seq.get(&seq) {
            s.records[i].stored_as = Some(path.into());
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
    /// This request's sequence number, for `set_stored_as`.
    ///
    /// A `u64` and not a [`RequestRecord`]. The log already owns the record;
    /// handing a second copy to every caller meant deep-cloning two
    /// `BTreeMap`s, a `String` URL and up to ~2 KB of response headers for each
    /// of 113 610 chunks, and then dropping all of it after reading one integer.
    pub seq: u64,
    /// The status as received.
    ///
    /// `Copy`, because `probe` genuinely needs it to fill a document field and
    /// reading it back out of the log would be a round trip for one `u16`.
    pub status: u16,
}

/// Where a fetched body is put.
///
/// §6.6.2. This exists so the retry loop, the status check and the request record
/// are written **once**. The first draft of this change had a second loop for
/// the file case, which is 30 lines that look identical until a retry policy is
/// edited in one of them.
pub enum Destination<'a> {
    /// Collected into a `Vec`, for the callers that parse the body: the API
    /// documents, the manifest archives, the probes. Small — the largest is a
    /// ~9 MB archive — and genuinely needed in memory.
    Memory,
    /// Written to this file and digested on the way past, so the body is never
    /// resident. Chunk blobs: the largest in the reference bundle is 3 689 476 B,
    /// and `--jobs 8` keeps eight of them in flight.
    File(&'a mut crate::hash::HashingFile),
}

/// A body that was streamed to disk: what the caller needs without the bytes.
///
/// The mirror of [`Fetched`], which carries the bytes and not the digests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Streamed {
    /// This request's sequence number, for `set_stored_as`.
    pub seq: u64,
    /// The status as received.
    pub status: u16,
    /// How many bytes arrived — the length of the body, which is also the length
    /// of the file.
    pub bytes_in: u64,
}

/// What one successful [`Fetcher::run`] produced.
///
/// `body` is `None` exactly when the body went to a file. `bytes_in` is read off
/// the request record rather than recomputed, so it is the same number
/// `api/requests.ndjson` carries.
struct Run {
    body: Option<Vec<u8>>,
    seq: u64,
    status: u16,
    bytes_in: u64,
}

/// Classify a failure that happened while the body was being moved.
///
/// **Reading and writing are different faults and get different variants.** A
/// read that fails is the network's fault: retryable, and often right to retry.
/// A write that fails is the disk's, and retrying it would spend the whole budget
/// discovering that a full disk stays full — so it is a [`ApiError::Shape`], which
/// the policy stops on.
///
/// This was a branch inside the pump, and then a test that needed a genuinely
/// full disk to reach it. Extracted so the decision can be read in one place and
/// tested without contriving a filesystem failure.
fn body_failure(
    url: &str,
    attempts: u32,
    reading: bool,
    detail: impl fmt::Display,
) -> ApiError {
    let detail = format!(
        "{} the body: {detail}",
        if reading { "reading" } else { "writing" }
    );
    if reading {
        ApiError::Transport {
            url: url.to_string(),
            attempts,
            detail,
        }
    } else {
        ApiError::Shape {
            subject: url.to_string(),
            detail,
        }
    }
}
/// Drain a response body into `sink`, refusing to exceed the transport's ceiling.
///
/// **The one place a body is read, for every destination.** §5.5 row 11. There
/// were four unbounded reads — the buffered path, the file path, `observe`, and
/// the drain of an *error* body — and a ceiling enforced in three of them would be
/// a ceiling with a hole in it.
///
/// Two checks, because one is not enough:
///
/// - against `Content-Length` **before** reading, so an origin declaring 10 GiB is
///   refused without transferring any of it;
/// - against the running total as bytes arrive, for an origin that omits the
///   header or understates it.
///
/// `sink` is generic rather than "either a `Vec` or a file" so all four callers
/// share this, including the error-body drain, which writes to
/// [`std::io::sink()`].
async fn drain_capped<W: std::io::Write>(
    mut resp: reqwest::Response,
    sink: &mut W,
    cap: u64,
    url: &str,
    attempts: u32,
) -> Result<u64, ApiError> {

    if let Some(declared) = resp.content_length()
        && declared > cap
    {
        return Err(too_big(url, declared, cap));
    }
    let mut seen = 0_u64;
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                seen += chunk.len() as u64;
                if seen > cap {
                    // The bytes are already gone, so this cannot stop the
                    // transfer — the running total stops the *memory*, which is
                    // what the ceiling is for.
                    return Err(too_big(url, seen, cap));
                }
                std::io::Write::write_all(sink, &chunk)
                    .map_err(|e| body_failure(url, attempts, false, e))?;
            }
            Ok(None) => return Ok(seen),
            Err(e) => return Err(body_failure(url, attempts, true, e)),
        }
    }
}

/// Whether a body read failed because it could not be read, rather than because
/// it was too large.
///
/// The distinction `observe` needs: unreadable means "there is no body", which is
/// a shape worth recording; over the ceiling means "there is a body and it is
/// larger than this tool will hold", which is a failure and not a shape.
///
/// A `Transport` is a read that failed. `body_failure` only produces those for
/// `reading = true`, so this is a variant test rather than a message match.
fn is_read_failure(e: &ApiError) -> bool {
    matches!(e, ApiError::Transport { .. })
}

/// A body that was larger than the ceiling allows.
///
/// A `Shape` and not a `Transport`, so it is not retried: the network worked, the
/// origin simply sent more than this tool will hold, and asking again produces the
/// same answer at the same cost.
fn too_big(url: &str, seen: u64, cap: u64) -> ApiError {
    ApiError::shape(
        url,
        format!(
            "the response body reached {seen} B, over the {cap} B ceiling \
             (--max-response-bytes); raise it with --max-response-bytes if this \
             origin is one you meant to trust"
        ),
    )
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
    // No `record` field. An observation's provenance row is in the log, which
    // owns it; `observe` used to clone the whole thing in order to return a
    // copy nobody read a field of.
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

/// The `Accept` the tool sends, and the value it records.
pub const ACCEPT: &str = "*/*";

/// The `Accept-Encoding` the tool sends, and the value it records.
///
/// **`identity`, deliberately.** See `RequestRecord::default_req_headers` for
/// why claiming `gzip` was wrong and why adopting it would be worse: reqwest's
/// `gzip` feature is off, so naming it on the wire would buy compressed bodies
/// this client cannot decompress.
pub const ACCEPT_ENCODING: &str = "identity";

/// The headers every request carries, as the record spells them.
///
/// **One list, two consumers.** [`client_default_headers`] turns this into the
/// `HeaderMap` the `reqwest` client is built with, so the record and the wire
/// cannot drift — which is the whole of §6.2, since the defect was two lists
/// that nobody compared.
fn shared_req_headers(user_agent: &str) -> BTreeMap<String, String> {
    let mut h = BTreeMap::new();
    h.insert("accept".to_string(), ACCEPT.to_string());
    h.insert("accept-encoding".to_string(), ACCEPT_ENCODING.to_string());
    h.insert("user-agent".to_string(), user_agent.to_string());
    h
}

/// [`shared_req_headers`] as a `HeaderMap`, for `ClientBuilder::default_headers`.
///
/// A name the builder rejects is a configuration error rather than a silently
/// dropped header, which is the failure mode this exists to remove.
fn client_default_headers(user_agent: &str) -> Result<reqwest::header::HeaderMap, ApiError> {
    let mut map = reqwest::header::HeaderMap::new();
    for (k, v) in shared_req_headers(user_agent) {
        let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
            .map_err(|e| ApiError::shape("header name", e))?;
        let value = reqwest::header::HeaderValue::from_str(&v)
            .map_err(|e| ApiError::shape("header value", e))?;
        map.insert(name, value);
    }
    Ok(map)
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
    /// The headers every request carries, minus any `Range`.
    ///
    /// **Built once.** `req_headers` is otherwise rebuilt for every record —
    /// three `String`s and a `BTreeMap` each, three times over for 113 610
    /// chunks, all of it identical except when a `Range` is present. `Range` is
    /// the exception because it is per-request; that is the only thing
    /// [`RequestRecord::req_headers`] adds.
    base_req_headers: BTreeMap<String, String>,
    /// The ceiling on one response body, from [Transport::max_response_bytes].
    max_response_bytes: u64,
}

impl Fetcher {
    /// Run `f` over `items`, at most `jobs` at a time, **in input order**.
    ///
    /// The pattern [`crate::blobs::Store::fill`] already uses: prime a
    /// `JoinSet` with `jobs` entries and refill it as each completes. Four
    /// callers had hand-written serial loops where only one of the requests had
    /// a dependency on an earlier one, so a `--no-chunk-blobs` metadata run
    /// spent six round trips where two would do, probed seven tags one after
    /// another, and fetched four ~9 MB manifests strictly one at a time.
    ///
    /// **Results come back in input order, not completion order.** That is the
    /// whole reason this is safe for anything that writes a document: a caller
    /// that got completion order would produce different bytes depending on
    /// which request happened to win the race. `RequestLog` numbers at issue
    /// time, so `api/requests.ndjson` is already in `seq` order — but the
    /// *documents* assembled from these results are not read back through the
    /// log, and a metadata document whose fields moved would be a different
    /// artifact.
    ///
    /// One failure does not abandon the rest: each item's `Err` stays in its own
    /// slot, and a task that panics is recorded as an `Err` at that index. A
    /// missing blob is better evidence than no bundle.
    ///
    /// `f` returns `(index, result)` and **never** an outer `Err`. The index has
    /// to come back even when the work failed, or a failed task is
    /// indistinguishable from one that died: `JoinSet` reports a panic but not
    /// which task panicked, so the only way to find out afterwards is to see
    /// which index never came home. An outer `Result` would throw the index away
    /// exactly when it is most needed.
    ///
    /// `f` receives a clone of this fetcher rather than a borrow, because
    /// `JoinSet::spawn` needs a `'static` future. Cloning is three `Arc`s and
    /// two `BTreeMap`s, so N clones are N handles on one pool and one log rather
    /// than N fetchers with N logs.
    pub async fn map_concurrently<T, R, F, Fut>(
        &self,
        items: &[T],
        jobs: usize,
        f: F,
    ) -> Vec<Result<R, String>>
    where
        T: Clone + Send + 'static,
        R: Send + 'static,
        F: Fn(Fetcher, usize, T) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = (usize, Result<R, String>)> + Send + 'static,
    {
        let n = items.len();
        let mut out: Vec<Option<Result<R, String>>> = (0..n).map(|_| None).collect();
        // Which items have been accounted for. A `JoinSet` will not say *which*
        // task panicked, so a task that dies takes its slot with it and the
        // index is recovered at the end from what never came back.
        let mut returned: Vec<bool> = vec![false; n];
        let mut set = tokio::task::JoinSet::new();
        let mut next = 0_usize;

        while next < n && set.len() < jobs.max(1) {
            let i = next;
            next += 1;
            set.spawn(f(self.clone(), i, items[i].clone()));
        }
        while let Some(joined) = set.join_next().await {
            if let Ok((i, r)) = joined {
                returned[i] = true;
                out[i] = Some(r);
            }
            if next < n {
                let i = next;
                next += 1;
                set.spawn(f(self.clone(), i, items[i].clone()));
            }
        }

        out.into_iter()
            .enumerate()
            .map(|(i, r)| {
                r.unwrap_or_else(|| {
                    Err(if returned[i] {
                        format!("item {i} was dispatched but never returned")
                    } else {
                        "a request task panicked".to_string()
                    })
                })
            })
            .collect()
    }

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
        // `default_headers` from the same list the record is built from, so the
        // bundle's claim about the request and the request itself cannot differ.
        // `user_agent` is set separately below only because that is the builder
        // call for it; it is the same value.
        let client = reqwest::Client::builder()
            .timeout(transport.timeout)
            .user_agent(ua.clone())
            .default_headers(client_default_headers(&ua)?)
            .build()
            .map_err(|e| ApiError::shape("http client", e))?;
        Ok(Fetcher {
            client,
            policy: RetryPolicy::new(transport.retries),
            sleeper,
            log,
            base_req_headers: RequestRecord::default_req_headers(&ua, None),
            max_response_bytes: transport.max_response_bytes,
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
        //
        // **But a body over the ceiling is a failure, and the two are different
        // things.** §5.5 row 11: this was `resp.bytes()`, unbounded, and a hostile
        // origin answering a *probe* — which is the one request an attacker can
        // most cheaply provoke — could fill RAM with it. Unreadable means "there
        // is no body to record"; over the ceiling means "this body exists and is
        // larger than this tool will hold", and recording an empty shape for that
        // would put a falsehood in `api/error_shapes.json`.
        let mut body: Vec<u8> = Vec::new();
        let bytes_in = match drain_capped(
            resp,
            &mut body,
            self.max_response_bytes,
            url,
            1,
        )
        .await
        {
            Ok(n) => n,
            Err(e) if is_read_failure(&e) => {
                body = Vec::new();
                0
            }
            Err(e) => return Err(e),
        };
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
        self.log.push(record);
        Ok(Observation {
            status,
            content_type,
            body,
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
        let run = self.run(url, kind, range, Destination::Memory).await?;
        // `run` is total over its destinations: `Memory` always yields the body
        // and `File` never does. This arm is unreachable by construction, and is
        // an `Err` rather than `unreachable!` so that no production path in this
        // tool can panic (§5.1).
        let Some(body) = run.body else {
            return Err(ApiError::shape(
                url,
                "internal: buffered fetch returned no body",
            ));
        };
        Ok(Fetched {
            body,
            seq: run.seq,
            status: run.status,
        })
    }

    /// Fetch one URL straight into `out`, never holding the body in memory.
    ///
    /// §6.6.2. The digests and the byte count come from the write itself, so they
    /// are the same numbers the buffered path produces over the same bytes — see
    /// `hash::tests::a_hashable_file_reports_the_digests_of_the_bytes_it_stored`.
    ///
    /// **The caller must not rename or read `out` until this returns `Ok`**, and
    /// must call [`crate::hash::HashingFile::finish`] for the digests: a partial
    /// body is on disk while this is still running, which is what lets a retry
    /// discard it.
    ///
    /// Retries behave as they always did, with one addition that is not optional:
    /// a failed attempt's bytes are discarded before the next one starts, or the
    /// digests would cover both responses. See
    /// `HashingFile::restart` and the test named in its doc.
    ///
    /// A **write** failure is reported as [`ApiError::Shape`] and so is not
    /// retried, while a **read** failure is [`ApiError::Transport`] and is. That
    /// asymmetry is the point: a full disk is not fixed by waiting half a second,
    /// and reporting it as a network failure would spend the whole retry budget
    /// discovering that.
    pub async fn get_to_file(
        &self,
        url: &str,
        kind: Kind,
        out: &mut crate::hash::HashingFile,
    ) -> Result<Streamed, ApiError> {
        let run = self.run(url, kind, None, Destination::File(out)).await?;
        debug_assert!(
            run.body.is_none(),
            "the file destination streams rather than collecting"
        );
        Ok(Streamed {
            seq: run.seq,
            status: run.status,
            bytes_in: run.bytes_in,
        })
    }


/// The retry loop, the status check and the record — for both destinations.
    async fn run(
        &self,
        url: &str,
        kind: Kind,
        range: Option<&str>,
        mut dest: Destination<'_>,
    ) -> Result<Run, ApiError> {
        let started = Instant::now();
        let mut attempts: u32 = 0;
        let mut last: Option<ApiError> = None;

        while attempts < self.policy.attempts {
            attempts += 1;
            let waiting = self.policy.delay_before(attempts);
            if !waiting.is_zero() {
                tracing::debug!(url, ?waiting, attempts, "retry backoff");
                self.sleeper.sleep(waiting).await;
            }

            match self.attempt(url, kind, range, attempts, started, &mut dest).await {
                Ok((body, record)) => {
                    // Only the sequence number and the byte count are read off
                    // `record` once it is logged -- by `blobs` and `manifest`, to
                    // call `set_stored_as` -- so the log is the owner and this
                    // returns a borrow-free view instead of a second deep copy
                    // of a record carrying two `BTreeMap`s, a `String` URL and
                    // up to ~2 KB of response headers. Once per chunk.
                    let run = Run {
                        body,
                        seq: record.seq,
                        status: record.status,
                        bytes_in: record.bytes_in,
                    };
                    self.log.push(record);
                    return Ok(run);
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
        dest: &mut Destination<'_>,
    ) -> Result<(Option<Vec<u8>>, RequestRecord), (ApiError, RequestRecord)> {
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
            // **The body of a failure is counted, not ignored.** §5.5 row 1: this
            // used to record `bytes_in: 0` and drop the response, so a CDN
            // `429`/`503` carrying a few KB of XML envelope was paid for on the
            // wire and reported as free. `bytes_in` is sold in this module's doc
            // as "the only way to price a manifest without re-downloading it", and
            // it under-reported *precisely the retried requests that cost the
            // most* — so a throttled run's cost figures were optimistic by the
            // amount the throttle cost.
            //
            // **Streamed and discarded**, not read into a buffer: the content is
            // useless, only the length is not. Draining also lets the connection
            // go back to the pool, which dropping a response mid-body would not.
            //
            // A body that fails to read mid-way counts what arrived, like every
            // other count here: reporting `0` for bytes that demonstrably crossed
            // the network is the defect being fixed.
            let mut resp = resp;
            let mut bytes_in: u64 = 0;
            loop {
                match resp.chunk().await {
                    Ok(Some(c)) => bytes_in += c.len() as u64,
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
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
                bytes_in,
                attempts,
                started,
            );
            return Err((err, record));
        }

        // **The body, and where it goes.** `Memory` collects the whole thing as
        // before; `File` pumps it into the destination chunk by chunk and never
        // materialises it. The two differ only here — the status check above and
        // the record below are shared, which is the reason this is one function
        // with a parameter rather than two functions.
        let bytes_in: u64;
        let body: Option<Vec<u8>> = match dest {
            Destination::Memory => {
                // One record for both outcomes, as before — the failure and the
                // size are both about the same request.
                let fail = |err: ApiError| {
                    let record = self.record(
                        seq,
                        kind,
                        url,
                        range,
                        status,
                        resp_headers.clone(),
                        &final_url,
                        redirected,
                        0,
                        attempts,
                        started,
                    );
                    (err, record)
                };
                let mut buf: Vec<u8> = Vec::new();
                match drain_capped(resp, &mut buf, self.max_response_bytes, url, attempts).await {
                    Ok(n) => {
                        bytes_in = n;
                        Some(buf)
                    }
                    Err(e) => return Err(fail(e)),
                }
            }
            Destination::File(out) => {
                // Discard whatever a failed attempt left before this one writes.
                // Without this the digests cover both responses — see
                // `HashingFile::restart`.
                let fail = |err: ApiError| {
                    let record = self.record(
                        seq,
                        kind,
                        url,
                        range,
                        status,
                        resp_headers.clone(),
                        &final_url,
                        redirected,
                        0,
                        attempts,
                        started,
                    );
                    (err, record)
                };

                if let Err(e) = out.restart() {
                    // Not retryable, and not a transport failure: a disk that will
                    // not truncate is a disk that will not write either.
                    return Err(fail(body_failure(url, attempts, false, format!("preparing the body file: {e}"))));
                }
                match drain_capped(resp, out, self.max_response_bytes, url, attempts).await {
                    Ok(n) => bytes_in = n,
                    Err(e) => return Err(fail(e)),
                }
                None
            }
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
            bytes_in,
            attempts,
            started,
        );
        Ok((body, record))
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
            req_headers: RequestRecord::req_headers(&self.base_req_headers, range),
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
///
/// **The keys are already lowercase.** `http::HeaderName` validates and normalises
/// to lowercase on construction and guarantees `as_str()` returns that form, so
/// the `to_ascii_lowercase()` this used to call allocated a `String` per header
/// name and then discarded the distinction it was protecting. The output is
/// unchanged; only the copy is gone.
fn header_map(h: &reqwest::header::HeaderMap) -> BTreeMap<String, String> {
    h.iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                v.to_str()
                    .map(str::to_string)
                    // A header with non-UTF-8 bytes is recorded as its hex, not
                    // dropped: losing it would make the record incomplete in a
                    // way nobody could see.
                    .unwrap_or_else(|_| crate::hash::hex_bytes(v.as_bytes())),
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

    #[test]
    fn the_record_claims_exactly_the_headers_the_client_is_configured_with() {
        // **§6.2, and the defect in one assertion.** The record used to name
        // `accept: */*` and `accept-encoding: gzip` while the `reqwest` client was
        // built with neither — reqwest only sends an `accept-encoding` when its
        // `gzip` feature is on, and it is not — so every row of
        // `api/requests.ndjson` made a claim about the request that was false.
        //
        // The two are now built from one list, so this cannot drift by editing
        // one of them. It is asserted rather than assumed because the whole
        // defect was two lists that nobody compared.
        let ua = "test-agent/1.0";
        let recorded = RequestRecord::default_req_headers(ua, None);
        let configured = client_default_headers(ua).expect("header map");

        let mut from_client: BTreeMap<String, String> = BTreeMap::new();
        for (name, value) in configured.iter() {
            from_client.insert(
                name.as_str().to_string(),
                value.to_str().expect("ascii header").to_string(),
            );
        }
        assert_eq!(
            recorded, from_client,
            "the bundle's claim about the request and the request must be the same"
        );
        assert!(recorded.contains_key("accept"), "accept is sent");
        assert!(recorded.contains_key("user-agent"), "user-agent is sent");
    }

    #[test]
    fn the_record_does_not_claim_a_compression_it_cannot_decode() {
        // Naming `gzip` on the wire without reqwest's `gzip` feature is not a
        // no-op: the CDN would answer compressed and reqwest, unable to
        // decompress, would hand back the compressed bytes — so every manifest
        // digest in the bundle would be a digest of compressed JSON. This is the
        // half of §6.2 the plan flagged as "be ready to drop".
        assert_eq!(
            ACCEPT_ENCODING, "identity",
            "the tool offers no compression, so it must not claim any"
        );
        let headers = RequestRecord::default_req_headers("ua", None);
        assert_eq!(headers["accept-encoding"], "identity");
    }

    #[test]
    fn a_range_is_recorded_and_is_not_part_of_the_shared_set() {
        // `Range` is the one per-request header, so it is the one the shared
        // list must not carry — a client-wide `Range` would make every request a
        // byte range.
        assert!(
            !shared_req_headers("ua").contains_key("range"),
            "a client-wide Range would break every request but the first"
        );
        let with = RequestRecord::default_req_headers("ua", Some("bytes=0-1023"));
        assert_eq!(with["range"], "bytes=0-1023");
    }

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
        // Pushed out of order, read back in **seq** order: a `--jobs 8` capture
        // lands its chunk fetches whenever the network finishes, so completion
        // order and `seq` order differ in exactly the run that has the most of
        // them.
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

    /// `set_stored_as` reaches the right row **whatever order the rows landed
    /// in** — which is the property the `by_seq` index buys, and the one a
    /// `--jobs 8` capture actually depends on.
    ///
    /// A linear scan that returned the *first* match rather than the one with
    /// the matching `seq` would pass the single-row test above and fail this.
    #[test]
    fn stored_as_finds_its_row_among_rows_that_landed_out_of_order() {
        let log = RequestLog::new();
        let mk = |seq: u64, url: &str| RequestRecord {
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
            bytes_in: 7,
            stored_as: None,
            attempts: 1,
        };
        let seqs: Vec<u64> = (0..64).map(|_| log.reserve()).collect();
        // Land them backwards, as a saturated chunk fetcher would.
        for &s in seqs.iter().rev() {
            log.push(mk(s, &format!("/chunk/{s}")));
        }
        assert_eq!(log.len(), 64);

        for &s in seqs.iter() {
            log.set_stored_as(s, format!("chunks/blobs/{s}.zst"));
        }

        for &s in seqs.iter() {
            let row = log
                .records()
                .into_iter()
                .find(|r| r.seq == s)
                .expect("the row is there");
            assert_eq!(
                row.stored_as.as_deref(),
                Some(format!("chunks/blobs/{s}.zst").as_str()),
                "seq {s} got someone else's path"
            );
        }
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
            req_headers: BTreeMap::from([(
                "user-agent".to_string(),
                "UnityPlayer/2019.4.40f1 (UnityWebRequest/1.0, libcurl/7.80.0-DEV)".to_string(),
            )]),
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
            spawn_answers(
                statuses
                    .into_iter()
                    .map(|(status, body)| Answer {
                        status,
                        body,
                        claim: None,
                        omit_length: false,
                    })
                    .collect(),
                extra_body,
            )
        }

        /// One scripted answer, and how much of it to actually deliver.
        ///
        /// `claim` is the `Content-Length` the server announces. Setting it
        /// larger than `body.len()` makes the server promise more than it sends
        /// and then hang up — **a truncated response**, which is the one failure
        /// a real CDN produces and the only way to exercise the retry path from
        /// *mid-body*. Every other mock failure happens before a single body byte
        /// is written, so it never reaches the code that has to cope with a
        /// partial write.
        pub struct Answer {
            pub status: u16,
            pub body: &'static [u8],
            pub claim: Option<usize>,
            /// Send **no** Content-Length, so the body is delimited by the
            /// connection closing. The one shape a Content-Length pre-check
            /// cannot see, and therefore the only way to test the running
            /// total on its own.
            pub omit_length: bool,
        }

        pub fn spawn_answers(answers: Vec<Answer>, extra_body: &'static str) -> Mock {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().unwrap().port();
            let hits = Arc::new(AtomicU32::new(0));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let (h, r) = (hits.clone(), requests.clone());
            std::thread::spawn(move || {
                for a in answers {
                    let Ok((stream, _)) = listener.accept() else {
                        return;
                    };
                    h.fetch_add(1, Ordering::SeqCst);
                    serve_one(stream, a.status, a.body, a.claim, a.omit_length, extra_body, &r);
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
            claim: Option<usize>,
            omit_length: bool,
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
                "HTTP/1.1 {status} X\r\n{}Content-Type: application/octet-stream\r\nETag: \"abc\"\r\nServer: MockOSS\r\n{extra_body}Connection: close\r\n\r\n",
                if omit_length {
                    String::new()
                } else {
                    format!("Content-Length: {}\r\n", claim.unwrap_or(body.len()))
                }
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
        fetcher_with(retries, sleeper, DEFAULT_MAX_RESPONSE_BYTES)
    }

    /// [`fetcher`] with an explicit body ceiling, for the ceiling's own tests.
    fn fetcher_with(retries: u32, sleeper: Arc<dyn Sleep>, max_response_bytes: u64) -> Fetcher {
        Fetcher::with_sleeper(
            Transport {
                retries,
                timeout: Duration::from_secs(5),
                max_response_bytes,
            },
            RequestLog::new(),
            sleeper,
        )
        .expect("client builds")
    }

    /// The provenance row a fetch logged, for asserting what was **recorded**.
    ///
    /// `Fetched` used to carry a second deep copy of its own record so tests
    /// could read `recorded(&f).status` and the rest. That copy is gone — it cost
    /// ~25 heap allocations per chunk in production to satisfy a test — so the
    /// assertions read the log instead, which is the thing that actually
    /// reaches the bundle.
    ///
    /// Returns the **last** row, because a retried request logs every attempt:
    /// the row carrying the final `attempts` count is the last one.
    fn recorded(f: &Fetcher) -> RequestRecord {
        let v = f.log().records();
        assert!(!v.is_empty(), "nothing was recorded");
        v.into_iter().next_back().expect("at least one row")
    }

/// A panic in one fetch must not take the log — or the run — with it.
///
/// Poisoning was genuinely reachable, not theoretical: the log's lock sits
/// inside the concurrent chunk fetcher, so one task panicking while it holds
/// this lock poisons it for every other task. The old
/// `.expect("request log is not poisoned")` at eight sites then made every
/// later writer panic too, so a single panic anywhere turned into a run that
/// could not even record its own failure — while `Store::one` was explicitly
/// built to tolerate a panicking fetch task and carry on.
#[test]
fn a_poisoned_request_log_still_takes_records() {
    let other = RequestLog::new();
    let log = RequestLog::new();
    assert_eq!(log.reserve(), 1, "issue order starts at 1");

    // Poison it exactly the way a panicking fetch task would.
    let panicker = log.clone();
    let died = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _held = panicker.0.lock().expect("held only in order to poison");
        panic!("a fetch task died holding the log");
    }));
    assert!(died.is_err(), "the panic is the point");
    assert!(log.0.is_poisoned(), "and it did poison the lock");

    // Which is the whole claim: the next writer carries on.
    let seq = log.reserve();
    assert_eq!(seq, 2, "a poisoned log still numbers");
    let r = RequestRecord {
        seq,
        ts: "2026-01-01T00:00:00.000Z".into(),
        kind: Kind::Chunk,
        method: "GET",
        url: "https://example.invalid/x".into(),
        range: None,
        status: 200,
        final_url: String::new(),
        redirected: false,
        elapsed_ms: 1,
        req_headers: BTreeMap::new(),
        resp_headers: BTreeMap::new(),
        bytes_in: 0,
        stored_as: None,
        attempts: 1,
    };
    log.push(r);
    log.set_stored_as(seq, "somewhere");
    assert_eq!(log.len(), 1, "and still records");
    assert_eq!(log.bytes_in(), 0);
    let back = log.records();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].stored_as.as_deref(), Some("somewhere"));
    // Poisoning is per-lock, not global: an unrelated log is unaffected.
    assert_eq!(other.len(), 0);
}
    #[tokio::test]
    async fn waiting_for_a_retry_yields_the_runtime_rather_than_blocking_it() {
        // `#[tokio::test]` runs a current-thread runtime: one worker, and
        // everything on it. `RealSleep` used to be `std::thread::sleep`, so a
        // retry backoff held that worker for its whole duration — which is what
        // made `--jobs 64` on a 4-core box stall with every worker asleep.
        //
        // The margin is deliberate and wide: blocked, this takes >= 300 ms;
        // yielding, it takes ~30 ms. A timing assertion is only meaningful if
        // the two outcomes are far apart rather than close.
        let waiter = tokio::spawn(async {
            RealSleep.sleep(Duration::from_millis(300)).await;
        });
        let started = Instant::now();
        tokio::time::sleep(Duration::from_millis(30)).await;
        let elapsed = started.elapsed();
        waiter.await.expect("the waiter finished");

        assert!(
            elapsed < Duration::from_millis(200),
            "a 30 ms sleep took {elapsed:?} while one task waited 300 ms: \
             the waiter is holding the only worker"
        );
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
        assert_eq!(recorded(&f).attempts, 1);
        assert_eq!(recorded(&f).status, 200);
        assert_eq!(recorded(&f).bytes_in, 5);
        assert_eq!(recorded(&f).method, "GET");
        assert_eq!(recorded(&f).kind, Kind::Chunk);
        assert!(!recorded(&f).redirected);
        assert_eq!(
            recorded(&f).req_headers["user-agent"],
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
            recorded(&f).attempts, 3,
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

    /// A body to stream: big enough to arrive in several chunks, binary enough to
    /// catch a text-mode accident, and with a zstd magic at the front so it is a
    /// plausible blob rather than an arbitrary blob.
    fn streamable_body() -> Vec<u8> {
        let mut v = b"\x28\xb5\x2f\xfd".to_vec();
        for i in 0..60_000u32 {
            v.push((i.wrapping_mul(31) >> 7) as u8);
            v.extend_from_slice(b"payload ");
        }
        v
    }

    /// The mock serves &'static [u8] from its own thread, so a generated body has
    /// to outlive the test. Leaking is the honest way to say so in a test; a
    /// Cow would borrow instead and not compile.
    fn leak(v: &[u8]) -> &'static [u8] {
        Box::leak(v.to_vec().into_boxed_slice())
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("gircap-stream-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("scratch dir");
        d
    }

    /// A body the mock will send, `n` bytes of it.
    fn big_body(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    #[tokio::test]
    async fn observe_refuses_a_body_over_the_ceiling_rather_than_recording_an_empty_shape() {
    // The probe path is the one a hostile origin can most cheaply provoke — it is
    // a single unauthenticated GET — and its reads were unbounded. Over the
    // ceiling it must **fail**, not record an empty shape: an empty shape in
    // `api/error_shapes.json` would be a falsehood about what the origin did.
    //
    // Contrast with an *unreadable* body, which `observe` deliberately turns into
    // an empty body rather than an error. That asymmetry is the point, so it is
    // what the assertion pins.
    let body = big_body(4096);
    let mock = spawn(vec![(200, leak(&body))], "");
    let f = fetcher_with(
        1,
        Arc::new(RecordingSleep::default()),
        body.len() as u64 - 1,
    );

    let err = f
        .observe(&url(&mock, "/shape"), Kind::Probe)
        .await
        .expect_err("over the ceiling is a failure, not a shape");
    assert!(err.to_string().contains("ceiling"), "{err}");
}

#[tokio::test]
    async fn a_body_over_the_ceiling_is_refused_rather_than_buffered() {
        // **§5.5 row 11, the whole point of the ceiling.** The client had
        // `timeout` and `user_agent` and nothing else, and every body read was
        // unbounded — so a broken or hostile origin that kept sending filled RAM
        // until the process died. A timeout does not help: a server streaming a
        // gigabyte steadily for ten minutes never trips one.
        let body = big_body(4096);
        let mock = spawn(vec![(200, leak(&body)), (200, leak(&body))], "");
        let f = fetcher_with(
            1,
            Arc::new(RecordingSleep::default()),
            body.len() as u64 - 1,
        );

        let err = f
            .get(&url(&mock, "/big"), Kind::Chunk, None)
            .await
            .expect_err("a body over the ceiling must not be buffered");
        let text = err.to_string();
        assert!(text.contains("ceiling"), "the message must name the ceiling: {text}");
        assert!(text.contains("--max-response-bytes"), "and how to raise it: {text}");
        // **Not retried.** An origin that sent too much will send too much again,
        // and the mock had a second answer that must not have been consumed.
        assert_eq!(
            mock.hits.load(Ordering::SeqCst),
            1,
            "a `Shape` is not retried, so one attempt is all it gets"
        );
    }

    #[tokio::test]
    async fn a_body_under_the_ceiling_is_untouched() {
        // The other half, or the ceiling would be a tool that fetches nothing.
        // Byte-for-byte equality, so this also pins that routing the buffered path
        // through `drain_capped` did not alter what a caller receives.
        let body = big_body(4096);
        let mock = spawn(vec![(200, leak(&body))], "");
        let f = fetcher_with(
            1,
            Arc::new(RecordingSleep::default()),
            body.len() as u64,
        );

        let got = f
            .get(&url(&mock, "/ok"), Kind::Chunk, None)
            .await
            .expect("a body exactly at the ceiling is allowed");
        assert_eq!(got.body, body, "the bytes must be exactly what was served");
        assert_eq!(got.body.len() as u64, body.len() as u64);
    }

    #[tokio::test]
    async fn a_declared_length_over_the_ceiling_is_refused_without_reading_it() {
        // The cheap half of the two checks: an origin that *says* it is sending
        // 10 GiB is refused before a byte is transferred, rather than after this
        // tool has spent the time and the bandwidth receiving it.
        //
        // Driven with `Content-Length` set to more than the body actually sent —
        // which is what `local_server::Answer::claim` is for, and is also the
        // shape of a lying origin.
        let body = big_body(64);
        let mock = spawn_answers(
            vec![Answer {
                status: 200,
                body: leak(&body),
                claim: Some(body.len() * 1000),
                omit_length: false,
            }],
            "",
        );
        let f = fetcher_with(
            1,
            Arc::new(RecordingSleep::default()),
            body.len() as u64,
        );

        let err = f
            .get(&url(&mock, "/lying"), Kind::Chunk, None)
            .await
            .expect_err("a declared length over the ceiling must be refused");
        assert!(
            err.to_string().contains("ceiling"),
            "the message must name the ceiling: {err}"
        );
    }

    #[tokio::test]
    async fn the_file_destination_is_capped_too() {
        // A ceiling enforced in the buffered path and not the file path is a
        // ceiling with a hole in it — and the file path is the one that fetches
        // blobs, so it is the hole that matters.
        let body = big_body(2048);
        let mock = spawn(vec![(200, leak(&body))], "");
        let dir = scratch("file-cap");
        let path = dir.join("blob.bin");
        let mut out = crate::hash::HashingFile::create(&path).expect("create");
        let f = fetcher_with(
            1,
            Arc::new(RecordingSleep::default()),
            16,
        );

        let err = f
            .get_to_file(&url(&mock, "/big"), Kind::Chunk, &mut out)
            .await
            .expect_err("the file destination must be capped as well");
        assert!(err.to_string().contains("ceiling"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_body_over_the_ceiling_is_stopped_even_with_no_content_length() {
    // **The second check, on its own.** Found by mutation: with an accurate
    // `Content-Length` the pre-check always fires first, so disabling the running
    // total changed nothing — all 37 http tests passed. An origin that omits the
    // header is the only shape the pre-check cannot see, and without the running
    // total it is unbounded exactly as before this change.
    let body = big_body(4096);
    let mock = spawn_answers(
        vec![Answer {
            status: 200,
            body: leak(&body),
            claim: None,
            omit_length: true,
        }],
        "",
    );
    let f = fetcher_with(
        1,
        Arc::new(RecordingSleep::default()),
        body.len() as u64 - 1,
    );

    let err = f
        .get(&url(&mock, "/no-length"), Kind::Chunk, None)
        .await
        .expect_err("with no Content-Length the running total is the only defence");
    assert!(
        err.to_string().contains("ceiling"),
        "the message must name the ceiling: {err}"
    );
}

#[tokio::test]
    async fn observe_records_a_server_error_once_and_never_retries() {
    // **§5.5 row 16, the half that can be pinned.** `kind: "probe"` covers two
    // paths with different retry behaviour: the tag probe goes through `get` and
    // the retry policy, the negative probes go through `observe`, which sends once
    // and records `attempts: 1` unconditionally.
    //
    // All 8 probe rows in the reference bundle have `attempts = 1`, so a reader
    // cannot tell the two apart from the log — which is why that row is being
    // documented rather than split. What *can* be pinned is the asymmetry the
    // documentation now asserts, and this is it: a `500`, which the retry policy
    // treats as retryable, is still recorded once by `observe`.
    let mock = spawn(vec![(503, b"<Error><Code>Busy</Code></Error>"), (200, b"unused")], "");
    let f = fetcher(3, Arc::new(RecordingSleep::default()));

    let o = f
        .observe(&url(&mock, "/shape"), Kind::Probe)
        .await
        .expect("a 5xx is data for a probe, not a failure");
    assert_eq!(o.status, 503, "the status is recorded as received");
    assert!(o.text().contains("Busy"), "and so is the body");
    assert_eq!(
        mock.hits.load(Ordering::SeqCst),
        1,
        "observe must not retry: the mock only had one answer for it to consume"
    );

    let rows: Vec<_> = f.log().records();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].attempts, 1, "`attempts: 1` is hardcoded, and is the claim");
    assert_eq!(rows[0].kind, Kind::Probe);
}

#[tokio::test]
    async fn a_failed_request_counts_the_body_it_threw_away() {
        // **§5.5 row 1.** `bytes_in` was recorded as `0` for every non-2xx and the
        // response was dropped without its body being read. So a `429` carrying a
        // few KB of XML envelope was transferred and reported as free — and the
        // requests it under-reported are the retried ones, which is to say the
        // expensive ones.
        let envelope = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error>\n  \
                         <Code>TooManyRequests</Code>\n  <Message>Slow down.</Message>\n\
                         </Error>\n";
        let mock = spawn(vec![(429, envelope)], "");
        let f = fetcher(1, Arc::new(RecordingSleep::default()));

        let err = f
            .get(&url(&mock, "/blob"), Kind::Chunk, None)
            .await
            .expect_err("429 is not a success");
        assert!(matches!(err, ApiError::Status { status: 429, .. }), "{err:?}");

        let rows: Vec<_> = f.log().records();
        assert_eq!(rows.len(), 1, "the failure must still be recorded");
        assert_eq!(
            rows[0].bytes_in,
            envelope.len() as u64,
            "bytes_in must count the body that arrived, not report the failure as free"
        );
        assert_eq!(rows[0].status, 429);
    }

    #[tokio::test]
    async fn a_rejected_request_is_priced_at_every_attempt_it_cost() {
        // The shape the module doc promises: a run that was throttled and retried
        // paid for every response it received, and the summary's total has to say
        // so. Two `429`s and a `200` is a run that cost three bodies.
        let envelope = b"<Error><Code>TooManyRequests</Code></Error>";
        let payload = b"the blob, at last";
        let mock = spawn_answers(
            vec![
                Answer { status: 429, body: envelope, claim: None, omit_length: false },
                Answer { status: 503, body: envelope, claim: None, omit_length: false },
                Answer { status: 200, body: leak(payload), claim: None, omit_length: false },
            ],
            "",
        );
        let f = fetcher(3, Arc::new(RecordingSleep::default()));

        let got = f
            .get(&url(&mock, "/blob"), Kind::Chunk, None)
            .await
            .expect("the third attempt succeeds");

        assert_eq!(got.body, payload, "the successful body is the third one");
        assert_eq!(mock.hits.load(Ordering::SeqCst), 3, "it retried twice");

        // Every attempt is logged — a run that recovered from a throttle must
        // show the 429s, or `attempts` is a claim nobody can check.
        let rows: Vec<_> = f.log().records();
        assert_eq!(rows.len(), 3, "all three attempts are recorded");
        let failed: u64 = rows[..2].iter().map(|r| r.bytes_in).sum();
        assert_eq!(
            failed,
            envelope.len() as u64 * 2,
            "both throttled responses must be priced, not reported as free"
        );
        assert_eq!(rows[2].bytes_in, payload.len() as u64);
        assert_eq!(
            f.log().bytes_in(),
            envelope.len() as u64 * 2 + payload.len() as u64,
            "the run's total is every body it received"
        );
    }

    #[tokio::test]
    async fn a_streamed_fetch_writes_the_body_and_reports_the_digests_of_those_bytes() {
        // **§6.6.2's gate, at the layer the change touches.** The buffered path is
        // the reference: this asserts the streamed path produced the same bytes on
        // disk and the same digests and the same `bytes_in`, so nothing
        // downstream — `chunks/index.json`'s md5, A6's four checks, the size in
        // `MANIFEST.json` — can tell the two apart.
        let body = streamable_body();
        // Two mocks, because the server answers exactly as many requests as it
        // was given answers and this test makes two.
        let buffered_mock = spawn(vec![(200, leak(&body))], "");
        let streamed_mock = spawn(vec![(200, leak(&body))], "");
        let dir = scratch("same-as-buffered");
        let f = fetcher(1, Arc::new(RecordingSleep::default()));

        let buffered = f
            .get(&url(&buffered_mock, "/blob"), Kind::Chunk, None)
            .await
            .expect("buffered");

        let path = dir.join("streamed.bin");
        let mut out = crate::hash::HashingFile::create(&path).expect("create");
        let streamed = f
            .get_to_file(&url(&streamed_mock, "/blob"), Kind::Chunk, &mut out)
            .await
            .expect("streamed");
        let digests = out.finish().expect("finish");

        let on_disk = std::fs::read(&path).expect("read back");
        assert_eq!(on_disk.len(), body.len(), "length moved");
        assert!(on_disk == body, "the bytes on disk are not the bytes served");
        assert_eq!(streamed.bytes_in, buffered.body.len() as u64);
        assert_eq!(streamed.status, buffered.status);
        assert_eq!(digests, crate::hash::md5_sha256(&body));
        assert_eq!(digests.bytes, body.len() as u64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_digests_of_a_retried_blob_cover_only_the_body_that_landed() {
        // **The failure this change had to get right, and the one the reference
        // bundle cannot check.** A healthy capture never retries, so the 91-of-97
        // gate says nothing about what happens when a body is truncated half way
        // through and the next attempt succeeds.
        //
        // The first answer under-delivers: it promises a length and sends less,
        // so the client has *already written part of it* when the read fails. The
        // digests must then describe the second answer alone — not the
        // concatenation, and not a prefix of the first.
        let first = streamable_body();
        let second = b"<?xml version=\"1.0\"?><Error><Code>NoSuchKey</Code></Error>".to_vec();
        let mock = spawn_answers(
            vec![
                Answer {
                    status: 200,
                    // Announce the whole length, send a third of it, then hang
                    // up — a truncated response. `claim` is what the header says
                    // and `body` is what goes on the wire, so they must differ.
                    body: leak(&first[..first.len() / 3]),
                    claim: Some(first.len()),
                    omit_length: false,
                },
                Answer {
                    status: 200,
                    body: leak(&second),
                    claim: None,
                    omit_length: false,
                },
            ],
            "",
        );
        let dir = scratch("retry-digests");
        let path = dir.join("blob.bin");
        let mut out = crate::hash::HashingFile::create(&path).expect("create");

        // Two attempts allowed, and a sleeper that does not sleep: the backoff is
        // not what is under test and a real one would cost a second per retry.
        let f = fetcher(2, Arc::new(RecordingSleep::default()));
        let got = f
            .get_to_file(&url(&mock, "/blob"), Kind::Chunk, &mut out)
            .await
            .expect("the retry should succeed");
        let digests = out.finish().expect("finish");

        assert_eq!(mock.hits.load(Ordering::SeqCst), 2, "it should have retried");
        assert_eq!(digests, crate::hash::md5_sha256(&second), "digests must be the second body");
        assert_eq!(digests.bytes, second.len() as u64);
        assert_eq!(got.bytes_in, second.len() as u64);

        let on_disk = std::fs::read(&path).expect("read back");
        assert_eq!(
            on_disk.len(),
            second.len(),
            "the file must not still hold the tail of the truncated attempt"
        );
        assert!(on_disk == second, "the file holds the wrong bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_write_failure_is_not_retried_as_if_it_were_the_network() {
        // A disk that will not take the bytes must stop the run, not spend the
        // retry budget rediscovering that a full disk stays full. Reported as a
        // `Transport` it would be retried, because a transport failure is
        // retryable by policy.
        //
        // **Tested at the classifier rather than through a real full disk.** A
        // portable way to make one is not available, and the decision is a
        // two-line branch: what matters is that the two causes land on different
        // variants, and that the policy then treats them differently.
        let read = body_failure("http://x/blob", 2, true, "connection reset");
        assert!(
            matches!(read, ApiError::Transport { .. }),
            "a read failure is the network's fault and must be retryable, got {read:?}"
        );

        let write = body_failure("http://x/blob", 2, false, "no space left on device");
        assert!(
            matches!(write, ApiError::Shape { .. }),
            "a write failure is the disk's fault and must not be retried, got {write:?}"
        );

        // The two halves of the claim: the policy keys off the variant, and the
        // variants differ. (A `Transport` retries; a `Shape` does not — see
        // `run`.)
        let retryable = |e: &ApiError| match e {
            ApiError::Transport { .. } => true,
            ApiError::Shape { .. } => false,
            ApiError::Status { .. } => true,
        };
        assert!(retryable(&read));
        assert!(!retryable(&write));
        assert!(
            write.to_string().contains("no space left on device"),
            "the local cause must survive into the message a reader sees: {write}"
        );
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
        assert_eq!(recorded(&f).status, 206, "a partial response is a success");
        assert_eq!(recorded(&f).range.as_deref(), Some("bytes=0-9"));
        assert_eq!(recorded(&f).req_headers["range"], "bytes=0-9");
        assert_eq!(recorded(&f).resp_headers["content-range"], "bytes 0-9/100");
        assert_eq!(recorded(&f).bytes_in, 10);

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
        f.get(&url(&mock, "/m"), Kind::Manifest, None)
            .await
            .unwrap();
        let got = recorded(&f);
        assert_eq!(got.range, None);
        assert!(!got.req_headers.contains_key("range"));
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
        f.get(&url(&mock, "/x"), Kind::Api, None).await.unwrap();
        let got = recorded(&f);
        assert!(
            got.resp_headers
                .keys()
                .all(|k| k.chars().all(|c| !c.is_ascii_uppercase()))
        );
        assert_eq!(got.resp_headers["server"], "MockOSS");
        assert_eq!(
            got.resp_headers["etag"], "\"abc\"",
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
