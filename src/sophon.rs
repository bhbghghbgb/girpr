use anyhow::Result;
use futures::StreamExt;
use prost::Message;
use std::collections::{HashMap, HashSet};
use std::io::Write;

use crate::hyp::{ChunkBuild, ChunkManifestMeta};

/// Sophon chunk manifest protobuf (Starward `Sophon.proto` parity).
/// NOTE(starward-parity): the repeated field is misspelled `chuncks` upstream;
/// kept verbatim so diffs against the reference stay obvious. Do not "fix" the
/// spelling.
/// See <https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/Sophon.proto#L8-L30>
/// message SophonChunkManifest { repeated SophonChunkFile chuncks = 1; }
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SophonChunkManifest {
    #[prost(message, repeated, tag = "1")]
    pub chuncks: Vec<SophonChunkFile>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SophonChunkFile {
    #[prost(string, tag = "1")]
    pub file: String,
    #[prost(message, repeated, tag = "2")]
    pub chunks: Vec<SophonChunk>,
    #[prost(bool, tag = "3")]
    pub is_folder: bool,
    #[prost(int64, tag = "4")]
    pub size: i64,
    #[prost(string, tag = "5")]
    pub md5: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SophonChunk {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(string, tag = "2")]
    pub uncompressed_md5: String,
    #[prost(int64, tag = "3")]
    pub offset: i64,
    #[prost(int64, tag = "4")]
    pub compressed_size: i64,
    #[prost(int64, tag = "5")]
    pub uncompressed_size: i64,
    // NOTE(starward-parity): opaque upstream field, preserved for decode compat.
    // See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/Sophon.proto#L22-L30
    #[prost(int64, tag = "6")]
    pub unknown: i64,
    #[prost(string, tag = "7")]
    pub compressed_md5: String,
}

#[derive(Debug, Clone)]
pub struct WantedManifest {
    pub meta: ChunkManifestMeta,
    pub files: Vec<SophonChunkFile>,
}

/// Filter manifests: drop res_category ignores + non-selected audio fields (Starward parity).
/// NOTE(starward-parity): the `len is 5 or 10 && contains('-')` audio heuristic
/// (`zh-cn` / `mini-zh-cn`) and the audio re-add loop that bypasses the ignore
/// set are both verbatim ports of `GetAvailableGameSophonChunkManifests` — the
/// heuristic is fragile upstream, but diverging here would break parity.
/// Do not "fix" without checking upstream first.
/// See <https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GamePackageService.cs#L742-L769>
/// Audio fields look like `zh-cn` (len 5 with '-') or `mini-zh-cn` (len 10 with '-').
pub fn select_manifests<'a>(
    build: &'a ChunkBuild,
    audio_langs: &HashSet<String>,
    ignore: &HashSet<String>,
) -> Vec<&'a ChunkManifestMeta> {
    let mut out: Vec<&ChunkManifestMeta> = Vec::new();
    for m in &build.manifests {
        if ignore.contains(&m.matching_field) || ignore.contains(&m.category_id) {
            continue;
        }
        let f = m.matching_field.as_str();
        if (f.len() == 5 || f.len() == 10) && f.contains('-') {
            continue; // audio/mini -> re-add only if selected
        }
        out.push(m);
    }
    for m in &build.manifests {
        if audio_langs.contains(&m.matching_field) {
            out.push(m);
        }
    }
    out
}

/// Download + zstd-decompress + MD5-verify + protobuf-parse one manifest.
/// NOTE(starward-parity): the checksum is the MD5 of the *decompressed* bytes,
/// verified case-insensitively before parsing, like `EnsureSophonManifestFileAsync`.
/// See <https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GamePackageService.cs#L822-L864>
///
/// The blob is **streamed**: the compressed body never sits in memory whole,
/// and the decoded bytes land in a buffer the API's reported
/// `uncompressed_size` **sizes**, so the decode does not have to grow it.
///
/// Manifests are still never cached inside `game_dir` (V1-SIMPLIFICATION S1,
/// see README). Starward caches the *compressed* manifest under the app
/// cache; `girpr` does not — lower disk use, and the manifest is fetched once
/// per run.
pub async fn fetch_manifest(
    client: &reqwest::Client,
    meta: &ChunkManifestMeta,
) -> Result<Vec<SophonChunkFile>> {
    let url = join_url(&meta.manifest_download.url_prefix, &meta.manifest.id);
    // The reported size *sizes* the buffer; it does not promise the length,
    // and it is not trusted enough to allocate on. A blob bigger than the
    // clamp grows the buffer; a bogus number cannot ask for an impossible
    // allocation.
    let hint = manifest_size_hint(meta.manifest.uncompressed_size);
    let mut decoded: Vec<u8> = Vec::with_capacity(hint);
    let mut last_err = anyhow::anyhow!("no attempts");
    for attempt in 1..=ATTEMPTS {
        match try_fetch_manifest(client, &url, meta, attempt, &mut decoded).await {
            Attempt::Ok(v) => return Ok(v),
            Attempt::Fatal(e) => {
                tracing::debug!(
                    manifest = %meta.manifest.id,
                    attempt,
                    "manifest fetch failed, not retryable: {e:#}"
                );
                return Err(e);
            }
            Attempt::Retry(e) => {
                tracing::debug!(manifest = %meta.manifest.id, attempt, "manifest fetch failed: {e:#}");
                last_err = e;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(attempt)).await;
    }
    Err(last_err)
}

/// The largest allocation the manifest's reported size is allowed to ask for.
///
/// The `uncompressed_size` in the API's `getBuild` body is a raw number, and
/// it is a *hint*, not a promise: the decode grows the buffer if the blob is
/// bigger, and a blob that decodes to nothing is fine. So the only thing the
/// number must not do is ask for an impossible allocation — `Vec::with_capacity`
/// above `isize::MAX` panics, and a merely-large value is an OOM abort on a
/// field nobody has checked. 64 MiB is well above the real manifest (16.8 MB
/// for the `game` manifest of build 7.1.0) and well below anything that would
/// hurt.
const MAX_MANIFEST_HINT: usize = 64 << 20;

/// The capacity to give the manifest's decoded buffer, from the size the API
/// reported.
///
/// Clamped, and never negative: a wrong number costs a reallocation, not a
/// crash.
fn manifest_size_hint(reported: i64) -> usize {
    reported.clamp(0, MAX_MANIFEST_HINT as i64) as usize
}

async fn try_fetch_manifest(
    client: &reqwest::Client,
    url: &str,
    meta: &ChunkManifestMeta,
    attempt: u64,
    decoded: &mut Vec<u8>,
) -> Attempt<Vec<SophonChunkFile>> {
    tracing::debug!(manifest = %meta.manifest.id, field = %meta.matching_field, url = %url, "GET manifest");
    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => {
            return Attempt::Retry(anyhow::anyhow!("GET manifest {}: {}", meta.manifest.id, e));
        }
    };
    let status = resp.status();
    if let Some(e) = status_failure(status, url) {
        return verdict(e, retryable(status.as_u16()));
    }
    // A retry starts the manifest over: the decoded bytes of the failed
    // attempt are in the buffer, and they are about to be replaced.
    if let Err(e) = decoded.restart() {
        return Attempt::Fatal(anyhow::anyhow!("reset the manifest buffer: {e}"));
    }
    // The whole blob is zstd-compressed; the checksum is the MD5 of the
    // DECOMPRESSED bytes. There is no ceiling on the decode: the reported
    // size sizes the buffer, it does not promise the length.
    let digest = match stream_body(resp.bytes_stream(), None, decoded).await {
        Ok(v) => v,
        Err(StreamError::Transport(e)) => return Attempt::Retry(e),
        Err(StreamError::Content(e)) => return Attempt::Retry(e),
    };
    tracing::debug!(manifest = %meta.manifest.id, compressed_bytes = digest.compressed_bytes, "manifest downloaded");
    tracing::debug!(manifest = %meta.manifest.id, decompressed_bytes = digest.bytes, expect_md5 = %meta.manifest.checksum, actual_md5 = %digest.md5, "manifest checksum check");
    // A checksum mismatch is worth another try: the origin may have served a
    // torn body, and the next attempt is a fresh one.
    if !digest.md5.eq_ignore_ascii_case(&meta.manifest.checksum) {
        return Attempt::Retry(anyhow::anyhow!(
            "manifest checksum mismatch id={} expect={} got={}",
            meta.manifest.id,
            meta.manifest.checksum,
            digest.md5
        ));
    }
    match SophonChunkManifest::decode(decoded.as_slice()) {
        Ok(m) => {
            let (files, chunks): (usize, usize) = (
                m.chuncks.len(),
                m.chuncks.iter().map(|f| f.chunks.len()).sum(),
            );
            tracing::debug!(manifest = %meta.manifest.id, field = %meta.matching_field, files, chunks, attempt, "manifest parsed");
            Attempt::Ok(m.chuncks)
        }
        Err(e) => Attempt::Retry(anyhow::Error::new(e).context("protobuf decode manifest")),
    }
}

// ---------------------------------------------------------------------------
// The streaming path
//
// The chunk never exists in memory as a whole: the compressed body is decoded
// as it arrives and the decoded bytes are written into the sink as they are
// produced, digested on the way past. What it costs is one thing — the digest
// of a decoded stream is known only when the stream ends, and the stream ends
// *after* the last byte has been written. The check therefore happens one byte
// too late to gate the write, which is the invariant the streaming path
// changes (see `docs/02` step 6).
// ---------------------------------------------------------------------------

/// What a streamed blob produced: how many bytes came down, how many landed,
/// and the digest of what landed.
///
/// The digest is of the **decompressed** bytes — the quantity a chunk's
/// `uncompressed_md5` and a manifest's `checksum` name — so it is the same
/// number the buffered path computed, and it is compared against the same
/// thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkDigest {
    /// Compressed bytes that came off the wire.
    pub compressed_bytes: u64,
    /// Decompressed bytes that reached the sink.
    pub bytes: u64,
    /// The MD5 of the decompressed bytes, lowercase hex.
    pub md5: String,
}

/// A writer that hashes what passes through it, counts it, and refuses to pass
/// more than a limit.
///
/// This is the sink of the streaming path: the decoded bytes are hashed on the
/// way past into the file, so the chunk is never resident as a whole.
///
/// **The limit is checked as the bytes go**, which is stricter than the
/// buffered path's "compare the lengths after the decode" and not looser: a
/// body that decodes to more than the chunk claims is refused at the byte that
/// crosses the line rather than after the whole thing has been written.
pub struct HashingWriter<W: Write> {
    inner: W,
    hasher: md5::Context,
    written: u64,
    limit: Option<u64>,
}

impl<W: Write> HashingWriter<W> {
    /// A hashing writer over `inner`, with an optional ceiling.
    ///
    /// `limit` is `None` for a blob whose length is not known ahead of the
    /// decode — the manifest's, where the reported size is a capacity hint
    /// rather than a promise.
    pub fn new(inner: W, limit: Option<u64>) -> Self {
        Self {
            inner,
            hasher: md5::Context::new(),
            written: 0,
            limit,
        }
    }

    /// How many bytes have passed through.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// The MD5 of the bytes that passed through, lowercase hex.
    pub fn md5(&self) -> String {
        format!("{:x}", self.hasher.clone().compute())
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // The ceiling is checked *before* the bytes are forwarded: a writer
        // that takes them and complains afterwards has already landed them.
        if let Some(limit) = self.limit
            && self.written.saturating_add(buf.len() as u64) > limit
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("more than the {limit} bytes this blob may hold"),
            ));
        }
        let n = self.inner.write(buf)?;
        // Only the bytes that were really taken are hashed: a short write is
        // the inner writer's business, and the caller retries with the rest.
        self.hasher.consume(&buf[..n]);
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Where a streamed blob's decoded bytes land.
///
/// The trait exists for one requirement: **a retry has to start the blob over.**
/// A streamed body is written as it decodes, so a failed attempt can leave half
/// a chunk in the sink, and the next attempt must not append to it.
/// [`stream_chunk_to`] therefore asks the sink to restart before it tries
/// again.
pub trait ChunkSink: Write {
    /// Throw away what the failed attempt wrote and start the blob again.
    fn restart(&mut self) -> std::io::Result<()>;
}

impl ChunkSink for Vec<u8> {
    fn restart(&mut self) -> std::io::Result<()> {
        // `clear` keeps the capacity, so a retried blob does not pay for the
        // buffer twice.
        self.clear();
        Ok(())
    }
}

impl<T: ChunkSink + ?Sized> ChunkSink for &mut T {
    fn restart(&mut self) -> std::io::Result<()> {
        (**self).restart()
    }
}

/// Why a streamed body could not be delivered.
///
/// The two kinds are the same *kind* of failure — the body arrived and it is
/// not what it should be — but the two callers read them differently, and the
/// difference is deliberate:
///
/// - **a chunk** treats it as [`Attempt::Fatal`]: the origin answered
///   definitively with a body that does not decode, and the file-level retry
///   is what asks for it again. That is what the buffered path did — a bad
///   frame failed the *file* attempt, not the chunk fetch — so the request
///   counts do not change;
/// - **a manifest** treats it as [`Attempt::Retry`]: a torn body is worth
///   another try of the whole fetch, which is what the manifest path has
///   always done with a decode failure.
#[derive(Debug)]
enum StreamError {
    /// The transport failed. Worth another try.
    Transport(anyhow::Error),
    /// The body arrived but is not what it should be.
    Content(anyhow::Error),
}

/// Stream one chunk's compressed body, decode it, and write the decoded bytes
/// into `sink` — digested on the way past.
///
/// The chunk is never resident as a whole: the compressed body is decoded as
/// it arrives and the decoded bytes are written into the sink as they are
/// produced. `expected` is the chunk's `uncompressed_size`, and it is enforced
/// as the bytes go.
///
/// The digest is known when the stream ends, which is one byte too late to
/// gate the write — the one thing the streaming path gives up. The file-level
/// gate (the whole-file MD5) is what decides whether the file is good.
pub async fn stream_chunk_to(
    client: &reqwest::Client,
    url_prefix: &str,
    id: &str,
    expected: u64,
    sink: &mut impl ChunkSink,
) -> Result<ChunkDigest> {
    let url = join_url(url_prefix, id);
    let mut last_err = anyhow::anyhow!("no attempts");
    for attempt in 1..=ATTEMPTS {
        match try_stream_chunk(client, &url, id, attempt, expected, sink).await {
            Attempt::Ok(d) => return Ok(d),
            Attempt::Fatal(e) => {
                tracing::debug!(chunk = %id, attempt, "chunk fetch failed, not retryable: {e:#}");
                return Err(e);
            }
            Attempt::Retry(e) => {
                tracing::debug!(chunk = %id, attempt, "chunk fetch failed: {e:#}");
                last_err = e;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(attempt)).await;
    }
    Err(last_err)
}

/// One attempt at a streamed chunk fetch.
///
/// The retry split is the buffered path's: a transport failure is worth
/// another try, a status that says so is worth another try, and a body that is
/// not what it should be is the origin's definitive answer — the file-level
/// retry is what asks for it again.
async fn try_stream_chunk(
    client: &reqwest::Client,
    url: &str,
    id: &str,
    attempt: u64,
    expected: u64,
    sink: &mut impl ChunkSink,
) -> Attempt<ChunkDigest> {
    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => return Attempt::Retry(anyhow::anyhow!("GET chunk: {}", e)),
    };
    let status = resp.status();
    if let Some(e) = status_failure(status, url) {
        return verdict(e, retryable(status.as_u16()));
    }
    // NOTE: the bridge parses this line to work out what was *downloaded*
    // rather than reused, so it has to say which chunk it was about.
    //
    // **It is emitted as soon as the origin has answered with the chunk**, not
    // when the body has been decoded: the marker means "this chunk was
    // fetched", and a body that turns out to be corrupt was still fetched.
    // The compressed byte count is on the `chunk decoded` line, which is
    // emitted when the stream ends.
    tracing::trace!(chunk = %id, attempt, "chunk downloaded");
    // A retry starts the chunk over: the decoded bytes are written as they are
    // produced, so a failed attempt may have left half a chunk behind.
    if let Err(e) = sink.restart() {
        return Attempt::Fatal(anyhow::anyhow!("reset the chunk sink: {e}"));
    }
    let digest = match stream_body(resp.bytes_stream(), Some(expected), sink).await {
        Ok(v) => v,
        Err(StreamError::Transport(e)) => return Attempt::Retry(e),
        Err(StreamError::Content(e)) => return Attempt::Fatal(e),
    };
    Attempt::Ok(digest)
}

/// Feed a compressed body through the decoder into `sink`, hashing on the way
/// past.
///
/// The body is a **stream of pieces**, not a buffer: the compressed bytes are
/// decoded as they arrive and the decoded bytes leave as they are produced, so
/// neither the compressed body nor the decoded blob is ever resident as a
/// whole. `expected` is the ceiling on the decoded size, and `None` means "no
/// ceiling" — the case for a blob whose length is not promised by the API.
async fn stream_body<S, B, E>(
    mut body: S,
    expected: Option<u64>,
    sink: &mut impl ChunkSink,
) -> Result<ChunkDigest, StreamError>
where
    S: futures::Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
{
    let mut hashing = HashingWriter::new(&mut *sink, expected);
    let mut compressed = 0u64;
    {
        // The write-side decoder, built on the zio writer rather than
        // `stream::write::Decoder`: the zio writer's `finish` is what tells a
        // complete frame from a truncated one, and the wrapper does not expose
        // it. (`write::Decoder` has `with_writer`, `get_ref`, `into_inner`,
        // `auto_flush` — and no `finish`: `zstd-0.13.3/src/stream/write/mod.rs`
        // puts `do_finish` on the *encoder*.)
        let raw = zstd::stream::raw::Decoder::new()
            .map_err(|e| StreamError::Content(anyhow::anyhow!("zstd decoder: {e}")))?;
        let mut dec = zstd::stream::zio::Writer::new(&mut hashing, raw);
        while let Some(piece) = body.next().await {
            let bytes = match piece {
                Ok(b) => b,
                Err(e) => return Err(StreamError::Transport(anyhow::anyhow!("read body: {e}"))),
            };
            compressed += bytes.as_ref().len() as u64;
            if let Err(e) = dec.write_all(bytes.as_ref()) {
                return Err(StreamError::Content(anyhow::anyhow!("zstd chunk: {e}")));
            }
        }
        // The frame has to be *complete*: a truncated body decodes to a short
        // result without an error of its own, and a short result must not
        // pass for a successful download.
        dec.finish()
            .map_err(|e| StreamError::Content(anyhow::anyhow!("zstd chunk: {e}")))?;
    }
    Ok(ChunkDigest {
        compressed_bytes: compressed,
        bytes: hashing.written(),
        md5: hashing.md5(),
    })
}

/// Attempts per blob fetch, and the backoff is `attempt` seconds: 1+2+3+4 s.
const ATTEMPTS: u64 = 5;

/// One attempt at a blob fetch, classified by whether an identical retry could
/// come back different.
///
/// The three-way split is the retry policy. Collapsing `Fatal` into `Retry` is
/// what makes a run spend ~15 s of sleep to learn something it already knew.
enum Attempt<T> {
    Ok(T),
    /// Transient, or a failure we cannot classify. Worth another try.
    Retry(anyhow::Error),
    /// The origin has answered definitively. A second identical request will
    /// answer identically.
    Fatal(anyhow::Error),
}

/// Split an error by whether the status says it can change.
fn verdict<T>(e: anyhow::Error, retryable_status: bool) -> Attempt<T> {
    if retryable_status {
        Attempt::Retry(e)
    } else {
        Attempt::Fatal(e)
    }
}

/// `Some` when `status` is a failure, carrying a message in the shape `reqwest`
/// would have produced for `error_for_status()`.
fn status_failure(status: reqwest::StatusCode, url: &str) -> Option<anyhow::Error> {
    if status.is_success() {
        return None;
    }
    Some(anyhow::anyhow!(
        "HTTP status client error ({status}) for url ({url})"
    ))
}

/// Whether a blob status is worth another attempt.
///
/// Retried: `408`, `429`, and any `5xx` — a server that is overloaded or briefly
/// broken may answer again. **Not** retried: every other `4xx`. A server that has
/// decided this URL does not exist will not change its mind, and retrying it only
/// makes the failure take longer and look like a network problem.
///
/// **Why a 404 is safe to give up on, here specifically.** A manifest or chunk id
/// is not invented by this tool: it comes out of the `getBuild` body for the
/// build it is fetched under, and that namespace is published atomically. There
/// is no window in which an id is advertised by `getBuild` but its blob is not yet
/// on the CDN, so a 404 means "this build does not have that object" rather than
/// "ask again in a second". The same id under the *other* build's prefix is also
/// a 404, which is what makes the build id part of a chunk's address.
///
/// Same rule as `gircap`'s `RetryPolicy::should_retry_status`. Two tools in one
/// repository disagreeing about the same status code is itself a defect.
pub fn retryable(status: u16) -> bool {
    matches!(status, 408 | 429) || (500..=599).contains(&status)
}

pub fn join_url(prefix: &str, id: &str) -> String {
    format!("{}/{}", prefix.trim_end_matches('/'), id)
}

/// Build local-chunk reuse map: path -> list of (uncompressed_md5, uncompressed_size, offset).
pub fn build_local_chunk_map(
    manifests: &[WantedManifest],
) -> HashMap<String, Vec<(String, i64, i64)>> {
    let mut map: HashMap<String, Vec<(String, i64, i64)>> = HashMap::new();
    for m in manifests {
        for f in &m.files {
            if f.is_folder {
                continue;
            }
            let e = map.entry(f.file.clone()).or_default();
            for c in &f.chunks {
                // keep first occurrence per md5 (duplicates exist)
                if !e
                    .iter()
                    .any(|(md5, sz, _)| md5 == &c.uncompressed_md5 && *sz == c.uncompressed_size)
                {
                    e.push((c.uncompressed_md5.clone(), c.uncompressed_size, c.offset));
                }
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyp::{ManifestFile, ManifestUrl};

    // --- the streaming path -------------------------------------------------

    /// A frame the differential runs over.
    fn frame(payload: &[u8]) -> Vec<u8> {
        zstd::encode_all(payload, 3).expect("encode")
    }

    /// The body as the transport delivers it: pieces, not one buffer.
    fn pieces(bytes: &[u8], size: usize) -> Vec<Result<Vec<u8>, std::io::Error>> {
        bytes.chunks(size).map(|c| Ok(c.to_vec())).collect()
    }

    /// The streamed decode of a frame, into a fresh sink.
    async fn stream(
        compressed: &[u8],
        expected: Option<u64>,
    ) -> Result<(Vec<u8>, ChunkDigest), StreamError> {
        let mut sink: Vec<u8> = Vec::new();
        let body = futures::stream::iter(pieces(compressed, 4096));
        let digest = stream_body(body, expected, &mut sink).await?;
        Ok((sink, digest))
    }

    /// The differential: the streamed decode and the buffered decode of the
    /// same frame agree — on the bytes, on the length and on the digest.
    ///
    /// This is the test that keeps the streaming path honest. It hashes what
    /// it wrote and compares against the same MD5 the buffered path computed,
    /// so "the same digest" is a fact about the bytes and not about two
    /// agreeing implementations of the same idea.
    #[tokio::test]
    async fn the_streamed_decode_agrees_with_the_buffered_one() {
        let payload: Vec<u8> = (0..64 * 1024).map(|i| (i % 251) as u8).collect();
        let compressed = frame(&payload);
        let (sink, digest) = stream(&compressed, Some(payload.len() as u64))
            .await
            .expect("the frame decodes");
        let buffered = zstd::decode_all(&compressed[..]).expect("buffered decode");
        assert_eq!(sink, payload, "the sink got the decoded bytes");
        assert_eq!(sink, buffered, "and they are the buffered decode's bytes");
        assert_eq!(digest.bytes, buffered.len() as u64);
        assert_eq!(
            digest.md5,
            format!("{:x}", md5::compute(&buffered)),
            "the same digest as the buffered path"
        );
        assert_eq!(
            digest.compressed_bytes,
            compressed.len() as u64,
            "the compressed count is what came off the wire"
        );
    }

    /// A payload that decodes to the wrong bytes is caught by the digest: the
    /// streaming path hashes what it wrote, so the comparison is the same one
    /// the buffered path made — and it is the digest, not the length, that
    /// catches it.
    #[tokio::test]
    async fn a_corrupt_payload_is_detected() {
        let payload: Vec<u8> = (0..8 * 1024).map(|i| (i % 251) as u8).collect();
        let mut corrupt = payload.clone();
        corrupt[100] ^= 0xFF;
        let compressed = frame(&corrupt);
        let (sink, digest) = stream(&compressed, Some(payload.len() as u64))
            .await
            .expect("the frame decodes");
        assert_eq!(sink, corrupt, "the bytes are the corrupt ones");
        assert_eq!(
            digest.bytes,
            payload.len() as u64,
            "and the length is right"
        );
        assert_ne!(
            digest.md5,
            format!("{:x}", md5::compute(&payload)),
            "a corrupt payload must not carry the wanted digest"
        );
    }

    /// A truncated frame is an error and not a short success.
    ///
    /// Half a frame decodes into bytes without complaining, so the length
    /// check alone would let a torn body through as a smaller chunk. The
    /// decode has to insist that the frame is *complete*.
    #[tokio::test]
    async fn a_truncated_frame_is_an_error_and_not_a_short_success() {
        let payload: Vec<u8> = (0..8 * 1024).map(|i| (i % 251) as u8).collect();
        let compressed = frame(&payload);
        let truncated = &compressed[..compressed.len() / 2];
        let err = stream(truncated, Some(payload.len() as u64))
            .await
            .expect_err("a truncated frame is not a download");
        assert!(matches!(err, StreamError::Content(_)), "{err:?}");
    }

    /// The ceiling is enforced as the bytes go: a body that decodes to more
    /// than the chunk claims is refused, rather than written and complained
    /// about afterwards.
    #[tokio::test]
    async fn a_body_longer_than_its_claim_is_refused() {
        let payload: Vec<u8> = (0..8 * 1024).map(|i| (i % 251) as u8).collect();
        let compressed = frame(&payload);
        let err = stream(&compressed, Some(1024))
            .await
            .expect_err("too many bytes is a refusal");
        assert!(matches!(err, StreamError::Content(_)), "{err:?}");
    }

    /// A retry starts the blob over: the sink is asked to restart, and the
    /// bytes of the failed attempt do not survive into the next one.
    #[tokio::test]
    async fn a_restart_throws_the_failed_attempt_away() {
        let mut sink: Vec<u8> = Vec::new();
        sink.extend_from_slice(b"half a chunk");
        sink.restart().expect("restart");
        assert!(sink.is_empty(), "the failed attempt's bytes are gone");
    }

    /// The manifest's differential: the streamed decode of a manifest frame
    /// yields the same bytes the buffered decode did, the same MD5 of the
    /// **decompressed** bytes, and the same parsed message.
    ///
    /// The buffer is the size the API reported, and the decode fits in it —
    /// which is the whole point of sizing it: a buffer that had to grow would
    /// have doubled the peak.
    #[tokio::test]
    async fn the_streamed_manifest_parses_like_the_buffered_one() {
        let manifest = SophonChunkManifest {
            chuncks: vec![SophonChunkFile {
                file: "a/b.dat".into(),
                chunks: vec![SophonChunk {
                    id: "abc".into(),
                    uncompressed_md5: "d41d8cd98f00b204e9800998ecf8427e".into(),
                    offset: 0,
                    compressed_size: 10,
                    uncompressed_size: 20,
                    unknown: 0,
                    compressed_md5: "x".into(),
                }],
                is_folder: false,
                size: 20,
                md5: "d41d8cd98f00b204e9800998ecf8427e".into(),
            }],
        };
        let mut raw = Vec::new();
        manifest.encode(&mut raw).expect("prost encode");
        let compressed = frame(&raw);
        // The buffer the API's `uncompressed_size` sizes.
        let mut sink: Vec<u8> = Vec::with_capacity(raw.len());
        let body = futures::stream::iter(pieces(&compressed, 4096));
        let digest = stream_body(body, None, &mut sink)
            .await
            .expect("the frame decodes");
        assert_eq!(sink, raw, "the decoded bytes are the manifest");
        assert_eq!(
            sink,
            zstd::decode_all(&compressed[..]).expect("buffered decode"),
            "and they are the buffered decode's bytes"
        );
        assert_eq!(
            digest.md5,
            format!("{:x}", md5::compute(&raw)),
            "the checksum is of the decompressed bytes"
        );
        assert_eq!(digest.bytes, raw.len() as u64);
        assert_eq!(digest.compressed_bytes, compressed.len() as u64);
        assert_eq!(
            sink.capacity(),
            raw.len(),
            "the decode fitted the buffer the API's size asked for"
        );
        let parsed = SophonChunkManifest::decode(sink.as_slice()).expect("parse");
        assert_eq!(parsed, manifest, "the same message as the buffered one");
    }

    /// The manifest's reported size is a **hint**, and a hint cannot ask for an
    /// impossible allocation.
    ///
    /// `Vec::with_capacity` above `isize::MAX` panics, and a merely-large
    /// value is an OOM abort — on a field out of the API's JSON that nothing
    /// has checked. The clamp is what makes the reported size safe to act on.
    #[test]
    fn the_manifest_size_hint_is_clamped() {
        assert_eq!(manifest_size_hint(0), 0);
        assert_eq!(manifest_size_hint(-5), 0, "a negative size is no size");
        assert_eq!(
            manifest_size_hint(16_827_424),
            16_827_424,
            "the real manifest's size passes through"
        );
        assert_eq!(
            manifest_size_hint(i64::MAX),
            MAX_MANIFEST_HINT,
            "a bogus one cannot ask for an impossible allocation"
        );
        assert_eq!(
            manifest_size_hint(MAX_MANIFEST_HINT as i64 + 1),
            MAX_MANIFEST_HINT
        );
    }

    // --- the policy ---------------------------------------------------------
    /// The policy, pinned as a table rather than a prose claim: a retryable
    /// status must be one where a second identical request could come back
    /// different.
    #[test]
    fn only_transient_statuses_are_retryable() {
        for s in [408, 429, 500, 502, 503, 504, 599] {
            assert!(retryable(s), "{s} is transient and must be retried");
        }
        // The blob routes answer a missing object with a 404 and an AliyunOSS
        // `NoSuchKey` body. Retrying that cannot change the answer: the id came
        // out of this build's own `getBuild` body, so it is not in a
        // propagation window.
        for s in [400, 401, 403, 404, 405, 410, 418, 451] {
            assert!(!retryable(s), "{s} is decided; retrying it changes nothing");
        }
    }

    /// A success is not a retryable status, and that is fine: success never
    /// reaches the verdict, which is only consulted once a response has already
    /// been judged a failure. Pinning it keeps someone from later widening the
    /// rule to "not an error at all".
    #[test]
    fn a_success_is_not_confused_with_a_retryable_failure() {
        assert!(!retryable(200));
        assert!(!retryable(204));
    }

    fn meta(field: &str) -> ChunkManifestMeta {
        ChunkManifestMeta {
            category_id: field.to_string(),
            category_name: field.to_string(),
            matching_field: field.to_string(),
            manifest: ManifestFile::default(),
            chunk_download: ManifestUrl::default(),
            manifest_download: ManifestUrl::default(),
        }
    }

    #[test]
    fn manifest_filter_audio_and_ignore() {
        let build = ChunkBuild {
            build_id: "b".into(),
            tag: "5.0".into(),
            manifests: vec![meta("game"), meta("en-us"), meta("ja-jp"), meta("10302")],
        };
        let audio: HashSet<String> = ["en-us".to_string()].into_iter().collect();
        let ignore: HashSet<String> = ["10302".to_string()].into_iter().collect();
        let sel = select_manifests(&build, &audio, &ignore);
        let fields: Vec<_> = sel.iter().map(|m| m.matching_field.as_str()).collect();
        assert_eq!(fields, vec!["game", "en-us"]);
    }

    #[test]
    fn proto_roundtrip() {
        let m = SophonChunkManifest {
            chuncks: vec![SophonChunkFile {
                file: "a/b.dat".into(),
                chunks: vec![SophonChunk {
                    id: "abc".into(),
                    uncompressed_md5: "d41d8cd98f00b204e9800998ecf8427e".into(),
                    offset: 0,
                    compressed_size: 10,
                    uncompressed_size: 20,
                    unknown: 0,
                    compressed_md5: "x".into(),
                }],
                is_folder: false,
                size: 20,
                md5: "d41d8cd98f00b204e9800998ecf8427e".into(),
            }],
        };
        let mut buf = Vec::new();
        prost::Message::encode(&m, &mut buf).unwrap();
        let back = SophonChunkManifest::decode(buf.as_slice()).unwrap();
        assert_eq!(back.chuncks[0].file, "a/b.dat");
    }

    fn chunk(id: &str, md5: &str, offset: i64, size: i64) -> SophonChunk {
        SophonChunk {
            id: id.into(),
            uncompressed_md5: md5.into(),
            offset,
            compressed_size: size + 1,
            uncompressed_size: size,
            unknown: 0,
            compressed_md5: "compressed".into(),
        }
    }

    #[test]
    fn local_chunk_map_dedups_by_md5_size_and_skips_folders() {
        let wm = vec![WantedManifest {
            meta: meta("game"),
            files: vec![
                SophonChunkFile {
                    file: "a/b.dat".into(),
                    chunks: vec![
                        chunk("c1", "d41d8cd98f00b204e9800998ecf8427e", 0, 20),
                        // same (md5,size) at a different offset must be dropped (first wins)
                        chunk("c1dup", "d41d8cd98f00b204e9800998ecf8427e", 999, 20),
                        // same size, different md5 must be kept
                        chunk("c2", "0cc175b9c0f1b6a831c399e269772661", 20, 20),
                    ],
                    is_folder: false,
                    size: 40,
                    md5: "m1".into(),
                },
                SophonChunkFile {
                    file: "dir/".into(),
                    chunks: vec![chunk("cf", "0cc175b9c0f1b6a831c399e269772661", 0, 5)],
                    is_folder: true,
                    size: 0,
                    md5: String::new(),
                },
            ],
        }];
        let map = build_local_chunk_map(&wm);
        let e = &map["a/b.dat"];
        assert_eq!(e.len(), 2);
        assert_eq!(
            e[0],
            ("d41d8cd98f00b204e9800998ecf8427e".to_string(), 20, 0)
        );
        assert_eq!(
            e[1],
            ("0cc175b9c0f1b6a831c399e269772661".to_string(), 20, 20)
        );
        // folders never enter the reuse map
        assert!(!map.contains_key("dir/"));
    }
}
