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
