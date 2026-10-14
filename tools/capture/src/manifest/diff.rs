//! The path diff, the reuse pool, and the `getBuild.stats` reconciliation.
//!
//! **M4.** This is the module that makes the artifact a *measurement* rather
//! than a dump. Four things live here, and they are deliberately kept apart:
//!
//! 1. **Set difference over paths** — common / identical / changed / added /
//!    removed. Cheap, and the only thing a reader needs to know whether a
//!    version pair was interesting at all.
//! 2. **Per-path scoring** — `reused`, `moved`, `patch_bytes`, `total_cost`.
//!    §5 defines these; M4 computes them and M5 *selects* on them.
//! 3. **The reuse pool** — how much same-file-only reuse forgoes. This is a
//!    measurement of a deliberate simplification in the tool under test, and
//!    nothing here acts on it.
//! 4. **Two identities, deliberately different.** Scoring compares chunks by
//!    `(uncompressed_md5, uncompressed_size)` — MD5 alone is not enough,
//!    because a truncated chunk can share one. The reuse-pool headline number is
//!    defined in §10 as *"an MD5 present somewhere in the old manifest"* and is
//!    counted that way, so it measures what the document says it measures.
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
type ChunkKey = (String, i64);

/// One common path's score (§5).
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

/// `diff/reuse_pool.json` — the cost of same-file-only reuse (§10).
///
/// Recorded because it bounds a deliberate simplification in the tool under test.
/// `gircap` never acts on any of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ReusePool {
    /// Every chunk in the new manifest.
    pub new_chunks_total: u64,
    /// How many of them have an MD5 present *somewhere* in the old manifest.
    ///
    /// MD5-only, because that is what the number in §10 measures. §5's scoring
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

/// Everything M4 derives from one pair of captured manifests.
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
    let old_by_path: HashMap<&str, (&SophonChunkFile, HashMap<ChunkKey, Vec<i64>>)> = old
        .entries
        .iter()
        .map(|e| (e.file.as_str(), (e, index_file(e))))
        .collect();

    // Old manifest-wide MD5 → set of paths holding it, for the reuse pool.
    let old_md5_paths: HashMap<&str, HashSet<&str>> = build_md5_path_index(&old.entries);

    let new_paths: HashSet<&str> = new.entries.iter().map(|e| e.file.as_str()).collect();

    let mut counts = FieldCounts::default();
    let mut scores = Vec::new();
    let mut rows: Vec<PathRow> = Vec::with_capacity(old.entries.len() + new.entries.len());

    // New side: common and added.
    for e in &new.entries {
        match old_by_path.get(e.file.as_str()) {
            Some((old_entry, file_index)) => {
                counts.common += 1;
                let same_md5 = old_entry.md5 == e.md5;
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

    let pool = reuse_pool(old, new, &old_md5_paths);

    Ok(FieldDiff {
        field,
        counts,
        scores,
        rows,
        pool,
    })
}

/// `(key) -> [offsets]` for one file's chunks.
fn index_file(e: &SophonChunkFile) -> HashMap<ChunkKey, Vec<i64>> {
    let mut m: HashMap<ChunkKey, Vec<i64>> = HashMap::with_capacity(e.chunks.len());
    for c in &e.chunks {
        m.entry(c.identity().into_key()).or_default().push(c.offset);
    }
    m
}

trait KeyExt {
    fn into_key(self) -> ChunkKey;
}

impl KeyExt for (&str, i64) {
    fn into_key(self) -> ChunkKey {
        (self.0.to_string(), self.1)
    }
}

/// `uncompressed_md5 -> the old paths whose chunks carry it`.
///
/// MD5-only and one path per hash rather than one per chunk: the question this
/// answers is "could this content come from *anywhere*", and a content hash
/// appearing 400 times in one file is still one file.
fn build_md5_path_index(entries: &[SophonChunkFile]) -> HashMap<&str, HashSet<&str>> {
    let mut m: HashMap<&str, HashSet<&str>> = HashMap::new();
    for e in entries {
        for c in &e.chunks {
            m.entry(c.uncompressed_md5.as_str())
                .or_default()
                .insert(e.file.as_str());
        }
    }
    m
}

/// Score one common path (§5).
pub fn score(
    new: &SophonChunkFile,
    old: &SophonChunkFile,
    old_index: &HashMap<ChunkKey, Vec<i64>>,
) -> PathScore {
    let mut reused = 0_u64;
    let mut moved = 0_u64;
    let mut patch_bytes = 0_u64;
    let mut new_bytes = 0_u64;
    for c in &new.chunks {
        new_bytes += c.compressed_size.max(0) as u64;
        match old_index.get(&c.identity().into_key()) {
            Some(offsets) => {
                reused += 1;
                if !offsets.contains(&c.offset) {
                    moved += 1;
                }
            }
            None => patch_bytes += c.compressed_size.max(0) as u64,
        }
    }
    let old_bytes = old
        .chunks
        .iter()
        .map(|c| c.compressed_size.max(0) as u64)
        .sum();
    PathScore {
        path: new.file.clone(),
        old_size: old.size,
        new_size: new.size,
        nchunks: new.chunks.len() as u64,
        reused,
        moved,
        same_md5: old.md5 == new.md5,
        old_bytes,
        new_bytes,
        patch_bytes,
        total_cost: old_bytes + new_bytes,
    }
}

/// Measure what same-file-only reuse forgoes.
fn reuse_pool(
    old: &CapturedManifest,
    new: &CapturedManifest,
    old_md5_paths: &HashMap<&str, HashSet<&str>>,
) -> ReusePool {
    let old_by_path: HashMap<&str, HashMap<ChunkKey, Vec<i64>>> = old
        .entries
        .iter()
        .map(|e| (e.file.as_str(), index_file(e)))
        .collect();

    let mut pool = ReusePool {
        new_chunks_total: 0,
        new_chunks_md5_present_in_old_manifest: 0,
        cross_file_only_chunks: 0,
        files_with_a_cross_file_only_chunk: 0,
        cross_file_only_bytes_forgone: 0,
        note: ReusePool::NOTE,
    };

    for e in &new.entries {
        let file_index = old_by_path.get(e.file.as_str());
        let mut file_has_cross = false;
        for c in &e.chunks {
            pool.new_chunks_total += 1;
            let in_manifest = old_md5_paths.contains_key(c.uncompressed_md5.as_str());
            if in_manifest {
                pool.new_chunks_md5_present_in_old_manifest += 1;
            }
            // Reachable from this same file?
            let in_same_file = file_index
                .is_some_and(|idx| idx.contains_key(&c.identity().into_key()));
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
pub fn aggregate(
    old_tag: &str,
    new_tag: &str,
    fields: &[FieldDiff],
) -> PathDiff {
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
        new_chunks_reusable_at_all: fields.iter().map(|f| f.pool.new_chunks_md5_present_in_old_manifest).sum(),
    }
}

/// `diff/paths.tsv` — the flat per-path form, sorted by path.
///
/// One row per path in either manifest: `path`, `state`, `old_size`, `new_size`,
/// `reused`, `moved`, `patch_bytes`, `total_cost`. Plain text always, header
/// line, LF endings — this is the file a reader greps.
pub fn paths_tsv(diffs: &[FieldDiff]) -> String {
    let header = "field\tpath\tstate\told_size\tnew_size\treused\tmoved\tpatch_bytes\ttotal_cost\n";
    let mut s = String::from(header);
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
                    s.push_str(&sc.old_size.to_string());
                    s.push('\t');
                    s.push_str(&sc.new_size.to_string());
                    s.push('\t');
                    s.push_str(&sc.reused.to_string());
                    s.push('\t');
                    s.push_str(&sc.moved.to_string());
                    s.push('\t');
                    s.push_str(&sc.patch_bytes.to_string());
                    s.push('\t');
                    s.push_str(&sc.total_cost.to_string());
                }
                None => s.push_str("\t\t0\t0\t0\t0"),
            }
            s.push('\n');
        }
    }
    s
}

/// The cheapest scores, already ordered for §5's tie-break.
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

    /// §5's formulas, computed by hand on the fixture above.
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
        assert_eq!(moved.total_cost, 22, "old_bytes + new_bytes, not patch_bytes");
        assert!(!moved.same_md5);

        let same = d.scores.iter().find(|s| s.path == "same").unwrap();
        assert_eq!(same.reused, 1);
        assert_eq!(same.moved, 0);
        assert_eq!(same.patch_bytes, 0, "nothing to fetch for an identical file");
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
        let old = manifest("7.0.0", vec![file("f", "o", vec![chunk("a", "shared", 0, 10, 5)])]);
        let new = manifest("7.1.0", vec![file("f", "n", vec![chunk("a", "shared", 0, 7, 5)])]);
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
            d.pool.new_chunks_md5_present_in_old_manifest,
            2,
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
        let old = manifest("7.0.0", vec![file("f", "o", vec![chunk("a", "aa", 0, 10, 5)])]);
        let new = manifest("7.1.0", vec![file("f", "n", vec![chunk("a", "aa", 0, 10, 5)])]);
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

    /// §5's tie-break chain, asserted on inputs designed to collide at each
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
    /// put a voice pack ahead of a game file on a cost tie — the opposite of §5.
    #[test]
    fn game_ranks_before_the_audio_fields_not_alphabetically() {
        assert!(field_rank(GAME_FIELD) < field_rank("en-us"));
        assert!(field_rank(GAME_FIELD) < field_rank("zh-cn"));
        assert!(field_rank("en-us") < field_rank("zh-cn"), "and then by name");
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
        let paths: Vec<&str> = lines[1..].iter().map(|l| l.split('\t').nth(1).unwrap()).collect();
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
        assert!(diff.added.iter().all(|s| s.contains(':')), "{:?}", diff.added);
        assert!(diff.added.contains(&"game:fresh".to_string()));
        assert!(diff.added.contains(&"en-us:fresh".to_string()));
        assert!(diff.is_a_real_upgrade());
    }

    #[test]
    fn a_pair_of_identical_manifests_is_not_an_upgrade() {
        let same = manifest("7.1.0", vec![file("a", "m", vec![chunk("x", "xx", 0, 1, 1)])]);
        let d = diff_field(&same, &same).unwrap();
        assert!(!aggregate("7.0.0", "7.1.0", &[d]).is_a_real_upgrade());
    }
}