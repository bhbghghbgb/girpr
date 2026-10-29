//! Blob properties: container sniff, digests, and the recorded A6 results.
//!
//! **M6.** Both digests are kept because both are *asserted* — the compressed
//! MD5 is what the CDN's `ETag` claims, the decompressed one is what the
//! manifest claims, and a blob that passes only one of them is evidence of
//! something.
//!
//! ## Why measurement never fails
//!
//! A6 is four checks: the raw length, the compressed MD5, the decompressed MD5
//! and the decoded length. [`BlobProps::measure`] returns all four as data and
//! returns `Ok` even when they disagree, because a capture that hit a
//! truncated CDN response is still worth keeping — the selfcheck is where that
//! gets said out loud, one row per chunk instance, and dropping the blob would
//! destroy the evidence. What *is* an error is a blob that is not zstd at all:
//! there is nothing to measure, and guessing would be worse.
//!
//! The decoded bytes are produced and dropped rather than cached. The store
//! measures here and assembles in M7, and holding a second copy of every chunk
//! between those two steps would be tens of megabytes of peak RSS to avoid a
//! second decompress.

use serde::{Deserialize, Serialize};

use crate::hash;
use crate::http::ApiError;
use crate::manifest::proto::SophonChunk;

/// The four A6 checks, kept apart so a failing capture can name which one
/// broke rather than only that something did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verified {
    /// `len(raw) == compressed_size` — the blob arrived whole.
    pub raw_size_matches: bool,
    /// `md5(raw) == compressed_md5` — it arrived *intact*, not merely complete.
    pub md5_compressed_matches: bool,
    /// `md5(zstd(raw)) == uncompressed_md5` — it is the chunk the manifest named.
    pub md5_decompressed_matches: bool,
    /// `len(decoded) == uncompressed_size`.
    pub decoded_size_matches: bool,
}

impl Verified {
    /// Whether all four hold.
    pub fn all_green(&self) -> bool {
        self.raw_size_matches
            && self.md5_compressed_matches
            && self.md5_decompressed_matches
            && self.decoded_size_matches
    }

    /// The individual A6 checks that failed, in the spec's order.
    ///
    /// Empty when green. This is what turns "selfcheck failed" into "the decoded
    /// MD5 of this blob does not match what the manifest said", which is the
    /// difference between a bisectable report and a shrug.
    pub fn failures(&self) -> Vec<&'static str> {
        self.failures_iter().collect()
    }

    /// Whether any of the four failed.
    ///
    /// For callers that only need the yes/no. `failures()` allocates a `Vec` to
    /// say the same thing, and it is called from inside a `filter` over every
    /// blob in the store — so the answer was being built and dropped once per
    /// blob per format.
    pub const fn any_failure(&self) -> bool {
        !self.raw_size_matches
            || !self.md5_compressed_matches
            || !self.md5_decompressed_matches
            || !self.decoded_size_matches
    }

    /// Which of the four did not hold, in the spec's order.
    ///
    /// The primitive: [`Verified::failures`] collects it and `Display` walks it
    /// without collecting, so rendering a blob does not allocate on the two paths
    /// that matter — all green, and exactly one failure.
    pub fn failures_iter(&self) -> impl Iterator<Item = &'static str> {
        [
            (!self.raw_size_matches).then_some("len(raw blob) == compressed_size"),
            (!self.md5_compressed_matches).then_some("md5(raw blob) == compressed_md5"),
            (!self.md5_decompressed_matches).then_some("md5(zstd(raw blob)) == uncompressed_md5"),
            (!self.decoded_size_matches).then_some("len(decoded) == uncompressed_size"),
        ]
        .into_iter()
        .flatten()
    }
}

impl std::fmt::Display for Verified {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Walked rather than collected: a `Display` that allocates a `Vec` to
        // decide between "none", "one" and "many" pays for the rare case on
        // every render, and this renders once per A6 row.
        //
        // `next()` is peeked, not destructured: consuming the second failure
        // into a `_` pattern would drop it from the "many" branch.
        let mut it = self.failures_iter();
        let Some(first) = it.next() else {
            return f.write_str("all four A6 checks hold");
        };
        let Some(second) = it.next() else {
            return write!(f, "{first} does not hold");
        };
        let rest: Vec<&str> = std::iter::once(first)
            .chain(std::iter::once(second))
            .chain(it)
            .collect();
        write!(
            f,
            "{} of the four A6 checks do not hold: {}",
            rest.len(),
            rest.join(", ")
        )
    }
}

/// Why a chunk fetch did not produce a complete blob.
///
/// **A fetch failure is data, not only a sentence.** The source tag, the URL
/// requested, the status or transport failure and the attempt count were all
/// known when the request failed and were previously folded into one string in
/// `decode_error`. Keeping them apart is what lets a reader answer "was this a
/// `404` from the old CDN, a timeout, or a write that failed?" without parsing
/// prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchError {
    /// The manifest tag whose source served (or failed to serve) the chunk.
    /// `null` when no source was reached or the failure was local.
    pub source: Option<String>,
    /// The exact URL requested. `null` for a local write or task failure.
    pub url: Option<String>,
    /// The HTTP status, when there was one.
    pub status: Option<u16>,
    /// The transport failure text, when the request never completed.
    pub transport: Option<String>,
    /// Attempts made for the final request, including the first.
    pub attempts: u32,
    /// The full human-readable reason, the same one the run's error list carries.
    pub detail: String,
}

impl FetchError {
    /// A failure with no HTTP exchange behind it.
    pub fn local(detail: impl Into<String>) -> FetchError {
        FetchError {
            source: None,
            url: None,
            status: None,
            transport: None,
            attempts: 0,
            detail: detail.into(),
        }
    }

    /// Build one from the fetcher's typed error.
    pub fn from_api(source: Option<&str>, url: &str, error: &ApiError) -> FetchError {
        match error {
            ApiError::Transport {
                url,
                attempts,
                detail,
            } => FetchError {
                source: source.map(str::to_string),
                url: Some(url.clone()),
                status: None,
                transport: Some(detail.clone()),
                attempts: *attempts,
                detail: error.to_string(),
            },
            ApiError::Status {
                url,
                status,
                attempts,
            } => FetchError {
                source: source.map(str::to_string),
                url: Some(url.clone()),
                status: Some(*status),
                transport: None,
                attempts: *attempts,
                detail: error.to_string(),
            },
            ApiError::Shape { .. } => FetchError {
                source: source.map(str::to_string),
                url: Some(url.to_string()),
                status: None,
                transport: None,
                attempts: 1,
                detail: error.to_string(),
            },
        }
    }
}

/// One stored blob and everything known about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobProps {
    /// The content address: the manifest's `chunk.id`.
    pub chunk_id: String,
    /// Bundle-relative path, or `null` when `--no-chunk-blobs` skipped it.
    pub stored_path: Option<String>,
    /// `len(raw)`, which is what the CDN sent and what the file holds.
    pub bytes: u64,
    /// The sniff result. `"zstd"` or `"identity"`, never a guess.
    ///
    /// An owned `String` rather than a `&'static str`: M9's `verify` reads this
    /// field back out of `chunks/index.json`, and a value that arrived from a
    /// document is not `'static`.
    pub container: String,
    /// MD5 of the bytes as served.
    pub md5_compressed: String,
    /// MD5 of the bytes after zstd. On an unmeasured row (`--no-chunk-blobs`)
    /// this is the manifest's declared `uncompressed_md5`, not a measurement.
    pub md5_decompressed: String,
    /// SHA-256 of the bytes as served, so a reader can check with a stock tool.
    pub sha256_compressed: String,
    /// `len(decoded)`. On an unmeasured row this is the manifest's declared
    /// `uncompressed_size`, not a measurement.
    pub uncompressed_size: u64,
    /// The manifest's opaque field 6, carried through a diff that never reads it.
    pub unknown_field6: i64,
    /// The four A6 checks, or `null` when the blob was not measured.
    pub verified: Option<Verified>,
    /// Why this blob has no usable bytes, when it has none.
    ///
    /// Separate from `verified` because it is a *different kind* of finding:
    /// the four checks are "is this the chunk the manifest named", while this is
    /// "these bytes are not a complete frame". A truncated download fails two of
    /// the four as a side effect, and without this field a reader would go
    /// looking for a wrong-content problem they do not have.
    ///
    /// **On a row with no `stored_path` it carries the fetch failure itself** —
    /// the source that refused, the write that failed, or the task that panicked.
    /// The name says "decode", but the question a reader (and A6) asks of this
    /// field is "why is this blob unusable", and a fetch that produced no body is
    /// the extreme case of that. Version 3 adds [`BlobProps::fetch_error`] for
    /// the structured half of that story; this field stays the decode-specific
    /// one and is `null` when the bytes arrived intact but would not decode.
    pub decode_error: Option<String>,
    /// The structured fetch failure, when a fetch was attempted and did not
    /// produce a complete body. `null` on a measured row and on a
    /// `--no-chunk-blobs` row, where nothing was asked for.
    pub fetch_error: Option<FetchError>,
    /// Whether these bytes are only the prefix of a failed fetch.
    ///
    /// **A partial body is evidence and must never look usable.** The row names
    /// the bytes that arrived, their digests and why the fetch stopped; A6 and
    /// [`BlobProps::is_usable`] reject it.
    pub partial: bool,
    /// Why the post-rename measurement did not complete.
    ///
    /// Separate from `decode_error`: a decode failure is a measured fact about
    /// bytes that are all there, while this is an I/O error, panic or
    /// cancellation in the measurement task. The blob is kept either way;
    /// `stored_path` stays set so a reader can tell this apart from "no bytes".
    pub measurement_error: Option<String>,
}

impl BlobProps {
    /// The artifact-relative path a blob of this id is stored at.
    ///
    /// One function, because the writer and the index must agree on it and a
    /// reader has to be able to predict it from the id alone.
    pub fn stored_path_for(chunk_id: &str) -> String {
        format!("chunks/blobs/{chunk_id}.zst")
    }

    /// Measure a fetched blob against the manifest's chunk.
    ///
    /// Never fails. A blob that arrived truncated, or with the wrong content,
    /// or not at all, is exactly the thing this capture exists to record, and a
    /// function that returned `Err` would be a caller with a choice between
    /// dropping the evidence or ignoring the error — and both lose it.
    ///
    /// The one thing that *is* an error is a blob with no zstd frame at all:
    /// the build declared `compression: 1`, so unframed bytes are a protocol
    /// violation rather than a digest mismatch, and there is nothing here to
    /// measure.
    pub fn measure(raw: &[u8], chunk: &SophonChunk) -> Result<BlobProps, ApiError> {
        let what = format!("chunk {}", chunk.id);
        if !hash::is_zstd(raw) {
            return Err(Self::not_a_frame(&what, raw));
        }
        let compressed = hash::md5_sha256(raw);
        // A frame that will not decode leaves the two decode-dependent checks
        // false and records why, rather than taking the whole blob down with it.
        //
        // Streamed, not decompressed: the two numbers this needs are the
        // decoded length and the decoded md5, and the largest measured chunk is
        // 3 689 476 B — so a `Vec<u8>` here is ~30 MB of peak RSS at `--jobs 8`,
        // alive at the same moment as the eight raw bodies, to compute two values
        // and throw the rest away. `zstd_measure` is the same decode with a
        // counter and a hasher where the buffer was.
        //
        // And the compressed digests are one pass, not two: `md5_compressed` and
        // `sha256_compressed` cover the same buffer and the crate records both.
        let measured = hash::zstd_measure(raw, &what);
        Ok(Self::assemble(compressed, measured, chunk))
    }

    /// [`BlobProps::measure`] for a blob already on disk, with its compressed
    /// digests already computed.
    ///
    /// The capture path streams a chunk to a file and digests it on the way past,
    /// so by the time there is anything to measure the bytes have never been in
    /// memory. `compressed` is what the *write* computed, which is the same number
    /// [`BlobProps::measure`] would have computed over the same bytes —
    /// `a_hashable_file_reports_the_digests_of_the_bytes_it_stored` is what keeps
    /// that true rather than merely intended.
    ///
    /// Only the decode reads the file, and it reads it through a 16 KB window.
    /// So the peak cost of measuring a blob is the cost of the network chunk that
    /// produced it.
    ///
    /// The one refusal `measure` makes — "not a zstd frame" — is **recorded**
    /// here rather than raised, because this is the path a fetched body takes.
    /// The bytes are already on disk and the compressed digests were computed by
    /// the write, so a mis-served response becomes a measured row with
    /// `decode_error` and the two decode-dependent checks false. Everything else
    /// that can go wrong is a decode failure (recorded by [`BlobProps::assemble`])
    /// or an I/O error that still has to be reported.
    pub fn measure_file(
        path: &std::path::Path,
        chunk: &SophonChunk,
        compressed: hash::Digests,
    ) -> Result<BlobProps, ApiError> {
        let what = format!("chunk {}", chunk.id);
        let file = std::fs::File::open(path).map_err(|e| {
            ApiError::shape(
                "chunk",
                format!("{what}: reopening what was just written: {e}"),
            )
        })?;
        let mut reader = std::io::BufReader::new(file);
        let mut head = [0_u8; 4];
        use std::io::Read as _;
        let n = reader
            .read(&mut head)
            .map_err(|e| ApiError::shape("chunk", format!("{what}: reading the magic: {e}")))?;
        if !hash::is_zstd(&head[..n]) {
            // **A fetched body, not a failed fetch.** The rename has already
            // published the bytes; refusing here would leave the file on disk
            // while the index said "not stored". The row carries the digests the
            // write computed and the real reason.
            let why = Self::not_a_frame(&what, &head[..n]).to_string();
            return Ok(Self::undecodable_from_digests(compressed, chunk, why));
        }
        // Rewind rather than chain the head back: `zstd_measure_from` reads its
        // own four bytes to sniff with, so the file must start at zero.
        use std::io::Seek as _;
        reader
            .rewind()
            .map_err(|e| ApiError::shape("chunk", format!("{what}: rewinding to decode: {e}")))?;
        let measured = hash::zstd_measure_from(reader, &what);
        Ok(Self::assemble(compressed, measured, chunk))
    }

    /// The one thing that is an error rather than a recorded decode failure.
    ///
    /// The build declared `compression: 1`, so unframed bytes are a protocol
    /// violation rather than a digest mismatch, and there is nothing to measure.
    /// One function so the buffered and file paths say the *same* thing — a blob
    /// that is not a frame gets one error text whichever way it arrived.
    fn not_a_frame(what: &str, seen: &[u8]) -> ApiError {
        ApiError::shape(
            "chunk",
            format!(
                "{what}: not a zstd frame ({} bytes, magic {:02x?}); the build declared \
                 compression=1",
                seen.len(),
                &seen[..seen.len().min(4)],
            ),
        )
    }

    /// The entry for a blob whose bytes arrived but are not a zstd frame.
    ///
    /// **This is the evidence, not a failure.** [`BlobProps::measure`] refuses a
    /// body that is not a zstd frame, and a refusal must not become a hole: the
    /// bytes are already on disk, so they are kept and the row says why they will
    /// not decode. A truncated or mis-served response is precisely the thing this
    /// tool exists to keep, and this module's stated purpose says so.
    ///
    /// The row carries **more** than a discarded one would. The compressed
    /// digests are computed because hashing the raw bytes does not need zstd, so
    /// a reader gets the exact bytes, their digests, and the reason they will
    /// not decode — rather than a hole and a sentence saying nothing arrived.
    ///
    /// `verified` keeps the four A6 checks: the two that need no decode are
    /// decided from the compressed bytes, and the two decode-dependent keys are
    /// `false` because the decode was attempted and did not produce a file. This
    /// is the spec's row shape (`docs/00-spec.md` §4.2 and
    /// `docs/01-artifact-schema.md`), and the same shape a framed blob whose
    /// decode fails gets from [`BlobProps::assemble`].
    pub fn undecodable(raw: &[u8], chunk: &SophonChunk, why: String) -> BlobProps {
        Self::undecodable_from_digests(hash::md5_sha256(raw), chunk, why)
    }

    /// [`BlobProps::undecodable`] for a body already written and digested.
    ///
    /// The capture path computes `compressed` while streaming to disk, so it must
    /// not hash the file again just to build this row.
    fn undecodable_from_digests(
        compressed: hash::Digests,
        chunk: &SophonChunk,
        why: String,
    ) -> BlobProps {
        BlobProps {
            chunk_id: chunk.id.clone(),
            stored_path: None,
            bytes: compressed.bytes,
            container: "zstd".to_string(),
            sha256_compressed: compressed.sha256,
            md5_compressed: compressed.md5.clone(),
            // The declared digest, not a computed one: the body cannot be
            // decoded, so there is no decompressed body to compare against.
            md5_decompressed: chunk.uncompressed_md5.clone(),
            uncompressed_size: chunk.uncompressed_size.max(0) as u64,
            unknown_field6: chunk.unknown_field6,
            verified: Some(Verified {
                raw_size_matches: compressed.bytes as i64 == chunk.compressed_size,
                md5_compressed_matches: compressed.md5 == chunk.compressed_md5,
                // The two decode-dependent checks were asked and could not be
                // satisfied: the bytes are not a frame, so there is no decoded
                // body to compare. `false`, not `null` — the spec's row shape.
                md5_decompressed_matches: false,
                decoded_size_matches: false,
            }),
            decode_error: Some(why),
            fetch_error: None,
            partial: false,
            measurement_error: None,
        }
    }

    /// The row, from the two things that were measured.
    ///
    /// **The tail shared by [`BlobProps::measure`] and [`BlobProps::measure_file`]**,
    /// and the reason they cannot disagree about a field: a decode that failed leaves
    /// the two decode-dependent checks false and records why, rather than taking the
    /// whole blob down with it.
    ///
    /// `measured` being `Err` is a *recorded* state, not a propagated one, so the
    /// compressed digests and `bytes` still land in the row — they were computed over
    /// bytes that exist on disk.
    fn assemble(
        compressed: hash::Digests,
        measured: Result<hash::Measure, String>,
        chunk: &SophonChunk,
    ) -> BlobProps {
        let decoded_size = measured.as_ref().ok().map(|m| m.len);
        let md5_decompressed = measured.as_ref().ok().map(|m| m.md5.clone());
        BlobProps {
            chunk_id: chunk.id.clone(),
            stored_path: None,
            bytes: compressed.bytes,
            container: "zstd".to_string(),
            sha256_compressed: compressed.sha256,
            md5_compressed: compressed.md5.clone(),
            md5_decompressed: md5_decompressed.clone().unwrap_or_default(),
            uncompressed_size: decoded_size.unwrap_or(0),
            unknown_field6: chunk.unknown_field6,
            decode_error: measured.as_ref().err().cloned(),
            fetch_error: None,
            partial: false,
            measurement_error: None,
            verified: Some(Verified {
                raw_size_matches: compressed.bytes as i64 == chunk.compressed_size,
                md5_compressed_matches: compressed.md5 == chunk.compressed_md5,
                md5_decompressed_matches: md5_decompressed.as_deref()
                    == Some(&chunk.uncompressed_md5),
                decoded_size_matches: matches!(decoded_size, Some(n) if n as i64 == chunk.uncompressed_size),
            }),
        }
    }

    /// The entry for a blob that was never fetched.
    ///
    /// `--no-chunk-blobs` produces these. Nothing was measured, so `bytes` is
    /// zero, the two **compressed** digests are empty strings rather than zeros
    /// — a zeroed digest is a claim and an absent one is not — and `verified`
    /// and `decode_error` are null. `md5_decompressed` and `uncompressed_size`
    /// do carry values, but they are the manifest's **declared** digest and
    /// length, not a measurement: they say which bytes are missing. M9's
    /// `verify` reports `verified: null` as "not measured", which is the honest
    /// reading and is why the spec says that flag breaks A7–A10 by design.
    pub fn unmeasured(chunk: &SophonChunk) -> BlobProps {
        BlobProps {
            chunk_id: chunk.id.clone(),
            stored_path: None,
            bytes: 0,
            container: "zstd".to_string(),
            md5_compressed: String::new(),
            md5_decompressed: chunk.uncompressed_md5.clone(),
            sha256_compressed: String::new(),
            uncompressed_size: chunk.uncompressed_size.max(0) as u64,
            unknown_field6: chunk.unknown_field6,
            verified: None,
            decode_error: None,
            fetch_error: None,
            partial: false,
            measurement_error: None,
        }
    }

    /// A row for bytes that are on disk but whose measurement never returned.
    ///
    /// The compressed digests and byte count came from the write, so they are
    /// real; `verified` is `None` because nothing measured the decoded side.
    /// `stored_path` is left for the caller to attach, and
    /// [`BlobProps::is_usable`] rejects the row until that and a successful
    /// measurement are both present.
    pub fn measurement_failed(
        compressed: hash::Digests,
        chunk: &SophonChunk,
        why: impl Into<String>,
    ) -> BlobProps {
        BlobProps {
            chunk_id: chunk.id.clone(),
            stored_path: None,
            bytes: compressed.bytes,
            container: "zstd".to_string(),
            sha256_compressed: compressed.sha256,
            md5_compressed: compressed.md5,
            md5_decompressed: chunk.uncompressed_md5.clone(),
            uncompressed_size: chunk.uncompressed_size.max(0) as u64,
            unknown_field6: chunk.unknown_field6,
            verified: None,
            decode_error: None,
            fetch_error: None,
            partial: false,
            measurement_error: Some(why.into()),
        }
    }

    /// Attach the fetch failure and mark a published prefix as partial.
    pub fn mark_partial(&mut self, error: FetchError) {
        self.partial = true;
        self.fetch_error = Some(error);
    }

    /// The A6 assertions this blob contributes, named for the selfcheck.
    ///
    /// One entry per failing check rather than one "A6 failed" row, because a
    /// selfcheck with 71 rows for 71 chunks has to be readable per chunk and a
    /// blob failing three of four is three findings.
    pub fn failures(&self) -> Vec<&'static str> {
        self.verified.map(|v| v.failures()).unwrap_or_default()
    }

    /// Whether this blob contributes any A6 row.
    ///
    /// See [`Verified::any_failure`]: this exists so the call site inside
    /// `Store::index`'s `filter` does not build a `Vec` per blob to ask a
    /// yes/no question.
    pub fn any_failure(&self) -> bool {
        self.verified.is_some_and(|v| v.any_failure())
    }

    /// Whether this blob can be assembled from.
    ///
    /// `verified: None` is *not* green, and neither is a blob that would not
    /// decode, a fetch that never completed, a partial body, or one whose
    /// post-rename measurement failed. In particular `stored_path` alone is not
    /// enough: version 3 keeps the bytes of a failed measurement, so a reader
    /// must ask this question rather than whether a path is present.
    pub fn is_usable(&self) -> bool {
        self.verified.is_some_and(|v| v.all_green())
            && self.decode_error.is_none()
            && self.fetch_error.is_none()
            && !self.partial
            && self.measurement_error.is_none()
            && self.stored_path.is_some()
    }

    /// Why this blob is unusable, in one line, for the run log.
    pub fn why_unusable(&self) -> Option<String> {
        if self.is_usable() {
            return None;
        }
        Some(
            match (
                &self.measurement_error,
                &self.fetch_error,
                &self.decode_error,
                self.verified,
            ) {
                (Some(e), _, _, _) => format!("{}: measurement failed: {e}", self.chunk_id),
                (_, Some(e), _, _) => format!("{}: fetch failed: {}", self.chunk_id, e.detail),
                (_, _, Some(e), _) => format!("{}: {e}", self.chunk_id),
                (_, _, _, None) => format!("{}: never measured", self.chunk_id),
                (_, _, _, Some(v)) => format!("{}: {v}", self.chunk_id),
            },
        )
    }

    /// The path, or a message naming the chunk.
    pub fn require_path(&self) -> Result<&str, String> {
        self.stored_path
            .as_deref()
            .ok_or_else(|| format!("chunk {} has no stored blob", self.chunk_id))
    }

    /// The recorded digest of the decoded bytes.
    pub fn decoded_md5(&self) -> Result<&str, String> {
        if self.verified.is_none() {
            return Err(format!("chunk {} was never measured", self.chunk_id));
        }
        Ok(&self.md5_decompressed)
    }

    /// Attach the path it was written to.
    pub fn set_stored(&mut self, path: impl Into<String>) {
        self.stored_path = Some(path.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body that is not a zstd frame, with the chunk that declares it.
    ///
    /// **The shape the CDN actually serves when a blob request goes wrong**: an
    /// XML `NoSuchKey` document, byte for byte the body `api/error_shapes.json`
    /// records for its own negative probes. My first attempt truncated a real
    /// frame, which does *not* work — `is_zstd` checks four magic bytes, so a
    /// truncated frame is still framed. The test that noticed that is
    /// `a_body_that_is_not_a_zstd_frame_is_refused_by_measure`'s own guard.
    fn mis_served_chunk() -> (SophonChunk, Vec<u8>) {
        let payload = b"the quick brown fox jumps over the lazy dog".repeat(64);
        let (chunk, _) = chunk_with(&payload);
        let xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error>\n  \
                   <Code>NoSuchKey</Code>\n  <Message>The specified key does not \
                   exist.</Message>\n</Error>\n";
        (chunk, xml.to_vec())
    }

    #[test]
    fn a_body_that_is_not_a_zstd_frame_is_refused_by_measure() {
        // `measure` must refuse an unframed body — the caller decides what to do
        // with the refusal, and the caller's decision is the other test.
        let (chunk, cut) = mis_served_chunk();
        assert!(
            !hash::is_zstd(&cut),
            "the fixture must actually be unframed, or this tests nothing"
        );
        assert!(BlobProps::measure(&cut, &chunk).is_err());
    }

    #[test]
    fn an_undecodable_body_keeps_its_bytes_digests_and_reason() {
        // The bytes are evidence, so they are kept: a body like this propagates
        // out of `measure` as an `Err`, and the row must carry the bytes, their
        // digests and the real reason rather than a "the fetch failed" story.
        let (chunk, cut) = mis_served_chunk();
        let why = BlobProps::measure(&cut, &chunk).expect_err("unframed");
        let p = BlobProps::undecodable(&cut, &chunk, why.to_string());

        assert_eq!(
            p.bytes,
            cut.len() as u64,
            "the row records how many bytes arrived, not zero"
        );
        assert_eq!(
            p.md5_compressed,
            hash::md5_hex(&cut),
            "the compressed digest is computable without zstd, so it is recorded"
        );
        assert_eq!(p.sha256_compressed, hash::sha256_hex(&cut));
        assert_eq!(
            p.md5_decompressed, chunk.uncompressed_md5,
            "and the declared digest is kept as declared - there is nothing to compare it against"
        );
        assert!(
            p.decode_error
                .as_deref()
                .unwrap_or_default()
                .contains("not a zstd frame"),
            "the row says why it will not decode: {:?}",
            p.decode_error
        );
        let v = p
            .verified
            .expect("the bytes were measured, so the row is not 'not measured'");
        assert!(
            !v.md5_decompressed_matches && !v.decoded_size_matches,
            "the decode was attempted and could not be satisfied, so both \
             decode-dependent checks are `false`, not `null`"
        );
    }

    #[test]
    fn an_undecodable_body_is_distinguishable_from_one_that_never_arrived() {
        // The two rows a reader must be able to tell apart: a blob whose bytes
        // are on disk and will not decode, and a blob with no bytes at all.
        let (chunk, cut) = mis_served_chunk();
        let kept = BlobProps::undecodable(&cut, &chunk, "not a zstd frame".to_string());
        let never = BlobProps::unmeasured(&chunk);
        assert_ne!(kept.bytes, never.bytes);
        assert!(kept.md5_compressed.len() == 32);
        assert!(
            never.md5_compressed.is_empty(),
            "an absent digest is empty, not zeroed - a zeroed one is a claim"
        );
        assert!(never.decode_error.is_none());
    }

    /// A chunk and the exact bytes that should satisfy it.
    ///
    /// Built by compressing real content rather than by hand-writing digests,
    /// so a test cannot pass because both sides of the assertion were written
    /// from the same wrong constant.
    fn chunk_with(payload: &[u8]) -> (SophonChunk, Vec<u8>) {
        let raw = zstd::encode_all(payload, 3).expect("encode");
        let c = SophonChunk {
            id: format!(
                "{:016x}_{}",
                0xabcd_ef01_2345_6789u64,
                hash::md5_hex(payload)
            ),
            uncompressed_md5: hash::md5_hex(payload),
            offset: 0,
            compressed_size: raw.len() as i64,
            uncompressed_size: payload.len() as i64,
            unknown_field6: -4_117_343_013_703_778_304,
            compressed_md5: hash::md5_hex(&raw),
        };
        (c, raw)
    }

    fn payload(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn a_blob_that_agrees_with_its_chunk_is_green_on_all_four() {
        let (c, raw) = chunk_with(&payload(4096));
        let p = BlobProps::measure(&raw, &c).expect("a zstd frame");
        let v = p.verified.expect("measured");
        assert!(v.all_green(), "{}", v);
        assert_eq!(v.failures(), Vec::<&str>::new());
        assert_eq!(v.to_string(), "all four A6 checks hold");
        assert_eq!(p.bytes, raw.len() as u64);
        assert_eq!(p.uncompressed_size, 4096);
        assert_eq!(p.container, "zstd");
        assert_eq!(p.md5_decompressed, c.uncompressed_md5);
        assert_eq!(
            p.unknown_field6, c.unknown_field6,
            "carried through verbatim"
        );
        assert_eq!(p.chunk_id, c.id);
    }

    /// A truncated download is *recorded*, not thrown away — it is the whole
    /// reason `measure` is infallible — and the reason it failed is recorded as
    /// "not a complete frame" rather than left to be inferred from two false
    /// digest checks.
    #[test]
    fn a_truncated_blob_is_recorded_with_the_decode_failure_attached() {
        let (c, raw) = chunk_with(&payload(4096));
        let cut = &raw[..raw.len() - 8];
        let p = BlobProps::measure(cut, &c).expect("it still has the zstd magic");
        let v = p.verified.expect("measured");
        assert!(!v.all_green());
        assert!(
            !v.raw_size_matches,
            "the manifest said {}",
            c.compressed_size
        );
        assert!(!v.md5_compressed_matches, "different bytes");
        assert!(!v.md5_decompressed_matches);
        assert!(!v.decoded_size_matches);
        let why = p.decode_error.as_deref().expect("and it says why");
        assert!(why.contains("incomplete frame"), "{why}");
        assert!(why.contains(&c.id), "names the chunk: {why}");
        assert!(!p.is_usable());
        assert!(p.why_unusable().unwrap().contains("incomplete frame"));
    }

    #[test]
    fn a_green_blob_has_no_decode_error_and_says_so() {
        let (c, raw) = chunk_with(&payload(4096));
        let mut p = BlobProps::measure(&raw, &c).unwrap();
        assert!(p.decode_error.is_none());
        // Measured green but not yet written is still not assemblable, and
        // `why_unusable` says which of the two it is.
        assert!(p.verified.unwrap().all_green());
        assert!(!p.is_usable());
        p.set_stored(BlobProps::stored_path_for(&c.id));
        assert!(p.is_usable());
        assert!(p.why_unusable().is_none());
    }

    /// Each of the four checks fails on its own when exactly its own input is
    /// wrong. A test that only ever produced a "several things are wrong" blob
    /// would pass with any two of the four swapped.
    #[test]
    fn each_a6_check_can_fail_alone() {
        let (c, raw) = chunk_with(&payload(4096));

        // 1. raw length: a different declared size, same bytes.
        let mut wrong_len = c.clone();
        wrong_len.compressed_size += 1;
        let v = BlobProps::measure(&raw, &wrong_len)
            .unwrap()
            .verified
            .unwrap();
        assert_eq!(v.failures(), vec!["len(raw blob) == compressed_size"]);

        // 2. compressed md5.
        let mut wrong_c = c.clone();
        wrong_c.compressed_md5 = "0".repeat(32);
        let v = BlobProps::measure(&raw, &wrong_c)
            .unwrap()
            .verified
            .unwrap();
        assert_eq!(v.failures(), vec!["md5(raw blob) == compressed_md5"]);

        // 3. decompressed md5, via the manifest's `uncompressed_md5`.
        let mut wrong_u = c.clone();
        wrong_u.uncompressed_md5 = "0".repeat(32);
        let v = BlobProps::measure(&raw, &wrong_u)
            .unwrap()
            .verified
            .unwrap();
        assert_eq!(
            v.failures(),
            vec!["md5(zstd(raw blob)) == uncompressed_md5"]
        );

        // 4. decoded length.
        let mut wrong_d = c.clone();
        wrong_d.uncompressed_size += 1;
        let v = BlobProps::measure(&raw, &wrong_d)
            .unwrap()
            .verified
            .unwrap();
        assert_eq!(v.failures(), vec!["len(decoded) == uncompressed_size"]);
    }

    #[test]
    fn several_failures_are_all_named_and_the_message_reads_as_a_sentence() {
        let mut c = chunk_with(&payload(64)).0;
        c.uncompressed_md5 = "0".repeat(32);
        c.compressed_md5 = "0".repeat(32);
        c.compressed_size += 1;
        c.uncompressed_size += 1;
        let (_, raw) = chunk_with(&payload(64));
        let v = BlobProps::measure(&raw, &c).unwrap().verified.unwrap();
        assert_eq!(v.failures().len(), 4);
        let s = v.to_string();
        assert!(s.starts_with("4 of the four"), "{s}");
        assert!(
            s.contains("compressed_md5") && s.contains("uncompressed_size"),
            "{s}"
        );
    }

    /// An unframed blob is a protocol violation, not a digest mismatch, and the
    /// message has to say so — a reader seeing four `false` would go looking for
    /// a corrupted download rather than for the wrong `compression` setting.
    #[test]
    fn bytes_that_are_not_zstd_are_an_error_not_a_measurement() {
        let (c, _) = chunk_with(&payload(64));
        let e = BlobProps::measure(b"plain bytes, sorry", &c).unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("not a zstd frame"), "{msg}");
        assert!(msg.contains("compression=1"), "{msg}");
        assert!(msg.contains(&c.id), "names the chunk: {msg}");
    }

    /// An unmeasured blob is not a green blob. `--no-chunk-blobs` produces a
    /// bundle that cannot be assembled from, and that has to be visible rather
    /// than defaulting to "no failures found".
    #[test]
    fn an_unmeasured_blob_is_not_usable_and_says_so() {
        let (c, _) = chunk_with(&payload(64));
        let p = BlobProps::unmeasured(&c);
        assert!(p.verified.is_none());
        assert!(
            !p.is_usable(),
            "nothing was measured, so nothing can be assembled"
        );
        assert!(p.failures().is_empty(), "no failures, but also no evidence");
        assert!(p.why_unusable().unwrap().contains("never measured"));
        assert!(p.require_path().unwrap_err().contains(&c.id));
        assert!(p.decoded_md5().unwrap_err().contains("never measured"));
        // The manifest's identity survives; the digests do not pretend to.
        assert_eq!(p.md5_decompressed, c.uncompressed_md5);
        assert_eq!(p.md5_compressed, "");
        assert_eq!(
            p.uncompressed_size, 64,
            "declared, so a reader knows the gap"
        );
    }

    #[test]
    fn the_stored_path_is_predictable_from_the_chunk_id_alone() {
        assert_eq!(
            BlobProps::stored_path_for("f5dc3ca1f3a0ba3d_291a7719a4a23664d37825a41fad3c5b"),
            "chunks/blobs/f5dc3ca1f3a0ba3d_291a7719a4a23664d37825a41fad3c5b.zst"
        );
        // The *file name* part is flat: an id is hex plus one underscore, so a
        // chunk id can never introduce a path separator or a `..`.
        let id = "616e6fbbf0a97dc9_a21a4420161bca81a78f73b27bfa359a";
        let name = BlobProps::stored_path_for(id)
            .rsplit('/')
            .next()
            .expect("a file name")
            .to_string();
        assert_eq!(name, format!("{id}.zst"));
        assert!(
            !name.contains('/') && !name.contains('\\') && !name.contains(".."),
            "a chunk id can never escape the bundle: {name}"
        );
    }

    #[test]
    fn the_serialised_shape_is_the_schema_doc_s_field_names() {
        let (c, raw) = chunk_with(&payload(128));
        let mut p = BlobProps::measure(&raw, &c).expect("a frame");
        p.set_stored(BlobProps::stored_path_for(&c.id));
        let v = serde_json::to_value(&p).expect("serialise");
        let mut keys: Vec<&str> = v
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "bytes",
                "chunk_id",
                "container",
                "decode_error",
                "fetch_error",
                "md5_compressed",
                "md5_decompressed",
                "measurement_error",
                "partial",
                "sha256_compressed",
                "stored_path",
                "uncompressed_size",
                "unknown_field6",
                "verified",
            ],
            "docs/01-artifact-schema.md §12: the four checks, the decode reason, \
             and version 3's fetch/partial/measurement evidence"
        );
        assert_eq!(
            v["verified"]
                .as_object()
                .expect("verified")
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>(),
            vec![
                "decoded_size_matches",
                "md5_compressed_matches",
                "md5_decompressed_matches",
                "raw_size_matches"
            ],
            "four checks, named as A6 names them"
        );
        // And `seq` is *not* here: provenance lives in api/requests.ndjson, so
        // the index cannot drift out of step with it.
        assert!(v.get("seq").is_none());
        assert!(
            v["decode_error"].is_null(),
            "a green blob serialises it away"
        );
        assert!(v["fetch_error"].is_null(), "nothing failed to fetch");
        assert_eq!(v["partial"], serde_json::json!(false));
        assert!(v["measurement_error"].is_null(), "the measurement returned");
    }
}
