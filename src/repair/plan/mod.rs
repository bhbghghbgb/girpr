//! The work list: which files the run will touch, in what order, and where each
//! one's chunks are downloaded from (docs/02 step 4) — plus the purge
//! candidates the run would delete (docs/02 steps 5 and 7).
//!
//! Both are **plans**: they decide, they do not act. The run builds both of them
//! before it mutates anything, so "what will this run do?" is answerable at one
//! point in the pipeline — which is what the scope-of-work ceilings
//! (`--max-files`, `--max-download-bytes`, `--max-delete-bytes`) and
//! `--print-plan` are written against.
//! [`purge`] owns the cleanup half; the repair half is this module.
//!
//! The repair plan is derived from the **latest** manifest alone, which is what
//! makes any-version → latest work: the local manifest is only ever an
//! optimization (chunk reuse), never an input to correctness.

pub mod purge;

use std::collections::{HashMap, HashSet};

use crate::sophon::{SophonChunk, SophonChunkFile};

pub use purge::{PurgeCandidate, PurgePlan};

use super::manifest::LocalChunkMap;
use super::scope::PathScope;

/// One target file, plus the chunk URL prefix its chunks live under.
pub struct PlannedFile {
    /// `/`-separated rel path, as it appears in the manifest.
    pub rel: String,
    pub size: i64,
    pub md5: String,
    /// The file's chunks, **sorted by offset** and without the zero-length
    /// ones (both established in [`build_plan`]).
    ///
    /// The sort is not cosmetic: the in-place repair's "is this chunk already
    /// correct where it belongs?" test and the ordered tier's proof that no
    /// source is overwritten before it is read both assume ascending
    /// offsets, and nothing else in the crate sorted them.
    pub chunks: Vec<SophonChunk>,
}

impl PlannedFile {
    /// Does the chunk layout **tile** `[0, size)` exactly, and where does it
    /// not?
    ///
    /// Returns `(tiled, gaps)`:
    ///
    /// - `tiled` — the chunks cover `[0, size)` exactly once each: no hole,
    ///   no overlap, and nothing past the end.
    /// - `gaps` — the ranges of `[0, size)` that are **not** covered exactly
    ///   once: a hole (covered zero times) or an overlap (covered more than
    ///   once), as `(offset, length)`.
    ///
    /// A hole is a hole in the install: the bytes there belong to no chunk,
    /// so nothing writes them and whatever the file happened to have stays.
    /// The in-place repair zero-fills them (and says so); the temp repair
    /// leaves them at whatever the file had.
    pub fn layout(&self) -> (bool, Vec<(u64, u64)>) {
        let size = self.size.max(0) as u64;
        let mut gaps: Vec<(u64, u64)> = Vec::new();
        let mut covered_end = 0u64;
        for c in &self.chunks {
            let offset = c.offset.max(0) as u64;
            let len = c.uncompressed_size.max(0) as u64;
            if len == 0 {
                continue;
            }
            if offset > covered_end {
                // A hole: `[covered_end, offset)` belongs to no chunk.
                gaps.push((covered_end, offset - covered_end));
            } else if offset < covered_end {
                // An overlap: `[offset, min(covered_end, offset+len))` is
                // claimed by two chunks.
                let end = (covered_end).min(offset + len);
                if end > offset {
                    gaps.push((offset, end - offset));
                }
            }
            covered_end = covered_end.max(offset + len);
        }
        if covered_end < size {
            // A hole at the tail: the file is longer than its chunks.
            gaps.push((covered_end, size - covered_end));
        }
        (gaps.is_empty() && covered_end == size, gaps)
    }
}

/// The sorted, deduped work list for one run.
pub struct RepairPlan {
    pub latest: String,
    pub files: Vec<PlannedFile>,
    pub url_prefix_by_file: HashMap<String, String>,
    /// The per-file verdicts, when the scan pass ran (`--scan`).
    ///
    /// `None` means "not scanned", and the download estimate is the
    /// lower bound it has always been. `Some` means the estimate is
    /// **exact**: every chunk's verdict is known before the repair starts.
    pub verdicts: Option<HashMap<String, super::scan::FileVerdict>>,
}

impl RepairPlan {
    /// Paths the files-cleanup must treat as expected (docs/02 step 7).
    pub fn expected_paths(&self) -> HashSet<String> {
        self.files.iter().map(|f| f.rel.clone()).collect()
    }

    /// `Σ compressed_size` over every planned file, against a local reuse map.
    ///
    /// This is a **lower bound, not a promise**: `try_reuse_slice` falls back
    /// to a download whenever a local slice fails its MD5, so a file estimated
    /// at 0 bytes can still download its whole body. It is what
    /// `--max-download-bytes` refuses on, and the runtime hard stop is what
    /// actually enforces.
    ///
    /// **With the scan pass (`--scan`) the estimate is exact**: the
    /// per-chunk verdicts say which chunks are already correct where they
    /// belong (kept, not fetched) and which have a salvageable source, so
    /// the figure is what the repair will actually download.
    pub fn est_download_bytes(&self, local_map: &LocalChunkMap) -> u64 {
        match &self.verdicts {
            Some(verdicts) => self
                .files
                .iter()
                .map(|f| {
                    let Some(v) = verdicts.get(&f.rel) else {
                        // A file the scan did not reach (it failed to read):
                        // fall back to the bound rather than to zero.
                        return est_file_download_bytes(f, local_map.get(&f.rel));
                    };
                    super::scan::chunks_to_download(f, v, local_map.get(&f.rel))
                        .iter()
                        .map(|c| c.compressed_size.max(0) as u64)
                        .sum()
                })
                .sum(),
            None => self
                .files
                .iter()
                .map(|f| est_file_download_bytes(f, local_map.get(&f.rel)))
                .sum(),
        }
    }
}

/// Per-file download estimate: `Σ c.compressed_size for c in plan[rel].chunks
/// where (c.uncompressed_md5, c.uncompressed_size) ∉ local_map[rel]`.
///
/// This is character-for-character `gircap`'s `patch_fetch` formula,
/// so the `PLAN` line, the `SUMMARY plan_download_bytes` key
/// and the capture's `totals.download_bytes` are the same figure. With no local
/// map (a fresh install) it degenerates to "every chunk downloads", which is
/// the honest worst case.
pub fn est_file_download_bytes(file: &PlannedFile, reuse: Option<&Vec<(String, i64, i64)>>) -> u64 {
    file.chunks
        .iter()
        .filter(|c| match reuse {
            Some(entries) => !entries.iter().any(|(md5, sz, _)| {
                md5.eq_ignore_ascii_case(&c.uncompressed_md5) && *sz == c.uncompressed_size
            }),
            None => true,
        })
        .map(|c| c.compressed_size.max(0) as u64)
        .sum()
}

/// Build sorted, deduped, blacklist-filtered, scope-filtered plan.
/// `per_manifest`: (chunk_url_prefix, files).
///
/// A path outside `scope` is dropped **here**, before the plan exists, so
/// `files_total`, the repair loop and `expected_paths()` all see the same
/// narrowed world. Dropping it later instead would leave the purge with a
/// *narrower* keep-set than an unfiltered run.
pub fn build_plan(
    per_manifest: Vec<(String, Vec<SophonChunkFile>)>,
    blacklist: &HashSet<String>,
    latest_tag: &str,
    scope: &PathScope,
) -> RepairPlan {
    let mut files: Vec<PlannedFile> = Vec::new();
    let mut prefix: HashMap<String, String> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (url_prefix, list) in per_manifest {
        for f in list {
            // Folders are dropped: directories are created on demand instead.
            if f.is_folder || f.file.is_empty() {
                continue;
            }
            let rel = f.file.replace('\\', "/");
            if blacklist.contains(&rel) || !scope.is_visible(&rel) || !seen.insert(rel.clone()) {
                continue;
            }
            prefix.insert(rel.clone(), url_prefix.clone());
            // The chunk list is normalized here, once: sorted by offset, and
            // without the zero-length chunks. Both are preconditions the
            // in-place repair's proof rests on (P1/P2 in docs/07 §4.1), and
            // neither is guaranteed by the manifest — a chunk with
            // `uncompressed_size == 0` would be a range that covers nothing,
            // and an unsorted list would make "is this chunk already correct
            // where it belongs?" unanswerable.
            let mut chunks = f.chunks;
            chunks.retain(|c| {
                let keep = c.uncompressed_size > 0;
                if !keep {
                    tracing::warn!(
                        file = %rel,
                        chunk = %c.id,
                        "zero-length chunk dropped from the plan"
                    );
                }
                keep
            });
            chunks.sort_by_key(|c| c.offset);
            files.push(PlannedFile {
                rel,
                size: f.size,
                md5: f.md5.clone(),
                chunks,
            });
        }
    }
    // Stable order so log lines and the parallel spawn order are reproducible.
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    RepairPlan {
        latest: latest_tag.to_string(),
        files,
        url_prefix_by_file: prefix,
        verdicts: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_sorted_dedup_blacklist() {
        let files = vec![
            SophonChunkFile {
                file: "b.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 2,
                md5: "m2".into(),
            },
            SophonChunkFile {
                file: "a.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 1,
                md5: "m1".into(),
            },
            SophonChunkFile {
                file: "drop.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 1,
                md5: "m".into(),
            },
        ];
        let per = vec![("http://x".to_string(), files)];
        let bl: HashSet<String> = ["drop.dat".to_string()].into_iter().collect();
        let plan = build_plan(per, &bl, "5.0", &PathScope::default());
        assert_eq!(
            plan.files
                .iter()
                .map(|f| f.rel.as_str())
                .collect::<Vec<_>>(),
            vec!["a.dat", "b.dat"]
        );
        assert_eq!(plan.latest, "5.0");
        assert_eq!(plan.url_prefix_by_file.get("a.dat").unwrap(), "http://x");
    }

    #[test]
    fn plan_dedups_across_manifests_and_drops_folders() {
        let dup = SophonChunkFile {
            file: "a\\b.dat".into(),
            chunks: vec![],
            is_folder: false,
            size: 1,
            md5: "m1".into(),
        };
        let folder = SophonChunkFile {
            file: "dir/".into(),
            chunks: vec![],
            is_folder: true,
            size: 0,
            md5: String::new(),
        };
        // Same path, backslash spelling, in a second manifest: one entry only.
        let per = vec![
            ("http://x".to_string(), vec![dup.clone()]),
            ("http://y".to_string(), vec![dup, folder]),
        ];
        let plan = build_plan(per, &HashSet::new(), "5.0", &PathScope::default());
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].rel, "a/b.dat");
        // First manifest wins the prefix.
        assert_eq!(plan.url_prefix_by_file.get("a/b.dat").unwrap(), "http://x");
        assert_eq!(plan.expected_paths().len(), 1);
    }

    /// The scope is applied *before* the plan exists, so `expected_paths()` —
    /// the only manifest-derived input to the files-cleanup's keep set — is
    /// narrowed too. Applying it anywhere downstream instead would leave the
    /// cleanup with a narrower keep set than an unfiltered run.
    #[test]
    fn out_of_scope_paths_never_reach_the_plan() {
        let files = vec![
            SophonChunkFile {
                file: "game/foo.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 1,
                md5: "m1".into(),
            },
            SophonChunkFile {
                file: "game/bar.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 1,
                md5: "m2".into(),
            },
        ];
        let per = vec![("http://x".to_string(), files)];
        let inc = vec!["game/foo.dat".to_string()];
        let plan = build_plan(
            per,
            &HashSet::new(),
            "5.0",
            &PathScope::new(&inc, &[]).unwrap(),
        );
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].rel, "game/foo.dat");
        assert_eq!(plan.expected_paths(), ["game/foo.dat".to_string()].into());
        assert!(
            !plan.url_prefix_by_file.contains_key("game/bar.dat"),
            "a dropped path must not keep a prefix entry either"
        );
    }

    fn chunk(id: &str, offset: i64, size: i64) -> SophonChunk {
        SophonChunk {
            id: id.into(),
            uncompressed_md5: format!("md5-{id}"),
            offset,
            compressed_size: size,
            uncompressed_size: size,
            unknown: 0,
            compressed_md5: String::new(),
        }
    }

    fn planned(size: i64, chunks: Vec<SophonChunk>) -> PlannedFile {
        PlannedFile {
            rel: "f.dat".into(),
            size,
            md5: "m".into(),
            chunks,
        }
    }

    /// P1: the chunks come out of the plan sorted by offset, whatever order
    /// the manifest listed them in.
    #[test]
    fn the_plan_sorts_a_files_chunks_by_offset() {
        let per = vec![(
            "http://x".to_string(),
            vec![SophonChunkFile {
                file: "f.dat".into(),
                chunks: vec![chunk("c2", 64, 64), chunk("c0", 0, 64), chunk("c1", 32, 32)],
                is_folder: false,
                size: 128,
                md5: "m".into(),
            }],
        )];
        let plan = build_plan(per, &HashSet::new(), "5.0", &PathScope::default());
        let offsets: Vec<i64> = plan.files[0].chunks.iter().map(|c| c.offset).collect();
        assert_eq!(offsets, vec![0, 32, 64], "ascending, always");
    }

    /// A zero-length chunk covers nothing, and a plan that carries one cannot
    /// tile the file. It is dropped at plan time rather than repaired around.
    #[test]
    fn a_zero_length_chunk_is_dropped_from_the_plan() {
        let per = vec![(
            "http://x".to_string(),
            vec![SophonChunkFile {
                file: "f.dat".into(),
                chunks: vec![chunk("c0", 0, 64), chunk("empty", 64, 0)],
                is_folder: false,
                size: 64,
                md5: "m".into(),
            }],
        )];
        let plan = build_plan(per, &HashSet::new(), "5.0", &PathScope::default());
        assert_eq!(plan.files[0].chunks.len(), 1);
        assert_eq!(plan.files[0].chunks[0].id, "c0");
    }

    /// P2: the chunks tile `[0, size)` exactly — no hole, no overlap, nothing
    /// past the end.
    #[test]
    fn a_tiling_layout_reports_no_gaps() {
        let f = planned(16, vec![chunk("c0", 0, 8), chunk("c1", 8, 8)]);
        let (tiled, gaps) = f.layout();
        assert!(tiled, "{gaps:?}");
        assert!(gaps.is_empty());
    }

    /// A hole is reported: the bytes there belong to no chunk.
    #[test]
    fn a_hole_in_the_layout_is_reported() {
        let f = planned(16, vec![chunk("c0", 0, 4), chunk("c1", 8, 8)]);
        let (tiled, gaps) = f.layout();
        assert!(!tiled, "a hole is not a tiling");
        assert_eq!(gaps, vec![(4, 4)], "the hole is [4,8)");
    }

    /// An overlapping pair is reported too: two chunks claim the same bytes.
    #[test]
    fn an_overlapping_pair_is_reported() {
        let f = planned(12, vec![chunk("c0", 0, 8), chunk("c1", 4, 8)]);
        let (tiled, gaps) = f.layout();
        assert!(!tiled, "an overlap is not a tiling");
        assert_eq!(gaps, vec![(4, 4)], "the overlap is [4,8)");
    }

    /// A file whose chunks stop short of its own size has a hole at the tail.
    #[test]
    fn a_tail_beyond_the_last_chunk_is_reported() {
        let f = planned(16, vec![chunk("c0", 0, 8)]);
        let (tiled, gaps) = f.layout();
        assert!(!tiled);
        assert_eq!(gaps, vec![(8, 8)]);
    }

    /// Chunks that reach past the file's own size are not a tiling either.
    #[test]
    fn chunks_past_the_end_of_the_file_are_not_a_tiling() {
        let f = planned(8, vec![chunk("c0", 0, 16)]);
        let (tiled, gaps) = f.layout();
        assert!(!tiled);
        assert!(gaps.is_empty(), "nothing is missing — it is too much");
    }
}
