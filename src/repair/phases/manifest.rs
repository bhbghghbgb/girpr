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
//! cap, not the file-worker count. Results are put back in selection order
//! before anything reads them, so the plan's per-manifest order and the error
//! `fetch_latest` reports do not depend on completion order.

use std::collections::HashMap;

use anyhow::Context;
use futures::StreamExt;
use reqwest::Client;

use crate::hyp::ChunkBuild;
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

/// Put concurrent fetch results back into selection order.
///
/// `buffer_unordered` yields in completion order; callers iterate the returned
/// slots by selection index, so the plan's manifest order and the first error
/// they report are independent of which fetch finished first. Every selection
/// produces exactly one result, so a slot is filled by the time a caller reads
/// it.
fn in_selection_order<T, E>(
    results: Vec<(usize, Result<T, E>)>,
    len: usize,
) -> Vec<Option<Result<T, E>>> {
    let mut slots: Vec<Option<Result<T, E>>> = (0..len).map(|_| None).collect();
    for (index, result) in results {
        slots[index] = Some(result);
    }
    slots
}

/// Fetch every wanted manifest of the latest build concurrently, bounded, and
/// keep each one's chunk URL prefix next to its file list (the plan needs the
/// pairing).
///
/// The output is in selection order. A failure is reported for the **first**
/// manifest in selection order that failed, not the first to finish: the
/// message is about what is wrong with the build, and completion order is an
/// accident of the network.
pub async fn fetch_latest(
    http: &Client,
    sleep: &dyn Sleep,
    latest_build: &ChunkBuild,
    audio_langs: &std::collections::HashSet<String>,
    ignore: &std::collections::HashSet<String>,
    jobs: usize,
) -> Result<Vec<(String, Vec<SophonChunkFile>)>, RunFailure> {
    let manifests = sophon::select_manifests(latest_build, audio_langs, ignore);
    let limit = manifest_concurrency(jobs);
    tracing::info!("manifest concurrency={limit}");
    let results = futures::stream::iter(manifests.iter().copied().enumerate())
        .map(|(index, m)| async move {
            let files = sophon::fetch_manifest(http, sleep, m).await;
            (index, files)
        })
        .buffer_unordered(limit)
        .collect::<Vec<_>>()
        .await;

    let mut per_manifest: Vec<(String, Vec<SophonChunkFile>)> = Vec::new();
    for (m, slot) in manifests
        .iter()
        .zip(in_selection_order(results, manifests.len()))
    {
        let files = slot
            .expect("every selected manifest is fetched once")
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
/// [`sophon::build_local_chunk_map`].
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
    let manifests = sophon::select_manifests(local_build, audio_langs, ignore);
    let limit = manifest_concurrency(jobs);
    tracing::info!("local manifest concurrency={limit}");
    let results = futures::stream::iter(manifests.iter().copied().enumerate())
        .map(|(index, m)| async move {
            let files = sophon::fetch_manifest(http, sleep, m).await;
            (index, files)
        })
        .buffer_unordered(limit)
        .collect::<Vec<_>>()
        .await;

    let mut wanted: Vec<WantedManifest> = Vec::new();
    for (m, slot) in manifests
        .iter()
        .zip(in_selection_order(results, manifests.len()))
    {
        match slot.expect("every selected manifest is fetched once") {
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
        let checksum = format!("{:x}", md5::compute(&raw));
        (
            zstd::encode_all(raw.as_slice(), 3).expect("zstd encode"),
            checksum,
        )
    }

    /// One file with one chunk at `offset`, so two manifests can describe the
    /// same path differently.
    fn file(path: &str, offset: i64) -> SophonChunkFile {
        let digest = format!("{:x}", md5::compute(b"payload"));
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

    /// The re-index step, with results arriving out of order: the first error
    /// a caller walking left to right meets is the second selection, not the
    /// first completion.
    #[test]
    fn out_of_order_results_are_reindexed_by_selection() {
        let results = vec![(2, Ok("third")), (1, Err("second")), (0, Ok("first"))];
        let ordered: Vec<_> = in_selection_order(results, 3)
            .into_iter()
            .map(|slot| slot.expect("every selection has a result"))
            .collect();
        assert_eq!(ordered, vec![Ok("first"), Err("second"), Ok("third")]);
        assert_eq!(
            ordered.iter().position(|r| r.is_err()),
            Some(1),
            "the first error is the first in selection order"
        );
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
