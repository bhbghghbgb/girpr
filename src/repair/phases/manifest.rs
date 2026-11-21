//! Manifest selection, fetch, and the local chunk-reuse map (docs/02 step 3).
//!
//! Two things come out of here:
//! 1. the per-manifest file lists of the **latest** build (the plan's input),
//! 2. `path -> FileReuse` for the **local** build, so an
//!    unchanged chunk can be copied from the file already on disk instead of
//!    downloaded.
//!
//! Manifests are never cached inside `game_dir` (V1-SIMPLIFICATION S1: the
//! blob is streamed into memory, parsed, and not kept) — see
//! [`crate::sophon::fetch_manifest`].
//!
//! The selected manifests are fetched **concurrently**, but the fan-out is
//! bounded: one decoded manifest can be tens of MiB, so opening every fetch at
//! once would trade latency for peak memory. The bound is a small dedicated
//! cap, not the file-worker count. Results are put back in selection order as
//! they arrive, so the plan's per-manifest order and the error `fetch_latest`
//! reports do not depend on completion order. Once the first failure in
//! selection order is settled, the remaining fetches are dropped instead of
//! waited out.

use std::collections::HashMap;
use std::pin::Pin;

use anyhow::Context;
use futures_util::{Stream, StreamExt};
use reqwest::Client;

use crate::hyp::ChunkBuild;
use crate::hyp::ChunkManifestMeta;
use crate::retry::BackoffPolicy;
use crate::sophon::{self, FileReuse, SophonChunkFile, WantedManifest};

use crate::config::RunFailure;

/// `path -> FileReuse`, built from the local build's manifests. Keyed by
/// path only: reuse is same-file-only (S2).
///
/// This map is keyed by the **local manifest's** rel, not by a
/// [`crate::repair::phases::plan::FileId`]. It is a producer-owned lookup that
/// records what an earlier install had, so it cannot become a field on a
/// planned file without joining the two manifests at plan time.
pub type LocalChunkMap = HashMap<String, FileReuse>;

/// The most manifests fetched at once.
///
/// One decoded manifest can be tens of MiB, so this is deliberately much
/// smaller than the file-worker count: the point is to overlap the network
/// latency of a handful of blobs, not to hold every manifest in memory at
/// its peak decode size.
const MAX_MANIFEST_CONCURRENCY: usize = 4;

/// The manifest fan-out for a run, bounded by [`MAX_MANIFEST_CONCURRENCY`].
fn manifest_concurrency(jobs: usize) -> usize {
    jobs.clamp(1, MAX_MANIFEST_CONCURRENCY)
}

/// Which build's manifests a fan-out belongs to, for the log line.
#[derive(Clone, Copy)]
enum BuildKind {
    Latest,
    Local,
}

impl BuildKind {
    /// The wording that distinguishes the two fan-outs in the log.
    const fn log_prefix(self) -> &'static str {
        match self {
            Self::Latest => "manifest",
            Self::Local => "local manifest",
        }
    }
}

/// Results indexed by selection, drained left to right.
///
/// `buffer_unordered` yields in completion order; pushing each result into its
/// slot lets a caller walk selection order without waiting for every fetch.
/// A slot that is not filled yet leaves [`Self::take_next`] empty, while
/// [`Self::has_next`] is still true, so "not yet" and "finished" stay distinct.
struct OrderedResults<T> {
    slots: Vec<Option<T>>,
    next: usize,
}

impl<T> OrderedResults<T> {
    fn new(len: usize) -> Self {
        Self {
            slots: (0..len).map(|_| None).collect(),
            next: 0,
        }
    }

    /// Whether a selection index is still waiting to be drained.
    fn has_next(&self) -> bool {
        self.next < self.slots.len()
    }

    /// Record a result that arrived for `index`; it is read once every earlier
    /// selection has been drained.
    ///
    /// `index` is the selection index from the same `enumerate` that sized
    /// `slots`, so `slots[index]` is in bounds by local invariant; no
    /// server-derived value reaches it.
    fn push(&mut self, index: usize, value: T) {
        self.slots[index] = Some(value);
    }

    /// Take the next selection's result if it has arrived.
    fn take_next(&mut self) -> Option<(usize, T)> {
        if !self.has_next() {
            return None;
        }
        let value = self.slots[self.next].take()?;
        let index = self.next;
        self.next += 1;
        Some((index, value))
    }
}

/// The outcome of fetching one selected manifest.
type ManifestResult = anyhow::Result<Vec<SophonChunkFile>>;

/// A completion-ordered stream of `(selection index, result)` pairs.
type ManifestStream<'a> = Pin<Box<dyn Stream<Item = (usize, ManifestResult)> + 'a>>;

/// A bounded, concurrent fetch of the selected manifests of one build, read
/// back in selection order.
struct ManifestFetches<'a> {
    manifests: Vec<&'a ChunkManifestMeta>,
    results: ManifestStream<'a>,
    ordered: OrderedResults<ManifestResult>,
}

impl<'a> ManifestFetches<'a> {
    /// Start fetching `build`'s selected manifests, at most
    /// [`MAX_MANIFEST_CONCURRENCY`] at once.
    fn start(
        http: &'a Client,
        backoff: &'a BackoffPolicy,
        build: &'a ChunkBuild,
        audio_langs: &'a std::collections::HashSet<String>,
        ignore: &'a std::collections::HashSet<String>,
        jobs: usize,
        kind: BuildKind,
    ) -> Self {
        let manifests = sophon::select_manifests(build, audio_langs, ignore);
        let limit = manifest_concurrency(jobs);
        tracing::info!("{} concurrency={limit}", kind.log_prefix());
        // The stream owns its copy of the selection (plain references), so it
        // does not borrow the vector the struct keeps for lookups.
        let results = futures_util::stream::iter(manifests.clone().into_iter().enumerate())
            .map(move |(index, m)| async move {
                let files = sophon::fetch_manifest(http, backoff, m).await;
                (index, files)
            })
            .buffer_unordered(limit);
        Self {
            ordered: OrderedResults::new(manifests.len()),
            manifests,
            results: Box::pin(results),
        }
    }

    /// Build the reader around a stream the test controls, so the ordering
    /// logic can be driven by real completions rather than hand-fed results.
    #[cfg(test)]
    fn from_stream(manifests: Vec<&'a ChunkManifestMeta>, results: ManifestStream<'a>) -> Self {
        Self {
            ordered: OrderedResults::new(manifests.len()),
            manifests,
            results,
        }
    }

    /// The next manifest in selection order with its result, waiting only for
    /// that selection's fetch.
    ///
    /// An error result is returned as soon as every earlier selection has
    /// succeeded, which is the first failure in selection order. A caller that
    /// then stops iterating drops the fan-out, cancelling the fetches that
    /// have not finished instead of waiting out a slow sibling.
    #[allow(clippy::expect_used)] // every selected manifest is fetched once
    async fn next_ordered(&mut self) -> Option<(&'a ChunkManifestMeta, ManifestResult)> {
        while self.ordered.has_next() {
            if let Some((index, result)) = self.ordered.take_next() {
                return Some((self.manifests[index], result));
            }
            let (index, result) = self
                .results
                .next()
                .await
                .expect("every selected manifest is fetched once");
            self.ordered.push(index, result);
        }
        None
    }
}

/// Fetch every wanted manifest of the latest build concurrently, bounded, and
/// keep each one's chunk URL prefix next to its file list (the plan needs the
/// pairing).
///
/// The output is in selection order. A failure is reported for the **first**
/// manifest in selection order that failed, not the first to finish: the
/// message is about what is wrong with the build, and completion order is an
/// accident of the network. Once that failure is settled — every earlier
/// selection has succeeded — the remaining fetches are cancelled rather than
/// awaited.
pub async fn fetch_latest(
    http: &Client,
    backoff: &BackoffPolicy,
    latest_build: &ChunkBuild,
    audio_langs: &std::collections::HashSet<String>,
    ignore: &std::collections::HashSet<String>,
    jobs: usize,
) -> Result<Vec<(String, Vec<SophonChunkFile>)>, RunFailure> {
    let mut fetches = ManifestFetches::start(
        http,
        backoff,
        latest_build,
        audio_langs,
        ignore,
        jobs,
        BuildKind::Latest,
    );
    let mut per_manifest: Vec<(String, Vec<SophonChunkFile>)> = Vec::new();
    // The loop visits selections left to right, so the first error it meets is
    // the first failure in selection order. Returning here drops `fetches` and
    // with it every fetch still in flight.
    while let Some((m, files)) = fetches.next_ordered().await {
        let files = files
            .with_context(|| format!("manifest {}", m.matching_field))
            .map_err(RunFailure::metadata)?;
        tracing::info!("manifest {}: {} entries", m.matching_field, files.len());
        per_manifest.push((m.chunk_download.url_prefix.clone(), files));
    }
    Ok(per_manifest)
}

/// Build the chunk-reuse map from the local build. Fully best-effort: a
/// manifest that will not fetch is skipped, and a missing local build simply
/// yields an empty map (every chunk then downloads).
///
/// The fetches are concurrent and bounded the same way as [`fetch_latest`],
/// and the assembled `wanted` list is in selection order: when two manifests
/// describe the same path, the earlier one wins the first-occurrence rules in
/// [`sophon::build_local_chunk_map`]. Unlike [`fetch_latest`], a failure does
/// not settle the fan-out: every reachable manifest still contributes.
pub async fn local_reuse_map(
    http: &Client,
    backoff: &BackoffPolicy,
    local_build: Option<&ChunkBuild>,
    audio_langs: &std::collections::HashSet<String>,
    ignore: &std::collections::HashSet<String>,
    jobs: usize,
) -> LocalChunkMap {
    let Some(local_build) = local_build else {
        return LocalChunkMap::new();
    };
    let mut fetches = ManifestFetches::start(
        http,
        backoff,
        local_build,
        audio_langs,
        ignore,
        jobs,
        BuildKind::Local,
    );

    let mut wanted: Vec<WantedManifest> = Vec::new();
    while let Some((m, result)) = fetches.next_ordered().await {
        match result {
            Ok(files) => wanted.push(WantedManifest {
                meta: (*m).clone(),
                files,
            }),
            Err(e) => tracing::warn!("local manifest {} failed: {}", m.matching_field, e),
        }
    }
    sophon::build_local_chunk_map(&wanted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyp::{ChunkManifestMeta, ManifestFile, ManifestUrl};
    use crate::test_origin::{InFlight, Reply, TestServer};
    use crate::util::md5::md5_digest;
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// The three deterministic manifest bodies the canned tests serve, frozen
    /// from the prost + zstd(level 3) encoding the old `manifest_body` helper
    /// produced on every call. Each companion checksum is the MD5 of the
    /// decompressed protobuf — what the origin advertises and the fetch path
    /// verifies — so freezing the bytes keeps that byte contract intact. The
    /// tests still serve these exact bytes and assert against these exact
    /// checksums; only the per-test re-encode is gone.
    const OK_BODY: &[u8] = &[
        0x28, 0xb5, 0x2f, 0xfd, 0x20, 0x62, 0x4d, 0x02, 0x00, 0x14, 0x04, 0x0a, 0x60, 0x0a, 0x0b,
        0x67, 0x61, 0x6d, 0x65, 0x2f, 0x6f, 0x6b, 0x2e, 0x64, 0x61, 0x74, 0x12, 0x2d, 0x0a, 0x05,
        0x63, 0x68, 0x75, 0x6e, 0x6b, 0x12, 0x20, 0x33, 0x32, 0x31, 0x63, 0x33, 0x63, 0x66, 0x34,
        0x38, 0x36, 0x65, 0x64, 0x35, 0x30, 0x39, 0x31, 0x36, 0x34, 0x65, 0x64, 0x65, 0x63, 0x31,
        0x65, 0x31, 0x39, 0x38, 0x31, 0x66, 0x65, 0x63, 0x38, 0x20, 0x01, 0x28, 0x07, 0x20, 0x07,
        0x2a, 0x01, 0x00, 0xc1, 0x1a, 0x55, 0x14,
    ];
    const OK_CHECKSUM: &str = "87073374716da339ccbaab78e2af1738";

    const FIRST_BODY: &[u8] = &[
        0x28, 0xb5, 0x2f, 0xfd, 0x20, 0x63, 0x55, 0x02, 0x00, 0x24, 0x04, 0x0a, 0x61, 0x0a, 0x0c,
        0x67, 0x61, 0x6d, 0x65, 0x2f, 0x66, 0x6f, 0x6f, 0x2e, 0x64, 0x61, 0x74, 0x12, 0x2d, 0x0a,
        0x05, 0x63, 0x68, 0x75, 0x6e, 0x6b, 0x12, 0x20, 0x33, 0x32, 0x31, 0x63, 0x33, 0x63, 0x66,
        0x34, 0x38, 0x36, 0x65, 0x64, 0x35, 0x30, 0x39, 0x31, 0x36, 0x34, 0x65, 0x64, 0x65, 0x63,
        0x31, 0x65, 0x31, 0x39, 0x38, 0x31, 0x66, 0x65, 0x63, 0x38, 0x20, 0x01, 0x28, 0x07, 0x20,
        0x07, 0x2a, 0x01, 0x00, 0xc2, 0x1a, 0x55, 0x14,
    ];
    const FIRST_CHECKSUM: &str = "898fcd4b256b0b4120387ec0fa9ac26e";

    const LAST_BODY: &[u8] = &[
        0x28, 0xb5, 0x2f, 0xfd, 0x20, 0x65, 0x65, 0x02, 0x00, 0x44, 0x04, 0x0a, 0x63, 0x0a, 0x0c,
        0x67, 0x61, 0x6d, 0x65, 0x2f, 0x66, 0x6f, 0x6f, 0x2e, 0x64, 0x61, 0x74, 0x12, 0x2f, 0x0a,
        0x05, 0x63, 0x68, 0x75, 0x6e, 0x6b, 0x12, 0x20, 0x33, 0x32, 0x31, 0x63, 0x33, 0x63, 0x66,
        0x34, 0x38, 0x36, 0x65, 0x64, 0x35, 0x30, 0x39, 0x31, 0x36, 0x34, 0x65, 0x64, 0x65, 0x63,
        0x31, 0x65, 0x31, 0x39, 0x38, 0x31, 0x66, 0x65, 0x63, 0x38, 0x18, 0x63, 0x20, 0x01, 0x28,
        0x07, 0x20, 0x07, 0x2a, 0x01, 0x00, 0x44, 0x1b, 0x55, 0x14,
    ];
    const LAST_CHECKSUM: &str = "e9bb76aba0043b5acb1dd5a216e29868";

    /// The frozen bodies and their advertised checksums are a byte contract:
    /// pin the compressed bytes (via their MD5) and the decompressed checksum
    /// so an accidental edit to either is caught here rather than in a fetch.
    #[test]
    fn frozen_manifest_bodies_keep_their_checksums() {
        assert_eq!(OK_BODY.len(), 82);
        assert_eq!(
            md5_digest(OK_BODY).to_hex(),
            "9f0b1a07721d35a92240c495d7daec9c"
        );
        assert_eq!(OK_CHECKSUM, "87073374716da339ccbaab78e2af1738");
        assert_eq!(FIRST_BODY.len(), 83);
        assert_eq!(
            md5_digest(FIRST_BODY).to_hex(),
            "ab963dc07c83c66010d6d32aead23602"
        );
        assert_eq!(FIRST_CHECKSUM, "898fcd4b256b0b4120387ec0fa9ac26e");
        assert_eq!(LAST_BODY.len(), 85);
        assert_eq!(
            md5_digest(LAST_BODY).to_hex(),
            "fc7bd176fa5eb2411a76fe4e3696b292"
        );
        assert_eq!(LAST_CHECKSUM, "e9bb76aba0043b5acb1dd5a216e29868");
    }

    fn meta(field: &str, id: &str, checksum: &str) -> ChunkManifestMeta {
        ChunkManifestMeta {
            category_id: field.into(),
            category_name: field.into(),
            matching_field: field.into(),
            manifest: ManifestFile {
                id: id.into(),
                checksum: checksum.into(),
                compressed_size: 0,
                uncompressed_size: 0,
            },
            chunk_download: ManifestUrl::default(),
            manifest_download: ManifestUrl::default(),
        }
    }

    /// A build whose manifests all download from `prefix`, in the given
    /// selection order.
    fn build(prefix: &str, manifests: Vec<ChunkManifestMeta>) -> ChunkBuild {
        ChunkBuild {
            build_id: "build_test".into(),
            tag: "tag_test".into(),
            manifests: manifests
                .into_iter()
                .map(|mut m| {
                    m.manifest_download.url_prefix = prefix.into();
                    m.chunk_download.url_prefix = prefix.into();
                    m
                })
                .collect(),
        }
    }

    /// Serve `id -> reply` on an ephemeral loopback port. The exact-id routing
    /// stays here; the shared origin owns the listener, the response bytes and
    /// the Drop-abort driver.
    async fn serve(routes: Vec<(&'static str, Reply)>) -> TestServer {
        let routes = Arc::new(routes);
        crate::test_origin::serve(move |target| {
            let id = target.rsplit('/').next().unwrap_or("");
            routes
                .iter()
                .find(|(rid, _)| *rid == id)
                .map(|(_, reply)| reply.clone())
        })
        .await
    }
    /// A result for a later selection is retained without unblocking the
    /// caller, so the output is read in selection order even when the network
    /// finishes out of order.
    #[test]
    fn results_are_reordered_incrementally_in_selection_order() {
        let mut ordered = OrderedResults::new(3);
        ordered.push(2, Ok::<_, &str>("third"));
        assert!(
            ordered.take_next().is_none(),
            "a later selection does not unblock the first"
        );
        ordered.push(1, Err("second"));
        assert!(
            ordered.take_next().is_none(),
            "the first selection is still missing"
        );

        ordered.push(0, Ok("first"));
        assert_eq!(ordered.take_next(), Some((0, Ok("first"))));
        let (index, result) = ordered.take_next().expect("the second is ready");
        assert_eq!(index, 1);
        assert_eq!(
            result,
            Err("second"),
            "the first failure in selection order"
        );
        assert_eq!(ordered.take_next(), Some((2, Ok("third"))));
        assert!(!ordered.has_next());
    }

    /// Drive the reader with a stream that settles in reverse selection order:
    /// the last selection finishes first, then its predecessor, and the error
    /// at the first selection settles last. The output still follows selection
    /// order, and the first selection's error is the one reported.
    #[tokio::test]
    async fn ordered_results_follow_selection_order_through_an_out_of_order_stream() {
        let first = meta("first", "first", "");
        let second = meta("second", "second", "");
        let third = meta("third", "third", "");
        let manifests = vec![&first, &second, &third];

        let completed = Arc::new(Mutex::new(Vec::new()));
        // Each completion releases the next selection towards the front, so
        // the stream settles in reverse order without any timing.
        let (release_third, wait_third) = tokio::sync::oneshot::channel();
        let (release_second, wait_second) = tokio::sync::oneshot::channel();
        let (release_first, wait_first) = tokio::sync::oneshot::channel();

        let log = completed.clone();
        let third_result = async move {
            let _ = wait_third.await;
            log.lock().expect("completion log").push(2);
            let _ = release_second.send(());
            (2, Ok(Vec::new()))
        };
        let log = completed.clone();
        let second_result = async move {
            let _ = wait_second.await;
            log.lock().expect("completion log").push(1);
            let _ = release_first.send(());
            (1, Ok(Vec::new()))
        };
        let log = completed.clone();
        let first_result = async move {
            let _ = wait_first.await;
            log.lock().expect("completion log").push(0);
            (0, Err(anyhow::anyhow!("the first manifest is down")))
        };

        release_third.send(()).expect("the third selection waits");
        type ResultFuture = Pin<Box<dyn std::future::Future<Output = (usize, ManifestResult)>>>;
        let futures: Vec<ResultFuture> = vec![
            Box::pin(first_result),
            Box::pin(second_result),
            Box::pin(third_result),
        ];
        let results = futures_util::stream::iter(futures).buffer_unordered(3);
        let mut fetches = ManifestFetches::from_stream(manifests, Box::pin(results));

        let mut seen: Vec<(&str, Result<(), anyhow::Error>)> = Vec::new();
        while let Some((m, result)) = fetches.next_ordered().await {
            seen.push((m.matching_field.as_str(), result.map(|_| ())));
        }

        assert_eq!(
            completed.lock().expect("completion log").as_slice(),
            [2, 1, 0],
            "the fetches completed out of selection order"
        );
        assert_eq!(
            seen.iter().map(|(field, _)| *field).collect::<Vec<_>>(),
            ["first", "second", "third"],
            "the reader yields in selection order"
        );
        let first_error = seen[0]
            .1
            .as_ref()
            .expect_err(
                "the first selection's failure is reported first even though it settled last",
            )
            .to_string();
        assert_eq!(
            first_error, "the first manifest is down",
            "the first selection's own failure is reported, not a later one"
        );
        assert!(seen[1].1.is_ok() && seen[2].1.is_ok());
    }

    /// The first manifest fails at once while a later one never answers. The
    /// error is returned without waiting for the sibling: once the failure is
    /// settled in selection order the rest of the fan-out is cancelled. The
    /// timeout is a guard against the test hanging, not a performance bound.
    #[tokio::test]
    async fn fetch_latest_cancels_fetches_after_the_first_failure_settles() {
        // `down` answers only after `hang` has been requested, so the failure
        // settles while the hanging fetch is provably in flight: a fan-out
        // that waited for it would never return.
        let hang_hits = Arc::new(AtomicUsize::new(0));
        let server = serve(vec![
            (
                "down",
                Reply::StatusAfterHit("404 Not Found", hang_hits.clone()),
            ),
            ("hang", Reply::NeverAfterHit(hang_hits.clone())),
        ])
        .await;
        let build = build(
            server.base(),
            vec![meta("game", "down", ""), meta("extra", "hang", "")],
        );
        let client = Client::new();
        let failure = tokio::time::timeout(
            Duration::from_secs(5),
            fetch_latest(
                &client,
                &BackoffPolicy::none(),
                &build,
                &HashSet::new(),
                &HashSet::new(),
                4,
            ),
        )
        .await
        .expect("the error does not wait for the hanging manifest")
        .expect_err("the game manifest is down");
        assert_eq!(
            hang_hits.load(Ordering::SeqCst),
            1,
            "the hanging fetch was requested and then dropped by the cancelled fan-out"
        );
        let message = failure.to_string();
        assert!(message.contains("manifest game"), "{message}");
        assert!(!message.contains("manifest extra"), "{message}");
    }

    /// The first manifest fails through a retryable 500 (retried to the cap)
    /// while a later one fails at once with a 404. The returned error must name
    /// the first manifest in selection order, not the first to finish. Backoff
    /// is skipped with `BackoffPolicy::none()`; the attempt count is asserted
    /// from the error chain instead of from time spent sleeping.
    #[tokio::test]
    async fn fetch_latest_reports_the_first_failure_in_selection_order() {
        let (ok_body, ok_checksum) = (OK_BODY.to_vec(), OK_CHECKSUM.to_owned());
        let server = serve(vec![
            ("slow", Reply::Status("500 Internal Server Error")),
            ("fast", Reply::Status("404 Not Found")),
            ("ok", Reply::Body(ok_body)),
        ])
        .await;
        let build = build(
            server.base(),
            vec![
                meta("game", "slow", ""),
                meta("res", "fast", ""),
                meta("extra", "ok", &ok_checksum),
            ],
        );
        let client = Client::new();
        let failure = fetch_latest(
            &client,
            &BackoffPolicy::none(),
            &build,
            &HashSet::new(),
            &HashSet::new(),
            4,
        )
        .await
        .expect_err("the game manifest is down");
        let message = failure.to_string();
        assert!(message.contains("manifest game"), "{message}");
        assert!(!message.contains("manifest res"), "{message}");
        assert!(
            message.contains("after 5 attempts"),
            "the retryable 500 is retried to the attempt cap, not failed early: {message}"
        );
    }

    /// The real HTTP fan-out starts at most `manifest_concurrency(jobs)`
    /// requests at once. The origin holds each request until the test releases
    /// it, so the in-flight count is observed rather than timed; `usize::MAX`
    /// jobs hits the manifest cap.
    #[tokio::test]
    async fn fetch_latest_bounds_live_http_fetches_to_manifest_concurrency() {
        for jobs in [1, 2, usize::MAX] {
            let bound = manifest_concurrency(jobs);
            let (body, checksum) = (OK_BODY.to_vec(), OK_CHECKSUM.to_owned());
            let gate = Arc::new(InFlight::new());
            let ids = ["m0", "m1", "m2", "m3", "m4", "m5"];
            let server = serve(
                ids.iter()
                    .map(|id| {
                        (
                            *id,
                            Reply::GatedBody {
                                body: body.clone(),
                                gate: gate.clone(),
                            },
                        )
                    })
                    .collect(),
            )
            .await;
            let build = build(
                server.base(),
                ids.iter()
                    .map(|id| meta(id, id, &checksum))
                    .collect::<Vec<_>>(),
            );
            let client = Client::new();
            let audio_langs = HashSet::new();
            let ignore = HashSet::new();
            let backoff = BackoffPolicy::none();

            let fetched = fetch_latest(&client, &backoff, &build, &audio_langs, &ignore, jobs);
            let observed = async {
                tokio::time::timeout(Duration::from_secs(5), gate.wait_for(bound))
                    .await
                    .expect("the fan-out reaches its bound");
                assert_eq!(
                    gate.current.load(Ordering::SeqCst),
                    bound,
                    "the fan-out starts no more than {bound} fetches at once"
                );
                let _ = gate.release.send(true);
            };
            let (manifests, ()) = tokio::join!(fetched, observed);

            assert_eq!(
                manifests.expect("every gated manifest answers").len(),
                ids.len()
            );
            assert_eq!(
                gate.peak.load(Ordering::SeqCst),
                bound,
                "the bound holds for the whole run, not just the first burst"
            );
        }
    }

    /// An aborted gated connection releases its in-flight slot. Dropping the
    /// server tears the connection task down while it waits on the gate, so
    /// the count must return to zero even though the handler never resumes.
    #[tokio::test]
    async fn an_aborted_gated_connection_releases_its_in_flight_slot() {
        let (body, checksum) = (OK_BODY.to_vec(), OK_CHECKSUM.to_owned());
        let gate = Arc::new(InFlight::new());
        let server = serve(vec![(
            "ok",
            Reply::GatedBody {
                body: body.clone(),
                gate: gate.clone(),
            },
        )])
        .await;
        let build = build(server.base(), vec![meta("game", "ok", &checksum)]);
        let client = Client::new();
        let audio_langs = HashSet::new();
        let ignore = HashSet::new();
        let backoff = BackoffPolicy::none();

        let mut fetched = Box::pin(fetch_latest(
            &client,
            &backoff,
            &build,
            &audio_langs,
            &ignore,
            1,
        ));
        tokio::select! {
            () = gate.wait_for(1) => {}
            result = &mut fetched => {
                panic!("the fetch finished before it reached the gate: {result:?}")
            }
        }

        // Cancelling the fetch and dropping the server aborts the connection
        // task while it is still held at the gate.
        drop(fetched);
        drop(server);

        tokio::time::timeout(Duration::from_secs(5), gate.wait_until_idle())
            .await
            .expect("the aborted gated connection releases its slot");
        assert_eq!(
            gate.current.load(Ordering::SeqCst),
            0,
            "the count returns to zero after the gated server is dropped"
        );
    }

    /// The registry drops a connection as soon as it finishes: a single
    /// request is reaped without another accept arriving to trigger it.
    #[tokio::test]
    async fn finished_connections_are_reaped_without_another_accept() {
        let (body, checksum) = (OK_BODY.to_vec(), OK_CHECKSUM.to_owned());
        let server = serve(vec![("ok", Reply::Body(body))]).await;
        let build = build(server.base(), vec![meta("game", "ok", &checksum)]);
        let client = Client::new();
        let manifests = fetch_latest(
            &client,
            &BackoffPolicy::none(),
            &build,
            &HashSet::new(),
            &HashSet::new(),
            1,
        )
        .await
        .expect("the manifest answers");
        assert_eq!(manifests.len(), 1);
        tokio::time::timeout(Duration::from_secs(5), server.wait_until_idle())
            .await
            .expect("the finished connection is reaped without another accept");
    }

    /// A failed local manifest is skipped, and the successful ones still
    /// contribute. The earlier selection's entry for a path wins, so
    /// completion order cannot leak into the map.
    #[tokio::test]
    async fn local_reuse_map_keeps_the_union_and_selection_order() {
        let (first_body, first_checksum) = (FIRST_BODY.to_vec(), FIRST_CHECKSUM.to_owned());
        let (last_body, last_checksum) = (LAST_BODY.to_vec(), LAST_CHECKSUM.to_owned());
        let server = serve(vec![
            ("first", Reply::Body(first_body)),
            ("gone", Reply::Status("404 Not Found")),
            ("last", Reply::Body(last_body)),
        ])
        .await;
        let build = build(
            server.base(),
            vec![
                meta("game", "first", &first_checksum),
                meta("res", "gone", ""),
                meta("extra", "last", &last_checksum),
            ],
        );
        let client = Client::new();
        let map = local_reuse_map(
            &client,
            &BackoffPolicy::none(),
            Some(&build),
            &HashSet::new(),
            &HashSet::new(),
            4,
        )
        .await;
        let entries = map
            .get("game/foo.dat")
            .expect("the successful manifests contribute to the map")
            .entries();
        assert_eq!(entries.len(), 1, "the same chunk is one entry");
        assert_eq!(
            entries[0].offset, 0,
            "the first selected manifest's entry wins"
        );
    }
}
