use anyhow::{Context, Result};
use prost::Message;
use std::collections::{HashMap, HashSet};

use crate::hyp::{ChunkBuild, ChunkManifestMeta};

/// Sophon chunk manifest protobuf (Starward Sophon.proto parity).
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
pub async fn fetch_manifest(client: &reqwest::Client, meta: &ChunkManifestMeta) -> Result<Vec<SophonChunkFile>> {
    let url = join_url(&meta.manifest_download.url_prefix, &meta.manifest.id);
    let mut last_err = anyhow::anyhow!("no attempts");
    for attempt in 1..=5u64 {
        match try_fetch_manifest(client, &url, meta).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = e;
                tokio::time::sleep(std::time::Duration::from_secs(attempt)).await;
            }
        }
    }
    Err(last_err)
}

async fn try_fetch_manifest(
    client: &reqwest::Client,
    url: &str,
    meta: &ChunkManifestMeta,
) -> Result<Vec<SophonChunkFile>> {
    tracing::debug!(manifest = %meta.manifest.id, field = %meta.matching_field, url = %url, "GET manifest");
    let bytes = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET manifest {}", meta.manifest.id))?
        .error_for_status()?
        .bytes()
        .await?;
    tracing::debug!(manifest = %meta.manifest.id, compressed_bytes = bytes.len(), "manifest downloaded");
    // Entire blob is zstd-compressed; checksum = MD5 of DECOMPRESSED bytes.
    let decoded = zstd::stream::decode_all(std::io::Cursor::new(&bytes))
        .context("zstd decompress manifest")?;
    let digest = format!("{:x}", md5::compute(&decoded));
    tracing::debug!(manifest = %meta.manifest.id, decompressed_bytes = decoded.len(), expect_md5 = %meta.manifest.checksum, actual_md5 = %digest, "manifest checksum check");
    if !digest.eq_ignore_ascii_case(&meta.manifest.checksum) {
        anyhow::bail!(
            "manifest checksum mismatch id={} expect={} got={}",
            meta.manifest.id,
            meta.manifest.checksum,
            digest
        );
    }
    let m = SophonChunkManifest::decode(decoded.as_slice()).context("protobuf decode manifest")?;
    let (files, chunks): (usize, usize) = (m.chuncks.len(), m.chuncks.iter().map(|f| f.chunks.len()).sum());
    tracing::debug!(manifest = %meta.manifest.id, field = %meta.matching_field, files, chunks, "manifest parsed");
    Ok(m.chuncks)
}

/// Download one chunk's compressed bytes.
pub async fn fetch_chunk_bytes(client: &reqwest::Client, url_prefix: &str, id: &str) -> Result<Vec<u8>> {
    let url = join_url(url_prefix, id);
    let mut last_err = anyhow::anyhow!("no attempts");
    for attempt in 1..=5u64 {
        match client.get(&url).send().await {
            Ok(r) => match r.error_for_status() {
                Ok(r) => match r.bytes().await {
                    Ok(b) => {
                        tracing::trace!(chunk = %id, compressed_bytes = b.len(), attempt, "chunk downloaded");
                        return Ok(b.to_vec());
                    }
                    Err(e) => last_err = anyhow::anyhow!("read chunk body: {}", e),
                },
                Err(e) => last_err = anyhow::anyhow!("chunk status: {}", e),
            },
            Err(e) => last_err = anyhow::anyhow!("GET chunk: {}", e),
        }
        tracing::debug!(chunk = %id, attempt, "chunk fetch failed: {}", last_err);
        tokio::time::sleep(std::time::Duration::from_secs(attempt)).await;
    }
    Err(last_err)
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
                if !e.iter().any(|(md5, sz, _)| md5 == &c.uncompressed_md5 && *sz == c.uncompressed_size) {
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
}
