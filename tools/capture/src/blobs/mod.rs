//! The content-addressed chunk store and the three retention sets.
//!
//! **M6.** Blobs are stored once by chunk id however many files or both builds
//! reference them. The three sets — `materialize_old`, `materialize_new`,
//! `patch_fetch` — are what make the bundle closed: nothing in it may depend on
//! a blob it did not retain.
//!
//! ## What the store does, and what it deliberately does not
//!
//! It fetches each distinct chunk id once, measures it against the manifest's
//! declaration ([`props`]), writes the bytes exactly as served, and records who
//! wanted them ([`chunks`]). It does **not** decide whether a file can be
//! assembled — that is M7 — and it does not fail the run on a blob that
//! disagrees with its manifest, because a capture that hit a truncated response
//! is the evidence this tool exists to keep.
//!
//! ## Writing
//!
//! Every blob goes to `tmp/` first and is renamed into `chunks/blobs/`. A
//! half-written `.zst` that a reader finds under its final name is worse than no
//! blob at all: `verify` would report a digest failure for a file that simply
//! had not finished arriving, and the two demand opposite responses.
//!
//! The content address is the chunk id, so a second capture that reuses the same
//! chunks writes byte-identical files at identical paths. That is what makes
//! `materialize` offline in M9 a matter of reading the bundle rather than
//! re-deriving anything.

pub mod chunks;
pub mod props;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::http::{ApiError, Fetcher, Kind};
use crate::manifest::CapturedManifest;
use chunks::{FilePair, Reference, Set, Want};
use props::BlobProps;

/// `chunks/index.json`, the bundle's own table of contents for its blobs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChunkIndex {
    /// The artifact schema version.
    pub schema_version: u32,
    /// One entry per distinct chunk id, ordered by id.
    pub blobs: Vec<IndexedBlob>,
    /// The three role lists, each a list of chunk ids.
    pub sets: BTreeMap<String, Vec<String>>,
}

/// One blob as `chunks/index.json` lists it: the measurement plus who wanted it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IndexedBlob {
    /// The measurement.
    #[serde(flatten)]
    pub props: BlobProps,
    /// Every file and role that wanted these bytes.
    pub referenced_by: Vec<Reference>,
    /// The distinct roles it is retained for.
    pub sets: Vec<Set>,
}

impl ChunkIndex {
    /// The index for a store's blobs, with the role lists derived from them.
    ///
    /// Derived rather than tracked separately, because two lists that are
    /// *meant* to agree and are written by different code is how an index comes
    /// to claim a blob is in `patch_fetch` while no reference says so.
    pub fn build(entries: Vec<(IndexedBlob, Vec<Set>)>) -> ChunkIndex {
        let mut sets: BTreeMap<String, Vec<String>> = Set::ALL
            .iter()
            .map(|s| (s.id().to_string(), Vec::new()))
            .collect();
        let mut blobs = Vec::with_capacity(entries.len());
        for (blob, roles) in entries {
            for role in &roles {
                sets
                    .entry(role.id().to_string())
                    .or_default()
                    .push(blob.props.chunk_id.clone());
            }
            blobs.push(blob);
        }
        ChunkIndex {
            schema_version: crate::SCHEMA_VERSION,
            blobs,
            sets,
        }
    }

    /// The blob entry for an id.
    pub fn get(&self, chunk_id: &str) -> Option<&IndexedBlob> {
        self.blobs.iter().find(|b| b.props.chunk_id == chunk_id)
    }

    /// Every blob that failed at least one A6 check.
    pub fn failing(&self) -> Vec<&IndexedBlob> {
        self.blobs
            .iter()
            .filter(|b| !b.props.failures().is_empty() || b.props.decode_error.is_some())
            .collect()
    }
}

/// Where a tag's chunks are served from.
#[derive(Debug, Clone)]
pub struct Sources {
    bases: BTreeMap<String, String>,
}

impl Sources {
    /// Build from the manifests the capture already fetched.
    ///
    /// From `CapturedManifest` rather than from the raw `getBuild` body so a
    /// chunk URL is built by exactly the same code the archive URL was.
    pub fn from_manifests<'a>(manifests: impl IntoIterator<Item = &'a CapturedManifest>) -> Sources {
        let mut bases = BTreeMap::new();
        for m in manifests {
            bases.insert(m.tag.clone(), m.chunk_url_prefix().to_string());
        }
        Sources { bases }
    }

    /// Build from explicit `tag -> base` pairs, for a test with no manifests.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Sources {
        Sources {
            bases: pairs.into_iter().collect(),
        }
    }

    /// The URL one chunk of one build is served from.
    pub fn url(&self, tag: &str, chunk_id: &str) -> Result<String, ApiError> {
        let base = self.bases.get(tag).ok_or_else(|| {
            ApiError::shape(
                "chunk",
                format!("no chunk source registered for tag {tag:?}"),
            )
        })?;
        Ok(format!("{base}/{chunk_id}"))
    }

    /// The tags that can serve chunks.
    pub fn tags(&self) -> Vec<&str> {
        self.bases.keys().map(String::as_str).collect()
    }
}

/// The store: fetch each distinct chunk once, measure it, write it, index it.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
    sources: Sources,
}

impl Store {
    /// The chunk sources, for a caller that has to build a URL itself.
    pub fn sources(&self) -> &Sources {
        &self.sources
    }

    /// A store writing under `root`, fetching from `sources`.
    ///
    /// Owns its root rather than borrowing it so [`Store::fill`] can spawn
    /// tasks: a `&Path` that has to outlive the call is a lifetime the runtime
    /// would have to invent, and a `Store` is two small allocations to clone.
    pub fn new(root: &Path, sources: Sources) -> Store {
        Store {
            root: root.to_path_buf(),
            sources,
        }
    }

    /// Fetch and store everything `wants` asks for.
    ///
    /// `jobs` bounds concurrency. Results come back in `wants` order regardless
    /// of which download finished first, so a capture with `--jobs 8` and one
    /// with `--jobs 1` produce the same index.
    ///
    /// One failure does not abandon the rest: each fetch's error is recorded on
    /// its blob and the store returns the index either way. A bundle with seven
    /// good blobs and one failed fetch is far more useful than no bundle, and
    /// `run` decides whether the gap is fatal.
    pub async fn fill(
        &self,
        fetcher: &Fetcher,
        wants: &[Want],
        jobs: usize,
    ) -> (Vec<(IndexedBlob, Vec<Set>)>, Vec<String>) {
        let mut out: Vec<Option<(IndexedBlob, Vec<Set>)>> = (0..wants.len()).map(|_| None).collect();
        let mut errors: Vec<String> = Vec::new();
        let mut next = 0_usize;
        let mut set = tokio::task::JoinSet::new();

        // Prime the set, then keep one job in flight per worker. A plain
        // `for want in wants` would serialise; this does not need `futures`.
        while next < wants.len() && set.len() < jobs.max(1) {
            let i = next;
            next += 1;
            set.spawn(Store::one(self.clone(), i, fetcher.clone(), wants[i].clone()));
        }
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(Ok((i, entry))) => out[i] = Some(entry),
                Ok(Err(e)) => errors.push(e),
                // A panicking task. Recorded rather than propagated: this is
                // bookkeeping around network I/O, and losing the whole index to
                // one thread would be worse than a hole in it.
                Err(_) => errors.push("a chunk fetch task panicked".to_string()),
            }
            if next < wants.len() {
                let i = next;
                next += 1;
                set.spawn(Store::one(self.clone(), i, fetcher.clone(), wants[i].clone()));
            }
        }

        // A `None` slot is a fetch that neither succeeded nor reported, which
        // can only be the panic above. Fill it so the index has no holes and
        // the count of stored blobs is honest.
        let missing: Vec<String> = out
            .iter()
            .enumerate()
            .filter(|(_, o)| o.is_none())
            .map(|(i, _)| wants[i].chunk.id.clone())
            .collect();
        for (i, o) in out.iter_mut().enumerate() {
            if o.is_none() {
                *o = Some(self.unfilled(&wants[i]));
            }
        }
        (out.into_iter().flatten().collect(), [errors, missing].concat())
    }

    /// The index for a run that stored no blobs at all (`--no-chunk-blobs`).
    ///
    /// The role lists are still written, because knowing which chunks *would*
    /// have been needed is the part of the document that survives, and an index
    /// that simply vanished would take that with it.
    pub fn plan_only(wants: &[Want]) -> Vec<(IndexedBlob, Vec<Set>)> {
        wants
            .iter()
            .map(|w| {
                (
                    IndexedBlob {
                        props: BlobProps::unmeasured(&w.chunk),
                        referenced_by: w.references.clone(),
                        sets: w.sets.clone(),
                    },
                    w.sets.clone(),
                )
            })
            .collect()
    }

    /// Fetch one want and write it.
    ///
    /// Takes the store by value rather than by reference so the future is
    /// 'static and can run on the runtime's threads.
    async fn one(
        this: Store,
        slot: usize,
        fetcher: Fetcher,
        want: Want,
    ) -> Result<(usize, (IndexedBlob, Vec<Set>)), String> {
        let self_ = &this;
        let mut last: Option<String> = None;
        // Sources are ordered newest-first, so the new build's CDN is asked
        // before the old one's, which is the one that may have stopped serving.
        for src in &want.sources {
            let url = self_
                .sources
                .url(&src.tag, &want.chunk.id)
                .map_err(|e| e.to_string())?;
            match fetcher.get(&url, Kind::Chunk, None).await {
                Ok(fetched) => {
                    let mut props = BlobProps::measure(&fetched.body, &want.chunk)
                        .map_err(|e| e.to_string())?;
                    let rel = BlobProps::stored_path_for(&want.chunk.id);
                    self_.write(&rel, &fetched.body)
                        .map_err(|e| format!("{}: write: {e}", want.chunk.id))?;
                    props.set_stored(rel.clone());
                    // Provenance: which request produced these bytes.
                    fetcher
                        .log()
                        .set_stored_as(fetched.seq(), rel.clone());
                    if !want.conflicts.is_empty() {
                        tracing::warn!(
                            chunk = %want.chunk.id,
                            detail = %want.describe_conflicts(),
                            "two builds declare this chunk id with different content; \
                             measured against the first",
                        );
                    }
                    return Ok((
                        slot,
                        (
                            IndexedBlob {
                                props,
                                referenced_by: want.references,
                                sets: want.sets.clone(),
                            },
                            want.sets,
                        ),
                    ));
                }
                Err(e) => {
                    tracing::debug!(chunk = %want.chunk.id, source = %src.tag, error = %e, "chunk fetch failed");
                    last = Some(format!("{} via {}: {e}", want.chunk.id, src.tag));
                }
            }
        }
        Err(last.unwrap_or_else(|| format!("{}: no source to fetch from", want.chunk.id)))
    }

    /// The index entry for a want whose bytes never arrived.
    ///
    /// Not `BlobProps::unmeasured` — that is for `--no-chunk-blobs`, where
    /// nothing was *asked* for. Here something was asked for and did not come,
    /// and the difference is the whole point of the row.
    fn unfilled(&self, want: &Want) -> (IndexedBlob, Vec<Set>) {
        let mut props = BlobProps::unmeasured(&want.chunk);
        props.decode_error = Some("fetched but not stored".to_string());
        (
            IndexedBlob {
                props,
                referenced_by: want.references.clone(),
                sets: want.sets.clone(),
            },
            want.sets.clone(),
        )
    }

    /// Write one blob, via `tmp/`, into `chunks/blobs/`.
    fn write(&self, rel: &str, bytes: &[u8]) -> std::io::Result<()> {
        let final_path = self.root.join(rel);
        let dir = final_path.parent().expect("a blob path has a parent");
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(
            ".{}.tmp",
            final_path.file_name().unwrap_or_default().to_string_lossy()
        ));
        std::fs::write(&tmp, bytes)?;
        match std::fs::rename(&tmp, &final_path) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }
}

/// Every file pair the selection admitted, given as manifest entries.
///
/// Pure, so the retention plan can be checked before a single byte is fetched:
/// `--max-download-bytes` in the run log is an upper bound, and it is only an
/// upper bound if this much really does get downloaded.
///
/// A path absent from a manifest yields a one-sided pair rather than being
/// skipped. `--files` may name a path that only the new build has, and the
/// honest shape for that is "no old side", which is what [`FilePair`] already
/// models for `added`.
pub fn file_pairs(
    selection: &crate::select::Selection,
    manifests: &BTreeMap<String, &CapturedManifest>,
    old_tag: &str,
    new_tag: &str,
) -> Vec<FilePair> {
    selection
        .admitted
        .iter()
        .map(|a| {
            let find = |tag: &str| -> Option<&crate::manifest::proto::SophonChunkFile> {
                manifests
                    .get(tag)
                    .and_then(|m| m.entries.iter().find(|e| e.file == a.path))
            };
            FilePair {
                capture_id: a.capture_id.clone(),
                path: a.path.clone(),
                old: find(old_tag).map(|e| chunks::OldFile {
                    tag: old_tag.to_string(),
                    entry: e.clone(),
                }),
                new: find(new_tag).map(|e| chunks::NewFile {
                    tag: new_tag.to_string(),
                    entry: e.clone(),
                }),
            }
        })
        .collect()
}

/// What a selection would cost the CDN, in bytes, if every wanted chunk were
/// fetched once.
///
/// `Σ compressed_size` over the distinct wants, which is the *transfer* cost and
/// not the fixture size — the figure `--max-download-bytes` is checked against
/// lives in M5 and is the other one.
pub fn projected_download(wants: &[Want]) -> u64 {
    wants
        .iter()
        .map(|w| w.chunk.compressed_size.max(0) as u64)
        .sum()
}

/// A one-line summary for the run log.
///
/// Counts *stored* blobs, not index rows: under `--no-chunk-blobs` the index
/// still lists every wanted chunk with its roles, and a log line reading
/// "38 stored" beside a `chunks/blobs/` directory that does not exist is worse
/// than no line at all.
pub fn describe(wants: &[Want], index: &ChunkIndex) -> String {
    let stored = index
        .blobs
        .iter()
        .filter(|b| b.props.stored_path.is_some())
        .count();
    format!(
        "{} distinct chunks, {} B to fetch, {} of {} stored, {} role references, \
         {} failing A6",
        wants.len(),
        projected_download(wants),
        stored,
        index.blobs.len(),
        index.blobs.iter().map(|b| b.referenced_by.len()).sum::<usize>(),
        index.failing().len(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash;
    use crate::manifest::proto::{SophonChunk, SophonChunkFile};

    fn chunk(payload: &[u8], offset: i64, prefix: u64) -> SophonChunk {
        let raw = zstd::encode_all(payload, 1).expect("encode");
        SophonChunk {
            id: format!("{prefix:016x}_{}", hash::md5_hex(payload)),
            uncompressed_md5: hash::md5_hex(payload),
            offset,
            compressed_size: raw.len() as i64,
            uncompressed_size: payload.len() as i64,
            unknown_field6: 7,
            compressed_md5: hash::md5_hex(&raw),
        }
    }

    fn entry(path: &str, chunks: Vec<SophonChunk>) -> SophonChunkFile {
        let size = chunks.iter().map(|c| c.uncompressed_size).sum();
        SophonChunkFile {
            file: path.into(),
            chunks,
            is_folder: false,
            size,
            md5: "ff".repeat(16),
        }
    }

    fn pair() -> FilePair {
        FilePair {
            capture_id: "partial_same_offset".into(),
            path: "dll".into(),
            old: Some(chunks::OldFile {
                tag: "7.0.0".into(),
                entry: entry(
                    "dll",
                    vec![
                        chunk(b"old-zero", 0, 0x7d3f),
                        chunk(b"shared", 10, 0xf5dc),
                        chunk(b"old-two", 20, 0xa431),
                    ],
                ),
            }),
            new: Some(chunks::NewFile {
                tag: "7.1.0".into(),
                entry: entry(
                    "dll",
                    vec![
                        chunk(b"new-zero", 0, 0xddb3),
                        chunk(b"shared", 10, 0xf5dc),
                        chunk(b"new-two", 20, 0x340c),
                    ],
                ),
            }),
        }
    }

    fn sources() -> Sources {
        Sources::from_pairs([
            ("7.0.0".to_string(), "https://cdn/old".to_string()),
            ("7.1.0".to_string(), "https://cdn/new".to_string()),
        ])
    }

    #[test]
    fn the_index_role_lists_are_derived_from_the_blobs_that_carry_them() {
        let w = chunks::wants(&[pair()]);
        let entries: Vec<(IndexedBlob, Vec<Set>)> = w
            .iter()
            .map(|x| {
                (
                    IndexedBlob {
                        props: BlobProps::unmeasured(&x.chunk),
                        referenced_by: x.references.clone(),
                        sets: x.sets.clone(),
                    },
                    x.sets.clone(),
                )
            })
            .collect();
        let idx = ChunkIndex::build(entries);
        assert_eq!(idx.schema_version, crate::SCHEMA_VERSION);
        assert_eq!(idx.sets.len(), 3);
        assert_eq!(idx.sets["materialize_old"].len(), 3);
        assert_eq!(idx.sets["materialize_new"].len(), 3);
        assert_eq!(idx.sets["patch_fetch"].len(), 2);
        // Every id in a role list is a blob in the index, and vice versa: a list
        // naming an absent blob would send M7 looking for bytes that are not there.
        for (role, ids) in &idx.sets {
            for id in ids {
                let b = idx.get(id).unwrap_or_else(|| panic!("{role} names a missing blob {id}"));
                assert!(
                    b.sets.iter().any(|s| s.id() == role),
                    "{id} is in the {role} list but its entry does not say so"
                );
            }
        }
        for b in &idx.blobs {
            for s in &b.sets {
                assert!(
                    idx.sets[s.id()].contains(&b.props.chunk_id),
                    "{} claims {} but the list omits it",
                    b.props.chunk_id,
                    s.id()
                );
            }
        }
    }

    #[test]
    fn the_index_lists_blobs_sorted_by_id_so_two_runs_agree() {
        let w = chunks::wants(&[pair()]);
        let entries = w
            .iter()
            .map(|x| {
                (
                    IndexedBlob {
                        props: BlobProps::unmeasured(&x.chunk),
                        referenced_by: x.references.clone(),
                        sets: x.sets.clone(),
                    },
                    x.sets.clone(),
                )
            })
            .collect();
        let ids: Vec<String> = ChunkIndex::build(entries).blobs.iter().map(|b| b.props.chunk_id.clone()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn a_url_needs_a_registered_tag_and_says_which_one_it_wanted() {
        let s = sources();
        assert_eq!(
            s.url("7.1.0", "abc").unwrap(),
            "https://cdn/new/abc",
            "the base already carries the build id"
        );
        let e = s.url("6.7.0", "abc").unwrap_err().to_string();
        assert!(e.contains("6.7.0"), "{e}");
        assert_eq!(s.tags(), vec!["7.0.0", "7.1.0"]);
    }

    #[test]
    fn plan_only_still_lists_every_chunk_and_its_roles() {
        let w = chunks::wants(&[pair()]);
        let entries = Store::plan_only(&w);
        assert_eq!(entries.len(), 5);
        assert!(entries.iter().all(|(b, _)| b.props.stored_path.is_none()));
        assert!(entries.iter().all(|(b, _)| b.props.verified.is_none()));
        let idx = ChunkIndex::build(entries);
        assert_eq!(idx.sets["patch_fetch"].len(), 2, "the plan survives");
    }

    #[test]
    fn the_projected_download_is_once_per_distinct_chunk() {
        let w = chunks::wants(&[pair()]);
        let sum: u64 = w.iter().map(|x| x.chunk.compressed_size as u64).sum();
        assert_eq!(projected_download(&w), sum);
        assert_eq!(w.len(), 5, "three old ids plus two new-only ones");
    }

    #[test]
    fn a_description_names_the_numbers_a_reader_wants_first() {
        let w = chunks::wants(&[pair()]);
        let idx = ChunkIndex::build(Store::plan_only(&w));
        let d = describe(&w, &idx);
        assert!(d.contains("5 distinct chunks"), "{d}");
        assert!(d.contains("0 failing A6"), "{d}");
        // Nothing was stored, and the line must not claim otherwise.
        assert!(d.contains("0 of 5 stored"), "{d}");
        assert!(!d.contains(", 5 stored,"), "{d}");
    }

    /// And with every blob written the same line says all five.
    #[test]
    fn a_description_counts_what_was_actually_stored() {
        let w = chunks::wants(&[pair()]);
        let entries: Vec<(IndexedBlob, Vec<Set>)> = w
            .iter()
            .map(|x| {
                let mut props = BlobProps::unmeasured(&x.chunk);
                props.stored_path = Some(BlobProps::stored_path_for(&x.chunk.id));
                (
                    IndexedBlob {
                        props,
                        referenced_by: x.references.clone(),
                        sets: x.sets.clone(),
                    },
                    x.sets.clone(),
                )
            })
            .collect();
        let d = describe(&w, &ChunkIndex::build(entries));
        assert!(d.contains("5 of 5 stored"), "{d}");
    }

    /// A blob that was never fetched is not silently absent from the index.
    #[test]
    fn an_unfilled_want_still_has_a_row_and_says_it_was_not_stored() {
        let w = chunks::wants(&[pair()]);
        let root = std::env::temp_dir().join(format!("gircap-m6-{}", std::process::id()));
        let store = Store::new(&root, sources());
        let (entry, roles) = store.unfilled(&w[0]);
        assert_eq!(roles, w[0].sets);
        assert!(entry.props.stored_path.is_none());
        assert!(
            entry.props.decode_error.as_deref().unwrap().contains("not stored"),
            "{:?}",
            entry.props.decode_error
        );
        assert!(!entry.props.is_usable());
        let _ = std::fs::remove_dir_all(&root);
    }
}
