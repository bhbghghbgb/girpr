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
//! Every blob is written to a dot-prefixed `.tmp` file beside its final path,
//! then renamed into `chunks/blobs/`. A
//! half-written `.zst` that a reader finds under its final name is worse than no
//! blob at all: `verify` would report a digest failure for a file that simply
//! had not finished arriving, and the two demand opposite responses.
//!
//! The content address is the chunk id, so a second capture that reuses the same
//! chunks writes byte-identical files at identical paths. That is what makes
//! `materialize` offline in M9 a matter of reading the bundle rather than
//! re-deriving anything.

pub mod chunks;
pub mod fill;
pub mod plan;
pub mod props;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::http::{ApiError, Fetcher, Kind};
use crate::manifest::CapturedManifest;
use chunks::{FilePair, Reference, Set, Want};
use props::{BlobProps, FetchError};

/// `chunks/index.json`, the bundle's own table of contents for its blobs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkIndex {
    /// The artifact schema version.
    pub schema_version: u32,
    /// One entry per distinct chunk id, ordered by id.
    pub blobs: Vec<IndexedBlob>,
    /// The three role lists, each a list of chunk ids.
    pub sets: BTreeMap<String, Vec<String>>,
}

/// The index row for a `Want`, given the measurement of its bytes.
///
/// **One constructor, three call sites.** A stored blob, a blob that was
/// `--no-chunk-blobs`-planned, and a blob whose fetch came back empty all built
/// the same struct with the same two clones, in three places. The rule that a
/// row's `referenced_by` and `sets` are *the want's* is exactly the kind of
/// thing that has to be said once: `chunks/index.json` is derived data, and a
/// row whose `sets` disagreed with its `referenced_by` is the failure mode the
/// `ChunkIndex::build` doc comment is about.
fn indexed_from(props: BlobProps, want: &Want) -> (IndexedBlob, Vec<Set>) {
    (
        IndexedBlob {
            props,
            referenced_by: want.references.clone(),
            sets: want.sets.clone(),
        },
        want.sets.clone(),
    )
}

/// One blob as `chunks/index.json` lists it: the measurement plus who wanted it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        // Keyed by `&str` and looked up by `&str`, so a blob's id is *borrowed*
        // while it is filed rather than cloned once per role it belongs to. The
        // three role lists are pre-seeded below, so the lookup never allocates a
        // key either; it only grows a `Vec`.
        //
        // Two passes: the first borrows, the second consumes. That is strictly
        // less work than the one pass it replaces, which cloned every blob in
        // order to move it into `blobs` a few lines later.
        let mut sets: BTreeMap<&str, Vec<&str>> =
            Set::ALL.iter().map(|s| (s.id(), Vec::new())).collect();
        for (blob, roles) in &entries {
            for role in roles {
                sets.entry(role.id())
                    .or_default()
                    .push(&blob.props.chunk_id);
            }
        }
        // Owned before `entries` is consumed, so the borrow of the ids ends.
        let sets: BTreeMap<String, Vec<String>> = sets
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.into_iter().map(str::to_string).collect()))
            .collect();
        ChunkIndex {
            schema_version: crate::SCHEMA_VERSION,
            blobs: entries.into_iter().map(|(blob, _)| blob).collect(),
            sets,
        }
    }

    /// The blob entry for an id.
    ///
    /// `blobs` is documented as ordered by id and [`ChunkIndex::build`] produces
    /// it that way, so this is a binary search over 38 entries rather than a
    /// scan of all of them.
    ///
    /// A bundle is a file someone can edit, though, and `blobs` is a `pub`
    /// field — so an index that is *not* sorted must not silently turn "found"
    /// into "not found". The fallback scan is what keeps the lookup correct for
    /// any order; it costs one pass, and only on the way to an error.
    pub fn get(&self, chunk_id: &str) -> Option<&IndexedBlob> {
        match self
            .blobs
            .binary_search_by_key(&chunk_id, |b| b.props.chunk_id.as_str())
        {
            Ok(i) => self.blobs.get(i),
            Err(_) => self.blobs.iter().find(|b| b.props.chunk_id == chunk_id),
        }
    }

    /// Every blob that carries evidence it is unusable.
    ///
    /// A pure `--no-chunk-blobs` row is not listed: nothing was asked for, A6 is
    /// skipped for the whole run, and counting those rows as failures would make
    /// the summary contradict the selfcheck. Every *attempted* gap is listed —
    /// a failed fetch, a partial body, a decode failure, a failed measurement or
    /// any of the four A6 checks.
    pub fn failing(&self) -> Vec<&IndexedBlob> {
        self.blobs
            .iter()
            .filter(|b| {
                let p = &b.props;
                p.any_failure()
                    || p.decode_error.is_some()
                    || p.fetch_error.is_some()
                    || p.partial
                    || p.measurement_error.is_some()
            })
            .collect()
    }

    /// Refuse an index whose offsets or decompressed sizes cannot be added.
    ///
    /// **The index is a document an operator does not have to have produced.**
    /// `Bundle::open` calls this before any command reads a blob, so a crafted
    /// `offset: i64::MAX` with a one-byte blob is a typed error naming the chunk
    /// and the values, not a debug abort or a release wrap inside the layout
    /// arithmetic. `uncompressed_size` is stored as `u64`; a value above
    /// `i64::MAX` would become negative when the rebuild casts it, so it is
    /// refused here as well.
    pub fn validate_arithmetic(&self) -> Result<(), String> {
        for blob in &self.blobs {
            let id = &blob.props.chunk_id;
            let size = blob.props.uncompressed_size;
            if size > i64::MAX as u64 {
                return Err(format!(
                    "chunks/index.json blob {id}: uncompressed_size {size} does not fit i64"
                ));
            }
            let size = size as i64;
            for reference in &blob.referenced_by {
                if reference.offset < 0 {
                    return Err(format!(
                        "chunks/index.json blob {id} referenced_by {}: offset {} is negative",
                        reference.path, reference.offset
                    ));
                }
                if reference.offset.checked_add(size).is_none() {
                    return Err(format!(
                        "chunks/index.json blob {id} referenced_by {}: offset {} + \
                         uncompressed_size {size} overflows i64",
                        reference.path, reference.offset
                    ));
                }
            }
        }
        Ok(())
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
    pub fn from_manifests<'a>(
        manifests: impl IntoIterator<Item = &'a CapturedManifest>,
    ) -> Sources {
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
    /// The run's `--max-decompressed-bytes`, used when measuring a published
    /// blob. A measurement decodes, so it is subject to the same ceiling as
    /// every other decode.
    max_decompressed_bytes: u64,
    /// `chunks/blobs/` has been created for this store.
    ///
    /// Every blob lands in that one directory and `write` runs once per blob
    /// from concurrent tasks, so without this a capture pays a `create_dir_all`
    /// — a stat chain, several of them on Windows — per blob to learn a fact
    /// that cannot change mid-run. Only success is cached: an `Err` is retried
    /// on the next blob, which is both harmless and the behaviour a caller
    /// would want if the cause were transient.
    ///
    /// `Clone` shares it, which is what makes it correct across tasks.
    blob_dir: Arc<std::sync::OnceLock<()>>,
}

/// What one attempt against one source produced.
///
/// Two successful shapes, because a fetch that stopped after writing bytes is
/// evidence and a fetch that completed is not. The distinction is carried here
/// rather than inferred from `decode_error`, which version 3 reserves for a body
/// that arrived whole and would not decode.
enum Attempt {
    /// The body arrived complete and was measured. Its bytes are already at the
    /// final path.
    Complete {
        props: BlobProps,
        rel: String,
        seq: u64,
    },
    /// The fetch failed after writing some bytes. The prefix is measured but
    /// **not published**: it waits at `path` until the source loop decides it is
    /// the longest one available.
    Partial {
        props: BlobProps,
        rel: String,
        seq: u64,
        error: FetchError,
        path: PathBuf,
    },
}

/// The longest prefix seen so far, waiting for the loop to decide whether a
/// later source can do better.
struct PartialCandidate {
    props: BlobProps,
    rel: String,
    seq: u64,
    error: FetchError,
    path: PathBuf,
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
    pub fn new(root: &Path, sources: Sources, max_decompressed_bytes: u64) -> Store {
        Store {
            root: root.to_path_buf(),
            sources,
            max_decompressed_bytes,
            blob_dir: Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Fetch and store everything `wants` asks for.
    ///
    /// `jobs` bounds concurrency. Results come back in `wants` order regardless
    /// of which download finished first, so a capture with `--jobs 8` and one
    /// with `--jobs 1` produce the same index.
    ///
    /// **`wants` is shared, not copied.** The caller owns the plan, and each
    /// task receives the same `Arc<[Want]>` and indexes its own slot, so a want
    /// — several allocations per chunk — is never cloned into a task.
    ///
    /// One failure does not abandon the rest: each fetch's evidence is recorded
    /// on its blob and the store returns the index either way. A bundle with
    /// seven good blobs and one failed fetch is far more useful than no bundle,
    /// and `run` decides whether the gap is fatal.
    ///
    /// **A failure that produced bytes is both a row and an error.** The row is
    /// `partial: true` with `fetch_error` set; the error list still names it so
    /// `run` reports the failed fetch. The bytes are never silently promoted to
    /// a complete chunk because [`BlobProps::is_usable`] and A6 reject them.
    pub async fn fill(
        &self,
        fetcher: &Fetcher,
        wants: &Arc<[Want]>,
        jobs: usize,
    ) -> (Vec<(IndexedBlob, Vec<Set>)>, Vec<String>) {
        let outcomes = crate::tasks::join_ordered(wants, jobs, "chunk fetch", |i, wants| {
            let store = self.clone();
            let fetcher = fetcher.clone();
            async move { Store::one(store, i, fetcher, wants).await }
        })
        .await;

        let mut entries = Vec::with_capacity(wants.len());
        let mut errors: Vec<String> = Vec::new();
        for (i, outcome) in outcomes.into_iter().enumerate() {
            let entry = match outcome {
                Ok(entry) => {
                    // A partial row succeeded at preserving bytes but failed to
                    // fetch the chunk. The run's failed-fetch count must see it
                    // once, through the structured reason it carries.
                    if let Some(e) = &entry.0.props.fetch_error {
                        errors.push(e.detail.clone());
                    }
                    entry
                }
                // **The index and the error are recorded together.** The slot is
                // known, so the row that says the chunk is absent can also say
                // why -- and the failure is pushed exactly once.
                Err(crate::tasks::JoinOutcome::Failed(e)) => {
                    let row = self.unfilled(&wants[i], &e);
                    errors.push(e.detail);
                    row
                }
                // A task that did not report still keeps its slot, so the row
                // and the error name the chunk it belonged to. The payload is in
                // `why` whenever the shared loop could map the task id back;
                // only an unrecoverable id leaves the generic sentence.
                Err(crate::tasks::JoinOutcome::Lost { why }) => {
                    let why = why
                        .unwrap_or_else(|| "the chunk fetch task panicked while storing it".into());
                    let failure = FetchError::local(format!("{}: {why}", wants[i].chunk.id));
                    errors.push(failure.detail.clone());
                    self.unfilled(&wants[i], &failure)
                }
            };
            entries.push(entry);
        }
        (entries, errors)
    }

    /// The index for a run that stored no blobs at all (`--no-chunk-blobs`).
    ///
    /// The role lists are still written, because knowing which chunks *would*
    /// have been needed is the part of the document that survives, and an index
    /// that simply vanished would take that with it.
    pub fn plan_only(wants: &[Want]) -> Vec<(IndexedBlob, Vec<Set>)> {
        wants
            .iter()
            .map(|w| indexed_from(BlobProps::unmeasured(&w.chunk), w))
            .collect()
    }

    /// Fetch one want and write it.
    ///
    /// Takes the store by value rather than by reference so the future is
    /// 'static and can run on the runtime's threads.
    ///
    /// **The slot number comes back even on failure.** A bare
    /// `Result<(usize, entry), String>` would throw the index away exactly when
    /// the caller needed it: the caller could no longer tell a slot that failed
    /// from one that never reported, so it recorded the failure as an error *and*
    /// as a missing chunk, and the index row could only say "not stored".
    ///
    /// **A partial body does not end the source loop.** A prefix is only the
    /// best evidence *so far*; a later source may serve the chunk whole. The
    /// attempt is kept as a candidate (its bytes renamed aside, measured and not
    /// yet published) and the remaining sources are tried. A complete body wins
    /// outright, however short the prefix was. If none completes, the candidate
    /// with the **largest** `props.bytes` is published as the partial row; a tie
    /// keeps the first, so the newest source wins. A failure that produced
    /// nothing moves on to the next source, as before.
    async fn one(
        this: Store,
        slot: usize,
        fetcher: Fetcher,
        wants: Arc<[Want]>,
    ) -> (usize, Result<(IndexedBlob, Vec<Set>), FetchError>) {
        let want = &wants[slot];
        let self_ = &this;
        let mut last: Option<FetchError> = None;
        let mut candidate: Option<PartialCandidate> = None;
        // Sources are ordered newest-first, so the new build's CDN is asked
        // before the old one's, which is the one that may have stopped serving.
        for src in &want.sources {
            let url = match self_.sources.url(&src.tag, &want.chunk.id) {
                Ok(u) => u,
                Err(e) => {
                    // A source with no registered base is a zero-byte failure
                    // like any other: remember it and give the rest a chance.
                    last = Some(FetchError::local(e.to_string()));
                    continue;
                }
            };
            match self_
                .stream_from(&fetcher, &url, want, &src.tag, slot)
                .await
            {
                Ok(Attempt::Complete {
                    mut props,
                    rel,
                    seq,
                }) => {
                    // A complete body always wins over any prefix, regardless of
                    // byte count. The candidate was never published, so removing
                    // its file is all that is left of it.
                    if let Some(loser) = candidate.take() {
                        Self::discard_tmp(&loser.path);
                    }
                    props.set_stored(rel.clone());
                    // Provenance: which request produced these bytes.
                    fetcher.log().set_stored_as(seq, rel);
                    self_.warn_conflicts(want);
                    // `props` moves into the row; the want is borrowed from the
                    // shared plan rather than consumed.
                    return (slot, Ok(indexed_from(props, want)));
                }
                Ok(Attempt::Partial {
                    props,
                    rel,
                    seq,
                    error,
                    path,
                }) => {
                    // Strictly larger replaces the candidate; equal keeps the
                    // one already seen, which is the newest source. Either way
                    // the loser's bytes are removed now rather than at the end.
                    let replace = candidate
                        .as_ref()
                        .is_none_or(|kept| props.bytes > kept.props.bytes);
                    if replace {
                        if let Some(loser) = candidate.replace(PartialCandidate {
                            props,
                            rel,
                            seq,
                            error,
                            path,
                        }) {
                            Self::discard_tmp(&loser.path);
                        }
                    } else {
                        Self::discard_tmp(&path);
                    }
                }
                Err(e) => {
                    tracing::debug!(chunk = %want.chunk.id, source = %src.tag, error = %e.detail, "chunk fetch failed");
                    last = Some(e);
                }
            }
        }

        if let Some(winner) = candidate {
            // Publish only now, so no losing prefix ever occupied the final path
            // and a complete body from a later source could not collide with it.
            let final_path = self_.root.join(&winner.rel);
            if let Err(e) = std::fs::rename(&winner.path, &final_path) {
                Self::discard_tmp(&winner.path);
                return (
                    slot,
                    Err(FetchError::local(format!(
                        "{}: {} -> {}: {e}",
                        want.chunk.id,
                        winner.path.display(),
                        final_path.display()
                    ))),
                );
            }
            let mut props = winner.props;
            props.set_stored(winner.rel.clone());
            fetcher.log().set_stored_as(winner.seq, winner.rel);
            self_.warn_conflicts(want);
            props.mark_partial(winner.error);
            return (slot, Ok(indexed_from(props, want)));
        }

        (
            slot,
            Err(last.unwrap_or_else(|| {
                FetchError::local(format!("{}: no source to fetch from", want.chunk.id))
            })),
        )
    }

    /// One warning for two builds that disagree about a chunk id.
    fn warn_conflicts(&self, want: &Want) {
        if !want.conflicts.is_empty() {
            tracing::warn!(
                chunk = %want.chunk.id,
                detail = %want.describe_conflicts(),
                "two builds declare this chunk id with different content; \
                 measured against the first",
            );
        }
    }

    /// The index entry for a want whose bytes never landed.
    ///
    /// Not `BlobProps::unmeasured` - that is for `--no-chunk-blobs`, where
    /// nothing was *asked* for. Here something was asked for and did not come,
    /// and the difference is the whole point of the row.
    ///
    /// **The row carries the structured reason it is unusable**, from the fetch
    /// itself: a source that refused, a `write` that failed, or a task that
    /// panicked. `decode_error` stays null because no bytes arrived to decode;
    /// `fetch_error` is the version-3 field that answers *why is this blob
    /// unusable* for a fetch that produced no body.
    fn unfilled(&self, want: &Want, failure: &FetchError) -> (IndexedBlob, Vec<Set>) {
        let mut props = BlobProps::unmeasured(&want.chunk);
        props.fetch_error = Some(failure.clone());
        indexed_from(props, want)
    }

    /// Fetch one chunk straight to disk, digest it on the way past, and measure it
    /// from the file it landed in.
    ///
    /// The largest chunk in the reference bundle is 3 689 476 B and `--jobs 8`
    /// keeps eight in flight, so a body held whole for the length of its fetch is
    /// up to ~30 MB of resident chunks.
    ///
    /// The order is deliberate and is why the digests can be trusted:
    ///
    /// 1. stream into `chunks/blobs/.<name>.<slot>.tmp`, hashing as it writes —
    ///    nothing is ever whole in memory;
    /// 2. take the digests from the write itself;
    /// 3. **rename** out of the tmp name, the atomic step: a complete body goes
    ///    straight to its final path, while a partial goes to a hidden candidate
    ///    path (`.<name>.<slot>.<seq>.partial`) that the source loop may publish
    ///    later or discard;
    /// 4. measure **from the file**, through a 16 KB window, for the decompressed
    ///    length and md5.
    ///
    /// Measuring happens after the rename so that a blob is on disk even if the
    /// decode task is cancelled, and so a decode failure is recorded on a row whose
    /// bytes a reader can actually check. A partial is measured from its candidate
    /// file for the same reason: the bytes are final, and only the name is waiting
    /// on the source loop.
    ///
    /// **An undecodable body is kept, not dropped.** The bytes are on disk before
    /// they are ever measured, so a body that is not a zstd frame becomes a row
    /// carrying its digests and the reason — rather than a hole, and rather than a
    /// bundle claiming the fetch failed when it succeeded. A successful `2xx` with
    /// an empty body is published and measured too, as a zero-byte row whose
    /// `decode_error` says there was no frame.
    ///
    /// **A fetch that stopped after writing bytes keeps them too.** Version 3
    /// measures the prefix and parks it at a candidate path instead of the final
    /// one: a later source may complete the chunk, in which case the complete body
    /// wins and this prefix is removed. If nothing completes, the longest prefix
    /// is published as a `partial` row with its `fetch_error`. Only a *failed*
    /// fetch's zero-byte temp is removed, because a row with no bytes and no path
    /// is the only case where there is nothing to preserve. The cap
    /// (`--max-response-bytes`) still bounds how much arrives.
    ///
    /// **A failed flush or rename removes the temp.** Those are local I/O
    /// failures before publication: there are no trustworthy digests to index and
    /// no complete path to point at. The fetch failure itself is still carried.
    /// The source loop removes every losing candidate and publishes only the
    /// winner, so on a normal return the blob directory holds published blobs and
    /// nothing named `.tmp` or `.partial`.
    ///
    /// A *panic* unwinds past that cleanup, so a `.tmp` or `.partial` can survive a
    /// panicking task; the hole-fill records the panic in the index, and a sweep
    /// for old `.*.{tmp,partial}` files remains a possible follow-up. The slot
    /// (and, for a candidate, the request's `seq`) is part of the name, so a
    /// leftover would only be reused by the same chunk id in the same slot — a
    /// different chunk set, `--files`, or the budget can move it, and nothing
    /// sweeps `chunks/blobs/`.
    ///
    /// Measuring is synchronous CPU on a zstd decode, milliseconds on the largest
    /// chunk, so it goes to `spawn_blocking`: inline this would block a tokio
    /// worker and stop any further fetch being issued.
    async fn stream_from(
        &self,
        fetcher: &Fetcher,
        url: &str,
        want: &Want,
        source_tag: &str,
        slot: usize,
    ) -> Result<Attempt, FetchError> {
        let id = want.chunk.id.clone();
        let rel = BlobProps::stored_path_for(&id);
        let (final_path, tmp_path) = self.paths_for(&rel, slot);
        self.ensure_blob_dir(&final_path)
            .map_err(FetchError::local)?;
        let chunk = want.chunk.clone();

        let mut out = crate::hash::HashingFile::create(&tmp_path)
            .map_err(|e| FetchError::local(format!("{id}: create {}: {e}", tmp_path.display())))?;
        let fetched = fetcher
            .get_to_file_detailed(url, Kind::Chunk, &mut out)
            .await;
        let digests = match out.finish() {
            Ok(d) => d,
            Err(e) => {
                Self::discard_tmp(&tmp_path);
                return Err(FetchError::local(format!("{id}: flush: {e}")));
            }
        };

        let (seq, failure) = match fetched {
            Ok(streamed) => (streamed.seq, None),
            Err(failed) => {
                let mut failure = FetchError::from_api(Some(source_tag), url, &failed.error);
                // `detail` spells "{id} via {tag}: {error}" — keeping that text
                // in `detail` means the summary and the row cannot tell different stories.
                failure.detail = format!("{id} via {source_tag}: {}", failed.error);
                (failed.seq, Some(failure))
            }
        };

        if digests.bytes == 0 {
            // **Only a failed fetch's empty temp is removed.** A successful
            // response with a zero-byte body is evidence about the origin: it is
            // renamed and measured like any other, and measurement records an
            // undecodable row whose digests cover no bytes. A failure that wrote
            // nothing has nothing to publish, so it falls through to the next
            // source with the fetch failure as the row's only content.
            if let Some(error) = failure {
                Self::discard_tmp(&tmp_path);
                return Err(error);
            }
        }

        // A partial waits at a path the source loop controls; only a complete
        // body goes to the final one. The candidate name carries the `seq`
        // because the fixed tmp path is reused by every attempt for this slot.
        let publish_path = match &failure {
            None => final_path.clone(),
            Some(_) => self.candidate_path_for(&rel, slot, seq),
        };

        // The atomic step: until this, nothing outside this function looks at the
        // path. A complete body is now published; a partial is only parked.
        if let Err(e) = std::fs::rename(&tmp_path, &publish_path) {
            Self::discard_tmp(&tmp_path);
            return Err(FetchError::local(format!(
                "{id}: {} -> {}: {e}",
                tmp_path.display(),
                publish_path.display()
            )));
        }

        let path = publish_path.clone();
        let measure_chunk = chunk.clone();
        let measure_digests = digests.clone();
        // A measurement that fails does not remove the blob: the row keeps
        // `stored_path` and records `measurement_error`, so a reader can tell
        // "measurement failed" from "no bytes". The bytes are still evidence.
        let ceiling = self.max_decompressed_bytes;
        let props = Self::measure_published(&publish_path, &id, digests, &chunk, move || {
            BlobProps::measure_file(&path, &measure_chunk, measure_digests, ceiling)
        })
        .await;

        match failure {
            None => Ok(Attempt::Complete { props, rel, seq }),
            Some(error) => Ok(Attempt::Partial {
                props,
                rel,
                seq,
                error,
                path: publish_path,
            }),
        }
    }

    /// Measure a published blob on the blocking pool, keeping the cause and the
    /// bytes when the task does not return.
    ///
    /// The closure is a parameter rather than a call to
    /// [`BlobProps::measure_file`] so the panic arm can be driven by a test:
    /// no input makes the real measurement panic on demand. A panic or
    /// cancellation is routed through [`crate::tasks::join_blocking`], so the
    /// payload survives, and the already-published blob is **kept** with
    /// [`BlobProps::measurement_error`] set. `stored_path` is attached by the
    /// caller, and [`BlobProps::is_usable`] rejects the row until a measurement
    /// succeeds.
    async fn measure_published<F>(
        final_path: &Path,
        id: &str,
        compressed: crate::hash::Digests,
        chunk: &crate::manifest::proto::SophonChunk,
        measure: F,
    ) -> BlobProps
    where
        F: FnOnce() -> Result<BlobProps, ApiError> + Send + 'static,
    {
        match crate::tasks::join_blocking("measure", measure).await {
            Ok(Ok(props)) => props,
            Ok(Err(e)) => {
                tracing::warn!(
                    blob = %final_path.display(),
                    error = %e,
                    "a published blob could not be measured; keeping its bytes"
                );
                BlobProps::measurement_failed(compressed, chunk, e.to_string())
            }
            Err(e) => {
                tracing::warn!(
                    blob = %final_path.display(),
                    error = %e,
                    "the measurement task did not return; keeping its bytes"
                );
                BlobProps::measurement_failed(compressed, chunk, format!("{id}: {e}"))
            }
        }
    }

    /// Best-effort removal of a temp file this run will never publish.
    ///
    /// **The handle is closed first.** On Windows an open handle refuses the
    /// delete, so every caller drops its [`crate::hash::HashingFile`] before
    /// calling this. A failure is logged and swallowed: the caller is already
    /// returning the error that sent it here, and a cleanup failure must not
    /// replace the reason the fetch failed.
    fn discard_tmp(path: &Path) {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(
                tmp = %path.display(),
                error = %e,
                "a failed blob left its temp file behind"
            ),
        }
    }

    /// The final path for a blob, and the tmp path it is written under.
    ///
    /// **The tmp round-trip is not removable.** A crash mid-write would otherwise
    /// leave a truncated file at the final path, where it is indistinguishable from
    /// a good blob to every reader, including `verify`. One rename is not worth
    /// that — and on Windows `std::fs::rename` *does* replace an existing
    /// destination (`renaming_a_blob_over_an_existing_one_replaces_it` in this
    /// module's tests), so the step is not redundant with anything.
    ///
    /// **The tmp name carries the slot, not just the chunk id.** Derived from the
    /// chunk id alone, two tasks that wanted the *same* id would write one file
    /// interleaved and the first `rename` would publish a half-written blob —
    /// silent corruption, with every digest in `chunks/index.json` quietly
    /// describing the wrong bytes.
    ///
    /// Those two tasks cannot exist today: `chunks::wants` keys a `BTreeMap` on
    /// `chunk.id`, so ids are unique by construction, and
    /// `wants_have_unique_ids` is what keeps that true rather than merely
    /// believed. The slot suffix is because that invariant lives in a *different
    /// module* from the code that depends on it — so a change to the planner could
    /// break the store silently, and this makes the store's correctness its own
    /// property rather than an import. It costs nothing: the slot is already
    /// threaded through `fill` → `one` to place the result.
    fn paths_for(&self, rel: &str, slot: usize) -> (PathBuf, PathBuf) {
        let final_path = self.root.join(rel);
        let dir = final_path.parent().unwrap_or(&self.root).to_path_buf();
        let tmp = dir.join(format!(
            ".{}.{slot}.tmp",
            final_path.file_name().unwrap_or_default().to_string_lossy()
        ));
        (final_path, tmp)
    }

    /// Where a partial attempt's bytes wait until the source loop decides whether
    /// they win.
    ///
    /// **Distinct per attempt, unlike the tmp path.** The tmp name is deliberately
    /// reused by every source tried for one slot, so a candidate cannot stay there
    /// while the next source fetches. The request's `seq` makes this name unique,
    /// so a longer prefix from a later source never overwrites the candidate it is
    /// being compared against. Only the winner is renamed to `rel`; every losing
    /// candidate is removed by [`Store::one`].
    fn candidate_path_for(&self, rel: &str, slot: usize, seq: u64) -> PathBuf {
        let final_path = self.root.join(rel);
        let dir = final_path.parent().unwrap_or(&self.root);
        dir.join(format!(
            ".{}.{slot}.{seq}.partial",
            final_path.file_name().unwrap_or_default().to_string_lossy()
        ))
    }

    /// The one directory blobs live in, created once per store.
    ///
    /// Not cached on an `Err`, so a transient failure does not skip the rest of
    /// the run.
    fn ensure_blob_dir(&self, final_path: &Path) -> Result<(), String> {
        if self.blob_dir.get().is_some() {
            return Ok(());
        }
        let dir = final_path.parent().unwrap_or(&self.root);
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let _ = self.blob_dir.set(());
        Ok(())
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
///
/// **The sides are looked up in the capture's own field.** A capture is scored
/// against the field it came from — `game` or an audio field — and the path
/// lives in *that* field's manifests. Looking it up in the `game` manifests
/// instead would find nothing for an audio capture, and the pair would come out
/// with **no sides at all**: a patchmap that says "this file must be gone
/// afterwards", and a fixture with no file in it. The capture would then assert
/// nothing, and the cross-tool claim would be vacuous for it.
///
/// The index is built once per field and tag, so the cost is one walk over the
/// manifests rather than one per admitted path per side.
pub fn file_pairs(
    selection: &crate::select::Selection,
    fields: &[String],
    manifests: &[Vec<CapturedManifest>],
    old_tag: &str,
    new_tag: &str,
) -> Vec<FilePair> {
    // One index per (field, tag), built once.
    //
    // `entries.iter().find(|e| e.file == a.path)` per admitted path, per side,
    // would be O(admitted × paths), twice. Over the measured pair that is a few
    // thousand 2 744-element string scans to build eight pairs — the single
    // largest waste in the selection→store hand-off. `diff` builds the same map
    // and throws it away, so the shape was already known; here it is built where
    // it is used, which costs one walk rather than A×n.
    let mut indexes: BTreeMap<
        (&str, &str),
        BTreeMap<&str, &crate::manifest::proto::SophonChunkFile>,
    > = BTreeMap::new();
    for (field, pair) in fields.iter().zip(manifests) {
        for m in pair {
            indexes.insert(
                (field.as_str(), m.tag.as_str()),
                m.entries.iter().map(|e| (e.file.as_str(), e)).collect(),
            );
        }
    }
    let build_id = |tag: &str, field: &str| -> &str {
        manifests
            .iter()
            .zip(fields)
            .filter(|(_, f)| f.as_str() == field)
            .flat_map(|(ms, _)| ms.iter())
            .find(|m| m.tag == tag)
            .map(|m| m.build_id.as_str())
            .unwrap_or_default()
    };
    selection
        .admitted
        .iter()
        .map(|a| {
            // The capture's **own** field, not the `game` field: the path was
            // scored against the field it came from, so that is where it
            // lives.
            let field = a.matching_field.as_str();
            let old_entry = indexes
                .get(&(field, old_tag))
                .and_then(|m| m.get(a.path.as_str()).copied());
            let new_entry = indexes
                .get(&(field, new_tag))
                .and_then(|m| m.get(a.path.as_str()).copied());
            let old_build_id = build_id(old_tag, field);
            let new_build_id = build_id(new_tag, field);
            FilePair {
                capture_id: a.capture_id.clone(),
                path: a.path.clone(),
                old: old_entry.map(|e| chunks::OldFile {
                    tag: old_tag.to_string(),
                    build_id: old_build_id.to_string(),
                    entry: e.clone(),
                }),
                new: new_entry.map(|e| chunks::NewFile {
                    tag: new_tag.to_string(),
                    build_id: new_build_id.to_string(),
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
///
/// **Saturating, because this is an estimate.** A hostile manifest can declare
/// `compressed_size: i64::MAX` on several chunks; the answer is "more than this
/// tool can count", not a debug abort or a release wrap. The capture-side plan
/// is a total to display and compare, not an address.
pub fn projected_download(wants: &[Want]) -> u64 {
    wants.iter().fold(0_u64, |total, w| {
        total.saturating_add(w.chunk.compressed_size.max(0) as u64)
    })
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
        index
            .blobs
            .iter()
            .map(|b| b.referenced_by.len())
            .sum::<usize>(),
        index.failing().len(),
    )
}

#[cfg(test)]
mod tests;
