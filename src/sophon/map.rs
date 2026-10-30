//! Build local-chunk reuse map: path -> list of (uncompressed_md5, uncompressed_size, offset).
//!
//! The key is the **cleaned** rel — the same string `build_plan` stores in
//! `PlannedFile::rel` — because every consumer (`est_download_bytes_by_file`,
//! the scan, the temp and in-place writers) looks the file up by that spelling.
//! Keying on the raw local-manifest spelling would lose reuse for a `./`- or
//! `//`-spelled local manifest while the map's whole purpose is to find the
//! bytes already on disk. Two raw spellings that clean to one key merge, which
//! is the intended "first occurrence per chunk" behaviour.
//!
//! The local manifest is best effort (`local_reuse_map`'s contract): an entry
//! that cannot be cleaned is skipped with a warning rather than failing the
//! run, because this map only ever saves downloads.
use std::collections::HashMap;

use crate::sophon::proto::WantedManifest;
use crate::util;

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
            let key = match util::clean_rel(&f.file) {
                Ok(key) => key,
                Err(e) => {
                    tracing::warn!(
                        file = %f.file,
                        error = %format!("{e:#}"),
                        "local reuse map: skipping a non-contained path"
                    );
                    continue;
                }
            };
            let e = map.entry(key).or_default();
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
    use crate::hyp::ChunkManifestMeta;
    use crate::sophon::proto::{SophonChunk, SophonChunkFile, WantedManifest};

    fn meta(field: &str) -> ChunkManifestMeta {
        ChunkManifestMeta {
            category_id: field.to_string(),
            category_name: field.to_string(),
            matching_field: field.to_string(),
            manifest: Default::default(),
            chunk_download: Default::default(),
            manifest_download: Default::default(),
        }
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

    /// V-3: the plan stores the cleaned rel (`build_plan` calls `clean_rel`),
    /// and every reuse consumer looks the file up by that string. A key built
    /// from the raw manifest spelling (`./game/foo.dat`) would never be hit, so
    /// a non-canonical local manifest would lose all reuse. The key is cleaned
    /// here, once, at the map's boundary; a path that cannot be cleaned is
    /// skipped like any other best-effort local-manifest degradation.
    #[test]
    fn local_chunk_map_keys_are_the_cleaned_plan_spelling() {
        let wm = vec![WantedManifest {
            meta: meta("game"),
            files: vec![
                SophonChunkFile {
                    file: "./game/foo.dat".into(),
                    chunks: vec![chunk("c1", "d41d8cd98f00b204e9800998ecf8427e", 0, 20)],
                    is_folder: false,
                    size: 20,
                    md5: "m1".into(),
                },
                SophonChunkFile {
                    file: "../escape.dat".into(),
                    chunks: vec![chunk("c2", "0cc175b9c0f1b6a831c399e269772661", 0, 20)],
                    is_folder: false,
                    size: 20,
                    md5: "m2".into(),
                },
            ],
        }];
        let map = build_local_chunk_map(&wm);
        assert!(
            map.contains_key("game/foo.dat"),
            "the key must be the plan's cleaned spelling: {:?}",
            map.keys().collect::<Vec<_>>()
        );
        assert!(!map.contains_key("./game/foo.dat"));
        assert!(
            map.keys().all(|k| k != "../escape.dat"),
            "a non-contained path must not enter the map"
        );
        assert_eq!(map.len(), 1);
    }
}
