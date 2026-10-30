//! The data types an assembly produces: entries, decisions, patchmaps, sides,
//! totals, expectations, shapes, results, and the patch outcome.

use serde::{Deserialize, Serialize};

use crate::select::Class;

use super::ranges::{Layout, Range};

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
    /// capture, and nothing outside [`super::assemble`] ever read them.
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
        use crate::emit::selfcheck::{Assertion as A, wording};
        let mut rows = Vec::new();
        if let Some(o) = &self.old {
            rows.push(A::check(
                "A7",
                wording("A7"),
                &self.map.path,
                &o.declared_md5,
                &o.md5,
            ));
            rows.extend(layout_rows(&o.tag, &o.layout));
        }
        if let Some(n) = &self.new {
            rows.push(A::check(
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
            rows.push(A::check(
                "A9",
                wording("A9"),
                &self.map.path,
                n.map(|n| n.md5.as_str()).unwrap_or("<no new side>"),
                &p.md5,
            ));
            for f in &p.reuse_failures {
                rows.push(A::fail(
                    "A10",
                    wording("A10"),
                    &self.map.path,
                    "every reused chunk's old-file slice MD5 == uncompressed_md5",
                    f,
                ));
            }
            if p.reuse_failures.is_empty() && p.reused > 0 {
                rows.push(A::pass(
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
                A::pass(
                    "A12",
                    wording("A12"),
                    &self.map.path,
                    "the fetched and reused chunk sets are disjoint",
                )
            } else {
                A::fail(
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
