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
use std::hash::{DefaultHasher, Hasher};

use crate::sophon::proto::{SophonChunk, WantedManifest};
use crate::util::{self, Md5};

/// One reuse candidate: a slice of the local file that may hold a chunk's
/// bytes, so it can be copied instead of downloaded.
///
/// The identity consumers ask about is `(md5, size)`; `offset` is where the
/// bytes live. The MD5 is parsed into its 16 bytes once, here at the map's
/// boundary (see [`build_local_chunk_map`]), so [`ReuseEntry::matches`] is a
/// byte comparison and an entry built elsewhere is not spelling-sensitive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReuseEntry {
    /// The chunk's uncompressed MD5.
    pub md5: Md5,
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
        Md5::parse_hex(&c.uncompressed_md5)
            .is_ok_and(|md5| self.matches_identity(md5, c.uncompressed_size))
    }

    /// The same predicate without a chunk to point at, for the index's insert
    /// path: an entry is a duplicate when its digest and size name the same
    /// chunk.
    fn matches_identity(&self, md5: Md5, size: i64) -> bool {
        self.size == size && self.md5 == md5
    }
}

/// One file's reuse entries, in first-match order, plus an index by chunk
/// identity for the hot lookup path.
///
/// The vector is the deterministic source of truth: [`Self::get`] answers from
/// `by_key`, which keeps the indices of every entry whose `(md5, size)` key
/// hashes alike, in first-insertion order. Each candidate is confirmed with
/// [`ReuseEntry::matches`], so a hash collision cannot change which entry wins,
/// and the first matching entry is the same one the old linear scan chose. The
/// key is a hash of the digest bytes, which needs no case folding: the digest
/// was already parsed at the map's boundary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileReuse {
    /// Every distinct chunk identity, in the order it first appeared.
    entries: Vec<ReuseEntry>,
    /// `(hash of the md5, size) -> indices into `entries`, in insertion
    /// order. The hash only selects candidates; `get` confirms them, so a
    /// collision can never pick a wrong entry.
    by_key: HashMap<(u64, i64), Vec<usize>>,
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
        let key = Self::key(entry.md5, entry.size);
        let duplicate = self.by_key.get(&key).is_some_and(|candidates| {
            candidates
                .iter()
                .any(|&i| self.entries[i].matches_identity(entry.md5, entry.size))
        });
        if duplicate {
            return;
        }
        self.by_key.entry(key).or_default().push(self.entries.len());
        self.entries.push(entry);
    }

    /// The entry holding `c`'s bytes, or `None` when the chunk must download.
    pub fn get(&self, c: &SophonChunk) -> Option<&ReuseEntry> {
        let md5 = Md5::parse_hex(&c.uncompressed_md5).ok()?;
        let key = Self::key(md5, c.uncompressed_size);
        self.by_key
            .get(&key)?
            .iter()
            .map(|&i| &self.entries[i])
            .find(|entry| entry.matches(c))
    }

    /// Does any entry hold `c`'s bytes?
    pub fn contains(&self, c: &SophonChunk) -> bool {
        self.get(c).is_some()
    }

    /// The entries in first-match order.
    pub fn entries(&self) -> &[ReuseEntry] {
        &self.entries
    }

    /// The lookup key: a hash of the digest bytes, plus the size.
    ///
    /// Deliberately **not** a `String`: the digest was parsed once at the
    /// boundary, and the hot path (`sources_for` asks once per chunk) hashes
    /// the 16 bytes it already has. `size` is returned separately so the
    /// digest hash never has to encode it.
    fn key(md5: Md5, size: i64) -> (u64, i64) {
        let mut hasher = DefaultHasher::new();
        hasher.write(md5.as_bytes());
        (hasher.finish(), size)
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
                // Keep first occurrence per chunk: parsing the digest here
                // makes two spellings the same value, so they cannot both
                // survive. A string that is not a digest is not a reuse
                // candidate at all — the chunk degrades to a download.
                let Ok(md5) = Md5::parse_hex(&c.uncompressed_md5) else {
                    continue;
                };
                e.insert(ReuseEntry {
                    md5,
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

    fn digest(hex: &str) -> Md5 {
        Md5::parse_hex(hex).unwrap()
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
                md5: digest("d41d8cd98f00b204e9800998ecf8427e"),
                size: 20,
                offset: 0,
            }
        );
        assert_eq!(
            e[1],
            ReuseEntry {
                md5: digest("0cc175b9c0f1b6a831c399e269772661"),
                size: 20,
                offset: 20,
            }
        );
        // folders never enter the reuse map
        assert!(!map.contains_key("dir/"));
    }

    /// A2: the hot key must be spelling-insensitive and keep size distinct
    /// without building a `String` per lookup.
    #[test]
    fn the_lookup_key_is_case_insensitive_and_size_sensitive() {
        assert_eq!(
            FileReuse::key(digest("D41D8CD98F00B204E9800998ECF8427E"), 20),
            FileReuse::key(digest("d41d8cd98f00b204e9800998ecf8427e"), 20)
        );
        assert_ne!(
            FileReuse::key(digest("d41d8cd98f00b204e9800998ecf8427e"), 20),
            FileReuse::key(digest("d41d8cd98f00b204e9800998ecf8427e"), 21)
        );
    }

    /// A2: the index is only a bucket. Even if two entries land under the same
    /// `(hash, size)` key, `get` must confirm each candidate with the chunk
    /// identity predicate instead of trusting the first index.
    #[test]
    fn a_hash_collision_falls_through_to_the_matching_entry() {
        let asked = chunk("asked", "d41d8cd98f00b204e9800998ecf8427e", 0, 20);
        let wrong = ReuseEntry {
            md5: digest("0cc175b9c0f1b6a831c399e269772661"),
            size: 20,
            offset: 1,
        };
        let right = ReuseEntry {
            md5: digest("D41D8CD98F00B204E9800998ECF8427E"),
            size: 20,
            offset: 2,
        };
        let key = FileReuse::key(digest(&asked.uncompressed_md5), asked.uncompressed_size);
        let reuse = FileReuse {
            entries: vec![wrong, right.clone()],
            by_key: std::collections::HashMap::from([(key, vec![0, 1])]),
        };
        assert_eq!(
            reuse.get(&asked),
            Some(&right),
            "the bucket's first entry is not a match; the collision must fall through"
        );
    }

    /// The map's dedup and the consumers' lookup share one predicate, so a
    /// later spelling of the same digest must not survive beside the first:
    /// the MD5 is parsed at this boundary, once.
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
        assert_eq!(entries[0].md5, digest("d41d8cd98f00b204e9800998ecf8427e"));
        assert_eq!(entries[0].offset, 0, "the first entry is kept");
    }

    /// A string that is not a digest (an empty MD5 above all) is "unknown",
    /// not a reuse identity: it must not enter the map, and the chunk it named
    /// degrades to a download.
    #[test]
    fn a_non_digest_never_becomes_a_reuse_candidate() {
        let wm = vec![WantedManifest {
            meta: meta("game"),
            files: vec![SophonChunkFile {
                file: "a/b.dat".into(),
                chunks: vec![chunk("empty", "", 0, 20), chunk("label", "md5-a", 20, 20)],
                is_folder: false,
                size: 40,
                md5: "m1".into(),
            }],
        }];
        let map = build_local_chunk_map(&wm);
        assert!(map["a/b.dat"].entries().is_empty());
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
