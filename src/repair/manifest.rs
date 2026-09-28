//! Manifest selection, fetch, and the local chunk-reuse map (docs/02 step 3).
//!
//! Two things come out of here:
//! 1. the per-manifest file lists of the **latest** build (the plan's input),
//! 2. `path -> [(md5, size, offset)]` for the **local** build, so an
//!    unchanged chunk can be copied from the file already on disk instead of
//!    downloaded.
//!
//! Manifests are never cached inside `game_dir` (V1-SIMPLIFICATION S1: the whole
//! blob is buffered and parsed in memory) — see [`crate::sophon::fetch_manifest`].

use std::collections::HashMap;

use anyhow::Context;
use reqwest::Client;

use crate::hyp::ChunkBuild;
use crate::sophon::{self, SophonChunkFile, WantedManifest};

use super::RunFailure;

/// `path -> [(uncompressed_md5, uncompressed_size, offset)]`, built from the
/// local build's manifests. Keyed by path only: reuse is same-file-only (S2).
pub type LocalChunkMap = HashMap<String, Vec<(String, i64, i64)>>;

/// Fetch every wanted manifest of the latest build, in order, keeping each
/// one's chunk URL prefix next to its file list (the plan needs the pairing).
pub async fn fetch_latest(
    http: &Client,
    latest_build: &ChunkBuild,
    audio_langs: &std::collections::HashSet<String>,
    ignore: &std::collections::HashSet<String>,
) -> Result<Vec<(String, Vec<SophonChunkFile>)>, RunFailure> {
    let mut per_manifest: Vec<(String, Vec<SophonChunkFile>)> = Vec::new();
    for m in sophon::select_manifests(latest_build, audio_langs, ignore) {
        let files = sophon::fetch_manifest(http, m)
            .await
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
    local_build: Option<&ChunkBuild>,
    audio_langs: &std::collections::HashSet<String>,
    ignore: &std::collections::HashSet<String>,
) -> LocalChunkMap {
    let Some(local_build) = local_build else {
        return LocalChunkMap::new();
    };
    let mut wanted: Vec<WantedManifest> = Vec::new();
    for m in sophon::select_manifests(local_build, audio_langs, ignore) {
        match sophon::fetch_manifest(http, m).await {
            Ok(files) => wanted.push(WantedManifest {
                meta: m.clone(),
                files,
            }),
            Err(e) => tracing::warn!("local manifest {} failed: {}", m.matching_field, e),
        }
    }
    sophon::build_local_chunk_map(&wanted)
}
