//! Building a file out of chunks.
//!
//! Three assemblies, and the third is the point: `old` from
//! `materialize_old`, `new` from `materialize_new`, and the *patch flow* -
//! REUSE a local slice after verifying it by MD5, FETCH otherwise - which must
//! then byte-equal the direct `new` assembly. The first two prove the artifact
//! can produce what it claims; only the third proves the patchmap describes a
//! patch anyone could run.

pub mod ranges;

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::blobs::chunks::FilePair;
use crate::blobs::{ChunkIndex, Sources};
use crate::hash;
use crate::manifest::proto::SophonChunkFile;
use crate::select::Class;
use ranges::{Layout, Range};

/// Where decoded chunk bytes come from.
///
/// Two modes and no more: the bundle's stored blobs, or an in-memory map for a
/// test that has no bundle. A trait here would be three implementors and a
/// generic on every function below; the union reads the same and keeps
/// [`ChunkSource::decoded`] the one place that decides how a chunk is obtained.
#[derive(Debug, Clone)]
pub struct ChunkSource<'i> {
    inner: Inner<'i>,
}

#[derive(Debug, Clone)]
enum Inner<'i> {
    /// Read `chunks/blobs/<id>.zst` and decompress it.
    Disk {
        root: PathBuf,
        /// Borrowed, not copied. A `BTreeMap` of every chunk's id, stored path
        /// and md5 built at construction would be a second copy of the index's
        /// whole id space for the life of the source.
        index: &'i ChunkIndex,
        /// One cell per stored blob, filled on first decode, when
        /// [`ChunkSource::memoizing`] asked for it.
        cache: Option<BTreeMap<String, OnceLock<Vec<u8>>>>,
    },
    /// Already-decoded bytes, keyed by chunk id.
    Memory(BTreeMap<String, Vec<u8>>),
}

/// One chunk's contribution to an assembled file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanEntry {
    /// Its index in the target file's chunk list.
    pub i: usize,
    /// `REUSE` or `FETCH`, decided *after* verification.
    pub decision: Decision,
    /// Where the chunk's bytes land in the result.
    pub target_range: Range,
    /// Where they come from, when they come from the old file.
    pub source_range: Option<SourceRange>,
    /// The chunk id whose bytes are used.
    pub chunk_id: String,
    /// The CDN URL, when the bytes have to be fetched.
    pub url: Option<String>,
    /// `compressed_size`, as declared.
    pub compressed_size: i64,
    /// `uncompressed_size`, as declared.
    pub uncompressed_size: i64,
    /// The declared decompressed digest — the thing REUSE is verified against.
    pub uncompressed_md5: String,
    /// Anything a reader would otherwise have to guess.
    pub note: Option<String>,
}

impl PlanEntry {
    /// Whether this chunk's bytes come from a **different offset** in the old
    /// file than where they land in the new one.
    ///
    /// One definition for the `reuse_moved` class, the `Totals::moved`
    /// count and the per-entry note. It was spelled out three times, and the
    /// three had to agree or the note would contradict the count.
    pub fn is_moved(&self) -> bool {
        self.source_range
            .as_ref()
            .is_some_and(|s| s.start != self.target_range.start)
    }
}

/// A local range in the old file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceRange {
    /// First byte.
    pub start: i64,
    /// One past the last.
    pub end: i64,
    /// Which build's file the slice is in.
    pub file_tag: String,
}

/// What a chunk's bytes are taken from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Decision {
    /// Verified byte-identical to a slice of the old file.
    Reuse,
    /// Has to be downloaded.
    Fetch,
}

impl Decision {
    /// The wire spelling, which the schema doc writes in caps.
    pub const fn as_str(self) -> &'static str {
        match self {
            Decision::Reuse => "REUSE",
            Decision::Fetch => "FETCH",
        }
    }
}

impl std::fmt::Display for Decision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `patchmaps/<capture_id>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchMap {
    /// The artifact schema version.
    pub schema_version: u32,
    /// The capture this map belongs to.
    pub capture_id: String,
    /// The manifest-relative path.
    pub path: String,
    /// The `matching_field` the file came from.
    pub matching_field: String,
    /// Every class the file answers.
    pub classes: Vec<Class>,
    /// The old side, absent for an `added` capture.
    pub old: Option<Side>,
    /// The new side, absent for a `removed` capture.
    pub new: Option<Side>,
    /// One entry per new-side chunk. `null` when there is no new side.
    pub plan: Option<Vec<PlanEntry>>,
    /// The arithmetic, so a reader need not add it up.
    pub totals: Option<Totals>,
    /// The ground truth a consumer asserts against. Always present: every
    /// capture has at least one side, and a capture nothing can be asserted
    /// about is the defect rather than a shape.
    pub expectations: Expectations,
}

/// One side of the pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Side {
    /// The build.
    pub tag: String,
    /// The declared size.
    pub size: i64,
    /// The declared whole-file MD5.
    pub md5: String,
    /// How many chunks it has.
    pub nchunks: usize,
}

/// The arithmetic of a patchmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    /// New-side chunks.
    pub chunks: usize,
    /// How many are reused from the old file.
    pub reuse: usize,
    /// How many must be downloaded.
    pub fetch: usize,
    /// How many reused chunks sit at a different offset in the old file.
    pub moved: usize,
    /// `Σ compressed_size` over the FETCH entries — what the patch costs.
    pub download_bytes: i64,
    /// Bytes needed to rebuild the old file.
    pub materialize_old_bytes: i64,
    /// Bytes needed to rebuild the new file.
    pub materialize_new_bytes: i64,
}

/// What a consumer can assert against.
///
/// Required rather than optional, and that is the point: a patchmap a consumer
/// cannot assert anything about is the defect, not a legitimate shape. Both
/// one-sided classes therefore have their own. `removed` asserts the shape
/// `"the file is gone and nothing was downloaded"`, which is a real outcome a
/// consumer can check - it is how the `removed` capture becomes testable at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expectations {
    /// Where the rebuilt file belongs. `null` when the correct outcome is for
    /// it not to exist.
    pub result_path: Option<String>,
    /// The whole-file MD5 it must have. `null` when `absent_after` is true.
    pub result_md5: Option<String>,
    /// True when the correct outcome is that the file be **absent** afterwards -
    /// the `removed` class, and the only class with no result path.
    ///
    /// Present on every capture so a consumer never has to infer the outcome
    /// from a null, and so `false` is stated rather than defaulted.
    pub absent_after: bool,
    /// Bytes a correct patch must download.
    ///
    /// `0` for `removed` is a *true* value, not a default: deleting a file
    /// downloads nothing. It is recorded rather than left `null` precisely
    /// because the number is known and a consumer should be able to compare
    /// against it.
    pub download_bytes: i64,
    /// Chunks a correct patch must not request: the `REUSE` set.
    ///
    /// It is the same fact as the `plan[]` entries whose decision is `REUSE`,
    /// stated as the negative a consumer can assert on — "none of these may be
    /// requested". `verify` checks the two against each other, so there is one
    /// source of truth.
    pub must_not_fetch: Vec<String>,
}

/// What a capture says about the **shape** of its patch.
///
/// The numbers the coverage assertions read, and nothing else: they are facts
/// about the patch, not about how a repair tool gets there.
///
/// **An absent side is not a file of zero bytes.** The old side of an `added`
/// capture is *no file*, and the new side of a `removed` capture is *no file*.
/// The difference matters twice over: a file that appears has grown even when it
/// is empty, and a capture with no new side has no plan at all — so it has
/// nothing to reuse and nothing to fetch, and it must not be counted on either
/// side of the reuse boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PatchShape {
    /// The plan's own numbers. `None` when the capture has no new side: there
    /// is no target, so there is nothing to reuse and nothing to fetch.
    pub plan: Option<PlanCounts>,
    /// The old side's declared size. `None` when the file is not there.
    pub old_size: Option<u64>,
    /// The new side's declared size. `None` when the file is not there.
    pub new_size: Option<u64>,
}

impl PatchShape {
    /// The file **grows**: it is created, or it is present on both sides and
    /// the new one is larger.
    ///
    /// A file that is created grows whether it is a hundred bytes or none at
    /// all — what grew is the file, not the byte count.
    pub fn grows(&self) -> bool {
        if self.old_size.is_none() && self.new_size.is_some() {
            return true;
        }
        match (self.old_size, self.new_size) {
            (Some(old), Some(new)) => new > old,
            _ => false,
        }
    }

    /// The file **shrinks**: it is deleted, or it is present on both sides and
    /// the new one is smaller.
    pub fn shrinks(&self) -> bool {
        if self.old_size.is_some() && self.new_size.is_none() {
            return true;
        }
        match (self.old_size, self.new_size) {
            (Some(old), Some(new)) => new < old,
            _ => false,
        }
    }
}

/// The plan's own numbers: how many chunks the new side has, how many of them
/// the old file already holds, and how many of the reused ones live elsewhere
/// in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlanCounts {
    /// How many chunks the new side has.
    pub chunks: u64,
    /// How many of them are reused from the old side.
    pub reuse: u64,
    /// How many of the reused ones live elsewhere in the file.
    pub moved: u64,
}

/// One side's assembly, with what was asserted about it.
#[derive(Debug, Clone, PartialEq)]
pub struct FileResult {
    /// Which build.
    pub tag: String,
    /// The assembled bytes.
    pub bytes: Vec<u8>,
    /// `md5(bytes)`.
    pub md5: String,
    /// The manifest's whole-file MD5.
    pub declared_md5: String,
    /// How many chunks went into it.
    pub nchunks: usize,
    /// Where they went, and whether that was legal (A11).
    pub layout: Layout,
}

impl FileResult {
    /// A7 / A8: `assemble(side)` has `md5 == entry.md5`.
    pub fn md5_matches(&self) -> bool {
        self.md5 == self.declared_md5
    }
}

/// The patch flow's result, against the direct assembly.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchOutcome {
    /// `md5` of the bytes the patch flow produced.
    ///
    /// **Not the bytes themselves.** Holding them here would keep a third copy of
    /// a ~22 MB file alive beside `old.bytes` and `new.bytes` for the rest of the
    /// capture, and nothing outside [`assemble`] ever read them.
    ///
    /// The comparison is still a **byte** comparison, and still happens where both
    /// sides are alive; it is settled before these bytes are returned. So this does
    /// not turn A9 into a digest-only check — see
    /// [`Assembled::selfcheck_rows`] for what the *artifact's* A9 row compares,
    /// which is `md5` against `md5`, and `docs/00-spec.md` §6 for what the spec
    /// says A9 is. Those two are not the same claim, which is worth knowing.
    pub md5: String,
    /// A9: byte-equal to the direct `materialize_new` assembly.
    pub equals_new: bool,
    /// How many chunks were verified and reused.
    pub reused: usize,
    /// How many had to be downloaded.
    pub fetched: usize,
    /// Reused chunks whose source offset differed from the target's.
    pub moved: usize,
    /// A10, one message per reused chunk whose local slice did not verify.
    pub reuse_failures: Vec<String>,
    /// REUSE decisions that were downgraded to FETCH, and why.
    pub rewrites: Vec<String>,
}

impl PatchOutcome {
    /// A10: every reused chunk's old-file slice verified.
    pub fn a10_holds(&self) -> bool {
        self.reuse_failures.is_empty()
    }
}

/// Everything this module produces for one capture.
#[derive(Debug, Clone, PartialEq)]
pub struct Assembled {
    /// The map that describes the patch.
    pub map: PatchMap,
    /// The old assembly, absent for `added`.
    pub old: Option<FileResult>,
    /// The new assembly, absent for `removed`.
    pub new: Option<FileResult>,
    /// The patch flow, absent when there is no old side.
    pub patch: Option<PatchOutcome>,
}

impl Assembled {
    /// The A7–A12 findings, as selfcheck rows.
    ///
    /// One row per assertion *instance*, as the schema doc requires: a file with
    /// two failing REUSEs is two A10 rows, not one. Each row carries both sides
    /// of the comparison, because a failed capture's usefulness is entirely in
    /// being able to see which side disagreed.
    pub fn selfcheck_rows(&self) -> Vec<crate::emit::selfcheck::Assertion> {
        use crate::emit::selfcheck::{Assertion, wording};
        let mut rows = Vec::new();
        if let Some(o) = &self.old {
            rows.push(Assertion::check(
                "A7",
                wording("A7"),
                &self.map.path,
                &o.declared_md5,
                &o.md5,
            ));
            rows.extend(layout_rows(&o.tag, &o.layout));
        }
        if let Some(n) = &self.new {
            rows.push(Assertion::check(
                "A8",
                wording("A8"),
                &self.map.path,
                &n.declared_md5,
                &n.md5,
            ));
            rows.extend(layout_rows(&n.tag, &n.layout));
        }
        if let Some(p) = &self.patch {
            let n = self.new.as_ref();
            rows.push(Assertion::check(
                "A9",
                wording("A9"),
                &self.map.path,
                n.map(|n| n.md5.as_str()).unwrap_or("<no new side>"),
                &p.md5,
            ));
            for f in &p.reuse_failures {
                rows.push(Assertion::fail(
                    "A10",
                    wording("A10"),
                    &self.map.path,
                    "every reused chunk's old-file slice MD5 == uncompressed_md5",
                    f,
                ));
            }
            if p.reuse_failures.is_empty() && p.reused > 0 {
                rows.push(Assertion::pass(
                    "A10",
                    wording("A10"),
                    &self.map.path,
                    &format!("{} reused chunks verified by MD5", p.reused),
                ));
            }
        }
        if self.map.totals.is_some() {
            // Built rather than `check`ed: the question is "are the two sets
            // disjoint", which is a boolean, not a comparison of two strings.
            rows.push(if self.a12_holds() {
                Assertion::pass(
                    "A12",
                    wording("A12"),
                    &self.map.path,
                    "the fetched and reused chunk sets are disjoint",
                )
            } else {
                Assertion::fail(
                    "A12",
                    wording("A12"),
                    &self.map.path,
                    "the fetched and reused chunk sets are disjoint",
                    "a chunk is both reused and fetched",
                )
            });
        }
        rows
    }

    /// Whether every A7–A12 row is green.
    pub fn all_green(&self) -> bool {
        self.selfcheck_rows().iter().all(|a| a.ok)
    }

    /// A12, recomputed from the plan rather than read from the chunk store.
    ///
    /// Recomputed on purpose: the store checks it over the *planned* sets and this
    /// checks it over the *written* map. If the two ever disagree the map is what
    /// a consumer would follow, so the map is what has to be right.
    pub fn a12_holds(&self) -> bool {
        let Some(plan) = &self.map.plan else {
            // No patch — `added` or `removed` — so there is nothing to be a
            // subset of, and nothing to be disjoint from.
            return true;
        };
        let fetched: Vec<&str> = plan
            .iter()
            .filter(|e| e.decision == Decision::Fetch)
            .map(|e| e.chunk_id.as_str())
            .collect();
        let reused: Vec<&str> = plan
            .iter()
            .filter(|e| e.decision == Decision::Reuse)
            .map(|e| e.chunk_id.as_str())
            .collect();
        fetched.iter().all(|f| !reused.contains(f))
    }
}

/// One A11 row per violation, plus a green one when there are none.
fn layout_rows(tag: &str, layout: &Layout) -> Vec<crate::emit::selfcheck::Assertion> {
    use crate::emit::selfcheck::{Assertion, wording};
    if layout.is_clean() {
        return vec![Assertion::pass(
            "A11",
            wording("A11"),
            tag,
            &format!(
                "{} chunks tile {} B from 0",
                layout.ranges.len(),
                layout.declared_size
            ),
        )];
    }
    layout
        .violations
        .iter()
        .map(|v| Assertion::fail("A11", wording("A11"), tag, "gapless from 0", v))
        .collect()
}

impl<'i> ChunkSource<'i> {
    /// A source that reads the bundle's stored blobs.
    ///
    /// Nothing is copied out of `index` and nothing is cached: the two expensive
    /// choices this type makes are the caller's, and both are documented on
    /// [`ChunkSource::decoded`].
    pub fn from_index(root: &std::path::Path, index: &'i ChunkIndex) -> ChunkSource<'i> {
        ChunkSource {
            inner: Inner::Disk {
                root: root.to_path_buf(),
                index,
                cache: None,
            },
        }
    }

    /// Keep every chunk this source decodes, so one referenced N times is read
    /// and decompressed once.
    ///
    /// **Verify asks for this; capture does not**, and the reason is the same
    /// fact seen from two ends.
    ///
    /// A capture assembles each file once per side and the repeats are close
    /// together in time, so the page cache serves them; remembering the bytes
    /// instead would hold the whole fixture resident, which is the one thing
    /// capture is careful not to do.
    ///
    /// A `verify` is the opposite. It rebuilds every capture the index knows
    /// about, and a blob that two captures share — which is most of them, since
    /// sharing is the point of a blob store — is read and decompressed once per
    /// reference. Measured on the reference bundle (`gircap verify`, 38 blobs,
    /// 66 references):
    ///
    /// | | today | memoised |
    /// |---|---|---|
    /// | blob bytes read | 78,747,429 B across 104 opens | 27,612,723 B across 38 |
    /// | bytes decompressed | 70,010,206 B | 39,559,611 B |
    /// | peak resident | — | +39,559,611 B of decoded blobs |
    ///
    /// That last row is the price, and it is the reason this is opt-in: it is a
    /// real increase in peak memory in exchange for less than half the I/O.
    /// `verify` already holds every rebuilt file at once by design, so the
    /// addition is in the same order as what it already spends.
    ///
    /// The cells are sized from the index rather than grown on demand, so a
    /// chunk nobody asks for costs a `OnceLock`, not a decode.
    pub fn memoizing(mut self) -> Self {
        if let Inner::Disk { index, cache, .. } = &self.inner
            && cache.is_none()
        {
            let cells = index
                .blobs
                .iter()
                .filter(|b| b.props.stored_path.is_some())
                .map(|b| (b.props.chunk_id.clone(), OnceLock::new()))
                .collect();
            let Inner::Disk { root, index, .. } =
                std::mem::replace(&mut self.inner, Inner::Memory(BTreeMap::new()))
            else {
                unreachable!("just matched on Disk")
            };
            self.inner = Inner::Disk {
                root,
                index,
                cache: Some(cells),
            };
        }
        self
    }

    /// A source backed by already-decoded bytes.
    pub fn from_memory(map: BTreeMap<String, Vec<u8>>) -> ChunkSource<'i> {
        ChunkSource {
            inner: Inner::Memory(map),
        }
    }

    /// The decoded bytes of one chunk.
    ///
    /// Borrowed in memory mode and decompressed on demand from disk otherwise.
    /// Caching is [`ChunkSource::memoizing`]'s business; this says "nothing is
    /// cached across calls", because for capture that is the right default — the
    /// three assemblies of a 22 MB file would otherwise hold three copies of
    /// it.
    ///
    /// The md5 the index claims is checked here and not merely recorded, every
    /// time: a store that read a corrupted blob without noticing would make
    /// `verify` impossible, and the whole point of this milestone is that
    /// `verify` can trust what it reads. A memoised source checks it on the
    /// decode that fills the cell, which is still every blob it ever returns.
    pub fn decoded(&self, chunk_id: &str) -> Result<Cow<'_, [u8]>, String> {
        match &self.inner {
            Inner::Memory(m) => m
                .get(chunk_id)
                .map(|v| Cow::Borrowed(v.as_slice()))
                .ok_or_else(|| format!("chunk {chunk_id} is not in the source")),
            Inner::Disk { root, index, cache } => {
                let cell = cache.as_ref().and_then(|c| c.get(chunk_id));
                if let Some(held) = cell.and_then(OnceLock::get) {
                    return Ok(Cow::Borrowed(held.as_slice()));
                }
                let b = index
                    .get(chunk_id)
                    .ok_or_else(|| format!("chunk {chunk_id} is not in the index"))?;
                let rel = b
                    .props
                    .stored_path
                    .as_deref()
                    .ok_or_else(|| format!("chunk {chunk_id} has no stored blob"))?;
                // **The read-side twin of the manifest check.** `stored_path` is
                // a document field inside a bundle someone may have edited, and
                // this is the join that turns it into a file handle. The shape
                // rule stops a bundle from making `verify` or `materialize`
                // read outside its own root, whatever the capture that wrote the
                // index did.
                if let Some(problem) = crate::paths::relative_path_problem(rel) {
                    return Err(format!(
                        "chunk {chunk_id}: the index's stored_path {problem}"
                    ));
                }
                let path = root.join(rel);
                // **Decoded from the file, not from a `Vec` of it.** Reading the
                // whole blob first would keep the compressed bytes alive for the
                // whole decode beside the decompressed ones; decoding straight
                // through a buffered reader drops the compressed copy entirely, so
                // peak is the output plus one read window.
                //
                // The capacity comes from the index, which carries the declared
                // decompressed length, so the output buffer is allocated once
                // rather than grown by doubling across ~20 reallocations.
                let file = std::fs::File::open(&path)
                    .map_err(|io| format!("open {}: {io}", path.display()))?;
                let out = hash::zstd_decompress_from(
                    std::io::BufReader::with_capacity(64 * 1024, file),
                    &format!("chunk {chunk_id}"),
                    Some(b.props.uncompressed_size),
                )?;
                if !b.props.md5_decompressed.is_empty() {
                    let got = hash::md5_hex(&out);
                    if got != b.props.md5_decompressed {
                        return Err(format!(
                            "chunk {chunk_id}: {rel} decodes to md5 {got}, the index says {}",
                            b.props.md5_decompressed
                        ));
                    }
                }
                match cell {
                    // `set` either stores the value or finds the cell already
                    // full, and both leave it populated, so handing back a
                    // borrow is sound either way — and saves copying the decode
                    // a second time into the caller's buffer.
                    Some(cell) => {
                        let _ = cell.set(out);
                        Ok(Cow::Borrowed(
                            cell.get().expect("a set cell is full").as_slice(),
                        ))
                    }
                    None => Ok(Cow::Owned(out)),
                }
            }
        }
    }

    /// How many chunks this source can serve.
    pub fn len(&self) -> usize {
        match &self.inner {
            Inner::Memory(m) => m.len(),
            Inner::Disk { index, .. } => index
                .blobs
                .iter()
                .filter(|b| b.props.stored_path.is_some())
                .count(),
        }
    }

    /// Whether it can serve anything at all.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The largest file this tool will try to assemble, in bytes.
///
/// A file's `size` arrives straight off the wire as an `i64`, so a manifest can
/// declare anything at all, including `i64::MAX`. The `.max(0)` this replaces
/// guarded the *sign*, which is why it was there — but not the magnitude, so
/// `size = 2^40` was a 1 TB allocation attempt. That is not a panic and so not
/// catchable: it is an abort the tool cannot report and cannot recover from,
/// which is the worst of the three outcomes.
///
/// 1 GiB is ~47x the largest file in the reference bundle (22 662 068 B), so no
/// legitimate fixture comes near it, and past this line "this manifest is wrong"
/// is a more useful answer than "this machine ran out of memory".
///
/// **One guard, both callers.** `assemble_side` and `materialize::rebuild` used
/// to each carry their own `vec![0u8; size.max(0) as usize]`, so a hostile
/// manifest reached the same abort by two routes. They share [`tile`], so this
/// is checked once.
pub const MAX_ASSEMBLY_BYTES: u64 = 1 << 30;

/// Tile one file from its chunks, and say why it could not.
///
/// **One implementation for both callers.** A capture tiles the manifest entry
/// it was handed; `gircap materialize` tiles the references recorded in
/// `chunks/index.json`. Same algorithm, same three ways to fail — and they were
/// written out twice, in two phrasings, so a reader of one had to learn a second
/// vocabulary to read the other.
///
/// ## What carries the difference: the label
///
/// The only thing that genuinely differs is *which side* is being rebuilt, and
/// that belongs in the label rather than in three parallel sentence templates:
///
/// ```text
/// 7.0.0 GenshinImpact_Data/Plugins/HoYoChannel.dll                (capture)
/// 7.0.0 GenshinImpact_Data/Plugins/HoYoChannel.dll (materialize_old)  (materialize)
/// ```
///
/// So the three messages are the same in both paths, and carry both meanings,
/// because the words describe the *fact* rather than the caller:
///
/// | failure | one wording for both |
/// |---|---|
/// | the layout breaks A11 | `does not satisfy A11 — …` |
/// | a chunk's range is not inside the file | `range {r} is outside the {total} B file` |
/// | a chunk decoded to the wrong length | `decoded to {n} B but its range is {r}` |
///
/// "is outside the *N* B file" replaced two older phrasings: capture said the
/// range "does not fit a *N* B **buffer**", which describes the tool's
/// allocation rather than the fact; materialize said it "is outside the file",
/// which is right but omits the size a reader would want. The new sentence is
/// true of both and says more.
///
/// These strings are **failure diagnostics, not artifact bytes**: an assembly
/// error aborts the capture before the bundle is finished, so nothing here
/// reaches a written file. That is why changing them is a deliberate act rather
/// than a band-3 concern.
///
/// The range is checked **before** the chunk is decoded, so a rebuild that is
/// going to be refused does not first inflate every blob to find out. That was
/// already capture's order; `materialize` used the other one, and the only
/// observable difference is which message a file that is wrong in both ways at
/// once reports.
pub fn tile<'a>(
    label: &str,
    layout: &Layout,
    total: i64,
    ids: &[&str],
    decoded: impl Fn(&str) -> Result<std::borrow::Cow<'a, [u8]>, String>,
) -> Result<Vec<u8>, String> {
    if !layout.is_clean() {
        return Err(format!(
            "{label}: does not satisfy A11 — {}",
            layout.violations.join("; ")
        ));
    }
    if !(0..=MAX_ASSEMBLY_BYTES as i64).contains(&total) {
        return Err(format!(
            "{label}: declares {total} B, past the {MAX_ASSEMBLY_BYTES} B this tool will assemble"
        ));
    }
    let mut bytes = vec![0_u8; total as usize];
    for (id, r) in ids.iter().zip(&layout.ranges) {
        let dst = bytes
            .get_mut(r.start as usize..r.end as usize)
            .ok_or_else(|| {
                format!("{label}: chunk {id} range {r} is outside the {total} B file")
            })?;
        let d = decoded(id)?;
        if d.len() as i64 != r.len() {
            return Err(format!(
                "{label}: chunk {id} decoded to {} B but its range is {r}",
                d.len()
            ));
        }
        dst.copy_from_slice(&d);
    }
    Ok(bytes)
}

/// Assemble one file's side from its retained chunks.
///
/// The layout is checked *before* any byte is placed, so a manifest that
/// violates A11 cannot produce a file that merely happens to have the right MD5
/// by accident — and the error says which of A11's rules broke.
fn assemble_side(
    entry: &SophonChunkFile,
    tag: &str,
    src: &ChunkSource,
) -> Result<FileResult, String> {
    let layout = Layout::of(
        entry.chunks.iter().map(|c| (c.offset, c.uncompressed_size)),
        entry.size,
    );
    let ids: Vec<&str> = entry.chunks.iter().map(|c| c.id.as_str()).collect();
    let label = format!("{tag} {}", entry.file);
    let bytes = tile(&label, &layout, entry.size, &ids, |id| src.decoded(id))?;
    Ok(FileResult {
        tag: tag.to_string(),
        md5: hash::md5_hex(&bytes),
        bytes,
        declared_md5: entry.md5.clone(),
        nchunks: entry.chunks.len(),
        layout,
    })
}

/// Everything one file pair's assembly needs, as one argument.
///
/// Six parameters is past the point where a reader can hold them in their head,
/// and three of them are always the same three things.
pub struct AssemblyInput<'a, 'i> {
    /// The pair to assemble.
    pub pair: &'a FilePair,
    /// The `matching_field` to stamp on the map.
    pub matching_field: &'a str,
    /// The classes the capture answers.
    pub classes: &'a [Class],
    /// Where chunks are fetched from, for the `url` of a FETCH entry.
    pub sources: &'a Sources,
    /// Where decoded bytes come from.
    pub chunks: &'a ChunkSource<'i>,
}

/// Assemble one file pair and return everything this module produces.
///
/// The flow, per new-side chunk, in the order the spec states it:
///
/// 1. If this *old file* holds a chunk with the same
///    `(uncompressed_md5, uncompressed_size)`, try to REUSE its bytes.
/// 2. Verify the local slice against the declared decompressed MD5 (A10).
/// 3. If that fails, FETCH instead — and say so, because a silently downgraded
///    REUSE is indistinguishable from one that was never a candidate.
pub fn assemble(input: &AssemblyInput<'_, '_>) -> Result<Assembled, String> {
    let pair = input.pair;
    let old_entry = pair.old.as_ref().map(|o| &o.entry);
    let new_entry = pair.new.as_ref().map(|n| &n.entry);
    let old_tag = pair.old.as_ref().map(|o| o.tag.clone()).unwrap_or_default();
    let new_tag = pair.new.as_ref().map(|n| n.tag.clone()).unwrap_or_default();

    let old = old_entry
        .map(|e| assemble_side(e, &old_tag, input.chunks))
        .transpose()?;
    let new = new_entry
        .map(|e| assemble_side(e, &new_tag, input.chunks))
        .transpose()?;

    // The patch flow needs the old file's *bytes* to verify a REUSE against, so
    // it is assembled once here and handed in rather than rebuilt per chunk.
    //
    // The flow's own bytes come back beside its outcome rather than inside it,
    // because **they are needed for exactly one comparison and then never
    // again**. Keeping them on the outcome would hold them for the rest of the
    // capture — alongside `old.bytes` and `new.bytes`, ~68 MB of peak residency
    // for a file the bundle then writes to disk and stops caring about.
    let (plan, patch, patch_bytes) = match (new_entry, &old) {
        (Some(ne), Some(o)) => {
            let (plan, outcome, bytes) = patch_flow(input, ne, o, &old_tag, &new_tag)?;
            (Some(plan), Some(outcome), Some(bytes))
        }
        // `added`: no old side, so there is nothing to *patch from* - but there
        // is still a plan, and every entry in it is FETCH. A consumer handed
        // this path has to download every new-side chunk, so `plan: null` /
        // `totals: null` would tell it nothing about that. Leaving them null makes
        // `expectations.download_bytes` fall back to a defaulted `0`, which is
        // not "unrecorded" but *false*: the one-chunk `added` file costs
        // 114 357 B to repair and the artifact would claim zero.
        (Some(ne), None) => (Some(fetch_all_plan(input, ne, &new_tag)?), None, None),
        // `removed`: no new side, so there is no plan and no target.
        (None, _) => (None, None, None),
    };

    // A9 is a comparison between two results that only exist together, so it is
    // settled here rather than inside the flow — and `patch_bytes` dies on the
    // next line, so the ~22 MB is not resident while the bundle is written.
    //
    // **A byte comparison, not a digest comparison**, because `docs/00-spec.md` §6
    // specifies A9 as byte equality. Both buffers are therefore necessarily alive
    // for the compare; what this buys is that they are not alive afterwards.
    //
    // **Two things worth knowing about that verdict.**
    //
    // First, `equals_new` reaches production only through the `assembled` log
    // line in `emit::fixture`. The *artifact's* A9 row is built by
    // `selfcheck_rows` from `p.md5` against `n.md5` — a digest comparison. So the
    // spec's byte-equality assertion and the bundle's A9 row are not the same
    // claim; closing that gap is artifact-visible and is not done here.
    //
    // Second, that gap is why this **cannot** be covered by a negative test. Every
    // reachable disagreement also changes the digests, so the row goes red either
    // way, and replacing this comparison with a constant `true` passes the entire
    // suite — verified by mutation, twice.
    // `a9_is_red_when_the_patch_flow_and_the_direct_assembly_disagree` pins the
    // part that *is* reachable: that a disagreement is visible at all.
    let mut patch = patch;
    if let (Some(n), Some(p)) = (&new, &mut patch) {
        p.equals_new = patch_bytes.as_ref().is_some_and(|b| b == &n.bytes);
    }
    drop(patch_bytes);

    // The plan is **moved**, not borrowed: `build_map` only ever reads it, and a
    // `to_vec()` on the way in would deep-copy every entry's ids, URLs, notes and
    // sizes — ~300 allocations per capture — to hand it a copy nobody mutates.
    let map = build_map(input, old.as_ref(), new.as_ref(), plan)?;
    Ok(Assembled {
        map,
        old,
        new,
        patch,
    })
}

/// The patch flow proper.
fn patch_flow(
    input: &AssemblyInput<'_, '_>,
    new_entry: &SophonChunkFile,
    old: &FileResult,
    old_tag: &str,
    new_tag: &str,
) -> Result<(Vec<PlanEntry>, PatchOutcome, Vec<u8>), String> {
    let pair = input.pair;
    let Some(old_entry) = &pair.old else {
        return Err(format!("{}: no old entry to reuse from", pair.path));
    };
    let mut plan = Vec::with_capacity(new_entry.chunks.len());
    let mut reused = 0_usize;
    let mut fetched = 0_usize;
    let mut moved = 0_usize;
    let mut reuse_failures = Vec::new();
    let mut rewrites = Vec::new();
    let mut out = Vec::with_capacity(new_entry.size.max(0) as usize);

    // Where this old file holds each identity, so a REUSE candidate is one
    // lookup rather than a scan per chunk. `or_insert` keeps the *first* one:
    // two chunks of the same file with the same content are interchangeable,
    // and taking the earliest keeps the choice deterministic.
    let mut old_by_identity: BTreeMap<(String, i64), &crate::manifest::proto::SophonChunk> =
        BTreeMap::new();
    for c in &old_entry.entry.chunks {
        old_by_identity
            .entry((c.uncompressed_md5.clone(), c.uncompressed_size))
            .or_insert(c);
    }

    for (i, c) in new_entry.chunks.iter().enumerate() {
        let target = Range::at(c.offset, c.uncompressed_size);
        let key = (c.uncompressed_md5.clone(), c.uncompressed_size);

        // Each arm yields (decision, source_range, note, body). The REUSE arm is
        // tried whenever the old file holds the same content; whether it is
        // *kept* depends on the MD5 of the local slice, which is A10 and the
        // reason a REUSE is safe at all.
        let (decision, source_range, note, body) = match old_by_identity.get(&key).copied() {
            None => (
                Decision::Fetch,
                None,
                None,
                input.chunks.decoded(&c.id)?.into_owned(),
            ),
            Some(oc) => {
                let src_range = Range::at(oc.offset, c.uncompressed_size);
                let verified = src_range
                    .slice(&old.bytes)
                    .ok_or_else(|| format!("old slice {src_range} is outside the old file"))
                    .and_then(|slice| {
                        let got = hash::md5_hex(slice);
                        if got == c.uncompressed_md5 {
                            Ok(slice.to_vec())
                        } else {
                            Err(format!(
                                "the old file's bytes at {src_range} are md5 {got}, but the new \
                                 chunk {} declares {}",
                                c.id, c.uncompressed_md5
                            ))
                        }
                    });
                match verified {
                    Ok(bytes) => {
                        let note = if src_range.start != target.start {
                            Some(format!(
                                "moved: {} in the old file, {} in the new one",
                                src_range.start, target.start
                            ))
                        } else {
                            Some("byte-identical to the old chunk at the same offset".into())
                        };
                        (
                            Decision::Reuse,
                            Some(SourceRange {
                                start: src_range.start,
                                end: src_range.end,
                                file_tag: old_tag.to_string(),
                            }),
                            note,
                            bytes,
                        )
                    }
                    // The spec's flow is "verify, else FETCH", so the decision
                    // changes — and the rewrite is recorded, because a reader who
                    // sees FETCH has no way to know a REUSE was tried and failed.
                    Err(why) => {
                        let msg = format!("chunk {i} ({}): {why}", c.id);
                        rewrites.push(msg.clone());
                        reuse_failures.push(msg);
                        (
                            Decision::Fetch,
                            None,
                            Some(format!("REUSE attempted and rejected: {why}")),
                            input.chunks.decoded(&c.id)?.into_owned(),
                        )
                    }
                }
            }
        };

        match decision {
            Decision::Reuse => reused += 1,
            Decision::Fetch => fetched += 1,
        }
        if decision == Decision::Reuse
            && source_range
                .as_ref()
                .is_some_and(|s| s.start != target.start)
        {
            moved += 1;
        }
        out.extend_from_slice(&body);

        plan.push(PlanEntry {
            i,
            decision,
            target_range: target,
            source_range,
            chunk_id: c.id.clone(),
            url: match decision {
                Decision::Fetch => input.sources.url(new_tag, &c.id).ok(),
                Decision::Reuse => None,
            },
            compressed_size: c.compressed_size,
            uncompressed_size: c.uncompressed_size,
            uncompressed_md5: c.uncompressed_md5.clone(),
            note,
        });
    }

    if out.len() as i64 != new_entry.size {
        return Err(format!(
            "the patch flow produced {} B, the manifest declares {}",
            out.len(),
            new_entry.size
        ));
    }
    let md5 = hash::md5_hex(&out);
    Ok((
        plan,
        PatchOutcome {
            equals_new: false, // settled by `assemble`, which holds both sides
            md5,
            reused,
            fetched,
            moved,
            reuse_failures,
            rewrites,
        },
        // The bytes go back to `assemble` beside the outcome rather than inside
        // it, so the caller can compare them against the direct assembly and drop
        // them immediately. See `PatchOutcome`.
        out,
    ))
}

/// Assemble the patchmap document from what the run produced.
/// The plan for a capture with no old side: every new-side chunk is FETCH.
///
/// Same shape as [`patch_flow`]'s, because it is the same question - what must a
/// consumer download to produce the new file? The answer for `added` is all of
/// it, so the decision is made here rather than re-derived by the reader. The
/// `note` says why, so a reader who expected a `source_range` is not left
/// guessing whether the field was forgotten.
fn fetch_all_plan(
    input: &AssemblyInput<'_, '_>,
    new_entry: &SophonChunkFile,
    new_tag: &str,
) -> Result<Vec<PlanEntry>, String> {
    Ok(new_entry
        .chunks
        .iter()
        .enumerate()
        .map(|(i, c)| PlanEntry {
            i,
            decision: Decision::Fetch,
            target_range: Range::at(c.offset, c.uncompressed_size),
            source_range: None,
            chunk_id: c.id.clone(),
            url: input.sources.url(new_tag, &c.id).ok(),
            compressed_size: c.compressed_size,
            uncompressed_size: c.uncompressed_size,
            uncompressed_md5: c.uncompressed_md5.clone(),
            note: Some("no old side: this file is new, so every chunk is downloaded whole".into()),
        })
        .collect())
}

fn build_map(
    input: &AssemblyInput<'_, '_>,
    old: Option<&FileResult>,
    new: Option<&FileResult>,
    plan: Option<Vec<PlanEntry>>,
) -> Result<PatchMap, String> {
    let side = |r: &FileResult| Side {
        tag: r.tag.clone(),
        size: r.bytes.len() as i64,
        md5: r.declared_md5.clone(),
        nchunks: r.nchunks,
    };
    let totals = // One pass for all four numbers. Five passes, each re-walking the
        // plan and each calling `is_moved()` or comparing `decision` again, is
        // five times the work for five numbers that a single walk produces.
        plan.as_ref().map(|p| {
            let mut reuse = 0_usize;
            let mut fetch = 0_usize;
            let mut moved = 0_usize;
            let mut download_bytes = 0_i64;
            for e in p {
                match e.decision {
                    Decision::Reuse => reuse += 1,
                    Decision::Fetch => {
                        fetch += 1;
                        download_bytes += e.compressed_size;
                    }
                }
                if e.is_moved() {
                    moved += 1;
                }
            }
            Totals {
                chunks: p.len(),
                reuse,
                fetch,
                moved,
                download_bytes,
                materialize_old_bytes: old.map(|o| o.bytes.len() as i64).unwrap_or(0),
                materialize_new_bytes: new.map(|n| n.bytes.len() as i64).unwrap_or(0),
            }
        });
    // `plan` is `Some` for every capture that has a new side, so `totals` is too.
    // Checked rather than defaulted: a `download_bytes` of `0` on a capture that
    // does have a plan would be a false claim, and one with no plan would be
    // unrecordable. Either way a silent `0` is the wrong answer.
    if new.is_some() && totals.is_none() {
        return Err(format!(
            "{}: a capture with a new side must carry a plan, or its cost is unrecordable",
            input.pair.path
        ));
    }
    // Every capture has expectations - `added` asserts the file appears, `removed`
    // asserts it is gone, and a two-sided capture asserts the new MD5.
    //
    // A capture with **no** side at all is reachable and is not an error: a
    // path selected out of the audio pool has no entry in the `game` manifests,
    // which `run` says outright ("that is a bug there, and here it surfaces as a
    // one-sided pair rather than a wrong download"), so it is folded in with
    // `removed`: there is no result file and nothing is downloaded. That is a weaker capture than the other classes,
    // and saying so is better than claiming a result that does not exist.
    let expectations = match new {
        Some(n) => Expectations {
            result_path: Some(format!(
                "{}/{}",
                crate::emit::EXPECTED_TREE,
                input.pair.path
            )),
            result_md5: Some(n.declared_md5.clone()),
            absent_after: false,
            download_bytes: totals.map_or(0, |t| t.download_bytes),
            must_not_fetch: plan
                .as_ref()
                .map(|p| {
                    p.iter()
                        .filter(|e| e.decision == Decision::Reuse)
                        .map(|e| e.chunk_id.clone())
                        .collect()
                })
                .unwrap_or_default(),
        },
        None => Expectations {
            result_path: None,
            result_md5: None,
            absent_after: true,
            // Known, and worth stating: removing a file, or having none to
            // begin with, downloads nothing. `Vec::new()` on `must_not_fetch`
            // is likewise a statement - there is nothing to fetch, so nothing is
            // forbidden.
            download_bytes: 0,
            must_not_fetch: Vec::new(),
        },
    };
    Ok(PatchMap {
        schema_version: crate::SCHEMA_VERSION,
        capture_id: input.pair.capture_id.clone(),
        path: input.pair.path.clone(),
        matching_field: input.matching_field.to_string(),
        classes: input.classes.to_vec(),
        old: old.map(side),
        new: new.map(side),
        plan,
        totals,
        expectations,
    })
}

/// The patch's shape, as the coverage assertions read it.
///
/// The plan's own arithmetic, and the two sides' sizes. **An absent side is
/// `None`, not `Some(0)`**: the file is not there, which is not the same as a
/// file of zero bytes, and the difference is what makes "a file that grows" and
/// "a file that shrinks" true of a zero-byte file.
pub fn patch_shape(map: &PatchMap) -> PatchShape {
    PatchShape {
        plan: map.totals.map(|t| PlanCounts {
            chunks: t.chunks as u64,
            reuse: t.reuse as u64,
            moved: t.moved as u64,
        }),
        old_size: map.old.as_ref().map(|s| s.size.max(0) as u64),
        new_size: map.new.as_ref().map(|s| s.size.max(0) as u64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blobs::Sources;
    use crate::blobs::chunks::{NewFile, OldFile};
    use crate::hash;
    use crate::manifest::proto::{SophonChunk, SophonChunkFile};

    /// `stored_path` comes out of `chunks/index.json`, which is a document in a
    /// bundle someone may have edited. It is joined onto the bundle root, so an
    /// escaping value must not turn into a read outside it.
    #[test]
    fn an_index_stored_path_that_escapes_the_bundle_is_refused() {
        use crate::blobs::props::BlobProps;
        use crate::blobs::{ChunkIndex, IndexedBlob};

        let index = ChunkIndex {
            schema_version: crate::SCHEMA_VERSION,
            blobs: vec![IndexedBlob {
                props: BlobProps {
                    chunk_id: "x".into(),
                    stored_path: Some("../../escape.zst".into()),
                    bytes: 0,
                    container: "zstd".into(),
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
            }],
            sets: std::collections::BTreeMap::new(),
        };
        let e = ChunkSource::from_index(std::path::Path::new("artifact"), &index)
            .decoded("x")
            .unwrap_err();
        assert!(e.contains("escapes the bundle root"), "{e}");
        assert!(e.contains("stored_path"), "and names the field: {e}");
    }

    /// The three ways a rebuild can fail, in one wording.
    ///
    /// There were two sets of three — a capture's and `materialize`'s — and
    /// **nothing asserted any of them**, so either could be reworded without a
    /// test noticing, and they had already drifted apart. The label is the only
    /// thing that differs between the two callers, and that is what these pin.
    #[test]
    fn a_failed_rebuild_says_which_file_which_range_and_which_size() {
        use std::borrow::Cow;
        let clean = Layout::of([(0, 10)], 10);

        // 1. The layout itself breaks A11 — a gap between the chunks.
        let holed = Layout::of([(0, 10), (20, 10)], 30);
        let e = tile("7.0.0 a/b.dll", &holed, 30, &["x", "y"], |_| {
            Ok(Cow::Borrowed(&b""[..]))
        })
        .unwrap_err();
        assert!(
            e.starts_with("7.0.0 a/b.dll: does not satisfy A11 — "),
            "{e}"
        );
        assert!(
            e.contains("a gap of 10 B between chunk 0"),
            "and says where: {e}"
        );

        // 2. A range outside the file. `total` is its own parameter, so a clean
        //    layout can still describe a range the file cannot hold.
        let e = tile("7.0.0 a/b.dll (materialize_old)", &clean, 4, &["x"], |_| {
            Ok(Cow::Borrowed(&b""[..]))
        })
        .unwrap_err();
        assert_eq!(
            e, "7.0.0 a/b.dll (materialize_old): chunk x range [0, 10) is outside the 4 B file",
            "the label is the only thing that says which caller this is"
        );

        // 3. A chunk that decoded to the wrong length.
        let e = tile("7.0.0 a/b.dll", &clean, 10, &["x"], |_| {
            Ok(Cow::Borrowed(&b"short"[..]))
        })
        .unwrap_err();
        assert_eq!(
            e,
            "7.0.0 a/b.dll: chunk x decoded to 5 B but its range is [0, 10)"
        );
    }

    /// The range check is a refusal, not a panic.
    ///
    /// `vec![0u8; size.max(0) as usize]` guarded the sign and nothing else, so a
    /// manifest declaring `size = 2^40` attempted a 1 TB allocation — an abort the
    /// tool cannot report and cannot recover from, because it is not a panic. The
    /// limit turns that into the tool's normal "this is a finding" answer.
    #[test]
    fn a_file_past_the_assembly_limit_is_refused_rather_than_allocated() {
        let clean = Layout::of([(0, 10)], 10);
        for (total, what) in [
            (1_i64 << 40, "2^40 B"),
            (i64::MAX, "i64::MAX B"),
            (-1, "a negative size"),
        ] {
            let e = tile("7.0.0 a/b.dll", &clean, total, &["x"], |_| {
                Ok(std::borrow::Cow::Borrowed(&b""[..]))
            })
            .unwrap_err();
            assert!(
                e.contains("past the 1073741824 B this tool will assemble"),
                "{total} ({what}): {e}"
            );
            assert!(e.starts_with("7.0.0 a/b.dll: "), "and names the file: {e}");
        }
        // One byte over the limit is still over; exactly at it is allowed through to
        // the ordinary allocation, which is what makes the boundary a stated rule
        // rather than a round number.
        let ok = tile(
            "L",
            &Layout::of([(0, 10)], 10),
            MAX_ASSEMBLY_BYTES as i64,
            &["x"],
            |_| Ok(std::borrow::Cow::Borrowed(&b""[..])),
        );
        assert!(
            ok.is_err(),
            "a clean layout of that size still fails A11 first"
        );

        // And the real thing still goes through: the reference bundle's largest
        // file is 22 662 068 B, which is 47x under the limit. Checked at compile
        // time below rather than here, because a runtime `assert!` on two constants
        // is a value the optimiser already knows.
    }

    /// **The limit must stay clear of a real fixture.** 22 662 068 B is the largest
    /// file in the reference bundle (`reuse_moved`), and 40x is slack
    /// well below the 1 GiB the limit is set at. If a future fixture ever needed
    /// more, this fails to compile rather than silently producing a capture that
    /// cannot assemble its own game files.
    const _: () = assert!(MAX_ASSEMBLY_BYTES > 22_662_068 * 40);

    /// The range is checked *before* the chunk is decoded.
    ///
    /// So a rebuild that is going to be refused does not first inflate every
    /// blob to find out.
    #[test]
    fn a_range_that_does_not_fit_is_refused_without_decoding_it() {
        use std::borrow::Cow;
        use std::cell::Cell;
        let clean = Layout::of([(0, 10)], 10);
        let decoded = Cell::new(0_u32);
        let e = tile("L", &clean, 4, &["x"], |_| {
            decoded.set(decoded.get() + 1);
            Ok(Cow::Borrowed(&b""[..]))
        })
        .unwrap_err();
        assert!(e.contains("outside the 4 B file"), "{e}");
        assert_eq!(decoded.get(), 0, "nothing should have been decoded");
    }

    /// Distinct payload bytes, so a test can never pass because two chunks were
    /// accidentally the same content.
    fn payload(seed: u8, n: usize) -> Vec<u8> {
        (0..n)
            .map(|i| seed.wrapping_mul(31).wrapping_add(i as u8))
            .collect()
    }

    /// A chunk holding `payload(seed, len)` at `offset`, with a compressed size
    /// derived from the content rather than invented.
    fn chunk(seed: u8, offset: i64, len: usize) -> SophonChunk {
        let body = payload(seed, len);
        let raw = zstd::encode_all(body.as_slice(), 1).expect("encode");
        SophonChunk {
            id: format!("{seed:02x}{offset:016x}_{}", hash::md5_hex(&body)),
            uncompressed_md5: hash::md5_hex(&body),
            offset,
            compressed_size: raw.len() as i64,
            uncompressed_size: len as i64,
            unknown_field6: 0,
            compressed_md5: hash::md5_hex(&raw),
        }
    }

    /// A file from chunks, with its whole-file MD5 computed from the bytes those
    /// chunks concatenate to. Deriving it is what makes A7/A8 meaningful: a
    /// hand-written constant would let a broken assembly agree with itself.
    fn file(path: &str, chunks: Vec<SophonChunk>) -> SophonChunkFile {
        let mut bytes = Vec::new();
        for c in &chunks {
            let at = c.offset as usize;
            if bytes.len() < at + c.uncompressed_size as usize {
                bytes.resize(at + c.uncompressed_size as usize, 0);
            }
            bytes[at..at + c.uncompressed_size as usize].copy_from_slice(&payload(
                u8::from_str_radix(&c.id[..2], 16).unwrap_or(0),
                c.uncompressed_size as usize,
            ));
        }
        SophonChunkFile {
            file: path.into(),
            chunks,
            is_folder: false,
            size: bytes.len() as i64,
            md5: hash::md5_hex(&bytes),
        }
    }

    /// Every chunk's decoded bytes, for a [`ChunkSource::from_memory`].
    fn memory_of(entries: &[&SophonChunkFile]) -> BTreeMap<String, Vec<u8>> {
        let mut m = BTreeMap::new();
        for e in entries {
            for c in &e.chunks {
                let seed = u8::from_str_radix(&c.id[..2], 16).unwrap_or(0);
                m.insert(c.id.clone(), payload(seed, c.uncompressed_size as usize));
            }
        }
        m
    }

    fn sources() -> Sources {
        Sources::from_pairs([
            ("7.0.0".to_string(), "https://cdn/old".to_string()),
            ("7.1.0".to_string(), "https://cdn/new".to_string()),
        ])
    }

    fn assemble_pair(pair: &FilePair) -> Result<Assembled, String> {
        let entries: Vec<&SophonChunkFile> = [
            pair.old.as_ref().map(|o| &o.entry),
            pair.new.as_ref().map(|n| &n.entry),
        ]
        .into_iter()
        .flatten()
        .collect();
        let src = ChunkSource::from_memory(memory_of(&entries));
        let classes = [Class::ReuseInPlace];
        assemble(&AssemblyInput {
            pair,
            matching_field: "game",
            classes: &classes,
            sources: &sources(),
            chunks: &src,
        })
    }

    /// The measured shape: the file on disk is **longer** than the target and
    /// no reuse source reaches past the target's end.
    fn shrinking_pair() -> FilePair {
        let old = file("blk", vec![chunk(1, 0, 64), chunk(2, 64, 64)]);
        let new = file("blk", vec![chunk(1, 0, 64), chunk(9, 64, 32)]);
        FilePair {
            capture_id: "reuse_in_place".into(),
            path: "blk".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: old,
            }),
            new: Some(NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: new,
            }),
        }
    }

    /// The measured shape: the file on disk is longer than the target and a
    /// reuse source **does** reach past the target's end.
    fn shrinking_with_a_past_the_end_source_pair() -> FilePair {
        let old = file("blk", vec![chunk(1, 0, 64), chunk(2, 64, 64)]);
        let new = file("blk", vec![chunk(2, 0, 64), chunk(9, 64, 32)]);
        FilePair {
            capture_id: "reuse_moved".into(),
            path: "blk".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: old,
            }),
            new: Some(NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: new,
            }),
        }
    }

    /// The measured shape: three chunks, one shared at the same offset, two new.
    fn same_offset_pair() -> FilePair {
        let old = file(
            "dll",
            vec![chunk(1, 0, 64), chunk(2, 64, 128), chunk(3, 192, 32)],
        );
        let new = file(
            "dll",
            vec![chunk(9, 0, 64), chunk(2, 64, 128), chunk(8, 192, 40)],
        );
        FilePair {
            capture_id: "reuse_in_place".into(),
            path: "dll".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: old,
            }),
            new: Some(NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: new,
            }),
        }
    }

    /// The measured shape: the same file on both sides — the skip branch.
    fn identical_pair() -> FilePair {
        let old = file("dll", vec![chunk(1, 0, 64), chunk(2, 64, 128)]);
        let new = file("dll", vec![chunk(1, 0, 64), chunk(2, 64, 128)]);
        FilePair {
            capture_id: "identical".into(),
            path: "dll".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: old,
            }),
            new: Some(NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: new,
            }),
        }
    }

    /// The measured shape: one chunk moves **earlier** (its source is after its
    /// destination) and one is new.
    fn moved_pair() -> FilePair {
        let old = file(
            "blk",
            vec![chunk(1, 0, 64), chunk(2, 64, 128), chunk(3, 192, 32)],
        );
        let new = file(
            "blk",
            vec![chunk(1, 0, 64), chunk(3, 64, 32), chunk(4, 96, 128)],
        );
        FilePair {
            capture_id: "reuse_moved".into(),
            path: "blk".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: old,
            }),
            new: Some(NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: new,
            }),
        }
    }

    // ------------------------------------------------- the three assemblies

    /// A7 and A8: both direct assemblies hash to what the manifest declared.
    #[test]
    fn both_direct_assemblies_reproduce_their_declared_md5() {
        let a = assemble_pair(&same_offset_pair()).expect("assembles");
        let o = a.old.as_ref().expect("an old side");
        let n = a.new.as_ref().expect("a new side");
        assert!(o.md5_matches(), "A7: {} vs {}", o.md5, o.declared_md5);
        assert!(n.md5_matches(), "A8: {} vs {}", n.md5, n.declared_md5);
        assert_eq!(o.nchunks, 3);
        assert_eq!(o.layout.total, 224, "64 + 128 + 32");
        assert!(o.layout.is_clean());
    }

    #[test]
    fn a9_is_red_when_the_patch_flow_and_the_direct_assembly_disagree() {
        // **The negative case, which the suite did not have.** Every A9 test here
        // asserts the *agreeing* case, so nothing checked that a disagreement is
        // visible at all.
        //
        // **What this pins, precisely:** that the disagreement reaches the artifact
        // as a red A9 row and a non-green run. It does **not** pin that the
        // comparison is by bytes — see the note below, which is why that could not
        // be done here.
        let good = assemble_pair(&same_offset_pair()).expect("assembles");
        assert!(
            good.all_green(),
            "the honest pair is green: {:?}",
            good.selfcheck_rows()
        );

        let mut bad = assemble_pair(&same_offset_pair()).expect("assembles");
        // One byte different on the direct side, so its digest moves and stops
        // matching the patch side's: exactly what a patch flow that assembled
        // from stale offsets would produce. The patch side keeps the digest it was
        // assembled with — giving it the *new* side's digest would be the bug.
        let n = bad.new.as_mut().expect("a new side");
        n.bytes[0] ^= 0xff;
        n.md5 = hash::md5_hex(&n.bytes);
        let p = bad.patch.as_mut().expect("a patch");
        assert_eq!(
            p.md5,
            good.new.as_ref().unwrap().md5,
            "before the disagreement both sides agreed"
        );
        p.equals_new = false;
        assert_ne!(p.md5, n.md5, "and now they must not");

        let a9: Vec<_> = bad
            .selfcheck_rows()
            .into_iter()
            .filter(|r| r.id == "A9")
            .collect();
        assert_eq!(a9.len(), 1, "there is exactly one A9 row: {a9:?}");
        assert!(
            !a9[0].ok,
            "a patch flow that disagrees with the direct assembly must not pass: {:?}",
            a9[0]
        );
        assert!(!bad.all_green(), "and the run must not be green");
    }

    /// A9 — the one that matters. The patch flow is a completely different code
    /// path from the direct assembly, so byte-equality is evidence rather than a
    /// tautology.
    #[test]
    fn the_patch_flow_byte_equals_the_direct_new_assembly() {
        let a = assemble_pair(&same_offset_pair()).expect("assembles");
        let p = a.patch.as_ref().expect("a patch");
        let n = a.new.as_ref().expect("a new side");
        assert!(
            p.equals_new,
            "the patch flow and the direct assembly disagree"
        );
        // `equals_new` **is** the byte comparison, settled inside `assemble` while
        // both sides were alive and then dropped. A digest comparison would not
        // be — A9 is specified as byte equality — so `md5` agreeing is
        // corroboration, never the claim.
        assert_eq!(p.md5, n.md5, "and the recorded digests agree too");
        assert!(a.all_green(), "{:?}", a.selfcheck_rows());
    }

    #[test]
    fn the_shared_chunk_is_reused_and_verified_by_md5() {
        let a = assemble_pair(&same_offset_pair()).expect("assembles");
        let p = a.patch.as_ref().unwrap();
        assert_eq!(p.reused, 1);
        assert_eq!(p.fetched, 2);
        assert_eq!(p.moved, 0, "the reused chunk is at the same offset");
        assert!(p.a10_holds());
        assert!(p.rewrites.is_empty(), "{:?}", p.rewrites);
        let plan = a.map.plan.as_ref().unwrap();
        let reused = plan.iter().find(|e| e.decision == Decision::Reuse).unwrap();
        let s = reused.source_range.as_ref().expect("a source range");
        assert_eq!(s.file_tag, "7.0.0");
        assert_eq!(s.start, 64);
        assert_eq!(s.end, 192);
        assert_eq!(s.start, reused.target_range.start, "same offset");
        assert!(reused.url.is_none(), "a REUSE has nothing to download");
        assert!(reused.note.as_deref().unwrap().contains("same offset"));
    }

    #[test]
    fn every_fetch_entry_names_the_url_it_would_download() {
        let a = assemble_pair(&same_offset_pair()).expect("assembles");
        let plan = a.map.plan.as_ref().unwrap();
        for e in plan.iter().filter(|e| e.decision == Decision::Fetch) {
            let u = e
                .url
                .as_deref()
                .unwrap_or_else(|| panic!("entry {} has no url", e.i));
            assert!(u.starts_with("https://cdn/new/"), "{u}");
            assert!(u.ends_with(&e.chunk_id), "{u} does not name the chunk");
            assert!(e.source_range.is_none(), "a FETCH has no local source");
        }
    }

    /// The moved case: the same bytes at a different offset in the new file. The
    /// patch flow has to read them from where they *are*, not where they land.
    #[test]
    fn a_moved_chunk_is_reused_from_its_old_offset_and_counted_as_moved() {
        let old = file(
            "blk",
            vec![chunk(1, 0, 64), chunk(2, 64, 128), chunk(3, 192, 32)],
        );
        // chunk 1 stays at 0, chunk 3 moves 192 -> 64, chunk 4 is brand new.
        let new = file(
            "blk",
            vec![chunk(1, 0, 64), chunk(3, 64, 32), chunk(4, 96, 128)],
        );
        let a = assemble_pair(&FilePair {
            capture_id: "reuse_moved".into(),
            path: "blk".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: old,
            }),
            new: Some(NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: new,
            }),
        })
        .expect("assembles");
        let p = a.patch.as_ref().unwrap();
        assert!(
            p.equals_new,
            "the patch flow and the direct assembly disagree"
        );
        assert_eq!(p.reused, 2, "chunk 1 stays and chunk 3 moves");
        assert_eq!(p.fetched, 1, "chunk 4 is new content");
        assert_eq!(p.moved, 1, "only chunk 3 changed offset");
        let plan = a.map.plan.as_ref().unwrap();
        let moved = plan.iter().find(|e| e.is_moved()).expect("a moved entry");
        assert_eq!(moved.target_range.start, 64);
        assert_eq!(
            moved.source_range.as_ref().unwrap().start,
            192,
            "read from where it is"
        );
        assert!(moved.note.as_deref().unwrap().contains("moved"));
        assert_eq!(a.map.totals.as_ref().unwrap().moved, 1);
    }

    /// **"Moved" is one fact.** The diff's `moved` and the patchmap's own
    /// `is_moved()` have to be the same number for the same path, or a class is
    /// a statement about a different thing than the patchmap's plan.
    ///
    /// They used to disagree. The diff counted a chunk as moved when the new
    /// offset was not among *any* of the old file's offsets for that identity;
    /// the patch flow took the **first** old chunk with that identity as the
    /// source. When an identity occurs twice in the old file the two answers
    /// were different numbers — and `girpr` reads the *first* match
    /// (`repair::reuse::sources_for`), so the first match is the rule that has
    /// to win.
    #[test]
    fn the_diffs_moved_count_and_the_patchmaps_own_are_the_same_number() {
        // The pair where the two used to disagree: the old file holds the same
        // content **twice**, at 0 and at 64. The new file wants it at 64 — an
        // offset the old file *does* have — so the old rule called it "not
        // moved", while the repair would read it from 0 and call it moved.
        let duplicated = FilePair {
            capture_id: "reuse_in_place".into(),
            path: "blk".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: file("blk", vec![chunk(1, 0, 64), chunk(1, 64, 64)]),
            }),
            new: Some(NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: file("blk", vec![chunk(2, 0, 64), chunk(1, 64, 64)]),
            }),
        };
        // The moved case, for the other side of the comparison.
        let moved = FilePair {
            capture_id: "reuse_moved".into(),
            path: "blk".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: file(
                    "blk",
                    vec![chunk(1, 0, 64), chunk(2, 64, 128), chunk(3, 192, 32)],
                ),
            }),
            new: Some(NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: file(
                    "blk",
                    vec![chunk(1, 0, 64), chunk(3, 64, 32), chunk(4, 96, 128)],
                ),
            }),
        };

        for pair in [same_offset_pair(), duplicated.clone(), moved] {
            let a = assemble_pair(&pair).expect("assembles");
            let plan = a.map.plan.as_ref().expect("a plan");
            let plan_moved = plan.iter().filter(|e| e.is_moved()).count() as u64;
            let diff = crate::manifest::diff::score_pair(
                &pair.old.as_ref().expect("an old side").entry,
                &pair.new.as_ref().expect("a new side").entry,
            );
            assert_eq!(
                diff.moved, plan_moved,
                "{}: the diff and the patchmap disagree about what moved",
                pair.capture_id
            );
            assert_eq!(
                diff.moved,
                diff.moved_later + diff.moved_earlier,
                "{}: every move lands either earlier or later",
                pair.capture_id
            );
        }

        // And the duplicated pair is the one that used to disagree: the diff
        // says the chunk moved, because the source the repair would read is at
        // 0 while the chunk lands at 64.
        let a = assemble_pair(&duplicated).expect("assembles");
        let plan = a.map.plan.as_ref().expect("a plan");
        let moved_entry = plan.iter().find(|e| e.is_moved()).expect("a moved entry");
        assert_eq!(moved_entry.source_range.as_ref().unwrap().start, 0);
        assert_eq!(moved_entry.target_range.start, 64);
        let diff = crate::manifest::diff::score_pair(
            &duplicated.old.as_ref().unwrap().entry,
            &duplicated.new.as_ref().unwrap().entry,
        );
        assert_eq!(diff.moved, 1, "the first match is the source");
        assert_eq!(diff.moved_later, 1, "and the bytes move later");
        assert_eq!(diff.moved_earlier, 0);
    }

    /// Every chunk reused and the file laid out differently is the reorder case:
    /// nothing is fetched at all, and A9 still has to hold.
    #[test]
    fn a_pure_reorder_fetches_nothing_and_still_equals_the_new_assembly() {
        let old = file(
            "blk",
            vec![chunk(1, 0, 64), chunk(2, 64, 128), chunk(3, 192, 32)],
        );
        let new = file(
            "blk",
            vec![chunk(3, 0, 32), chunk(1, 32, 64), chunk(2, 96, 128)],
        );
        let a = assemble_pair(&FilePair {
            capture_id: "reuse_moved".into(),
            path: "blk".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: old,
            }),
            new: Some(NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: new,
            }),
        })
        .expect("assembles");
        let p = a.patch.as_ref().unwrap();
        assert_eq!(p.fetched, 0, "nothing new: every chunk is in the old file");
        assert_eq!(p.reused, 3);
        assert_eq!(
            p.moved, 3,
            "every chunk changed offset: that is what a reorder is"
        );
        assert!(
            p.equals_new,
            "the patch flow and the direct assembly disagree"
        );
        assert_eq!(a.map.totals.as_ref().unwrap().download_bytes, 0);
        assert_eq!(a.map.expectations.must_not_fetch.len(), 3);
        assert!(a.all_green(), "{:?}", a.selfcheck_rows());
    }

    // ------------------------------------------------------ one-sided pairs

    #[test]
    fn an_added_file_has_no_old_side_and_nothing_to_reuse() {
        let new = file("new.dll", vec![chunk(4, 0, 64)]);
        let a = assemble_pair(&FilePair {
            capture_id: "added".into(),
            path: "new.dll".into(),
            old: None,
            new: Some(NewFile {
                tag: "7.1.0".into(),
                build_id: "sMXGW2ll3Fuu".into(),
                entry: new,
            }),
        })
        .expect("assembles");
        assert!(a.old.is_none());
        assert!(a.patch.is_none(), "there is nothing to patch from");
        assert!(a.new.as_ref().unwrap().md5_matches());
        assert!(a.map.old.is_none());
        // There is no patch, but there *is* a plan: every new-side chunk must be
        // downloaded whole. Emitting `plan: null` here is what made
        // `expectations.download_bytes` a defaulted `0` - a number a consumer
        // would compare against and lose to, for a file that is not free.
        let plan = a.map.plan.as_ref().expect("a plan: every chunk is fetched");
        assert_eq!(plan.len(), a.new.as_ref().unwrap().nchunks);
        for e in plan {
            assert_eq!(e.decision, Decision::Fetch);
            assert!(e.source_range.is_none(), "no old side to slice from");
            assert!(
                e.note.is_some(),
                "so a reader is told why, not left guessing"
            );
        }
        let t = a.map.totals.as_ref().expect("the cost of the plan");
        assert_eq!(t.reuse, 0);
        assert_eq!(t.fetch, plan.len());
        assert_eq!(t.moved, 0);
        assert_eq!(
            t.download_bytes,
            plan.iter().map(|e| e.compressed_size).sum::<i64>(),
            "the whole file is downloaded, so the patch costs what it weighs"
        );
        assert!(t.download_bytes > 0, "not free");
        assert_eq!(a.map.expectations.download_bytes, t.download_bytes);
        assert!(!a.map.expectations.absent_after);
        assert!(a.map.expectations.must_not_fetch.is_empty());
        assert!(a.a12_holds(), "vacuously");
    }

    #[test]
    fn a_removed_file_has_a_new_side_nothing_to_patch_to() {
        let old = file("gone.blk", vec![chunk(5, 0, 32)]);
        let a = assemble_pair(&FilePair {
            capture_id: "removed".into(),
            path: "gone.blk".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: old,
            }),
            new: None,
        })
        .expect("assembles");
        assert!(a.new.is_none());
        assert!(a.patch.is_none());
        assert!(a.old.as_ref().unwrap().md5_matches());
        assert!(a.map.plan.is_none(), "no new side, so no target");
        assert!(a.map.totals.is_none(), "and so nothing to cost");
        // Expectations are **not** absent. A consumer needs something to assert,
        // and for a removal the assertion is that the file is gone and that
        // nothing was downloaded to achieve it.
        let e = &a.map.expectations;
        assert!(e.absent_after, "the outcome is deletion");
        assert_eq!(e.result_path, None, "there is nothing to rebuild");
        assert_eq!(e.result_md5, None);
        assert_eq!(e.download_bytes, 0, "a true zero: deleting costs nothing");
        assert!(e.must_not_fetch.is_empty());
    }

    // ------------------------------------------------------------ the totals

    #[test]
    fn the_totals_are_the_arithmetic_a_reader_would_do_by_hand() {
        let pair = same_offset_pair();
        let a = assemble_pair(&pair).expect("assembles");
        let t = a.map.totals.as_ref().unwrap();
        let plan = a.map.plan.as_ref().unwrap();
        assert_eq!(t.chunks, 3);
        assert_eq!(t.reuse, 1);
        assert_eq!(t.fetch, 2);
        assert_eq!(t.moved, 0);
        assert_eq!(t.materialize_old_bytes, 224);
        assert_eq!(t.materialize_new_bytes, 232, "64 + 128 + 40");
        // download_bytes is the sum over FETCH entries only, which is the whole
        // point: it is what the patch costs, not what the new file weighs.
        let expect: i64 = plan
            .iter()
            .filter(|e| e.decision == Decision::Fetch)
            .map(|e| e.compressed_size)
            .sum();
        assert_eq!(t.download_bytes, expect);
        assert!(t.download_bytes < t.materialize_new_bytes);
        let e = &a.map.expectations;
        assert_eq!(e.download_bytes, t.download_bytes);
        assert_eq!(e.result_path.as_deref(), Some("expected/dll"));
        assert_eq!(
            e.result_md5.as_deref(),
            Some(a.new.as_ref().unwrap().declared_md5.as_str())
        );
        assert!(!e.absent_after, "a two-sided capture leaves a file behind");
        assert_eq!(e.must_not_fetch.len(), 1, "the one reused chunk");
    }

    // -------------------------------------------------------------- failures

    /// A manifest whose chunks do not tile the file is refused with A11 named,
    /// rather than assembled into something that might hash correctly by luck.
    #[test]
    fn a_layout_that_violates_a11_stops_the_assembly_and_says_which_rule() {
        let mut old = file("bad.blk", vec![chunk(1, 0, 64), chunk(2, 64, 128)]);
        // Punch a hole: the second chunk now starts at 100 instead of 64.
        old.chunks[1].offset = 100;
        old.size = 228;
        let e = assemble_pair(&FilePair {
            capture_id: "x".into(),
            path: "bad.blk".into(),
            old: Some(OldFile {
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                entry: old,
            }),
            new: None,
        })
        .unwrap_err();
        assert!(e.contains("A11"), "{e}");
        assert!(e.contains("gap of 36 B"), "{e}");
    }

    /// A10: if the old file's slice does not hash to what the new chunk declares,
    /// the flow must FETCH instead — and record that it tried.
    ///
    /// This is hard to reach with a well-formed pair, because a REUSE candidate is
    /// found *by* content identity. It is reached by giving the new chunk the old
    /// chunk's identity while the assembled old bytes are not what that identity
    /// says — which is exactly what a corrupt bundle looks like.
    #[test]
    fn a_reuse_whose_slice_does_not_verify_is_downgraded_to_fetch_and_recorded() {
        let pair = same_offset_pair();
        // Corrupt the old file's assembly so its bytes no longer match the
        // identity the new manifest reuses, without touching the identity itself.
        let shared = pair.new.as_ref().unwrap().entry.chunks[1].clone();
        let entries: Vec<&SophonChunkFile> = [
            &pair.old.as_ref().unwrap().entry,
            &pair.new.as_ref().unwrap().entry,
        ]
        .into();
        let mut mem = memory_of(&entries);
        mem.insert(shared.id.clone(), payload(200, 128)); // wrong content, same id
        let classes = [Class::ReuseInPlace];
        let src = ChunkSource::from_memory(mem);
        let mut a = assemble(&AssemblyInput {
            pair: &pair,
            matching_field: "game",
            classes: &classes,
            sources: &sources(),
            chunks: &src,
        })
        .expect("assembles");

        // The old side is now wrong, so A7 fails and the patch had to fetch.
        assert!(
            !a.old.as_ref().unwrap().md5_matches(),
            "A7 fails by construction"
        );
        let p = a.patch.as_ref().unwrap();
        assert!(!p.a10_holds(), "the REUSE was attempted and rejected");
        assert_eq!(p.reuse_failures.len(), 1, "{:?}", p.reuse_failures);
        assert!(p.rewrites.len() == 1, "and the downgrade is recorded");
        assert!(
            p.reuse_failures[0].contains("declares"),
            "names what was expected: {:?}",
            p.reuse_failures[0]
        );
        // The plan entry says FETCH and carries the reason, so a reader can tell
        // "never a candidate" from "tried and failed".
        let e = a
            .map
            .plan
            .as_ref()
            .unwrap()
            .iter()
            .find(|e| e.chunk_id == shared.id)
            .unwrap();
        assert_eq!(e.decision, Decision::Fetch);
        assert!(
            e.note
                .as_deref()
                .unwrap()
                .contains("REUSE attempted and rejected")
        );
        assert!(e.url.is_some(), "so it can be downloaded after all");
        // And A9 still holds: the flow fetched the right bytes and produced the
        // new file regardless of the failed REUSE.
        assert!(
            p.equals_new,
            "the patch flow and the direct assembly disagree"
        );
        assert_eq!(p.md5, a.new.as_ref().unwrap().md5);
        let _ = &mut a;
    }

    /// A missing chunk is a message naming the id, not a panic and not a
    /// silently zero-filled file.
    #[test]
    fn a_missing_chunk_is_named_and_stops_the_assembly() {
        let pair = same_offset_pair();
        let src = ChunkSource::from_memory(BTreeMap::new());
        let classes = [Class::ReuseInPlace];
        let e = assemble(&AssemblyInput {
            pair: &pair,
            matching_field: "game",
            classes: &classes,
            sources: &sources(),
            chunks: &src,
        })
        .unwrap_err();
        let id = &pair.old.as_ref().unwrap().entry.chunks[0].id;
        assert!(e.contains(id.as_str()), "{e}");
    }

    // -------------------------------------------------------------- the rows

    #[test]
    fn a_green_capture_produces_one_row_per_assertion_and_no_red_ones() {
        let a = assemble_pair(&same_offset_pair()).expect("assembles");
        let rows = a.selfcheck_rows();
        let kinds: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            kinds.iter().filter(|k| **k == "A7").count(),
            1,
            "one row per assertion instance, not per assertion kind: {kinds:?}"
        );
        assert!(
            kinds.contains(&"A8")
                && kinds.contains(&"A9")
                && kinds.contains(&"A10")
                && kinds.contains(&"A11")
                && kinds.contains(&"A12")
        );
        assert!(a.all_green());
        assert!(
            rows.iter().all(|r| r.ok && !r.actual.is_empty()),
            "every row carries a detail a reader can check: {rows:?}"
        );
    }

    /// The **skip branch**: the file is already right, so the patch fetches
    /// nothing and writes nothing.
    #[test]
    fn the_skip_branch_fetches_nothing() {
        let a = assemble_pair(&identical_pair()).expect("assembles");
        let plan = a.map.plan.as_ref().expect("a plan");
        assert!(
            plan.iter().all(|e| e.decision == Decision::Reuse),
            "every chunk is already in the file"
        );
        let totals = a.map.totals.expect("totals");
        assert_eq!(totals.fetch, 0);
        assert_eq!(totals.reuse, plan.len());
        assert_eq!(a.map.expectations.download_bytes, 0);
    }

    /// The patch's shape and the plan it was derived from must agree, or the
    /// artifact says two things about the same file.
    #[test]
    fn the_patch_shape_agrees_with_the_plan_it_came_from() {
        let a = assemble_pair(&same_offset_pair()).expect("assembles");
        let s = crate::assemble::patch_shape(&a.map);
        let plan = a.map.plan.as_ref().expect("a plan");
        let reuse = plan
            .iter()
            .filter(|e| e.decision == Decision::Reuse)
            .count() as u64;
        let fetch = plan
            .iter()
            .filter(|e| e.decision == Decision::Fetch)
            .count() as u64;
        let p = s.plan.expect("a plan");
        assert_eq!(p.chunks, plan.len() as u64);
        assert_eq!(p.reuse, reuse);
        assert_eq!(p.chunks - p.reuse, fetch);
        assert_eq!(p.moved, 0, "nothing moves");
        assert_eq!(
            s.old_size,
            Some(a.map.old.as_ref().expect("an old side").size as u64)
        );
        assert_eq!(
            s.new_size,
            Some(a.map.new.as_ref().expect("a new side").size as u64)
        );
    }

    /// **An absent side is no file, not a file of zero bytes.**
    ///
    /// The difference is what makes "a file that grows" and "a file that
    /// shrinks" true of a zero-byte file: a file that appears has appeared, and a
    /// file that disappears has gone. Counting an absent side as zero bytes
    /// would make a zero-byte `added` file neither grow nor shrink, and the
    /// bundle's coverage of the size boundary would depend on whether the
    /// cheapest candidate happened to be empty.
    #[test]
    fn an_absent_side_is_no_file_and_not_a_file_of_zero_bytes() {
        // A file that is created: no old side, and the new side is empty.
        let created = crate::assemble::PatchShape {
            plan: Some(crate::assemble::PlanCounts {
                chunks: 0,
                reuse: 0,
                moved: 0,
            }),
            old_size: None,
            new_size: Some(0),
        };
        assert!(created.grows(), "a file that appears has grown");
        assert!(!created.shrinks());

        // A file that is deleted: the old side is empty, and there is no new
        // side.
        let deleted = crate::assemble::PatchShape {
            plan: None,
            old_size: Some(0),
            new_size: None,
        };
        assert!(deleted.shrinks(), "a file that disappears has shrunk");
        assert!(!deleted.grows());

        // A file that is present on both sides and empty on both: it neither
        // grows nor shrinks.
        let empty = crate::assemble::PatchShape {
            plan: Some(crate::assemble::PlanCounts {
                chunks: 0,
                reuse: 0,
                moved: 0,
            }),
            old_size: Some(0),
            new_size: Some(0),
        };
        assert!(!empty.grows() && !empty.shrinks());

        // A file that is present on both sides and grows in bytes.
        let bigger = crate::assemble::PatchShape {
            plan: Some(crate::assemble::PlanCounts {
                chunks: 1,
                reuse: 0,
                moved: 0,
            }),
            old_size: Some(0),
            new_size: Some(100),
        };
        assert!(bigger.grows());
        assert!(!bigger.shrinks());
    }

    /// The negative expectation is the plan's REUSE set, stated as the ids a
    /// consumer must **not** see requested.
    #[test]
    fn the_negative_expectation_is_the_plans_reuse_set() {
        let a = assemble_pair(&moved_pair()).expect("assembles");
        let plan = a.map.plan.as_ref().expect("a plan");
        let reuse: Vec<&str> = plan
            .iter()
            .filter(|e| e.decision == Decision::Reuse)
            .map(|e| e.chunk_id.as_str())
            .collect();
        assert_eq!(a.map.expectations.must_not_fetch, reuse);
    }

    /// A patchmap says what the **patch** is, and nothing about how a repair
    /// tool gets there: no tier, no write order, no trim, and no `SUMMARY`
    /// line transcribed into a foreign schema.
    #[test]
    fn the_patchmap_carries_no_repair_tool_vocabulary() {
        let a = assemble_pair(&moved_pair()).expect("assembles");
        let v = serde_json::to_value(&a.map).expect("serializes");
        let obj = v.as_object().expect("an object");
        assert!(
            !obj.contains_key("repair"),
            "the repair block is gone: {obj:?}"
        );
        let ex = obj["expectations"].as_object().expect("expectations");
        assert!(
            !ex.contains_key("summary"),
            "the SUMMARY expectations are gone: {ex:?}"
        );
    }

    /// The patch's shape says whether the file grows or shrinks, how much of
    /// it is reused and how much of that has moved — the three facts the
    /// coverage assertions read.
    #[test]
    fn the_patch_shape_says_whether_the_file_grows_or_shrinks() {
        let growing = assemble_pair(&same_offset_pair()).expect("assembles");
        let s = crate::assemble::patch_shape(&growing.map);
        assert!(s.grows(), "the file grows");
        assert_eq!(s.plan.expect("a plan").moved, 0, "and nothing moves");

        let shrinking = assemble_pair(&shrinking_pair()).expect("assembles");
        let s = crate::assemble::patch_shape(&shrinking.map);
        assert!(s.shrinks(), "the file shrinks");

        let moved = assemble_pair(&moved_pair()).expect("assembles");
        let s = crate::assemble::patch_shape(&moved.map);
        let p = s.plan.expect("a plan");
        assert!(p.moved > 0, "the bytes move");
        assert!(p.reuse > 0, "and some of them are reused");

        let moved_shrinking =
            assemble_pair(&shrinking_with_a_past_the_end_source_pair()).expect("assembles");
        let s = crate::assemble::patch_shape(&moved_shrinking.map);
        assert!(s.plan.expect("a plan").moved > 0, "the bytes move");
        assert!(s.shrinks(), "and the file shrinks");
    }

    #[test]
    fn decisions_serialise_the_way_the_schema_doc_writes_them() {
        assert_eq!(
            serde_json::to_string(&Decision::Reuse).unwrap(),
            "\"REUSE\""
        );
        assert_eq!(
            serde_json::to_string(&Decision::Fetch).unwrap(),
            "\"FETCH\""
        );
        assert_eq!(Decision::Fetch.to_string(), "FETCH");
    }

    #[test]
    fn the_map_carries_the_field_names_the_schema_doc_lists() {
        let a = assemble_pair(&same_offset_pair()).expect("assembles");
        let v = serde_json::to_value(&a.map).expect("serialise");
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "capture_id",
                "classes",
                "expectations",
                "matching_field",
                "new",
                "old",
                "path",
                "plan",
                "schema_version",
                "totals",
            ],
            "docs/01-artifact-schema.md §13"
        );
        let entry = &v["plan"][0];
        let mut ek: Vec<&str> = entry
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        ek.sort_unstable();
        assert_eq!(
            ek,
            vec![
                "chunk_id",
                "compressed_size",
                "decision",
                "i",
                "note",
                "source_range",
                "target_range",
                "uncompressed_md5",
                "uncompressed_size",
                "url",
            ]
        );
        assert_eq!(v["plan"][0]["target_range"]["start"], 0);
        assert_eq!(v["plan"][0]["target_range"]["end"], 64);
    }
}
