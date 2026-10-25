use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

use crate::hash::hex_bytes;

// `shared_req_headers` lives in the parent module while it has not been moved
// to `fetcher.rs` yet; after step 2.5 it arrives via `mod.rs`'s
// `pub(crate) use fetcher::shared_req_headers;`, so this import keeps working.
use super::shared_req_headers;

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
    /// **Two paths share this kind, and they differ in retry behaviour.** The tag
    /// probe goes through [`Fetcher::get`] and so through [`RetryPolicy`]; the
    /// negative probes go through [`Fetcher::observe`], which sends once and
    /// records `attempts: 1` unconditionally. A reader of `api/requests.ndjson`
    /// therefore cannot tell them apart from the row alone, and the `kind` was
    /// deliberately not split to fix that — see the note on [`Kind`]'s wire
    /// spelling below.
    Probe,
}

impl Kind {
    /// The wire spelling.
    ///
    /// **This is a closed vocabulary and the schema doc says so**, which is why
    /// the two probe behaviours are answered with documentation rather than
    /// distinct kinds. Splitting `probe` into `tag_probe` and `shape_probe` would
    /// not be additive: it would *remove* the value `probe`, so any reader matching
    /// on it — which is the natural way to ask "what did the metadata cost?" —
    /// would silently start seeing nothing. Widening a closed vocabulary by
    /// subtraction is a breaking change to a recorded field, which is a larger
    /// decision than an ambiguous log row deserves.
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
    /// Where the body actually came from.
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
    /// **The record and the client must agree.** reqwest sends an
    /// `accept-encoding` only when its `gzip` feature is on, and it is not, so the
    /// wire carries `identity`. Naming `gzip` instead would be the worse of the
    /// two: adding the header *without* the feature is not a no-op — the CDN would
    /// answer with a gzip body and reqwest, unable to decompress it, would hand
    /// back the compressed bytes, and every manifest digest in the bundle would be
    /// a digest of compressed JSON. So the record states what the wire does, which
    /// is identity, and the client is configured from the same list.
    pub(crate) fn default_req_headers(
        user_agent: &str,
        range: Option<&str>,
    ) -> BTreeMap<String, String> {
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
    pub(crate) fn req_headers(
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

/// The `Accept` the tool sends, and the value it records.
pub const ACCEPT: &str = "*/*";

/// The `Accept-Encoding` the tool sends, and the value it records.
///
/// **`identity`, deliberately.** See `RequestRecord::default_req_headers` for
/// why claiming `gzip` was wrong and why adopting it would be worse: reqwest's
/// `gzip` feature is off, so naming it on the wire would buy compressed bodies
/// this client cannot decompress.
pub const ACCEPT_ENCODING: &str = "identity";

/// Lowercase every header key, keeping the value exactly as received.
///
/// Sorted, because a `BTreeMap` makes two captures of the same request diff
/// cleanly — and an `ETag` is *not* a string this tool parses, so nothing is
/// lost to the normalisation.
///
/// **The keys are already lowercase.** `http::HeaderName` validates and normalises
/// to lowercase on construction and guarantees `as_str()` returns that form, so
/// normalising again would allocate a `String` per header name and discard the
/// distinction it was protecting. The output is unchanged; only the copy is gone.
pub(crate) fn header_map(h: &reqwest::header::HeaderMap) -> BTreeMap<String, String> {
    h.iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                v.to_str()
                    .map(str::to_string)
                    // A header with non-UTF-8 bytes is recorded as its hex, not
                    // dropped: losing it would make the record incomplete in a
                    // way nobody could see.
                    .unwrap_or_else(|_| hex_bytes(v.as_bytes())),
            )
        })
        .collect()
}
