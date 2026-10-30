//! Build local-chunk reuse map: path -> [`FileReuse`].
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

use crate::sophon::proto::{SophonChunk, WantedManifest};
use crate::util;

/// One reuse candidate: a slice of the local file that may hold a chunk's
/// bytes, so it can be copied instead of downloaded.
///
/// The identity consumers ask about is `(md5, size)`; `offset` is where the
/// bytes live. The MD5 is normalized to lowercase once, here at the map's
/// boundary (see [`build_local_chunk_map`]), while [`ReuseEntry::matches`]
/// still compares case-insensitively so an entry built elsewhere is not
/// spelling-sensitive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReuseEntry {
    /// The chunk's uncompressed MD5, lowercase.
    pub md5: String,
    /// The chunk's uncompressed size.
    pub size: i64,
    /// Where the chunk's bytes live in the local file.
    pub offset: i64,
}

impl ReuseEntry {
    /// Does this entry hold `c`'s bytes?
    ///
    /// This is the one chunk-identity predicate: [`FileReuse`]'s index, the
    /// plan estimate, the in-place source lookup and the temp path's reuse
    /// lookup all agree with it, so "the same chunk" cannot mean different
    /// things there.
    pub fn matches(&self, c: &SophonChunk) -> bool {
        self.size == c.uncompressed_size && self.md5.eq_ignore_ascii_case(&c.uncompressed_md5)
    }
}

/// One file's reuse entries, in first-match order, plus an index by chunk
/// identity for the hot lookup path.
///
/// The vector is the deterministic source of truth: [`Self::get`] answers from
/// `by_key`, which points at the **first** entry for a `(md5, size)` key, so
/// the index cannot change which entry wins. The key is normalized to lowercase
/// here, which keeps lookups case-insensitive even for an entry built by hand.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileReuse {
    /// Every distinct chunk identity, in the order it first appeared.
    entries: Vec<ReuseEntry>,
    /// `(lowercase md5, size) -> index into `entries`, first insertion wins.
    by_key: HashMap<(String, i64), usize>,
}

impl FileReuse {
    /// Index `entries`, keeping the first entry for each chunk identity.
    pub fn new(entries: Vec<ReuseEntry>) -> Self {
        let mut reuse = Self::default();
        for entry in entries {
            reuse.insert(entry);
        }
        reuse
    }

    /// Add `entry` unless its chunk identity is already present.
    fn insert(&mut self, entry: ReuseEntry) {
        let key = Self::key(&entry.md5, entry.size);
        if !self.by_key.contains_key(&key) {
            self.by_key.insert(key, self.entries.len());
            self.entries.push(entry);
        }
    }

    /// The entry holding `c`'s bytes, or `None` when the chunk must download.
    pub fn get(&self, c: &SophonChunk) -> Option<&ReuseEntry> {
        let key = Self::key(&c.uncompressed_md5, c.uncompressed_size);
        let entry = self.by_key.get(&key).map(|&i| &self.entries[i])?;
        debug_assert!(
            entry.matches(c),
            "reuse index disagrees with the chunk-identity predicate"
        );
        Some(entry)
    }

    /// Does any entry hold `c`'s bytes?
    pub fn contains(&self, c: &SophonChunk) -> bool {
        self.get(c).is_some()
    }

    /// The entries in first-match order.
    pub fn entries(&self) -> &[ReuseEntry] {
        &self.entries
    }

    fn key(md5: &str, size: i64) -> (String, i64) {
        (md5.to_ascii_lowercase(), size)
    }
}

/// Build local-chunk reuse map: path -> indexed reuse entries.
pub fn build_local_chunk_map(manifests: &[WantedManifest]) -> HashMap<String, FileReuse> {
    let mut map: HashMap<String, FileReuse> = HashMap::new();
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
                // Keep first occurrence per chunk: the MD5 is normalized here,
                // so two spellings of the same digest cannot both survive.
                e.insert(ReuseEntry {
                    md5: c.uncompressed_md5.to_ascii_lowercase(),
                    size: c.uncompressed_size,
                    offset: c.offset,
                });
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
        let e = map["a/b.dat"].entries();
        assert_eq!(e.len(), 2);
        assert_eq!(
            e[0],
            ReuseEntry {
                md5: "d41d8cd98f00b204e9800998ecf8427e".to_string(),
                size: 20,
                offset: 0,
            }
        );
        assert_eq!(
            e[1],
            ReuseEntry {
                md5: "0cc175b9c0f1b6a831c399e269772661".to_string(),
                size: 20,
                offset: 20,
            }
        );
        // folders never enter the reuse map
        assert!(!map.contains_key("dir/"));
    }

    /// The map's dedup and the consumers' lookup share one predicate, so a
    /// later spelling of the same digest must not survive beside the first:
    /// the MD5 is normalized (lowercased) at this boundary, once.
    #[test]
    fn local_chunk_map_collapses_case_variants_and_keeps_the_first() {
        let wm = vec![WantedManifest {
            meta: meta("game"),
            files: vec![SophonChunkFile {
                file: "a/b.dat".into(),
                chunks: vec![
                    chunk("c1", "D41D8CD98F00B204E9800998ECF8427E", 0, 20),
                    // the same chunk spelled in lowercase: dropped, first wins
                    chunk("c1dup", "d41d8cd98f00b204e9800998ecf8427e", 999, 20),
                ],
                is_folder: false,
                size: 40,
                md5: "m1".into(),
            }],
        }];
        let map = build_local_chunk_map(&wm);
        let entries = map["a/b.dat"].entries();
        assert_eq!(entries.len(), 1, "case variants name the same chunk");
        assert_eq!(entries[0].md5, "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(entries[0].offset, 0, "the first entry is kept");
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
