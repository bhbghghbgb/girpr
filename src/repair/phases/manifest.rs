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
use crate::sleep::Sleep;
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
        sleep: &'a dyn Sleep,
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
                let files = sophon::fetch_manifest(http, sleep, m).await;
                (index, files)
            })
            .buffer_unordered(limit);
        Self {
            ordered: OrderedResults::new(manifests.len()),
            manifests,
            results: Box::pin(results),
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
    sleep: &dyn Sleep,
    latest_build: &ChunkBuild,
    audio_langs: &std::collections::HashSet<String>,
    ignore: &std::collections::HashSet<String>,
    jobs: usize,
) -> Result<Vec<(String, Vec<SophonChunkFile>)>, RunFailure> {
    let mut fetches = ManifestFetches::start(
        http,
        sleep,
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
    sleep: &dyn Sleep,
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
        sleep,
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
    use crate::sleep::{NoSleep, SleepFuture};
    use crate::sophon::{SophonChunk, SophonChunkFile, SophonChunkManifest};
    use crate::util::md5::md5_digest;
    use prost::Message;
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A manifest body as the origin sends it: zstd over the protobuf, with
    /// the checksum of the **decompressed** bytes.
    fn manifest_body(files: Vec<SophonChunkFile>) -> (Vec<u8>, String) {
        let manifest = SophonChunkManifest { chuncks: files };
        let mut raw = Vec::new();
        manifest.encode(&mut raw).expect("prost encode");
        let checksum = md5_digest(&raw).to_hex();
        (
            zstd::encode_all(raw.as_slice(), 3).expect("zstd encode"),
            checksum,
        )
    }

    /// One file with one chunk at `offset`, so two manifests can describe the
    /// same path differently.
    fn file(path: &str, offset: i64) -> SophonChunkFile {
        let digest = md5_digest(b"payload").to_hex();
        SophonChunkFile {
            file: path.into(),
            chunks: vec![SophonChunk {
                id: "chunk".into(),
                uncompressed_md5: digest.clone(),
                offset,
                compressed_size: 1,
                uncompressed_size: 7,
                unknown: 0,
                compressed_md5: String::new(),
            }],
            is_folder: false,
            size: 7,
            md5: digest,
        }
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

    enum Reply {
        Body(Vec<u8>),
        Status(&'static str),
        /// Read the request and keep the socket open without answering, so an
        /// uncancelled fetch would wait forever.
        Never,
    }

    /// Serve `id -> reply` on an ephemeral loopback port.
    async fn serve(routes: Vec<(&'static str, Reply)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let routes = Arc::new(routes);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    continue;
                };
                let routes = routes.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let mut got = 0;
                    while got < buf.len() {
                        match socket.read(&mut buf[got..]).await {
                            Ok(0) => break,
                            Ok(n) => {
                                got += n;
                                if buf[..got].windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            Err(_) => return,
                        }
                    }
                    let head = String::from_utf8_lossy(&buf[..got]).to_string();
                    let target = head
                        .lines()
                        .next()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .unwrap_or("/");
                    let id = target.rsplit('/').next().unwrap_or("");
                    let (status, body) = match routes.iter().find(|(rid, _)| *rid == id) {
                        Some((_, Reply::Body(body))) => ("200 OK", body.clone()),
                        Some((_, Reply::Status(status))) => (*status, Vec::new()),
                        Some((_, Reply::Never)) => {
                            std::future::pending::<()>().await;
                            unreachable!("a never-answering route holds the socket open")
                        }
                        None => ("404 Not Found", Vec::new()),
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                    let _ = socket.flush().await;
                });
            }
        });
        base
    }

    /// A short backoff, so a retryable failure takes longer to finish than a
    /// decided one without spending the real 1+2+3+4 s.
    #[derive(Debug)]
    struct ShortSleep;

    impl Sleep for ShortSleep {
        fn sleep(&self, _d: Duration) -> SleepFuture {
            Box::pin(tokio::time::sleep(Duration::from_millis(5)))
        }
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

    /// The first manifest fails at once while a later one never answers. The
    /// error is returned without waiting for the sibling: once the failure is
    /// settled in selection order the rest of the fan-out is cancelled. The
    /// timeout is a guard against the test hanging, not a performance bound.
    #[tokio::test]
    async fn fetch_latest_cancels_fetches_after_the_first_failure_settles() {
        let base = serve(vec![
            ("down", Reply::Status("404 Not Found")),
            ("hang", Reply::Never),
        ])
        .await;
        let build = build(
            &base,
            vec![meta("game", "down", ""), meta("extra", "hang", "")],
        );
        let client = Client::new();
        let failure = tokio::time::timeout(
            Duration::from_secs(5),
            fetch_latest(
                &client,
                &NoSleep,
                &build,
                &HashSet::new(),
                &HashSet::new(),
                4,
            ),
        )
        .await
        .expect("the error does not wait for the hanging manifest")
        .expect_err("the game manifest is down");
        let message = failure.to_string();
        assert!(message.contains("manifest game"), "{message}");
        assert!(!message.contains("manifest extra"), "{message}");
    }

    /// The first manifest fails slowly (a retryable 500) while a later one
    /// fails at once with a 404. The returned error must name the first
    /// manifest in selection order, not the first to finish.
    #[tokio::test]
    async fn fetch_latest_reports_the_first_failure_in_selection_order() {
        let (ok_body, ok_checksum) = manifest_body(vec![file("game/ok.dat", 0)]);
        let base = serve(vec![
            ("slow", Reply::Status("500 Internal Server Error")),
            ("fast", Reply::Status("404 Not Found")),
            ("ok", Reply::Body(ok_body)),
        ])
        .await;
        let build = build(
            &base,
            vec![
                meta("game", "slow", ""),
                meta("res", "fast", ""),
                meta("extra", "ok", &ok_checksum),
            ],
        );
        let client = Client::new();
        let failure = fetch_latest(
            &client,
            &ShortSleep,
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
    }

    /// A failed local manifest is skipped, and the successful ones still
    /// contribute. The earlier selection's entry for a path wins, so
    /// completion order cannot leak into the map.
    #[tokio::test]
    async fn local_reuse_map_keeps_the_union_and_selection_order() {
        let (first_body, first_checksum) = manifest_body(vec![file("game/foo.dat", 0)]);
        let (last_body, last_checksum) = manifest_body(vec![file("game/foo.dat", 99)]);
        let base = serve(vec![
            ("first", Reply::Body(first_body)),
            ("gone", Reply::Status("404 Not Found")),
            ("last", Reply::Body(last_body)),
        ])
        .await;
        let build = build(
            &base,
            vec![
                meta("game", "first", &first_checksum),
                meta("res", "gone", ""),
                meta("extra", "last", &last_checksum),
            ],
        );
        let client = Client::new();
        let map = local_reuse_map(
            &client,
            &NoSleep,
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
