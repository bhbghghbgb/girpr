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
        wants: &[Want],
        jobs: usize,
    ) -> (Vec<(IndexedBlob, Vec<Set>)>, Vec<String>) {
        let mut out: Vec<Option<(IndexedBlob, Vec<Set>)>> =
            (0..wants.len()).map(|_| None).collect();
        let mut errors: Vec<String> = Vec::new();
        let mut next = 0_usize;
        let mut set = tokio::task::JoinSet::new();

        // Prime the set, then keep one job in flight per worker. A plain
        // `for want in wants` would serialise; this does not need `futures`.
        while next < wants.len() && set.len() < jobs.max(1) {
            let i = next;
            next += 1;
            set.spawn(Store::one(
                self.clone(),
                i,
                fetcher.clone(),
                wants[i].clone(),
            ));
        }
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((i, Ok(entry))) => {
                    // A partial row succeeded at preserving bytes but failed to
                    // fetch the chunk. The run's failed-fetch count must see it
                    // once, through the structured reason it carries.
                    if let Some(e) = &entry.0.props.fetch_error {
                        errors.push(e.detail.clone());
                    }
                    out[i] = Some(entry);
                }
                // **The index and the error are recorded together.** The slot is
                // known, so the row that says the chunk is absent can also say
                // why -- and the failure is pushed exactly once.
                Ok((i, Err(e))) => {
                    out[i] = Some(self.unfilled(&wants[i], &e));
                    errors.push(e.detail);
                }
                // A panicking task takes its slot number with it, so its hole is
                // filled below. Pushing a second, slotless error here would count
                // the same failure twice, which is the bug this function had.
                // The JoinError itself would be dropped here, so name the panic
                // in the log before falling through to the generic row text.
                Err(e) => {
                    tracing::error!(
                        error = %crate::tasks::join_failure(e, "chunk fetch"),
                        "a chunk fetch task did not report; its slot is filled below"
                    );
                }
            }
            if next < wants.len() {
                let i = next;
                next += 1;
                set.spawn(Store::one(
                    self.clone(),
                    i,
                    fetcher.clone(),
                    wants[i].clone(),
                ));
            }
        }

        // A `None` slot is a fetch that neither succeeded nor reported, which can
        // only be the panic above. Fill it so the index has no holes, and record
        // it once, with the chunk id the task could not report.
        for (i, o) in out.iter_mut().enumerate() {
            if o.is_none() {
                let why = "the chunk fetch task panicked while storing it";
                let failure = FetchError::local(why);
                errors.push(format!("{}: {why}", wants[i].chunk.id));
                *o = Some(self.unfilled(&wants[i], &failure));
            }
        }
        (out.into_iter().flatten().collect(), errors)
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
    /// **The slot number comes back even on failure.** It used to be a bare
    /// `Result<(usize, entry), String>`, which threw the index away exactly when
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
        want: Want,
    ) -> (usize, Result<(IndexedBlob, Vec<Set>), FetchError>) {
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
                .stream_from(&fetcher, &url, &want, &src.tag, slot)
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
                    self_.warn_conflicts(&want);
                    // The two fields are moved rather than cloned: this is the last use of `want`.
                    return (slot, Ok(indexed_from(props, &want)));
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
            self_.warn_conflicts(&want);
            props.mark_partial(winner.error);
            return (slot, Ok(indexed_from(props, &want)));
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
                // The run's error list used to spell "{id} via {tag}: {error}".
                // Keeping that text in `detail` means the summary and the row
                // cannot tell different stories.
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
    // This used to `entries.iter().find(|e| e.file == a.path)` **per admitted
    // path, per side**: O(admitted × paths), twice. Over the measured pair that
    // is a few thousand 2 744-element string scans to build eight pairs — the
    // single largest waste in the selection→store hand-off. `diff` builds the
    // same map and throws it away, so the shape was already known; here it is
    // built where it is used, which costs one walk rather than A×n.
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
mod tests {
    use super::*;
    use crate::hash;
    use crate::manifest::proto::{SophonChunk, SophonChunkFile};

    fn scratch(name: &str) -> girpr_tempdir::TempDir {
        girpr_tempdir::TempDir::new("gircap-blobs", name)
    }

    /// `fs::rename` **does** replace an existing destination on Windows: it calls
    /// `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`. This is checked rather
    /// than assumed, because "rename fails when the target exists" is a common
    /// belief and acting on it here would mean deleting the atomic step the code
    /// needs. The destination is deliberately *longer* than the source, so a
    /// copy-not-replace implementation would leave a tail behind and be caught.
    #[test]
    fn renaming_a_blob_over_an_existing_one_replaces_it() {
        let dir = scratch("rename-over");
        let (final_path, tmp) = (dir.join("blob.zst"), dir.join(".blob.zst.tmp"));
        std::fs::write(&final_path, b"the older and longer blob").expect("seed destination");
        std::fs::write(&tmp, b"NEW").expect("write tmp");

        std::fs::rename(&tmp, &final_path).expect("rename onto an existing file");

        assert_eq!(
            std::fs::read(&final_path).expect("read"),
            b"NEW",
            "the destination must be the new blob and nothing else"
        );
        assert!(
            !tmp.exists(),
            "the tmp file must be gone after the rename publishes it"
        );
    }

    #[test]
    fn two_wants_for_one_chunk_id_would_not_share_a_temp_file() {
        // The store half of the tmp-naming invariant: derived from the chunk id
        // alone, two tasks wanting the same id would write one file interleaved
        // and the first `rename` would publish a **half-written blob** — silent
        // corruption, with every digest in `chunks/index.json` quietly describing
        // the wrong bytes and nothing failing.
        //
        // Fed the *same* id twice on purpose, because that is the only input that
        // distinguishes a slot-suffixed name from an id-derived one.
        let dir = scratch("tmp-per-slot");
        let store = Store::new(
            &dir,
            Sources::from_pairs([]),
            crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
        );
        let (a, at) = store.paths_for("chunks/blobs/dead_beef.zst", 0);
        let (b, bt) = store.paths_for("chunks/blobs/dead_beef.zst", 1);

        assert_eq!(
            a, b,
            "the published path is the blob's, and does not depend on the slot"
        );
        assert_ne!(
            at, bt,
            "two tasks writing the same blob must not share one tmp file"
        );
        assert!(
            at.parent() == a.parent(),
            "and the tmp file stays beside the blob it becomes, not somewhere else"
        );
        for t in [&at, &bt] {
            let name = t.file_name().unwrap().to_string_lossy().to_string();
            assert!(
                name.starts_with('.') && name.ends_with(".tmp"),
                "{name} must stay hidden and temporary"
            );
        }
    }

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

    /// Serve one HTTP response over loopback and close the connection.
    ///
    /// `declared` is the `Content-Length` promised; `body` is what is actually
    /// written. A body shorter than the declaration is a truncated CDN read.
    fn serve_blob_once(declared: usize, body: Vec<u8>) -> (u16, std::thread::JoinHandle<()>) {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).expect("nonblocking accept");
        let handle = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(pair) => break pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(e) => panic!("accept: {e}"),
                }
            };
            // The accept loop needs a nonblocking listener so a regression that
            // never connects cannot hang the suite. An accepted socket on Windows
            // can inherit that, and a mock that reads before the client has sent
            // its request would then answer and close mid-send. Back to blocking
            // so the request is read first and the reply cannot race it.
            stream.set_nonblocking(false).expect("blocking stream");
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).ok();
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                    break;
                }
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
            stream.flush().unwrap();
        });
        (port, handle)
    }

    /// The two builds' files declaring the same chunk, which gives the want its
    /// newest-first sources: `7.1.0` then `7.0.0`.
    fn pair_declared_by_both(c: &SophonChunk, capture_id: &str) -> FilePair {
        FilePair {
            capture_id: capture_id.into(),
            path: "dll".into(),
            old: Some(chunks::OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: entry("dll", vec![c.clone()]),
            }),
            new: Some(chunks::NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: entry("dll", vec![c.clone()]),
            }),
        }
    }

    /// Every file under `chunks/blobs/`, sorted, so a leftover temp or losing
    /// candidate fails the test by name.
    fn blob_dir_names(root: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(root.join("chunks/blobs"))
            .expect("the blob dir was created by the first attempt")
            .map(|entry| {
                entry
                    .expect("read a dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    /// `blobs` is documented as ordered by id, which is what makes the lookup a
    /// binary search. A bundle is a file someone can edit and `blobs` is a `pub`
    /// field, so the lookup has to stay correct for an index that is *not*
    /// sorted — otherwise a hand-edited bundle turns "found" into "not found",
    /// and the failure reads like a missing blob rather than a corrupt index.
    #[test]
    fn a_blob_is_found_whether_or_not_the_index_is_sorted() {
        let ids = ["aaa", "mmm", "zzz"];
        let blob = |id: &str| IndexedBlob {
            props: BlobProps {
                chunk_id: id.to_string(),
                stored_path: None,
                bytes: 0,
                container: String::new(),
                md5_compressed: String::new(),
                md5_decompressed: String::new(),
                sha256_compressed: String::new(),
                uncompressed_size: 0,
                unknown_field6: 0,
                verified: None,
                decode_error: None,
                fetch_error: None,
                partial: false,
                measurement_error: None,
            },
            referenced_by: Vec::new(),
            sets: Vec::new(),
        };
        let build = |order: &[&str]| {
            let entries: Vec<(IndexedBlob, Vec<Set>)> = order
                .iter()
                .map(|id| (blob(id), vec![Set::MaterializeOld]))
                .collect();
            ChunkIndex::build(entries)
        };
        let sorted = build(&ids);
        let reversed = build(&ids.iter().rev().copied().collect::<Vec<_>>());
        assert_eq!(
            sorted
                .blobs
                .iter()
                .map(|b| &b.props.chunk_id)
                .collect::<Vec<_>>(),
            ids,
            "build orders by id, which is what the lookup assumes"
        );
        for id in ids {
            assert_eq!(
                sorted.get(id).map(|b| b.props.chunk_id.as_str()),
                Some(id),
                "found in a sorted index"
            );
            assert_eq!(
                reversed.get(id).map(|b| b.props.chunk_id.as_str()),
                Some(id),
                "and still found in one that is not sorted"
            );
        }
        assert!(reversed.get("no-such-blob").is_none());
    }

    fn pair() -> FilePair {
        FilePair {
            capture_id: "reuse_in_place".into(),
            path: "dll".into(),
            old: Some(chunks::OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
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
                build_id: "sMXGW2ll3Fuu".into(),
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

    /// **A capture's sides come from its own field.**
    ///
    /// The audio manifests widen the *selection pool*, so a capture can be an
    /// audio file. Its sides are then in the audio manifests, and looking them
    /// up in the `game` manifests instead would find nothing: the pair would
    /// come out with **no sides at all**, and the capture would assert nothing
    /// — a patchmap that says "this file must be gone afterwards" for a file
    /// that is simply there.
    #[test]
    fn a_capture_from_another_field_gets_its_sides_from_that_field() {
        use crate::select::{Admission, Selection};

        let audio = |tag: &str, build: &str, path: &str| CapturedManifest {
            tag: tag.into(),
            build_id: build.into(),
            entries: vec![entry(path, vec![chunk(b"vo", 0, 0x1234)])],
            ..CapturedManifest::empty(tag, "en-us", vec![])
        };
        let manifests = vec![
            // The `game` field: a different path entirely.
            vec![
                CapturedManifest {
                    tag: "7.0.0".into(),
                    build_id: "gameold".into(),
                    entries: vec![entry("game/other.dat", vec![chunk(b"x", 0, 1)])],
                    ..CapturedManifest::empty("7.0.0", "game", vec![])
                },
                CapturedManifest {
                    tag: "7.1.0".into(),
                    build_id: "gamenew".into(),
                    entries: vec![entry("game/other.dat", vec![chunk(b"x", 0, 1)])],
                    ..CapturedManifest::empty("7.1.0", "game", vec![])
                },
            ],
            // The `en-us` field: the file the capture is about.
            vec![
                audio("7.0.0", "audiold", "vo.dat"),
                audio("7.1.0", "audinew", "vo.dat"),
            ],
        ];
        let selection = Selection {
            admitted: vec![Admission {
                capture_id: "identical".into(),
                primary_class: crate::select::Class::Identical,
                path: "vo.dat".into(),
                matching_field: "en-us".into(),
                state: crate::manifest::diff::State::Identical,
                also_satisfies: Vec::new(),
                budget_cost: 10,
                patch_bytes: 0,
            }],
            rejected: Vec::new(),
            forced_skipped: Vec::new(),
            forced_unscored: Vec::new(),
            matrix: Vec::new(),
            bytes_admitted: 10,
            budget: crate::select::Budget::unlimited(),
        };
        let pairs = file_pairs(
            &selection,
            &["game".into(), "en-us".into()],
            &manifests,
            "7.0.0",
            "7.1.0",
        );
        assert_eq!(pairs.len(), 1);
        let p = &pairs[0];
        assert!(p.old.is_some(), "the old side is in the audio manifest");
        assert!(p.new.is_some(), "and so is the new one");
        assert_eq!(p.old.as_ref().expect("an old side").build_id, "audiold");
        assert_eq!(p.new.as_ref().expect("a new side").build_id, "audinew");
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
                let b = idx
                    .get(id)
                    .unwrap_or_else(|| panic!("{role} names a missing blob {id}"));
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
        let ids: Vec<String> = ChunkIndex::build(entries)
            .blobs
            .iter()
            .map(|b| b.props.chunk_id.clone())
            .collect();
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

    /// **K1.** The projection is a *total*, not an address: a manifest that
    /// declares `i64::MAX` on several chunks saturates instead of aborting the
    /// run or wrapping to a small, plausible cost.
    #[test]
    fn a_hostile_projected_download_saturates() {
        let mut w = chunks::wants(&[pair()]);
        for want in &mut w {
            want.chunk.compressed_size = i64::MAX;
        }
        assert_eq!(
            projected_download(&w),
            u64::MAX,
            "five maxima cannot be represented, so the answer is 'more than this'"
        );
    }

    /// **K1.** The index is a document someone may have edited. A reference at
    /// `i64::MAX` is refused with the blob, path and arithmetic named, before
    /// `Bundle::open` lets any command add it to a length.
    #[test]
    fn a_crafted_index_reference_offset_is_refused_with_its_values() {
        let w = chunks::wants(&[pair()]);
        let mut entries = Store::plan_only(&w);
        let blob = &mut entries[0].0;
        let id = blob.props.chunk_id.clone();
        blob.referenced_by[0].offset = i64::MAX;
        let idx = ChunkIndex::build(entries);
        let e = idx.validate_arithmetic().expect_err("overflowing offset");
        assert!(e.contains(&id), "names the blob: {e}");
        assert!(
            e.contains(&i64::MAX.to_string()) && e.contains("overflows i64"),
            "names the value and the arithmetic: {e}"
        );
    }

    /// **K1.** An `uncompressed_size` above `i64::MAX` cannot be a length; the
    /// index refuses it rather than letting a `u64 as i64` cast turn it negative.
    #[test]
    fn an_index_size_that_does_not_fit_i64_is_refused() {
        let w = chunks::wants(&[pair()]);
        let mut entries = Store::plan_only(&w);
        let id = entries[0].0.props.chunk_id.clone();
        entries[0].0.props.uncompressed_size = u64::MAX;
        let idx = ChunkIndex::build(entries);
        let e = idx.validate_arithmetic().expect_err("unrepresentable size");
        assert!(e.contains(&id), "names the blob: {e}");
        assert!(
            e.contains(&u64::MAX.to_string()) && e.contains("does not fit i64"),
            "names the value and the bound: {e}"
        );
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

    /// A blob that was never fetched is not silently absent from the index, and
    /// its row carries **why** in the structured field version 3 added.
    #[test]
    fn an_unfilled_want_still_has_a_row_and_carries_the_fetch_failure() {
        let w = chunks::wants(&[pair()]);
        let root = girpr_tempdir::TempDir::new("gircap-m6", "unfilled_want");
        let store = Store::new(
            &root,
            sources(),
            crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
        );
        let failure = FetchError {
            source: Some("7.1.0".into()),
            url: Some("https://cdn/new/old-zero".into()),
            status: Some(404),
            transport: None,
            attempts: 1,
            detail: "old-zero via 7.1.0: HTTP 404 Not Found".into(),
        };
        let (entry, roles) = store.unfilled(&w[0], &failure);
        assert_eq!(roles, w[0].sets);
        assert!(entry.props.stored_path.is_none());
        assert!(
            entry.props.decode_error.is_none(),
            "no bytes arrived, so there is nothing to decode"
        );
        assert_eq!(
            entry.props.fetch_error.as_ref(),
            Some(&failure),
            "the row must carry the structured fetch failure"
        );
        assert!(!entry.props.is_usable());
    }

    /// **One failed fetch is one failure.** The slot's index and the error are
    /// returned together, so the row can carry the reason and the run's failed
    /// count cannot see the same chunk twice.
    ///
    /// Before this, `Store::one` threw the slot away on failure: `fill` pushed
    /// the error, the `None` slot looked like a task that never reported, and a
    /// second bare chunk id was pushed for it. One failure, two entries, and a
    /// row that could only say "not stored".
    #[tokio::test]
    async fn a_chunk_the_cdn_will_not_serve_is_one_failure_with_its_reason_in_the_row() {
        let w = chunks::wants(&[pair()]);
        let root = girpr_tempdir::TempDir::new("gircap-m6", "one_failed_fetch");
        // Port 0 is never bound: every attempt is an immediate refusal, with no
        // network, no timeout and no retry (one attempt is the whole policy).
        let dead = "http://127.0.0.1:0".to_string();
        let store = Store::new(
            &root,
            Sources::from_pairs([
                ("7.0.0".to_string(), dead.clone()),
                ("7.1.0".to_string(), dead),
            ]),
            crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
        );
        let fetcher = crate::http::Fetcher::new(
            crate::http::Transport::local(),
            crate::http::RequestLog::new(),
        )
        .expect("a fetcher with no configured headers to reject");

        let (entries, errors) = store.fill(&fetcher, &w, 1).await;

        assert_eq!(
            errors.len(),
            w.len(),
            "one entry per failed chunk, not one plus its bare id: {errors:?}"
        );
        assert_eq!(entries.len(), w.len(), "every want still has a row");
        for (entry, want) in entries.iter().zip(&w) {
            assert_eq!(entry.0.props.chunk_id, want.chunk.id);
            assert!(entry.0.props.stored_path.is_none());
            assert!(
                entry.0.props.decode_error.is_none(),
                "a fetch that produced no bytes has nothing to decode"
            );
            let why = entry
                .0
                .props
                .fetch_error
                .as_ref()
                .expect("the row says why it is absent");
            assert_ne!(
                why.detail, "not stored",
                "the generic message tells nobody why"
            );
            assert!(
                why.detail.starts_with(&want.chunk.id) && why.detail.contains(" via 7."),
                "the reason names the chunk and the source it last tried: {}",
                why.detail
            );
            assert!(
                why.transport.is_some(),
                "a refused connection is a transport failure: {why:?}"
            );
        }

        // F11: every one of those attempts created `chunks/blobs/.<id>.<slot>.tmp`
        // before the connection was refused. A leftover would be invisible to a
        // reader but permanent: the slot is part of the name, so a later run only
        // truncates it if the same id lands in the same slot again.
        let blobs = root.join("chunks/blobs");
        let leftovers: Vec<String> = std::fs::read_dir(&blobs)
            .expect("the blob dir was created before the first attempt")
            .map(|entry| {
                entry
                    .expect("read a dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "a failed fetch must leave nothing behind: {leftovers:?}"
        );
    }

    /// **K2.** A fetched body that is not a zstd frame is evidence, not a failed
    /// fetch: the row and the disk agree, and the run reports no error.
    ///
    /// The body is served over loopback through the real [`Store::fill`] path so
    /// this covers the rename, the measure-from-file step and the index entry,
    /// not just [`BlobProps::measure_file`]. Before the fix the file stayed on
    /// disk while the row said `bytes: 0`, `stored_path: null` and the run counted
    /// a failed fetch — three contradictory accounts of one body.
    #[tokio::test]
    async fn an_undecodable_body_is_stored_and_measured_rather_than_a_failed_fetch() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error>\n  \
                    <Code>NoSuchKey</Code>\n  <Message>The specified key does not \
                    exist.</Message>\n</Error>\n"
            .to_vec();

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).expect("nonblocking accept");
        let served = xml.clone();
        let server = std::thread::spawn(move || {
            // Bounded rather than a blocking `accept`: a future regression that
            // makes the fetch fail before connecting must fail the assertions,
            // not hang the suite on `join`.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(pair) => break pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(e) => panic!("accept: {e}"),
                }
            };
            // The accept loop needs a nonblocking listener so a regression that
            // never connects cannot hang the suite. An accepted socket on Windows
            // can inherit that, and a mock that reads before the client has sent
            // its request would then answer and close mid-send. Back to blocking
            // so the request is read first and the reply cannot race it.
            stream.set_nonblocking(false).expect("blocking stream");
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).ok();
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                    break;
                }
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                served.len()
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(&served).unwrap();
            stream.flush().unwrap();
        });

        let c = chunk(b"a chunk the CDN will not serve", 0, 0x5a5a);
        let p = FilePair {
            capture_id: "undecodable".into(),
            path: "dll".into(),
            old: None,
            new: Some(chunks::NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: entry("dll", vec![c.clone()]),
            }),
        };
        let w = chunks::wants(&[p]);
        assert_eq!(w.len(), 1, "the fixture asks for exactly one chunk");

        let root = girpr_tempdir::TempDir::new("gircap-k2", "undecodable_kept");
        let store = Store::new(
            &root,
            Sources::from_pairs([("7.1.0".to_string(), format!("http://127.0.0.1:{port}/cdn"))]),
            crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
        );
        let fetcher = crate::http::Fetcher::new(
            crate::http::Transport::local(),
            crate::http::RequestLog::new(),
        )
        .expect("a fetcher with no configured headers to reject");

        let (entries, errors) = store.fill(&fetcher, &w, 1).await;
        server.join().expect("the mock served the body");

        assert!(
            errors.is_empty(),
            "an unframed body is a measured row, not a failed fetch: {errors:?}"
        );
        assert_eq!(entries.len(), 1);
        let props = &entries[0].0.props;
        assert_eq!(
            props.bytes,
            xml.len() as u64,
            "the row counts the bytes that arrived, not zero"
        );
        assert_eq!(
            props.stored_path.as_deref(),
            Some(BlobProps::stored_path_for(&c.id).as_str()),
            "the row names the file the store published"
        );
        assert_eq!(props.md5_compressed, hash::md5_hex(&xml));
        assert_eq!(props.sha256_compressed, hash::sha256_hex(&xml));
        assert!(
            props
                .decode_error
                .as_deref()
                .unwrap_or_default()
                .contains("not a zstd frame"),
            "the row says why it will not decode: {:?}",
            props.decode_error
        );
        let v = props.verified.expect("the body was measured");
        assert!(!v.md5_decompressed_matches && !v.decoded_size_matches);

        let on_disk = std::fs::read(root.join(BlobProps::stored_path_for(&c.id)))
            .expect("the indexed path exists on disk");
        assert_eq!(
            on_disk, xml,
            "the published bytes are exactly what the CDN served"
        );
    }

    /// **A successful empty body is evidence, not a failed fetch.** A `2xx` whose
    /// body is zero bytes still says something about the origin, so the empty file
    /// is published and measured. The row keeps `stored_path`, `bytes: 0` and a
    /// `decode_error`, and the run does not count the chunk among the failed
    /// fetches. Only a *failed* fetch's zero-byte temp is removed.
    #[tokio::test]
    async fn a_successful_empty_body_is_stored_and_measured_as_undecodable() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).expect("nonblocking accept");
        let server = std::thread::spawn(move || {
            // Bounded rather than a blocking `accept`: a future regression that
            // makes the fetch fail before connecting must fail the assertions,
            // not hang the suite on `join`.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(pair) => break pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(e) => panic!("accept: {e}"),
                }
            };
            // The accept loop needs a nonblocking listener so a regression that
            // never connects cannot hang the suite. An accepted socket on Windows
            // can inherit that, and a mock that reads before the client has sent
            // its request would then answer and close mid-send. Back to blocking
            // so the request is read first and the reply cannot race it.
            stream.set_nonblocking(false).expect("blocking stream");
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).ok();
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                    break;
                }
            }
            // A successful answer that simply carries nothing: the origin was
            // reached and said "no bytes", which is a fact worth a row.
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();
        });

        let c = chunk(b"a chunk the CDN answers with an empty body", 0, 0x5c5c);
        let p = FilePair {
            capture_id: "empty".into(),
            path: "dll".into(),
            old: None,
            new: Some(chunks::NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: entry("dll", vec![c.clone()]),
            }),
        };
        let w = chunks::wants(&[p]);
        assert_eq!(w.len(), 1, "the fixture asks for exactly one chunk");

        let root = girpr_tempdir::TempDir::new("gircap-k2", "empty_body_kept");
        let store = Store::new(
            &root,
            Sources::from_pairs([("7.1.0".to_string(), format!("http://127.0.0.1:{port}/cdn"))]),
            crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
        );
        let fetcher = crate::http::Fetcher::new(
            crate::http::Transport::local(),
            crate::http::RequestLog::new(),
        )
        .expect("a fetcher with no configured headers to reject");

        let (entries, errors) = store.fill(&fetcher, &w, 1).await;
        server.join().expect("the mock served the empty body");

        assert!(
            errors.is_empty(),
            "a successful empty body is not a failed fetch: {errors:?}"
        );
        assert_eq!(entries.len(), 1, "the chunk still has a row");
        let props = &entries[0].0.props;
        assert_eq!(
            props.stored_path.as_deref(),
            Some(BlobProps::stored_path_for(&c.id).as_str()),
            "the empty body was published like any other"
        );
        assert_eq!(props.bytes, 0, "the row counts the bytes that arrived");
        assert!(
            !props.partial,
            "the fetch completed, so nothing is a prefix"
        );
        assert!(
            props.fetch_error.is_none(),
            "nothing failed to fetch: {:?}",
            props.fetch_error
        );
        assert!(
            props
                .decode_error
                .as_deref()
                .unwrap_or_default()
                .contains("not a zstd frame"),
            "the row says why it will not decode: {:?}",
            props.decode_error
        );
        assert!(
            !props.is_usable(),
            "zero bytes cannot be assembled into the chunk the manifest named"
        );

        let on_disk = std::fs::read(root.join(BlobProps::stored_path_for(&c.id)))
            .expect("the indexed path exists on disk");
        assert!(
            on_disk.is_empty(),
            "the published file is exactly the empty body: {} bytes",
            on_disk.len()
        );
    }

    /// **F11.** A fetch that stops after writing bytes publishes those bytes as
    /// an indexed, unusable row: `partial: true`, a structured `fetch_error`,
    /// and the digests of exactly what arrived. The bytes stay on disk so a
    /// reader can compare them with a later capture.
    #[tokio::test]
    async fn a_truncated_response_keeps_its_prefix_and_marks_the_row_partial() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let payload = b"a chunk the CDN will stop serving half way through".repeat(64);
        let full = zstd::encode_all(payload.as_slice(), 1).expect("encode");
        let prefix_len = full.len() / 2;
        let prefix = full[..prefix_len].to_vec();
        let declared = full.len();

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).expect("nonblocking accept");
        let served = prefix.clone();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(pair) => break pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(e) => panic!("accept: {e}"),
                }
            };
            // The accept loop needs a nonblocking listener so a regression that
            // never connects cannot hang the suite. An accepted socket on Windows
            // can inherit that, and a mock that reads before the client has sent
            // its request would then answer and close mid-send. Back to blocking
            // so the request is read first and the reply cannot race it.
            stream.set_nonblocking(false).expect("blocking stream");
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).ok();
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                    break;
                }
            }
            // The declaration promises the whole body; the connection closes
            // after the prefix, which is the real shape of a truncated CDN read.
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(&served).unwrap();
            stream.flush().unwrap();
        });

        let c = chunk(&payload, 0, 0x5b5b);
        let p = FilePair {
            capture_id: "truncated".into(),
            path: "dll".into(),
            old: None,
            new: Some(chunks::NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: entry("dll", vec![c.clone()]),
            }),
        };
        let w = chunks::wants(&[p]);
        let root = girpr_tempdir::TempDir::new("gircap-f11", "truncated_kept");
        let store = Store::new(
            &root,
            Sources::from_pairs([("7.1.0".to_string(), format!("http://127.0.0.1:{port}/cdn"))]),
            crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
        );
        let fetcher = crate::http::Fetcher::new(
            crate::http::Transport::local(),
            crate::http::RequestLog::new(),
        )
        .expect("a fetcher with no configured headers to reject");

        let (entries, errors) = store.fill(&fetcher, &w, 1).await;
        server.join().expect("the mock served the prefix");

        assert_eq!(errors.len(), 1, "the failed fetch is reported once");
        assert_eq!(entries.len(), 1, "and its bytes still have a row");
        let props = &entries[0].0.props;
        assert!(props.partial, "the row says the body is a prefix");
        assert!(props.stored_path.is_some(), "the prefix was published");
        assert_eq!(
            props.bytes, prefix_len as u64,
            "the row counts the bytes that arrived"
        );
        assert_eq!(props.md5_compressed, hash::md5_hex(&prefix));
        let failure = props.fetch_error.as_ref().expect("and why it stopped");
        assert!(
            failure.transport.is_some(),
            "a connection that closed early is a transport failure: {failure:?}"
        );
        assert!(
            failure.detail.contains("via 7.1.0"),
            "the source is named: {}",
            failure.detail
        );
        assert!(!props.is_usable(), "a prefix must never look complete");
        assert!(
            props
                .why_unusable()
                .unwrap_or_default()
                .contains("fetch failed"),
            "and the reason is the fetch, not a decode"
        );
        let on_disk = std::fs::read(root.join(BlobProps::stored_path_for(&c.id)))
            .expect("the indexed path exists");
        assert_eq!(on_disk, prefix, "the bytes are exactly what arrived");
    }

    /// **A truncated newest source does not end the loop.** The older source is
    /// asked; its complete body wins outright, replaces the parked prefix and is
    /// the only attempt that claims `stored_as`. The losing prefix leaves no file.
    #[tokio::test]
    async fn a_complete_older_source_wins_over_a_truncated_newer_one() {
        let payload = b"a chunk the newest CDN truncates but the older one serves".repeat(64);
        let full = zstd::encode_all(payload.as_slice(), 1).expect("encode");
        let prefix_len = full.len() / 2;
        let prefix = full[..prefix_len].to_vec();
        let (new_port, new_server) = serve_blob_once(full.len(), prefix);
        let (old_port, old_server) = serve_blob_once(full.len(), full.clone());

        let c = chunk(&payload, 0, 0xca11);
        let p = pair_declared_by_both(&c, "complete_fallback");
        let w = chunks::wants(&[p]);
        assert_eq!(
            w[0].sources
                .iter()
                .map(|s| s.tag.as_str())
                .collect::<Vec<_>>(),
            vec!["7.1.0", "7.0.0"],
            "the newest source is tried first"
        );
        let root = girpr_tempdir::TempDir::new("gircap-f11", "complete_fallback");
        let store = Store::new(
            &root,
            Sources::from_pairs([
                (
                    "7.1.0".to_string(),
                    format!("http://127.0.0.1:{new_port}/cdn"),
                ),
                (
                    "7.0.0".to_string(),
                    format!("http://127.0.0.1:{old_port}/cdn"),
                ),
            ]),
            crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
        );
        let fetcher = crate::http::Fetcher::new(
            crate::http::Transport::local(),
            crate::http::RequestLog::new(),
        )
        .expect("a fetcher with no configured headers to reject");

        let (entries, errors) = store.fill(&fetcher, &w, 1).await;
        new_server
            .join()
            .expect("the newest server served its prefix");
        old_server
            .join()
            .expect("the older server served the whole body");

        assert!(
            errors.is_empty(),
            "a complete fallback is a success, not a failed fetch: {errors:?}"
        );
        let props = &entries[0].0.props;
        let rel = BlobProps::stored_path_for(&c.id);
        assert!(!props.partial, "the complete body is not a prefix");
        assert!(
            props.fetch_error.is_none(),
            "nothing failed after the older source completed: {:?}",
            props.fetch_error
        );
        assert_eq!(props.stored_path.as_deref(), Some(rel.as_str()));
        assert!(
            props.is_usable(),
            "the whole chunk arrived and measured green"
        );
        assert_eq!(
            std::fs::read(root.join(&rel)).expect("the published blob"),
            full,
            "the complete body replaced the prefix"
        );

        let rows = fetcher.log().records();
        assert_eq!(rows.len(), 2, "both sources were asked: {rows:?}");
        let newer = rows
            .iter()
            .find(|r| r.url.contains(&format!(":{new_port}/")))
            .expect("the newest request is logged");
        let older = rows
            .iter()
            .find(|r| r.url.contains(&format!(":{old_port}/")))
            .expect("the older request is logged");
        assert_eq!(
            older.stored_as.as_deref(),
            Some(rel.as_str()),
            "only the complete winner claims the published blob"
        );
        assert_eq!(
            newer.stored_as, None,
            "the losing prefix must not claim the path"
        );
        assert_eq!(
            blob_dir_names(&root),
            vec![format!("{}.zst", c.id)],
            "no temp or candidate file survives"
        );
    }

    /// A partial is retried, and the longest prefix wins even when it came from
    /// the older source. The row carries that attempt's `fetch_error` and bytes.
    #[tokio::test]
    async fn the_longest_prefix_wins_when_no_source_completes() {
        let payload = b"both CDNs truncate this chunk at different points".repeat(64);
        let full = zstd::encode_all(payload.as_slice(), 1).expect("encode");
        let newer_len = full.len() / 3;
        let older_len = full.len() * 2 / 3;
        let newer_prefix = full[..newer_len].to_vec();
        let older_prefix = full[..older_len].to_vec();
        let (new_port, new_server) = serve_blob_once(full.len(), newer_prefix);
        let (old_port, old_server) = serve_blob_once(full.len(), older_prefix.clone());

        let c = chunk(&payload, 0, 0xca12);
        let p = pair_declared_by_both(&c, "longest_older");
        let w = chunks::wants(&[p]);
        let root = girpr_tempdir::TempDir::new("gircap-f11", "longest_older");
        let store = Store::new(
            &root,
            Sources::from_pairs([
                (
                    "7.1.0".to_string(),
                    format!("http://127.0.0.1:{new_port}/cdn"),
                ),
                (
                    "7.0.0".to_string(),
                    format!("http://127.0.0.1:{old_port}/cdn"),
                ),
            ]),
            crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
        );
        let fetcher = crate::http::Fetcher::new(
            crate::http::Transport::local(),
            crate::http::RequestLog::new(),
        )
        .expect("a fetcher with no configured headers to reject");

        let (entries, errors) = store.fill(&fetcher, &w, 1).await;
        new_server
            .join()
            .expect("the newest server served its prefix");
        old_server
            .join()
            .expect("the older server served its prefix");

        assert_eq!(errors.len(), 1, "only the published failure is reported");
        let props = &entries[0].0.props;
        let rel = BlobProps::stored_path_for(&c.id);
        assert!(props.partial);
        assert_eq!(
            props.bytes, older_len as u64,
            "the longest prefix is the one published"
        );
        assert_eq!(props.stored_path.as_deref(), Some(rel.as_str()));
        let failure = props.fetch_error.as_ref().expect("and why it stopped");
        assert!(
            failure.detail.contains("via 7.0.0"),
            "the row names the source whose prefix won: {}",
            failure.detail
        );
        assert_eq!(
            std::fs::read(root.join(&rel)).expect("the published blob"),
            older_prefix,
            "the file is exactly the winning prefix"
        );

        let rows = fetcher.log().records();
        let newer = rows
            .iter()
            .find(|r| r.url.contains(&format!(":{new_port}/")))
            .expect("the newest request is logged");
        let older = rows
            .iter()
            .find(|r| r.url.contains(&format!(":{old_port}/")))
            .expect("the older request is logged");
        assert_eq!(
            older.stored_as.as_deref(),
            Some(rel.as_str()),
            "only the winning prefix claims the published blob"
        );
        assert_eq!(
            newer.stored_as, None,
            "the shorter prefix must not claim the path"
        );
        assert_eq!(
            blob_dir_names(&root),
            vec![format!("{}.zst", c.id)],
            "no temp or candidate file survives"
        );
    }

    /// The mirror image: the newer source's prefix is longer, so a shorter later
    /// prefix is discarded and the newest one keeps the row and the file.
    #[tokio::test]
    async fn a_shorter_later_prefix_keeps_the_newest_candidate() {
        let payload = b"the newest CDN gets further before it truncates".repeat(64);
        let full = zstd::encode_all(payload.as_slice(), 1).expect("encode");
        let newer_len = full.len() * 2 / 3;
        let older_len = full.len() / 3;
        let newer_prefix = full[..newer_len].to_vec();
        let older_prefix = full[..older_len].to_vec();
        let (new_port, new_server) = serve_blob_once(full.len(), newer_prefix.clone());
        let (old_port, old_server) = serve_blob_once(full.len(), older_prefix);

        let c = chunk(&payload, 0, 0xca13);
        let p = pair_declared_by_both(&c, "longest_newer");
        let w = chunks::wants(&[p]);
        let root = girpr_tempdir::TempDir::new("gircap-f11", "longest_newer");
        let store = Store::new(
            &root,
            Sources::from_pairs([
                (
                    "7.1.0".to_string(),
                    format!("http://127.0.0.1:{new_port}/cdn"),
                ),
                (
                    "7.0.0".to_string(),
                    format!("http://127.0.0.1:{old_port}/cdn"),
                ),
            ]),
            crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
        );
        let fetcher = crate::http::Fetcher::new(
            crate::http::Transport::local(),
            crate::http::RequestLog::new(),
        )
        .expect("a fetcher with no configured headers to reject");

        let (entries, errors) = store.fill(&fetcher, &w, 1).await;
        new_server
            .join()
            .expect("the newest server served its prefix");
        old_server
            .join()
            .expect("the older server served its prefix");

        assert_eq!(errors.len(), 1, "only the published failure is reported");
        let props = &entries[0].0.props;
        let rel = BlobProps::stored_path_for(&c.id);
        assert!(props.partial);
        assert_eq!(
            props.bytes, newer_len as u64,
            "the newest, longer prefix is kept"
        );
        assert_eq!(props.stored_path.as_deref(), Some(rel.as_str()));
        let failure = props.fetch_error.as_ref().expect("and why it stopped");
        assert!(
            failure.detail.contains("via 7.1.0"),
            "the row names the source whose prefix won: {}",
            failure.detail
        );
        assert_eq!(
            std::fs::read(root.join(&rel)).expect("the published blob"),
            newer_prefix,
            "the file is exactly the winning prefix"
        );

        let rows = fetcher.log().records();
        let newer = rows
            .iter()
            .find(|r| r.url.contains(&format!(":{new_port}/")))
            .expect("the newest request is logged");
        let older = rows
            .iter()
            .find(|r| r.url.contains(&format!(":{old_port}/")))
            .expect("the older request is logged");
        assert_eq!(
            newer.stored_as.as_deref(),
            Some(rel.as_str()),
            "only the winning prefix claims the published blob"
        );
        assert_eq!(
            older.stored_as, None,
            "the shorter prefix must not claim the path"
        );
        assert_eq!(
            blob_dir_names(&root),
            vec![format!("{}.zst", c.id)],
            "no temp or candidate file survives"
        );
    }

    /// A later source that fails without writing anything leaves the first
    /// prefix as the row; its temp must not displace the candidate.
    #[tokio::test]
    async fn a_zero_byte_failure_after_a_prefix_keeps_the_prefix() {
        let payload = b"the older CDN refuses this chunk entirely".repeat(64);
        let full = zstd::encode_all(payload.as_slice(), 1).expect("encode");
        let prefix_len = full.len() / 2;
        let prefix = full[..prefix_len].to_vec();
        let (new_port, new_server) = serve_blob_once(full.len(), prefix.clone());

        let c = chunk(&payload, 0, 0xca14);
        let p = pair_declared_by_both(&c, "zero_after_prefix");
        let w = chunks::wants(&[p]);
        let root = girpr_tempdir::TempDir::new("gircap-f11", "zero_after_prefix");
        let store = Store::new(
            &root,
            Sources::from_pairs([
                (
                    "7.1.0".to_string(),
                    format!("http://127.0.0.1:{new_port}/cdn"),
                ),
                // Port 0 is never bound: an immediate refusal that writes nothing.
                ("7.0.0".to_string(), "http://127.0.0.1:0".to_string()),
            ]),
            crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
        );
        let fetcher = crate::http::Fetcher::new(
            crate::http::Transport::local(),
            crate::http::RequestLog::new(),
        )
        .expect("a fetcher with no configured headers to reject");

        let (entries, errors) = store.fill(&fetcher, &w, 1).await;
        new_server
            .join()
            .expect("the newest server served its prefix");

        assert_eq!(errors.len(), 1, "only the published failure is reported");
        let props = &entries[0].0.props;
        let rel = BlobProps::stored_path_for(&c.id);
        assert!(props.partial);
        assert_eq!(props.bytes, prefix_len as u64);
        assert_eq!(props.stored_path.as_deref(), Some(rel.as_str()));
        let failure = props.fetch_error.as_ref().expect("and why it stopped");
        assert!(
            failure.detail.contains("via 7.1.0"),
            "the row keeps the prefix's failure: {}",
            failure.detail
        );
        assert_eq!(
            std::fs::read(root.join(&rel)).expect("the published blob"),
            prefix,
            "the zero-byte attempt did not replace the prefix"
        );
        assert_eq!(
            blob_dir_names(&root),
            vec![format!("{}.zst", c.id)],
            "no temp or candidate file survives"
        );
    }

    /// **N1/K2.** A measurement that does not return loses neither its cause nor
    /// the bytes: the panic payload reaches the row as `measurement_error`, and
    /// the already-published blob stays on disk with `stored_path` set. Only
    /// `is_usable` rejects it, so a reader can tell "measurement failed" from
    /// "no bytes were fetched".
    ///
    /// The real [`BlobProps::measure_file`] cannot be made to panic on demand,
    /// so this drives the same seam [`Store::stream_from`] uses with a closure
    /// that panics deterministically.
    #[tokio::test]
    async fn a_panicking_measurement_keeps_the_published_blob_and_the_cause() {
        let dir = scratch("measure-panic");
        let id = "00000000_00000000000000000000000000000000";
        let bytes = b"published but never measured";
        let final_path = dir.join(BlobProps::stored_path_for(id));
        std::fs::create_dir_all(final_path.parent().expect("blob dir")).expect("create blob dir");
        std::fs::write(&final_path, bytes).expect("seed the blob");

        let chunk = crate::manifest::proto::SophonChunk {
            id: id.to_string(),
            ..Default::default()
        };
        let mut props =
            Store::measure_published(&final_path, id, hash::md5_sha256(bytes), &chunk, || {
                panic!("measure exploded at frame 7")
            })
            .await;
        props.set_stored(BlobProps::stored_path_for(id));

        let why = props
            .measurement_error
            .as_deref()
            .expect("the row says the measurement failed");
        assert!(why.starts_with(id), "the site's context survives: {why}");
        assert!(
            why.contains("the measure task panicked"),
            "the shared rule names the site: {why}"
        );
        assert!(
            why.contains("measure exploded at frame 7"),
            "the payload is the part worth reporting: {why}"
        );
        assert_eq!(
            props.bytes,
            bytes.len() as u64,
            "the write's byte count survives"
        );
        assert_eq!(
            props.md5_compressed,
            hash::md5_hex(bytes),
            "the write's digest survives"
        );
        assert!(
            props.verified.is_none(),
            "nothing measured the decoded side"
        );
        assert!(props.stored_path.is_some(), "the row still names its bytes");
        assert!(final_path.exists(), "the published blob must survive");
        assert!(
            !props.is_usable(),
            "bytes are not usable just because a path is set"
        );
        assert!(
            props
                .why_unusable()
                .unwrap_or_default()
                .contains("measurement failed"),
            "the reason distinguishes this from a fetch that never arrived"
        );
    }
}
