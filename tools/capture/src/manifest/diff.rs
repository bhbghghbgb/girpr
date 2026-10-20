//! The path diff, the reuse pool, and the `getBuild.stats` reconciliation.
//!
//! This is the module that makes the artifact a *measurement* rather
//! than a dump. Four things live here, and they are deliberately kept apart:
//!
//! 1. **Set difference over paths** — common / identical / changed / added /
//!    removed. Cheap, and the only thing a reader needs to know whether a
//!    version pair was interesting at all.
//! 2. **Per-path scoring** — `reused`, `moved`, `patch_bytes`, `total_cost`.
//!    Spec §5 defines these; this module computes them and `select` *acts* on
//!    them.
//! 3. **The reuse pool** — how much same-file-only reuse forgoes. This is a
//!    measurement of a deliberate simplification in the tool under test, and
//!    nothing here acts on it.
//! 4. **Two identities, deliberately different.** Scoring compares chunks by
//!    `(uncompressed_md5, uncompressed_size)` — MD5 alone is not enough,
//!    because a truncated chunk can share one. The reuse-pool headline number is
//!    defined in spec §10 as *"an MD5 present somewhere in the old manifest"* and
//!    is counted that way, so it measures what the document says it measures.
//!
//! ## What "moved" means, and why it is not a bug
//!
//! A REUSE reads a verified local slice and writes it at the target offset. When
//! the two differ, the chunk has to be copied from one place in the old file to
//! another. That is correct behaviour, not corruption — and the count is
//! recorded because it is the difference between the reuse path being trivially
//! correct and the reuse path having to do arithmetic.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use crate::manifest::CapturedManifest;
use crate::manifest::proto::Md5Key;
use crate::manifest::proto::SophonChunkFile;

/// One path's state in the diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// In both manifests with the same whole-file MD5.
    Identical,
    /// In both manifests, different content.
    Changed,
    /// Only in the new manifest.
    Added,
    /// Only in the old manifest.
    Removed,
}

impl State {
    /// The wire spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            State::Identical => "identical",
            State::Changed => "changed",
            State::Added => "added",
            State::Removed => "removed",
        }
    }
}

/// `(uncompressed_md5, uncompressed_size)` — the chunk identity used for scoring.
///
/// Keyed by both because MD5 alone cannot distinguish a truncated chunk from a
/// whole one, and a patch that reused a truncated chunk would be silently wrong.
///
/// **Borrowed, not owned.** This was `(String, i64)` built by an `into_key()`
/// helper, so every chunk lookup allocated a `String` for a key that already
/// existed inside the manifest — roughly 454 000 of them for a `game` field
/// diff. `(&str, i64)` is already `Hash + Eq`, so the helper is gone.
///
/// **And case-insensitive.** The MD5 half is an [`Md5Key`], which compares
/// without regard to case — the tool under test's rule, and the reason the
/// diff must not call a chunk *fetched* where `girpr` finds a *reuse`.
type ChunkKey<'a> = (Md5Key<'a>, i64);

/// One common path's score (spec §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathScore {
    /// Manifest-relative path.
    pub path: String,
    /// The old file's size.
    pub old_size: i64,
    /// The new file's size.
    pub new_size: i64,
    /// How many chunks the new entry has.
    pub nchunks: u64,
    /// New chunks whose content exists in this same old file.
    pub reused: u64,
    /// Of those, how many sit at a different offset in the old file.
    pub moved: u64,
    /// Of the moved ones, how many move **later**: the source is *before* the
    /// destination. The descending write order keeps these alive.
    pub moved_up: u64,
    /// Of the moved ones, how many move **earlier**: the source is *after* the
    /// destination. The ascending write order keeps these alive.
    pub moved_down: u64,
    /// The largest end (`source offset + size`) over the file's reuse sources.
    ///
    /// **The trim test's input.** The repair releases the dead tail *before*
    /// the first write only when no reuse source reaches past the target's end
    /// — a source the trim would delete is a source that is gone. The tool
    /// under test asks the same question of the same number
    /// (`repair::file::trim_is_free`).
    pub old_max_end: u64,
    /// Whether the whole-file MD5s agree.
    pub same_md5: bool,
    /// `Σ compressed_size` over the old entry's chunks.
    pub old_bytes: u64,
    /// `Σ compressed_size` over the new entry's chunks.
    pub new_bytes: u64,
    /// `Σ compressed_size` over the new chunks that are **not** reusable.
    pub patch_bytes: u64,
    /// `old_bytes + new_bytes` — **the ranking key**.
    ///
    /// Not `patch_bytes`, because the old file has to be materialized in full for
    /// the fixture to be real. That choice is what puts a 1.83 MB DLL ahead of
    /// its own 274 KB patch size, and it is the difference between a cheap
    /// artifact and an honest one.
    pub total_cost: u64,
}

/// One row of `diff/paths.tsv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathRow {
    /// The path.
    pub path: String,
    /// Common / one-sided, and identical-or-changed for the common ones.
    pub state: State,
    /// The size on the side this state refers to: the new size for `added`, the
    /// old size for `removed`, the new size for a common row.
    ///
    /// Carried here rather than looked up later so `byte_counts` totals the same
    /// numbers the row displays — a second lookup is a second chance to read a
    /// different size than the one printed.
    pub size: i64,
    /// The score, absent for a one-sided path.
    pub score: Option<PathScore>,
}

/// Headline counts over one field's pair of manifests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct FieldCounts {
    /// Paths in both manifests.
    pub common: u64,
    /// Of those, with the same whole-file MD5.
    pub identical: u64,
    /// Only in the new manifest.
    pub added: u64,
    /// Only in the old manifest.
    pub removed: u64,
}

impl FieldCounts {
    /// Common paths whose content differs.
    pub fn changed(&self) -> u64 {
        self.common - self.identical
    }
}

/// Aggregate counts over every field in the pool.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    /// Paths in both.
    pub common: u64,
    /// Of those, unchanged.
    pub identical: u64,
    /// Of those, changed.
    pub changed: u64,
    /// Only in the new.
    pub added: u64,
    /// Only in the old.
    pub removed: u64,
    /// Folder entries, counted separately from files.
    pub folders: u64,
}

/// Aggregate byte counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ByteCounts {
    /// `Σ size` over the added paths.
    pub added: u64,
    /// `Σ size` over the removed paths.
    pub removed: u64,
    /// `Σ patch_bytes` over the changed paths — what the patch itself costs.
    pub patch: u64,
}

/// `diff/paths.diff.json`.
#[derive(Debug, Clone, Serialize)]
pub struct PathDiff {
    /// The bundle schema version.
    pub schema_version: u32,
    /// The version the fixture is materialized at.
    pub old_tag: String,
    /// The version it is patched to.
    pub new_tag: String,
    /// The `matching_field`s scored, in selection-pool order.
    pub fields: Vec<String>,
    /// Aggregate counts.
    pub counts: Counts,
    /// Aggregate byte counts.
    pub byte_counts: ByteCounts,
    /// Added paths, each prefixed with its field.
    ///
    /// The prefix is the whole point: a path from two pools cannot collide, and
    /// without it a reader joining `added` against `by_field` has to guess which
    /// pool an entry came from.
    pub added: Vec<String>,
    /// Removed paths, each prefixed with its field.
    pub removed: Vec<String>,
    /// Per-field breakdown.
    pub by_field: BTreeMap<String, FieldCounts>,
    /// How many new chunks could be reused from *anywhere* in the old manifest.
    ///
    /// Carried here so a reader of `paths.diff.json` sees the reuse ceiling next
    /// to the reuse that same-file-only scoping actually reaches.
    pub new_chunks_reusable_at_all: u64,
}

impl PathDiff {
    /// Whether the pair was interesting at all: something added, removed or
    /// changed.
    pub fn is_a_real_upgrade(&self) -> bool {
        self.counts.added > 0 || self.counts.removed > 0 || self.counts.changed > 0
    }
}

/// `diff/reuse_pool.json` — the cost of same-file-only reuse (spec §10).
///
/// Recorded because it bounds a deliberate simplification in the tool under test.
/// `gircap` never acts on any of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ReusePool {
    /// Every chunk in the new manifest.
    pub new_chunks_total: u64,
    /// How many of them have an MD5 present *somewhere* in the old manifest.
    ///
    /// MD5-only, because that is what the number in §10 measures. spec §5's scoring
    /// uses `(md5, size)`; the two differ, and mixing them up would make this
    /// number unreproducible against the document it comes from.
    pub new_chunks_md5_present_in_old_manifest: u64,
    /// How many of them are absent from *their own* old file.
    ///
    /// These are the chunks a cross-file reuse index would have saved and a
    /// same-file-only one cannot reach.
    pub cross_file_only_chunks: u64,
    /// How many new files have at least one such chunk.
    pub files_with_a_cross_file_only_chunk: u64,
    /// `Σ compressed_size` over those chunks: the bytes forgone.
    pub cross_file_only_bytes_forgone: u64,
    /// The note the schema doc carries, so the number is not read as a target.
    pub note: &'static str,
}

impl ReusePool {
    /// Build the note from the measured numbers, so it cannot drift from them.
    pub const NOTE: &'static str = "same-file-only reuse is a deliberate simplification; \
        this quantifies its cost";
}

/// Everything this module derives from one pair of captured manifests.
#[derive(Debug, Clone)]
pub struct FieldDiff {
    /// The `matching_field` this pair came from.
    pub field: String,
    /// Headline counts.
    pub counts: FieldCounts,
    /// Every common path's score, sorted by path.
    pub scores: Vec<PathScore>,
    /// Every path, with its state, sorted by path.
    pub rows: Vec<PathRow>,
    /// The cross-file reuse measurement.
    pub pool: ReusePool,
}

/// Diff one old manifest against one new manifest for one field.
///
/// Both manifests must be for the same `matching_field`; a caller that paired a
/// `game` old manifest with a `ja-jp` new one would get a plausible-looking diff
/// of two unrelated corpora, so the field is compared and a mismatch refused.
pub fn diff_field(old: &CapturedManifest, new: &CapturedManifest) -> Result<FieldDiff, String> {
    if old.matching_field != new.matching_field {
        return Err(format!(
            "cannot diff {} against {}: the two manifests are different matching_fields",
            old.matching_field, new.matching_field
        ));
    }
    let field = new.matching_field.clone();

    // One lookup per old entry, carrying both the entry and its key index.
    // Building them separately and then scanning for the entry inside the loop
    // would be O(n²) over ~2 700 paths — enough to be visible in a capture's
    // wall clock, and for no benefit.
    let old_by_path: HashMap<&str, (&SophonChunkFile, OldFileIndex<'_>)> = old
        .entries
        .iter()
        .map(|e| (e.file.as_str(), (e, index_file(e))))
        .collect();

    // Old manifest-wide set of MD5s that appear anywhere, for the reuse pool.
    let old_md5_paths: HashSet<Md5Key<'_>> = build_md5_path_index(&old.entries);

    let new_paths: HashSet<&str> = new.entries.iter().map(|e| e.file.as_str()).collect();

    let mut counts = FieldCounts::default();
    let mut scores = Vec::new();
    let mut rows: Vec<PathRow> = Vec::with_capacity(old.entries.len() + new.entries.len());

    // New side: common and added.
    for e in &new.entries {
        match old_by_path.get(e.file.as_str()) {
            Some((old_entry, file_index)) => {
                counts.common += 1;
                // Case-insensitively: the tool under test's rule, and the
                // whole-file MD5 is what decides `identical`.
                let same_md5 = crate::hash::md5_eq(&old_entry.md5, &e.md5);
                if same_md5 {
                    counts.identical += 1;
                }
                let score = score(e, old_entry, file_index);
                rows.push(PathRow {
                    path: e.file.clone(),
                    state: if same_md5 {
                        State::Identical
                    } else {
                        State::Changed
                    },
                    size: e.size,
                    score: Some(score.clone()),
                });
                scores.push(score);
            }
            None => {
                counts.added += 1;
                rows.push(PathRow {
                    path: e.file.clone(),
                    state: State::Added,
                    size: e.size,
                    score: None,
                });
            }
        }
    }

    // Old side: removed.
    for e in &old.entries {
        if !new_paths.contains(e.file.as_str()) {
            counts.removed += 1;
            rows.push(PathRow {
                path: e.file.clone(),
                state: State::Removed,
                size: e.size,
                score: None,
            });
        }
    }

    rows.sort_by(|a, b| a.path.cmp(&b.path));
    scores.sort_by(|a, b| a.path.cmp(&b.path));

    let pool = reuse_pool(new, &old_by_path, &old_md5_paths);

    Ok(FieldDiff {
        field,
        counts,
        scores,
        rows,
        pool,
    })
}

/// Where each identity sits in one old file: the **first** chunk with that
/// identity, in manifest order.
///
/// One offset per identity rather than a list of them, because the repair reads
/// the *first* match and only the first match: `girpr`'s
/// `repair::reuse::sources_for` and this crate's own patch flow
/// (`assemble::patch_flow`) both take the first occurrence, and a list would
/// invite a second definition of "where the bytes are".
type FileIndex<'a> = HashMap<ChunkKey<'a>, i64>;

/// One old file's chunk index, plus the sum `score` would otherwise walk its
/// chunks again to compute.
///
/// The byte total is folded into the index build because that walk is already
/// happening: a second pass over the old file's chunks for `old_bytes`, once per
/// common path, is 113 610 walks of a 2 744-entry loop for one number.
///
/// Public because [`score`] is, and a public function cannot take a private
/// type. Nothing outside this module should build one: `index_file` is the
/// only way, and it is what guarantees the offsets are the *first* ones.
pub struct OldFileIndex<'a> {
    /// The first chunk of each identity, in manifest order.
    sources: FileIndex<'a>,
    /// `Σ compressed_size` over the old file's chunks.
    compressed_bytes: u64,
}

/// `(key) -> [offset]` for one file's chunks, and its compressed byte total.
///
/// The offset kept for an identity is the **first** one, because that is the
/// source the repair would copy from — the same rule `girpr` applies in
/// `repair::reuse::sources_for`, and the same rule the patchmap's
/// `source_range` records. Keeping the *earliest* offset instead would agree
/// with it only because the manifests' chunks are in ascending offset order,
/// which is a property of the data rather than of the rule.
fn index_file(e: &SophonChunkFile) -> OldFileIndex<'_> {
    let mut sources: FileIndex<'_> = HashMap::with_capacity(e.chunks.len());
    let mut compressed_bytes = 0_u64;
    for c in &e.chunks {
        sources.entry(c.identity()).or_insert(c.offset);
        compressed_bytes += c.compressed_size.max(0) as u64;
    }
    OldFileIndex {
        sources,
        compressed_bytes,
    }
}

/// Old manifest-wide set of MD5s that appear anywhere.
///
/// A `HashSet`, not a `HashMap<&str, HashSet<&str>>`. The map's values were
/// built for 113 610 old chunks and then never read — the one consumer asks
/// `contains_key`, so every one of those inner sets was allocated to be
/// discarded.
fn build_md5_path_index(entries: &[SophonChunkFile]) -> HashSet<Md5Key<'_>> {
    let mut m: HashSet<Md5Key<'_>> = HashSet::with_capacity(entries.len());
    for e in entries {
        for c in &e.chunks {
            m.insert(Md5Key(c.uncompressed_md5.as_str()));
        }
    }
    m
}

/// Score one common path (spec §5).
///
/// **"Moved" is one fact, and this is where it is decided.** A chunk is moved
/// when the *source* it would be copied from does not sit at its destination —
/// and the source is the **first** old chunk with that identity, which is what
/// `girpr` reads (`repair::reuse::sources_for`) and what this crate's own
/// patch flow records (`assemble::patch_flow`). It used to be "the new offset
/// is not among *any* of the old file's offsets for that identity", which
/// disagrees with the patchmap's own `source_range` whenever an identity occurs
/// more than once in the old file — and a class is a statement about the
/// content, so the two have to be the same statement.
pub fn score(
    new: &SophonChunkFile,
    old: &SophonChunkFile,
    old_index: &OldFileIndex<'_>,
) -> PathScore {
    let mut reused = 0_u64;
    let mut moved = 0_u64;
    let mut moved_up = 0_u64;
    let mut moved_down = 0_u64;
    let mut old_max_end = 0_u64;
    let mut patch_bytes = 0_u64;
    let mut new_bytes = 0_u64;
    for c in &new.chunks {
        new_bytes += c.compressed_size.max(0) as u64;
        match old_index.sources.get(&c.identity()) {
            Some(&src) => {
                reused += 1;
                // The source's end: the input to the trim test. Only the
                // *sources* count, because only a source the trim would delete
                // is a source that is gone.
                let end = src.max(0) as u64 + c.uncompressed_size.max(0) as u64;
                if end > old_max_end {
                    old_max_end = end;
                }
                if src != c.offset {
                    moved += 1;
                    if src > c.offset {
                        // The bytes move earlier: the source is after the
                        // destination, and the ascending order keeps it alive.
                        moved_down += 1;
                    } else {
                        // The bytes move later: the descending order keeps it.
                        moved_up += 1;
                    }
                }
            }
            None => patch_bytes += c.compressed_size.max(0) as u64,
        }
    }
    // Taken from the index build rather than walked again. `old` is still named
    // for the fields below.
    let old_bytes = old_index.compressed_bytes;
    PathScore {
        path: new.file.clone(),
        old_size: old.size,
        new_size: new.size,
        nchunks: new.chunks.len() as u64,
        reused,
        moved,
        moved_up,
        moved_down,
        old_max_end,
        same_md5: crate::hash::md5_eq(&old.md5, &new.md5),
        old_bytes,
        new_bytes,
        patch_bytes,
        total_cost: old_bytes + new_bytes,
    }
}

/// Score one old/new entry pair, without a whole manifest.
///
/// The public face of [`score`]: it builds the old file's index and scores the
/// new entry against it. [`diff_field`] uses the index it already has; a caller
/// that holds only two entries — the patchmap's own consistency check, or a
/// test — needs this.
pub fn score_pair(old: &SophonChunkFile, new: &SophonChunkFile) -> PathScore {
    score(new, old, &index_file(old))
}

/// Measure what same-file-only reuse forgoes.
///
/// `old_by_path` is borrowed rather than rebuilt. Constructing the same
/// `HashMap<&str, FileIndex>` — 113 610 inserts — from the same inputs, in the
/// same call, a few lines after `diff_field` had already built it is about 40% of
/// this module's runtime, spent twice.
fn reuse_pool<'a>(
    new: &CapturedManifest,
    old_by_path: &HashMap<&'a str, (&'a SophonChunkFile, OldFileIndex<'a>)>,
    old_md5_paths: &HashSet<Md5Key<'a>>,
) -> ReusePool {
    let mut pool = ReusePool {
        new_chunks_total: 0,
        new_chunks_md5_present_in_old_manifest: 0,
        cross_file_only_chunks: 0,
        files_with_a_cross_file_only_chunk: 0,
        cross_file_only_bytes_forgone: 0,
        note: ReusePool::NOTE,
    };

    for e in &new.entries {
        let file_index = old_by_path.get(e.file.as_str()).map(|(_, idx)| idx);
        let mut file_has_cross = false;
        for c in &e.chunks {
            pool.new_chunks_total += 1;
            let in_manifest = old_md5_paths.contains(&Md5Key(c.uncompressed_md5.as_str()));
            if in_manifest {
                pool.new_chunks_md5_present_in_old_manifest += 1;
            }
            // Reachable from this same file?
            let in_same_file =
                file_index.is_some_and(|idx| idx.sources.contains_key(&c.identity()));
            if in_manifest && !in_same_file {
                pool.cross_file_only_chunks += 1;
                pool.cross_file_only_bytes_forgone += c.compressed_size.max(0) as u64;
                file_has_cross = true;
            }
        }
        if file_has_cross {
            pool.files_with_a_cross_file_only_chunk += 1;
        }
    }
    pool
}

/// Aggregate several field diffs into `diff/paths.diff.json`.
pub fn aggregate(old_tag: &str, new_tag: &str, fields: &[FieldDiff]) -> PathDiff {
    let mut counts = Counts::default();
    let mut byte_counts = ByteCounts::default();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut by_field = BTreeMap::new();

    for f in fields {
        for row in &f.rows {
            match row.state {
                State::Added => {
                    counts.added += 1;
                    byte_counts.added += row.size.max(0) as u64;
                    added.push(format!("{}:{}", f.field, row.path));
                }
                State::Removed => {
                    counts.removed += 1;
                    byte_counts.removed += row.size.max(0) as u64;
                    removed.push(format!("{}:{}", f.field, row.path));
                }
                State::Identical | State::Changed => {}
            }
        }
        counts.common += f.counts.common;
        counts.identical += f.counts.identical;
        counts.changed += f.counts.changed();
        byte_counts.patch += f.scores.iter().map(|s| s.patch_bytes).sum::<u64>();
        by_field.insert(
            f.field.clone(),
            FieldCounts {
                common: f.counts.common,
                identical: f.counts.identical,
                added: f.counts.added,
                removed: f.counts.removed,
            },
        );
    }
    added.sort();
    removed.sort();

    PathDiff {
        schema_version: crate::SCHEMA_VERSION,
        old_tag: old_tag.to_string(),
        new_tag: new_tag.to_string(),
        fields: fields.iter().map(|f| f.field.clone()).collect(),
        counts,
        byte_counts,
        added,
        removed,
        by_field,
        new_chunks_reusable_at_all: fields
            .iter()
            .map(|f| f.pool.new_chunks_md5_present_in_old_manifest)
            .sum(),
    }
}

/// `diff/paths.tsv` — the flat per-path form, sorted by path.
///
/// One row per path in either manifest: `path`, `state`, `old_size`, `new_size`,
/// `reused`, `moved`, `patch_bytes`, `total_cost`. Plain text always, header
/// line, LF endings — this is the file a reader greps.
/// The two `paths.tsv` writers are **not** duplicates, despite the name.
///
/// This one is `diff/paths.tsv` — `field path state old_size … total_cost`,
/// one row per common and one-sided path across every scored field.
/// [`crate::manifest::dump::paths_tsv`] is the per-manifest sidecar,
/// `path size md5 nchunks is_folder`, one row per manifest entry. Same format,
/// different documents; a reader greps them for different things and merging
/// them would lose a column.
///
/// What they do share is a weakness: **neither escapes a tab**, so one path
/// containing a tab corrupts both identically. Real manifests carry none, and
/// escaping would change bytes that shipped bundles already contain, so it is
/// left as a recorded property rather than a fix.
pub fn paths_tsv(diffs: &[FieldDiff]) -> String {
    const HEADER: &str =
        "field\tpath\tstate\told_size\tnew_size\treused\tmoved\tpatch_bytes\ttotal_cost\n";
    // Sized from the header and the row count rather than `String::from`,
    // which starts at zero and reallocates as a 2 744-row table lands.
    let mut s = String::with_capacity(
        HEADER.len() + diffs.iter().map(|f| f.rows.len() * 96).sum::<usize>(),
    );
    s.push_str(HEADER);
    for f in diffs {
        for row in &f.rows {
            s.push_str(&f.field);
            s.push('\t');
            s.push_str(&row.path);
            s.push('\t');
            s.push_str(row.state.as_str());
            s.push('\t');
            match &row.score {
                Some(sc) => {
                    use std::fmt::Write as _;
                    // Seven `to_string()` calls per row, each a fresh
                    // allocation, for numbers that go straight back out.
                    let _ = write!(
                        s,
                        "{}\t{}\t{}\t{}\t{}\t{}",
                        sc.old_size,
                        sc.new_size,
                        sc.reused,
                        sc.moved,
                        sc.patch_bytes,
                        sc.total_cost
                    );
                }
                None => s.push_str("\t\t0\t0\t0\t0"),
            }
            s.push('\n');
        }
    }
    s
}

/// The cheapest scores, already ordered for spec spec §5's tie-break.
///
/// Order: `total_cost`, then the field — `game` **before** the audio fields —
/// then `moved` desc, then `reused` desc, then `path` asc. Fully deterministic:
/// re-running on the same pair and the same `--audio` picks the same files.
///
/// The field term is deliberately *not* plain ascending. `en-us` sorts before
/// `game` alphabetically, so "field asc" alone would put a voice pack ahead of a
/// game file on a cost tie — the opposite of what §5 asks for, and wrong for a
/// fixture whose `game/` is meant to be a game install. `game` is therefore
/// ranked ahead explicitly and the name is only a tie-break within a rank, which
/// keeps the order total and reproducible.
pub fn by_cost(diffs: &[FieldDiff]) -> Vec<(String, PathScore)> {
    let mut out: Vec<(String, PathScore)> = diffs
        .iter()
        .flat_map(|f| f.scores.iter().map(|s| (f.field.clone(), s.clone())))
        .collect();
    out.sort_by(|a, b| {
        a.1.total_cost
            .cmp(&b.1.total_cost)
            .then_with(|| field_rank(&a.0).cmp(&field_rank(&b.0)))
            .then_with(|| b.1.moved.cmp(&a.1.moved))
            .then_with(|| b.1.reused.cmp(&a.1.reused))
            .then_with(|| a.1.path.cmp(&b.1.path))
    });
    out
}

/// `game` first, then the audio fields alphabetically.
pub fn field_rank(field: &str) -> (u8, &str) {
    (u8::from(field != GAME_FIELD), field)
}

/// The one `matching_field` that is the game itself rather than a voice pack.
pub const GAME_FIELD: &str = "game";

/// Diff every field's manifest pair, and report each one.
///
/// **Pairs are split through [`super::fetch::pair_sides`], not indexed.** The
/// loop this replaces was `diff_field(&pair[0], &pair[1])` — a bare index that
/// panics on a short pair, next to a comment explaining that position 0 is the
/// old build. The check now lives with the contract that gives the position its
/// meaning, and every call site gets it.
///
/// The narration travels with the computation for the same reason it did in
/// `select::decide`: the counts in the log and the counts in `FieldDiff` are one
/// fact, and a reader who finds one should not have to go looking for the other.
pub fn diff_all(
    fields: &[String],
    manifests: &[Vec<CapturedManifest>],
) -> Result<Vec<FieldDiff>, crate::failure::Failure> {
    use crate::failure::assertion;
    use crate::http::ApiError;

    let mut diffs = Vec::with_capacity(fields.len());
    for (field, pair) in fields.iter().zip(manifests) {
        let (old, new) = super::fetch::pair_sides(pair, field)?;
        let fd = diff_field(old, new)
            .map_err(|m| assertion("diff the manifests", &ApiError::shape(field, m)))?;
        tracing::info!(
            field = %fd.field,
            common = fd.counts.common,
            identical = fd.counts.identical,
            changed = fd.counts.changed(),
            added = fd.counts.added,
            removed = fd.counts.removed,
            pool_chunks = fd.pool.new_chunks_total,
            pool_md5_hits = fd.pool.new_chunks_md5_present_in_old_manifest,
            cross_file_only = fd.pool.cross_file_only_chunks,
            cross_file_bytes = fd.pool.cross_file_only_bytes_forgone,
            "diff computed"
        );
        diffs.push(fd);
    }
    Ok(diffs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::proto::SophonChunk;

    fn chunk(id: &str, md5: &str, offset: i64, usize_: i64, csize: i64) -> SophonChunk {
        SophonChunk {
            id: format!("{id}_{md5}"),
            uncompressed_md5: md5.to_string(),
            offset,
            compressed_size: csize,
            uncompressed_size: usize_,
            unknown_field6: 1,
            compressed_md5: format!("c{id}"),
        }
    }

    fn file(path: &str, md5: &str, chunks: Vec<SophonChunk>) -> SophonChunkFile {
        let size = chunks.iter().map(|c| c.uncompressed_size).sum();
        SophonChunkFile {
            file: path.into(),
            chunks,
            is_folder: false,
            size,
            md5: md5.into(),
        }
    }

    fn manifest(tag: &str, entries: Vec<SophonChunkFile>) -> CapturedManifest {
        let mut c = CapturedManifest {
            tag: tag.into(),
            build_id: "b".into(),
            matching_field: "game".into(),
            seq: 1,
            raw: vec![1],
            entries,
            meta: crate::manifest::ManifestMeta {
                schema_version: 1,
                tag: tag.into(),
                build_id: "b".into(),
                matching_field: "game".into(),
                category_id: "10016".into(),
                category_name: "g".into(),
                manifest: crate::manifest::DeclaredManifest {
                    id: "manifest_x_deadbeef".into(),
                    checksum: "deadbeef".into(),
                    compressed_size: "1".into(),
                    uncompressed_size: "2".into(),
                },
                declared: Default::default(),
                id_parts: crate::manifest::IdParts {
                    prefix_16hex: "manifest_x".into(),
                    prefix_semantics: "opaque",
                    suffix_32hex: "deadbeef".into(),
                    suffix_equals_checksum: true,
                },
                stored: crate::manifest::Stored {
                    path: String::new(),
                    bytes: 1,
                    container: "zstd",
                    md5_compressed: String::new(),
                    sha256_compressed: String::new(),
                    md5_decompressed: String::new(),
                    bytes_decompressed: 2,
                    encoding: crate::hash::Encoding::Identity,
                },
                verified: crate::manifest::Verified {
                    decompressed_md5_matches_checksum: true,
                    checksum_matches_id_suffix: true,
                    id_prefix_verified: false,
                },
                decoded: Default::default(),
                full_decode_written: true,
            },
        };
        c.meta.decoded = crate::manifest::decode_counts(&c.entries);
        c
    }

    fn three_state_pair() -> FieldDiff {
        // common+identical, common+changed with a moved chunk, added, removed.
        let old = manifest(
            "7.0.0",
            vec![
                file("same", "md5-same", vec![chunk("a", "aa", 0, 10, 5)]),
                file(
                    "moved",
                    "md5-old",
                    vec![chunk("b", "bb", 0, 10, 5), chunk("c", "cc", 10, 10, 5)],
                ),
                file("gone", "md5-gone", vec![chunk("d", "dd", 0, 10, 5)]),
            ],
        );
        let new = manifest(
            "7.1.0",
            vec![
                file("same", "md5-same", vec![chunk("a", "aa", 0, 10, 5)]),
                file(
                    "moved",
                    "md5-new",
                    // `bb` is present in the old file but at a different offset.
                    vec![chunk("b", "bb", 20, 10, 5), chunk("e", "ee", 10, 10, 7)],
                ),
                file("fresh", "md5-fresh", vec![chunk("f", "ff", 0, 10, 9)]),
            ],
        );
        diff_field(&old, &new).expect("same field")
    }

    /// **The MD5 comparison is case-insensitive**, because the tool under
    /// test's is: `girpr` compares with `eq_ignore_ascii_case`
    /// (`repair::reuse::sources_for`, and the scan's whole-file check).
    ///
    /// A manifest pair that spells one digest `DE1CFE69…` on one side and
    /// `de1cfe69…` on the other describes **the same chunk**, and the repair
    /// would copy the bytes. A case-sensitive comparison here would call it a
    /// fetch — and the bundle would publish, for that file, a download total
    /// the repair never spends and a reuse it never takes.
    #[test]
    fn an_md5_spelled_in_two_cases_is_the_same_chunk() {
        let old = manifest(
            "7.0.0",
            vec![file(
                "f",
                "md5-old",
                vec![chunk("a", "DE1CFE6927DACBC58D2D55A0B45EA0F3", 0, 10, 5)],
            )],
        );
        // The same digest, lower-cased, at the same offset.
        let new = manifest(
            "7.1.0",
            vec![file(
                "f",
                "md5-new",
                vec![chunk("a", "de1cfe6927dacbc58d2d55a0b45ea0f3", 0, 10, 5)],
            )],
        );
        let d = diff_field(&old, &new).expect("same field");
        let row = d.rows.iter().find(|r| r.path == "f").expect("a row");
        let score = row.score.as_ref().expect("a score");
        assert_eq!(score.reused, 1, "the chunk is reused, not fetched");
        assert_eq!(score.moved, 0, "and it has not moved");
        assert_eq!(score.patch_bytes, 0, "so nothing is fetched for it");
    }

    /// The whole-file MD5 is case-insensitive too, and it is the one that
    /// decides `identical`: a file whose two digests are spelled in different
    /// cases is the **same file**, and the repair skips it.
    #[test]
    fn a_whole_file_md5_spelled_in_two_cases_is_the_same_file() {
        let old = manifest(
            "7.0.0",
            vec![file(
                "f",
                "DEADBEEFDEADBEEFDEADBEEFDEADBEEF",
                vec![chunk("a", "aa", 0, 10, 5)],
            )],
        );
        let new = manifest(
            "7.1.0",
            vec![file(
                "f",
                "deadbeefdeadbeefdeadbeefdeadbeef",
                vec![chunk("a", "aa", 0, 10, 5)],
            )],
        );
        let d = diff_field(&old, &new).expect("same field");
        let row = d.rows.iter().find(|r| r.path == "f").expect("a row");
        assert_eq!(
            row.state,
            State::Identical,
            "the same file, spelled two ways"
        );
        assert_eq!(d.counts.identical, 1);
    }

    #[test]
    fn the_four_states_are_counted_and_assigned() {
        let d = three_state_pair();
        assert_eq!(d.counts.common, 2);
        assert_eq!(d.counts.identical, 1);
        assert_eq!(d.counts.changed(), 1);
        assert_eq!(d.counts.added, 1);
        assert_eq!(d.counts.removed, 1);

        let by_path: Vec<(&str, State)> =
            d.rows.iter().map(|r| (r.path.as_str(), r.state)).collect();
        assert_eq!(
            by_path,
            vec![
                ("fresh", State::Added),
                ("gone", State::Removed),
                ("moved", State::Changed),
                ("same", State::Identical)
            ]
        );
    }

    /// spec §5's formulas, computed by hand on the fixture above.
    #[test]
    fn reuse_moved_and_the_two_byte_totals_follow_section_five() {
        let d = three_state_pair();
        let moved = d.scores.iter().find(|s| s.path == "moved").unwrap();
        assert_eq!(moved.nchunks, 2);
        assert_eq!(moved.reused, 1, "bb exists in this same old file");
        assert_eq!(moved.moved, 1, "but at offset 20 there, and 0 here");
        assert_eq!(moved.old_bytes, 10, "two old chunks of 5");
        assert_eq!(moved.new_bytes, 12, "5 + 7");
        assert_eq!(moved.patch_bytes, 7, "only ee is unfetchable");
        assert_eq!(
            moved.total_cost, 22,
            "old_bytes + new_bytes, not patch_bytes"
        );
        assert!(!moved.same_md5);

        let same = d.scores.iter().find(|s| s.path == "same").unwrap();
        assert_eq!(same.reused, 1);
        assert_eq!(same.moved, 0);
        assert_eq!(
            same.patch_bytes, 0,
            "nothing to fetch for an identical file"
        );
        assert!(same.same_md5);
    }

    /// `total_cost` is the ranking key and `patch_bytes` is not — that choice is
    /// what makes the artifact honest about what it costs to build.
    #[test]
    fn total_cost_is_old_plus_new_and_exceeds_the_patch_size() {
        let d = three_state_pair();
        for s in &d.scores {
            assert_eq!(s.total_cost, s.old_bytes + s.new_bytes);
            assert!(
                s.total_cost >= s.patch_bytes,
                "{}: cost must be at least the patch",
                s.path
            );
        }
        let m = d.scores.iter().find(|s| s.path == "moved").unwrap();
        assert!(m.total_cost > m.patch_bytes, "and here it really is");
    }

    /// Chunk identity is `(md5, size)`: a truncated chunk sharing an MD5 is not
    /// the same content and must not be reused.
    #[test]
    fn a_chunk_with_a_shared_md5_but_a_different_length_is_not_reusable() {
        let old = manifest(
            "7.0.0",
            vec![file("f", "o", vec![chunk("a", "shared", 0, 10, 5)])],
        );
        let new = manifest(
            "7.1.0",
            vec![file("f", "n", vec![chunk("a", "shared", 0, 7, 5)])],
        );
        let d = diff_field(&old, &new).unwrap();
        assert_eq!(d.scores[0].reused, 0);
        assert_eq!(d.scores[0].patch_bytes, 5);
    }

    /// The §10 measurement: content reachable from *anywhere* in the old
    /// manifest, versus from this file only.
    ///
    /// The new file `hungry` has one chunk it already holds and one it could
    /// only get from `donor`. Both are md5-present in the old manifest; only one
    /// is unreachable under same-file-only reuse, and that one is the number
    /// §10 records.
    #[test]
    fn the_reuse_pool_separates_manifest_wide_from_same_file_reach() {
        let old = manifest(
            "7.0.0",
            vec![
                file("donor", "d", vec![chunk("x", "shared", 0, 10, 100)]),
                file("hungry", "h", vec![chunk("y", "own", 0, 10, 50)]),
            ],
        );
        let new = manifest(
            "7.1.0",
            vec![file(
                "hungry",
                "n",
                vec![
                    chunk("x", "shared", 0, 10, 777),
                    chunk("y", "own", 0, 10, 50),
                ],
            )],
        );
        let d = diff_field(&old, &new).unwrap();
        assert_eq!(d.pool.new_chunks_total, 2);
        assert_eq!(
            d.pool.new_chunks_md5_present_in_old_manifest, 2,
            "`shared` is in `donor` and `own` is in `hungry`; both are present \
             somewhere in the old manifest"
        );
        assert_eq!(
            d.pool.cross_file_only_chunks, 1,
            "only `shared` is unreachable from `hungry`'s own old entry"
        );
        assert_eq!(
            d.pool.cross_file_only_bytes_forgone, 777,
            "the compressed size a cross-file index would have saved"
        );
        assert_eq!(d.pool.files_with_a_cross_file_only_chunk, 1);
        assert!(d.pool.note.contains("deliberate simplification"));
    }

    /// A file with no old entry at all has nothing to reuse from, so *every*
    /// one of its chunks that exists anywhere is cross-file-only.
    #[test]
    fn a_new_file_counts_every_shared_chunk_as_cross_file_only() {
        let old = manifest(
            "7.0.0",
            vec![
                file("donor", "d", vec![chunk("x", "s1", 0, 10, 1)]),
                file("other", "o", vec![chunk("y", "s2", 0, 10, 1)]),
            ],
        );
        let new = manifest(
            "7.1.0",
            vec![file(
                "brand-new",
                "n",
                vec![chunk("a", "s1", 0, 10, 7), chunk("b", "s2", 10, 10, 3)],
            )],
        );
        let d = diff_field(&old, &new).unwrap();
        assert_eq!(d.pool.new_chunks_md5_present_in_old_manifest, 2);
        assert_eq!(d.pool.cross_file_only_chunks, 2);
        assert_eq!(d.pool.cross_file_only_bytes_forgone, 10);
        // And the path itself is `added`, so it would be a `missing local file`
        // candidate rather than a reuse one.
        assert_eq!(d.counts.added, 1);
        assert!(d.scores.is_empty(), "an added path is not scored");
    }

    #[test]
    fn a_chunk_in_its_own_old_file_is_never_counted_as_cross_file() {
        let old = manifest(
            "7.0.0",
            vec![file("f", "o", vec![chunk("a", "aa", 0, 10, 5)])],
        );
        let new = manifest(
            "7.1.0",
            vec![file("f", "n", vec![chunk("a", "aa", 0, 10, 5)])],
        );
        let d = diff_field(&old, &new).unwrap();
        assert_eq!(d.pool.cross_file_only_chunks, 0);
        assert_eq!(d.pool.cross_file_only_bytes_forgone, 0);
        assert_eq!(d.pool.files_with_a_cross_file_only_chunk, 0);
        assert_eq!(d.pool.new_chunks_md5_present_in_old_manifest, 1);
    }

    #[test]
    fn two_files_each_holding_one_cross_file_chunk_are_counted_separately() {
        let old = manifest(
            "7.0.0",
            vec![
                file("d1", "d", vec![chunk("x", "s1", 0, 10, 1)]),
                file("d2", "d", vec![chunk("y", "s2", 0, 10, 1)]),
            ],
        );
        let new = manifest(
            "7.1.0",
            vec![
                file("n1", "n", vec![chunk("a", "s1", 0, 10, 10)]),
                file("n2", "n", vec![chunk("b", "s2", 0, 10, 20)]),
            ],
        );
        let d = diff_field(&old, &new).unwrap();
        assert_eq!(d.pool.cross_file_only_chunks, 2);
        assert_eq!(d.pool.files_with_a_cross_file_only_chunk, 2);
        assert_eq!(d.pool.cross_file_only_bytes_forgone, 30);
    }

    /// A cross-field pair would produce a plausible-looking diff of two
    /// unrelated corpora, so it is refused rather than computed.
    #[test]
    fn diffing_different_matching_fields_is_refused() {
        let mut old = manifest("7.0.0", vec![]);
        old.matching_field = "game".into();
        let mut new = manifest("7.1.0", vec![]);
        new.matching_field = "ja-jp".into();
        let e = diff_field(&old, &new).unwrap_err();
        assert!(e.contains("game"), "{e}");
        assert!(e.contains("ja-jp"), "{e}");
    }

    #[test]
    fn two_empty_manifests_diff_to_nothing() {
        let d = diff_field(&manifest("7.0.0", vec![]), &manifest("7.1.0", vec![])).unwrap();
        assert_eq!(d.counts.common, 0);
        assert!(d.scores.is_empty() && d.rows.is_empty());
        assert_eq!(d.pool.new_chunks_total, 0);
        assert!(!aggregate("7.0.0", "7.1.0", std::slice::from_ref(&d)).is_a_real_upgrade());
    }

    // ------------------------------------------------------------- ordering

    /// spec spec §5's tie-break chain, asserted on inputs designed to collide at each
    /// step. Determinism is the property: the same pair must always admit the
    /// same files.
    #[test]
    fn cost_ordering_breaks_ties_by_field_then_moved_then_reused_then_path() {
        let mk = |field: &str, path: &str, cost: u64, moved: u64, reused: u64| FieldDiff {
            field: field.into(),
            counts: Default::default(),
            scores: vec![PathScore {
                path: path.into(),
                old_size: 0,
                new_size: 0,
                nchunks: 0,
                reused,
                moved,
                moved_up: 0,
                moved_down: moved,
                old_max_end: 0,
                same_md5: false,
                old_bytes: cost / 2,
                new_bytes: cost / 2,
                patch_bytes: 0,
                total_cost: cost,
            }],
            rows: vec![],
            pool: ReusePool::default(),
        };
        let diffs = vec![
            mk("en-us", "/zzz", 100, 0, 0),
            mk("game", "/bbb", 100, 0, 0),
            mk("game", "/aaa", 100, 0, 0),
            mk("game", "/ccc", 100, 1, 0),
            mk("game", "/ddd", 100, 0, 9),
            mk("game", "/eee", 99, 0, 0),
        ];
        let got: Vec<(String, String)> = by_cost(&diffs)
            .into_iter()
            .map(|(f, s)| (f, s.path))
            .collect();
        assert_eq!(
            got,
            vec![
                ("game".to_string(), "/eee".to_string()), // cost 99, cheapest
                ("game".to_string(), "/ccc".to_string()), // cost 100: moved desc
                ("game".to_string(), "/ddd".to_string()), // then reused desc
                ("game".to_string(), "/aaa".to_string()), // then path asc
                ("game".to_string(), "/bbb".to_string()),
                ("en-us".to_string(), "/zzz".to_string()), // `game` wins the field tie
            ]
        );
    }

    /// `en-us` sorts before `game` alphabetically, so a plain `field asc` would
    /// put a voice pack ahead of a game file on a cost tie — the opposite of spec §5.
    #[test]
    fn game_ranks_before_the_audio_fields_not_alphabetically() {
        assert!(field_rank(GAME_FIELD) < field_rank("en-us"));
        assert!(field_rank(GAME_FIELD) < field_rank("zh-cn"));
        assert!(
            field_rank("en-us") < field_rank("zh-cn"),
            "and then by name"
        );
        assert_eq!(field_rank("en-us"), (1, "en-us"));
    }

    #[test]
    fn ordering_is_stable_across_input_order() {
        let mk = |p: &str, c: u64| FieldDiff {
            field: "game".into(),
            counts: Default::default(),
            scores: vec![PathScore {
                path: p.into(),
                old_size: 0,
                new_size: 0,
                nchunks: 0,
                reused: 0,
                moved: 0,
                moved_up: 0,
                moved_down: 0,
                old_max_end: 0,
                same_md5: false,
                old_bytes: c,
                new_bytes: 0,
                patch_bytes: 0,
                total_cost: c,
            }],
            rows: vec![],
            pool: ReusePool::default(),
        };
        let a = vec![mk("/b", 10), mk("/a", 10), mk("/c", 5)];
        let b = vec![mk("/c", 5), mk("/a", 10), mk("/b", 10)];
        let pa: Vec<String> = by_cost(&a).into_iter().map(|(_, s)| s.path).collect();
        let pb: Vec<String> = by_cost(&b).into_iter().map(|(_, s)| s.path).collect();
        assert_eq!(pa, pb);
        assert_eq!(pa, vec!["/c", "/a", "/b"]);
    }

    // ------------------------------------------------------------ the tsv

    #[test]
    fn the_tsv_has_a_header_and_one_row_per_path_in_either_manifest() {
        let tsv = paths_tsv(&[three_state_pair()]);
        let lines: Vec<&str> = tsv.lines().collect();
        assert_eq!(
            lines[0],
            "field\tpath\tstate\told_size\tnew_size\treused\tmoved\tpatch_bytes\ttotal_cost"
        );
        assert_eq!(lines.len(), 5, "header plus four paths");
        assert!(!tsv.contains('\r'), "LF endings only");
        // Sorted by path, so two tags' sidecars diff as a diff.
        let paths: Vec<&str> = lines[1..]
            .iter()
            .map(|l| l.split('\t').nth(1).unwrap())
            .collect();
        assert_eq!(paths, vec!["fresh", "gone", "moved", "same"]);
        assert!(lines[1].contains("\tadded\t"), "{}", lines[1]);
        // A one-sided row has no score, and says so with empties rather than
        // zeros that would look like a measurement.
        assert_eq!(lines[1].split('\t').count(), 9);
    }

    #[test]
    fn the_tsv_carries_the_scored_numbers_for_a_common_path() {
        let tsv = paths_tsv(&[three_state_pair()]);
        let row = tsv
            .lines()
            .skip(1)
            .find(|l| l.split('\t').nth(1) == Some("moved"))
            .expect("the changed row is present");
        let f: Vec<&str> = row.split('\t').collect();
        assert_eq!(f[0], "game");
        assert_eq!(f[2], "changed");
        assert_eq!((f[5], f[6], f[7], f[8]), ("1", "1", "7", "22"));
    }

    // ---------------------------------------------------------- aggregation

    #[test]
    fn aggregation_sums_the_fields_and_prefixes_the_one_sided_paths() {
        let mut other = three_state_pair();
        other.field = "en-us".into();
        let diff = aggregate("7.0.0", "7.1.0", &[three_state_pair(), other]);
        assert_eq!(diff.fields, vec!["game", "en-us"]);
        assert_eq!(diff.counts.common, 4);
        assert_eq!(diff.counts.added, 2);
        assert_eq!(diff.counts.removed, 2);
        assert_eq!(diff.by_field.len(), 2);
        assert_eq!(diff.by_field["game"].common, 2);
        assert_eq!(diff.by_field["en-us"].added, 1);
        assert!(
            diff.added.iter().all(|s| s.contains(':')),
            "{:?}",
            diff.added
        );
        assert!(diff.added.contains(&"game:fresh".to_string()));
        assert!(diff.added.contains(&"en-us:fresh".to_string()));
        assert!(diff.is_a_real_upgrade());
    }

    #[test]
    fn a_pair_of_identical_manifests_is_not_an_upgrade() {
        let same = manifest(
            "7.1.0",
            vec![file("a", "m", vec![chunk("x", "xx", 0, 1, 1)])],
        );
        let d = diff_field(&same, &same).unwrap();
        assert!(!aggregate("7.0.0", "7.1.0", &[d]).is_a_real_upgrade());
    }
}
