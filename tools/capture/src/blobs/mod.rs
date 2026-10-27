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
use props::BlobProps;

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

    /// Every blob that failed at least one A6 check.
    pub fn failing(&self) -> Vec<&IndexedBlob> {
        self.blobs
            .iter()
            .filter(|b| b.props.any_failure() || b.props.decode_error.is_some())
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
            blob_dir: Arc::new(std::sync::OnceLock::new()),
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
                Ok((i, Ok(entry))) => out[i] = Some(entry),
                // **The index and the error are recorded together.** The slot is
                // known, so the row that says the chunk is absent can also say
                // why -- and the failure is pushed exactly once.
                Ok((i, Err(e))) => {
                    out[i] = Some(self.unfilled(&wants[i], &e));
                    errors.push(e);
                }
                // A panicking task takes its slot number with it, so its hole is
                // filled below. Pushing a second, slotless error here would count
                // the same failure twice, which is the bug this function had.
                // The JoinError itself would be dropped here, so name the panic
                // in the log before falling through to the generic row text.
                Err(e) => {
                    tracing::error!(
                        error = %join_failure(e),
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
                *o = Some(self.unfilled(&wants[i], why));
                errors.push(format!("{}: {why}", wants[i].chunk.id));
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
    async fn one(
        this: Store,
        slot: usize,
        fetcher: Fetcher,
        want: Want,
    ) -> (usize, Result<(IndexedBlob, Vec<Set>), String>) {
        let self_ = &this;
        let mut last: Option<String> = None;
        // Sources are ordered newest-first, so the new build's CDN is asked
        // before the old one's, which is the one that may have stopped serving.
        for src in &want.sources {
            let url = match self_.sources.url(&src.tag, &want.chunk.id) {
                Ok(u) => u,
                Err(e) => return (slot, Err(e.to_string())),
            };
            match self_.stream_from(&fetcher, &url, &want, slot).await {
                Ok((mut props, rel, seq)) => {
                    props.set_stored(rel.clone());
                    // Provenance: which request produced these bytes.
                    fetcher.log().set_stored_as(seq, rel);
                    if !want.conflicts.is_empty() {
                        tracing::warn!(
                            chunk = %want.chunk.id,
                            detail = %want.describe_conflicts(),
                            "two builds declare this chunk id with different content; \
                             measured against the first",
                        );
                    }
                    // The two fields are moved rather than cloned: this is the last use of `want`.
                    return (slot, Ok(indexed_from(props, &want)));
                }
                Err(e) => {
                    tracing::debug!(chunk = %want.chunk.id, source = %src.tag, error = %e, "chunk fetch failed");
                    last = Some(format!("{} via {}: {e}", want.chunk.id, src.tag));
                }
            }
        }
        (
            slot,
            Err(last.unwrap_or_else(|| format!("{}: no source to fetch from", want.chunk.id))),
        )
    }

    /// The index entry for a want whose bytes never landed.
    ///
    /// Not `BlobProps::unmeasured` - that is for `--no-chunk-blobs`, where
    /// nothing was *asked* for. Here something was asked for and did not come,
    /// and the difference is the whole point of the row.
    ///
    /// **The row carries the reason it is unusable**, from the fetch itself: a
    /// source that refused, a `write` that failed, or a task that panicked. The
    /// message is the one the run's error list carries, so the summary count and
    /// the row cannot tell different stories. `decode_error` is where it goes
    /// because that is the field a reader (and `selfcheck`'s A6) already uses to
    /// ask *why is this blob unusable*; the name says "decode" but the answer is
    /// "no usable bytes", and a fetch that never produced a body is the extreme
    /// case of that.
    fn unfilled(&self, want: &Want, why: &str) -> (IndexedBlob, Vec<Set>) {
        let mut props = BlobProps::unmeasured(&want.chunk);
        props.decode_error = Some(why.to_string());
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
    /// 3. **rename** the tmp file into place, the atomic step, and the reason a
    ///    reader never sees a half-written blob;
    /// 4. measure **from the file**, through a 16 KB window, for the decompressed
    ///    length and md5.
    ///
    /// Measuring happens after the rename so that a blob is on disk even if the
    /// decode task is cancelled, and so a decode failure is recorded on a row whose
    /// bytes a reader can actually check.
    ///
    /// **An undecodable body is kept, not dropped.** The bytes are on disk before
    /// they are ever measured, so a body that is not a zstd frame becomes a row
    /// carrying its digests and the reason — rather than a hole, and rather than a
    /// bundle claiming the fetch failed when it succeeded.
    ///
    /// **A temp file is never orphaned.** Every error between creating it and
    /// renaming it removes it, best-effort. The slot is part of the name, so a
    /// leftover would only be reused if the same chunk id landed in the same
    /// slot — a different chunk set, `--files`, or the budget can move it, and
    /// nothing sweeps `chunks/blobs/`. A failed fetch is not a blob.
    ///
    /// Measuring is synchronous CPU on a zstd decode, milliseconds on the largest
    /// chunk, so it goes to `spawn_blocking`: inline this would block a tokio
    /// worker and stop any further fetch being issued.
    async fn stream_from(
        &self,
        fetcher: &Fetcher,
        url: &str,
        want: &Want,
        slot: usize,
    ) -> Result<(BlobProps, String, u64), String> {
        let id = want.chunk.id.clone();
        let rel = BlobProps::stored_path_for(&id);
        let (final_path, tmp_path) = self.paths_for(&rel, slot);
        self.ensure_blob_dir(&final_path)?;
        let chunk = want.chunk.clone();

        let mut out = crate::hash::HashingFile::create(&tmp_path)
            .map_err(|e| format!("{id}: create {}: {e}", tmp_path.display()))?;
        let seq = match fetcher.get_to_file(url, Kind::Chunk, &mut out).await {
            Ok(s) => s.seq,
            Err(e) => {
                // Close before unlinking: on Windows an open handle refuses the
                // delete. Both steps are best-effort, because this path has
                // already failed once and the fetch error is what the row must
                // carry.
                drop(out);
                Self::discard_tmp(&tmp_path);
                return Err(format!("{id}: {e}"));
            }
        };
        let digests = match out.finish() {
            Ok(d) => d,
            Err(e) => {
                Self::discard_tmp(&tmp_path);
                return Err(format!("{id}: flush: {e}"));
            }
        };

        // The atomic step: until this, nothing outside this function looks at the
        // path; after it, the blob is complete.
        if let Err(e) = std::fs::rename(&tmp_path, &final_path) {
            Self::discard_tmp(&tmp_path);
            return Err(format!(
                "{id}: {} -> {}: {e}",
                tmp_path.display(),
                final_path.display()
            ));
        }

        let path = final_path;
        let props =
            tokio::task::spawn_blocking(move || BlobProps::measure_file(&path, &chunk, digests))
                .await
                .map_err(|e| format!("{id}: measure task panicked: {e}"))?
                .map_err(|e| e.to_string())?;

        Ok((props, rel, seq))
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

/// The most useful text a [`tokio::task::JoinError`] carries.
///
/// **`JoinError`'s `Display` does not include the panic payload.** It says only
/// that the task panicked, so a message the failing task wrote — the reason an
/// operator actually needs — has to be downcast out of [`into_panic`]. The
/// index row stays generic on purpose (it is a stable document), but the log
/// must not throw the cause away with the task.
///
/// [`into_panic`]: tokio::task::JoinError::into_panic
fn join_failure(e: tokio::task::JoinError) -> String {
    if !e.is_panic() {
        return format!("the chunk fetch task was cancelled: {e}");
    }
    let payload = e.into_panic();
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a non-string panic payload".to_string());
    format!("the chunk fetch task panicked: {message}")
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

    /// A task that panics reports the payload, not just "task panicked".
    ///
    /// The `fill` loop has no slot to attach the failure to, so it logs this and
    /// lets the hole-fill write the generic index row. Before the helper, the
    /// `JoinError` was discarded and the operator could not see the cause at all.
    #[tokio::test]
    async fn a_panicking_chunk_task_reports_its_panic_message() {
        let handle = tokio::spawn(async { panic!("chunk store exploded") });
        let e = handle.await.expect_err("the task panicked");
        let why = join_failure(e);
        assert!(
            why.contains("chunk store exploded"),
            "the panic payload is the part worth logging: {why}"
        );
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
        let store = Store::new(&dir, Sources::from_pairs([]));
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
    /// its row carries **why**.
    #[test]
    fn an_unfilled_want_still_has_a_row_and_carries_the_fetch_failure() {
        let w = chunks::wants(&[pair()]);
        let root = girpr_tempdir::TempDir::new("gircap-m6", "unfilled_want");
        let store = Store::new(&root, sources());
        let why = "old-zero via 7.1.0: HTTP 404 Not Found";
        let (entry, roles) = store.unfilled(&w[0], why);
        assert_eq!(roles, w[0].sets);
        assert!(entry.props.stored_path.is_none());
        assert_eq!(
            entry.props.decode_error.as_deref(),
            Some(why),
            "the row must name the fetch failure, not a generic `not stored`"
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
            let why = entry
                .0
                .props
                .decode_error
                .as_deref()
                .expect("the row says why it is absent");
            assert_ne!(why, "not stored", "the generic message tells nobody why");
            assert!(
                why.starts_with(&want.chunk.id) && why.contains(" via 7."),
                "the reason names the chunk and the source it last tried: {why}"
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
}
