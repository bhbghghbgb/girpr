use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use super::{
    ACCEPT, ACCEPT_ENCODING, ApiError, Kind, RealSleep, RequestLog, RequestRecord, RetryPolicy,
    Sleep, Transport, header_map, now_rfc3339,
};

/// A fetched body and the record of how it arrived.
#[derive(Debug)]
pub struct Fetched {
    /// The body, byte-exact.
    pub body: Vec<u8>,
    /// This request's sequence number, for `set_stored_as`.
    ///
    /// A `u64` and not a [`RequestRecord`]. The log already owns the record, so
    /// handing a second copy to every caller means deep-cloning two `BTreeMap`s, a
    /// `String` URL and up to ~2 KB of response headers for each of 113 610 chunks,
    /// and then dropping all of it after reading one integer.
    pub seq: u64,
    /// The status as received.
    ///
    /// `Copy`, because `probe` genuinely needs it to fill a document field and
    /// reading it back out of the log would be a round trip for one `u16`.
    pub status: u16,
}

/// Where a fetched body is put.
///
/// This exists so the retry loop, the status check and the request record are
/// written **once**. A second loop for the file case would be 30 lines that look
/// identical until a retry policy is edited in one of them.
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
pub(crate) fn body_failure(
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
/// The length a response declares, when that declaration alone is over `cap`.
///
/// The cheap half of the ceiling, shared so a successful body and an error body
/// cannot disagree about what "declared too large" means.
fn declared_over_ceiling(resp: &reqwest::Response, cap: u64) -> Option<u64> {
    resp.content_length().filter(|d| *d > cap)
}

/// Drain a **successful** response body into `sink`, refusing to exceed the
/// transport's ceiling.
///
/// Every successful read goes through here — the buffered path, the file path and
/// `observe`. A ceiling enforced in two of those three would be a ceiling with a
/// hole in it.
///
/// Two checks, because one is not enough:
///
/// - against `Content-Length` **before** reading, so an origin declaring 10 GiB is
///   refused without transferring any of it;
/// - against the running total as bytes arrive, for an origin that omits the
///   header or understates it.
///
/// `sink` is generic rather than "either a `Vec` or a file" so all three callers
/// share this.
///
/// **An error body does not come through here.** It has the same two checks in
/// [`drain_error_body`], but it must not *fail* the way this does: the primary
/// fact of a non-2xx response is its status, and replacing a `404` with a
/// `Shape`/`Transport` about the body both misreports the cause and — for
/// `Transport` — would retry a request that will answer `404` again.
///
/// **A failure carries the bytes that arrived.** The count is local while the
/// body is read, and returning only the error discarded it: a `2xx` that closed
/// after 10 KiB was recorded with `bytes_in: 0`, which says the attempt was
/// free. The caller prices the request with the second half of the tuple, so a
/// partial body costs what it actually cost.
async fn drain_capped<W: std::io::Write>(
    mut resp: reqwest::Response,
    sink: &mut W,
    cap: u64,
    url: &str,
    attempts: u32,
) -> Result<u64, (ApiError, u64)> {
    if let Some(d) = declared_over_ceiling(&resp, cap) {
        // Nothing has crossed the wire yet, so `0` is the honest count.
        return Err((too_big(url, d, cap), 0));
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
                    return Err((too_big(url, seen, cap), seen));
                }
                std::io::Write::write_all(sink, &chunk)
                    .map_err(|e| (body_failure(url, attempts, false, e), seen))?;
            }
            Ok(None) => return Ok(seen),
            Err(e) => return Err((body_failure(url, attempts, true, e), seen)),
        }
    }
}

/// Count the body of a failed response, stopping at the transport's ceiling.
///
/// **Counting only, and never fallible.** The body of a `4xx`/`5xx` is discarded;
/// only its length is data (`bytes_in` prices the request, including the ones a
/// throttle made expensive). So this applies the same two checks as
/// [`drain_capped`] — the declared length before reading, the running total as
/// bytes arrive — and returns how many bytes crossed the wire, without turning
/// any outcome into a different error.
///
/// That last part is the whole reason this is not `drain_capped` plus
/// `io::sink()`:
///
/// - a body that fails mid-read is still a `Status` failure, so it is counted up
///   to the failure and not retried as if the network were at fault;
/// - a body over the ceiling is still a `Status` failure, so a lying `503` does
///   not become a `Shape` (which would hide the status) nor a `Transport` (which
///   would retry it);
/// - a `404` with a truncated body stays a `404` and is not fetched again.
///
/// The ceiling still bounds time and bandwidth: an origin that declares a huge
/// body is not read at all, and one that omits or understates the header is
/// stopped once the running total crosses it.
async fn drain_error_body(mut resp: reqwest::Response, cap: u64, url: &str) -> u64 {
    if let Some(declared) = declared_over_ceiling(&resp, cap) {
        // Nothing has crossed the wire yet, so `0` is the honest count.
        tracing::warn!(
            url,
            declared,
            cap,
            "error body declares more than --max-response-bytes; not read"
        );
        return 0;
    }
    let mut seen = 0_u64;
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                seen += chunk.len() as u64;
                if seen > cap {
                    // The chunk is already received, so it is counted; what the
                    // ceiling prevents is waiting for the rest of it.
                    tracing::warn!(
                        url,
                        seen,
                        cap,
                        "error body crossed --max-response-bytes; stopped reading"
                    );
                    break;
                }
            }
            Ok(None) => break,
            // A read failure ends the count at what arrived. The status is still
            // the failure the caller sees: an unreadable error body is not a
            // reason to lose the `404`/`503` that explains the request. Log the
            // failure anyway, or a suspiciously short `bytes_in` has no cause.
            Err(e) => {
                tracing::debug!(
                    url,
                    bytes = seen,
                    error = %e,
                    "error-body read failed; counting what arrived"
                );
                break;
            }
        }
    }
    seen
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
    // owns it; nothing here reads a field of it.
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

/// The `Accept-Encoding` the tool sends, and the value it records.
///
/// **`identity`, deliberately.** See `RequestRecord::default_req_headers` for
/// why claiming `gzip` was wrong and why adopting it would be worse: reqwest's
/// `gzip` feature is off, so naming it on the wire would buy compressed bodies
///
/// The headers every request carries, as the record spells them.
///
/// **One list, two consumers.** [`client_default_headers`] turns this into the
/// `HeaderMap` the `reqwest` client is built with, so the record and the wire
/// cannot drift — the defect this exists to prevent was two lists that nobody
/// compared.
pub(crate) fn shared_req_headers(user_agent: &str) -> BTreeMap<String, String> {
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
pub(crate) fn client_default_headers(
    user_agent: &str,
) -> Result<reqwest::header::HeaderMap, ApiError> {
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
        // have already been captured by this point. The bytes that did arrive
        // are still priced, for the same reason every other attempt's are.
        //
        // **But a body over the ceiling is a failure, and the two are different
        // things.** A probe is the one request an attacker can most cheaply
        // provoke, so it is exactly where an unbounded read does the most damage.
        // Unreadable means "there is no body to record"; over the ceiling means
        // "this body exists and is larger than this tool will hold", and recording
        // an empty shape for that would put a falsehood in
        // `api/error_shapes.json`.
        let mut body: Vec<u8> = Vec::new();
        let bytes_in = match drain_capped(resp, &mut body, self.max_response_bytes, url, 1).await {
            Ok(n) => n,
            Err((e, seen)) if is_read_failure(&e) => {
                body = Vec::new();
                seen
            }
            Err((e, _)) => return Err(e),
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
        // tool can panic.
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
    /// The digests and the byte count come from the write itself, so they are the
    /// same numbers the buffered path produces over the same bytes — see
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

            match self
                .attempt(url, kind, range, attempts, started, &mut dest)
                .await
            {
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
            // **The body of a failure is counted, not ignored.** A CDN
            // `429`/`503` carrying a few KB of XML envelope is paid for on the
            // wire, so recording `bytes_in: 0` reports it as free.
            // `bytes_in` is sold in this module's doc as "the only way to price a
            // manifest without re-downloading it", and it must not under-report
            // *precisely the retried requests that cost the most* — a throttled
            // run's cost figures are optimistic by exactly the amount the throttle
            // cost.
            //
            // **Streamed and discarded, and capped**, not read into a buffer: the
            // content is useless, only the length is not. Draining also lets the
            // connection go back to the pool, which dropping a response mid-body
            // would not. `drain_error_body` stops at `--max-response-bytes`, so a
            // hostile `503` costs the ceiling rather than the timeout.
            //
            // A body that fails to read mid-way counts what arrived, like every
            // other count here: reporting `0` for bytes that demonstrably crossed
            // the network would be a lie in the direction that matters least to
            // notice. What it must not do is replace the status.
            let bytes_in = drain_error_body(resp, self.max_response_bytes, url).await;
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
                // size are both about the same request. The count comes from the
                // error, because a body that failed mid-read still crossed the
                // wire and `bytes_in` prices what crossed it.
                let fail = |err: ApiError, bytes_in: u64| {
                    let record = self.record(
                        seq,
                        kind,
                        url,
                        range,
                        status,
                        resp_headers.clone(),
                        &final_url,
                        redirected,
                        bytes_in,
                        attempts,
                        started,
                    );
                    (err, record)
                };
                let mut buf: Vec<u8> =
                    Vec::with_capacity(resp.content_length().unwrap_or(0) as usize);
                match drain_capped(resp, &mut buf, self.max_response_bytes, url, attempts).await {
                    Ok(n) => {
                        bytes_in = n;
                        Some(buf)
                    }
                    Err((e, seen)) => return Err(fail(e, seen)),
                }
            }
            Destination::File(out) => {
                // Discard whatever a failed attempt left before this one writes.
                // Without this the digests cover both responses — see
                // `HashingFile::restart`.
                let fail = |err: ApiError, bytes_in: u64| {
                    let record = self.record(
                        seq,
                        kind,
                        url,
                        range,
                        status,
                        resp_headers.clone(),
                        &final_url,
                        redirected,
                        bytes_in,
                        attempts,
                        started,
                    );
                    (err, record)
                };

                if let Err(e) = out.restart() {
                    // Not retryable, and not a transport failure: a disk that will
                    // not truncate is a disk that will not write either. No body
                    // byte of this attempt has been read, so its cost is zero.
                    return Err(fail(
                        body_failure(
                            url,
                            attempts,
                            false,
                            format!("preparing the body file: {e}"),
                        ),
                        0,
                    ));
                }
                match drain_capped(resp, out, self.max_response_bytes, url, attempts).await {
                    Ok(n) => bytes_in = n,
                    Err((e, seen)) => return Err(fail(e, seen)),
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
