//! The work list: which files the run will touch, in what order, and where each
//! one's chunks are downloaded from (docs/02 step 4) — plus the purge
//! candidates the run would delete (docs/02 steps 5 and 7).
//!
//! Both are **plans**: they decide, they do not act. The run builds both of them
//! before it mutates anything, so "what will this run do?" is answerable at one
//! point in the pipeline — which is what the scope-of-work ceilings
//! (`--max-files`, `--max-download-bytes`, `--max-delete-bytes`) and
//! `--print-plan` are written against.
//! [`super::super::purge`] owns the cleanup half; the repair half is this module.
//!
//! The repair plan is derived from the **latest** manifest alone, which is what
//! makes any-version → latest work: the local manifest is only ever an
//! optimization (chunk reuse), never an input to correctness.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};

use crate::sophon::{SophonChunk, SophonChunkFile};

use super::manifest::LocalChunkMap;
use crate::repair::scope::PathScope;
use crate::util;
use crate::util::path::path_key;

/// One target file, plus the chunk URL prefix its chunks live under.
#[derive(Clone)]
pub struct PlannedFile {
    /// `/`-separated rel path, as it appears in the manifest.
    pub rel: String,
    pub size: i64,
    pub md5: String,
    /// The file's chunks, **sorted by offset** (see `build_plan`) and with
    /// the empty ones dropped.
    pub chunks: Vec<SophonChunk>,
}

impl PlannedFile {
    /// The shape of this file's chunk layout.
    ///
    /// Computed from the chunk list alone — no I/O — so it can be asked at
    /// plan time (before anything is written) and again by the writer.
    ///
    /// Two things are answered, and both matter to an in-place repair:
    ///
    /// - **do the chunks tile `[0, size)`?** A hole in the coverage would
    ///   otherwise keep whatever the old file had there — stale bytes that
    ///   no chunk ever wrote, and that the whole-file MD5 would then reject.
    ///   An overlap means two chunks fighting over the same bytes, and which
    ///   one wins is not a question the writer should have to answer.
    /// - **where are the holes and the overlaps?** The writer needs the holes
    ///   by offset: they are the places it has to zero-fill.
    ///
    /// The answer does not depend on the order the chunks are given in: the
    /// ranges are sorted here, so a caller that builds a `PlannedFile` by
    /// hand gets the same verdict as one that came out of `build_plan`.
    pub fn layout(&self) -> Layout {
        let size = self.size.max(0) as u64;
        let mut out = Layout {
            tiled: true,
            ..Default::default()
        };
        let mut ranges: Vec<(u64, u64)> = self
            .chunks
            .iter()
            .filter_map(|c| {
                // A negative offset or size is not a range at all; it cannot
                // be placed, so it cannot tile anything.
                if c.offset < 0 || c.uncompressed_size < 0 {
                    return None;
                }
                Some((c.offset as u64, c.uncompressed_size as u64))
            })
            .collect();
        ranges.sort_unstable();
        // `cursor` is the end of the coverage so far: everything below it is
        // claimed, everything above it is not.
        let mut cursor = 0u64;
        let mut prev_offset = 0u64;
        for (offset, len) in ranges {
            if offset > cursor {
                out.gaps.push((cursor, offset - cursor));
                out.tiled = false;
            } else if offset < cursor {
                out.overlaps.push((prev_offset, offset));
                out.tiled = false;
            }
            if offset + len > cursor {
                cursor = offset + len;
                prev_offset = offset;
            }
        }
        if cursor < size {
            // The tail of the file that no chunk covers.
            out.gaps.push((cursor, size - cursor));
            out.tiled = false;
        } else if cursor > size {
            // The chunks claim more bytes than the file has.
            out.tiled = false;
        }
        out
    }
}

/// The shape of a file's chunk layout — see [`PlannedFile::layout`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Layout {
    /// The chunks cover `[0, size)` exactly: no hole, no overlap, and the
    /// last chunk ends exactly at the file's size.
    pub tiled: bool,
    /// Holes in the coverage: `(offset, len)` that no chunk covers. The
    /// writer has to zero-fill these.
    pub gaps: Vec<(u64, u64)>,
    /// Overlapping chunks: `(earlier_offset, later_offset)`. Two chunks
    /// claiming the same bytes.
    pub overlaps: Vec<(u64, u64)>,
}

/// The sorted, deduped work list for one run.
pub struct RepairPlan {
    pub latest: String,
    pub files: Vec<PlannedFile>,
    pub url_prefix_by_file: HashMap<String, String>,
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
    pub fn est_download_bytes(&self, local_map: &LocalChunkMap) -> u64 {
        self.est_download_bytes_by_file(local_map)
            .iter()
            .fold(0u64, |total, n| total.saturating_add(*n))
    }

    /// The same estimate, per file, in `plan.files` order.
    ///
    /// One derivation, two consumers: the total on the `PLAN` line and the
    /// per-file halves in the `--plan-out` document. Re-deriving the split
    /// elsewhere is what let the two disagree under `--scan`.
    pub fn est_download_bytes_by_file(&self, local_map: &LocalChunkMap) -> Vec<u64> {
        self.files
            .iter()
            .map(|f| est_file_download_bytes(f, local_map.get(&f.rel)))
            .collect()
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
        // Server-controlled sizes can overflow a `u64` (three chunks at
        // `i64::MAX`); saturate so a corrupt manifest is an over-approximation,
        // not a panic in the plan phase.
        .fold(0u64, |total, c| {
            total.saturating_add(c.compressed_size.max(0) as u64)
        })
}

/// Reject a chunk range a corrupt manifest can carry before it reaches a seek.
///
/// `offset` and `size` are `i64` values from the server, and the old end
/// computation added them as `i64`, which panicked on an overflow. A negative
/// value is not a range at all, and a range that ends past the file's declared
/// size cannot produce the file the manifest describes.
///
/// This is **plan-time** validation: [`build_plan`] runs it for every file that
/// survives the blacklist/scope/dedup filters, so a malformed range is a
/// metadata error (exit `2`) before any mode runs. `repair_one_file` also calls
/// it as a defensive second gate for a plan that was not built by
/// [`build_plan`], so the write path cannot seek on a bad range either.
pub(crate) fn validate_chunk_ranges(file: &PlannedFile) -> Result<()> {
    if file.size < 0 {
        anyhow::bail!("file has an invalid size: {}", file.size);
    }
    let file_size = file.size as u64;
    for c in &file.chunks {
        if c.offset < 0 {
            anyhow::bail!("chunk {} has a negative offset: {}", c.id, c.offset);
        }
        if c.uncompressed_size < 0 {
            anyhow::bail!(
                "chunk {} has a negative size: {}",
                c.id,
                c.uncompressed_size
            );
        }
        let end = c.offset.checked_add(c.uncompressed_size).ok_or_else(|| {
            anyhow::anyhow!(
                "chunk {} range overflows: {} + {}",
                c.id,
                c.offset,
                c.uncompressed_size
            )
        })?;
        if end as u64 > file_size {
            anyhow::bail!(
                "chunk {} ends at {end}, past the file's size {file_size}",
                c.id
            );
        }
    }
    Ok(())
}

/// Build sorted, deduped, blacklist-filtered, scope-filtered plan.
/// `per_manifest`: (chunk_url_prefix, files).
///
/// A blacklisted path leaves the **work list** here but does not leave the
/// expected install: [`crate::repair::purge::plan_purge`] receives the same
/// blacklist as a keep input, so a correctly spelled entry can never be
/// deleted by `--purge-*` (docs/01: blacklisted entries are never deleted).
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
) -> Result<RepairPlan> {
    let mut files: Vec<PlannedFile> = Vec::new();
    let mut prefix: HashMap<String, String> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (url_prefix, list) in per_manifest {
        for f in list {
            // Folders are dropped: directories are created on demand instead.
            if f.is_folder || f.file.is_empty() {
                continue;
            }
            // The server chose this path, and every sink below (scan read,
            // repair write, `_tmp` promote) joins it under `game_dir`. Refuse
            // an escape here, before any of them exists: a bad manifest is
            // metadata, exit `2`, not a file to repair.
            let rel =
                util::clean_rel(&f.file).with_context(|| format!("manifest path {:?}", f.file))?;
            // Windows paths are case-insensitive, and the path scope is
            // matched case-insensitively too. Two manifest spellings that
            // differ only in case therefore name **one** file; planning both
            // would hand the same target to two workers at once.
            if blacklist.contains(&rel) || !scope.is_visible(&rel) || !seen.insert(path_key(&rel)) {
                continue;
            }
            prefix.insert(rel.clone(), url_prefix.clone());
            // The chunk list is made **ascending by offset** here, once,
            // because everything downstream depends on it and nothing
            // upstream guarantees it: the manifest is whatever the server
            // served. The in-place repair writes each chunk at its own
            // offset and the ordered tier's proof talks about
            // "the bytes below the cursor", so both need the order to be a
            // fact rather than an accident of the manifest.
            //
            // `sort_by_key` is stable, so two chunks at the same offset keep
            // the manifest's relative order.
            let mut chunks = f.chunks;
            chunks.sort_by_key(|c| c.offset);
            let mut file = PlannedFile {
                rel,
                size: f.size,
                md5: f.md5.clone(),
                chunks,
            };
            // Validate **before** the empty-chunk drop: a zero-size chunk is an
            // allowed no-op that is removed below, but a negative size is a
            // malformed range and must be refused, not quietly discarded. The
            // filters above have already run, so a corrupt entry the run
            // excludes cannot block the rest of the plan.
            validate_chunk_ranges(&file)
                .with_context(|| format!("validate chunk ranges for {:?}", file.rel))?;
            // A chunk with no content is a no-op at best and a hole in the
            // coverage at worst: it claims an offset and covers nothing.
            // Dropped defensively — a zero-size chunk is allowed, but the file
            // can be repaired without it. The drop and the count describe the
            // same chunk list, so they are one pass.
            let before = file.chunks.len();
            file.chunks.retain(|c| c.uncompressed_size > 0);
            let empty = before - file.chunks.len();
            if empty > 0 {
                tracing::warn!(rel = %file.rel, empty, "dropped empty chunk(s) from the plan");
            }
            // The layout is checked **at plan time**, before anything is
            // written: a hole in the coverage is a thing the writer has to
            // zero-fill, and an overlap is a thing it has to resolve, and
            // both are better known before the first byte moves than after.
            let layout = file.layout();
            if !layout.tiled {
                tracing::warn!(
                    rel = %file.rel,
                    gaps = layout.gaps.len(),
                    overlaps = layout.overlaps.len(),
                    "chunk layout does not tile the file"
                );
            }
            files.push(file);
        }
    }
    // Stable order so log lines and the parallel spawn order are reproducible.
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(RepairPlan {
        latest: latest_tag.to_string(),
        files,
        url_prefix_by_file: prefix,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chunk with the given offset and length.
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

    /// A `PlannedFile` with the given chunks, built by hand — so the layout
    /// can be asked about a chunk list no `build_plan` would produce.
    fn file_with(size: i64, chunks: Vec<SophonChunk>) -> PlannedFile {
        PlannedFile {
            rel: "f.dat".into(),
            size,
            md5: "m".into(),
            chunks,
        }
    }

    /// A manifest entry with the given path, declared size and chunks.
    fn manifest_file(rel: &str, size: i64, chunks: Vec<SophonChunk>) -> SophonChunkFile {
        SophonChunkFile {
            file: rel.into(),
            chunks,
            is_folder: false,
            size,
            md5: "m".into(),
        }
    }

    /// Build a plan from one manifest, returning the error text if it refuses.
    fn plan_error(file: SophonChunkFile) -> String {
        let per = vec![("http://x".to_string(), vec![file])];
        match build_plan(per, &HashSet::new(), "5.0", &PathScope::default()) {
            Ok(_) => panic!("a malformed range must be a plan error"),
            Err(err) => format!("{err:#}"),
        }
    }

    // -----------------------------------------------------------------------
    // the chunk layout
    // -----------------------------------------------------------------------

    #[test]
    fn a_layout_that_covers_the_file_exactly_is_tiled() {
        let f = file_with(128, vec![chunk("a", 0, 64), chunk("b", 64, 64)]);
        let layout = f.layout();
        assert!(layout.tiled);
        assert!(layout.gaps.is_empty());
        assert!(layout.overlaps.is_empty());
    }

    #[test]
    fn a_hole_in_the_coverage_is_reported() {
        // 64 bytes of hole between the two chunks.
        let f = file_with(192, vec![chunk("a", 0, 64), chunk("b", 128, 64)]);
        let layout = f.layout();
        assert!(!layout.tiled);
        assert_eq!(layout.gaps, vec![(64, 64)]);
        assert!(layout.overlaps.is_empty());
    }

    #[test]
    fn a_tail_no_chunk_covers_is_a_hole_too() {
        let f = file_with(200, vec![chunk("a", 0, 64), chunk("b", 64, 64)]);
        let layout = f.layout();
        assert!(!layout.tiled);
        assert_eq!(layout.gaps, vec![(128, 72)]);
    }

    #[test]
    fn an_overlapping_pair_is_reported() {
        let f = file_with(96, vec![chunk("a", 0, 64), chunk("b", 32, 64)]);
        let layout = f.layout();
        assert!(!layout.tiled);
        assert_eq!(layout.overlaps, vec![(0, 32)]);
    }

    #[test]
    fn chunks_that_claim_more_than_the_file_has_is_not_tiled() {
        let f = file_with(64, vec![chunk("a", 0, 128)]);
        let layout = f.layout();
        assert!(!layout.tiled);
        assert!(layout.gaps.is_empty());
    }

    #[test]
    fn the_layout_answer_does_not_depend_on_the_order_of_the_chunks() {
        let a = chunk("a", 0, 64);
        let b = chunk("b", 64, 64);
        let forward = file_with(128, vec![a.clone(), b.clone()]).layout();
        let backward = file_with(128, vec![b, a]).layout();
        assert_eq!(forward, backward);
    }

    #[test]
    fn a_file_with_no_chunks_and_no_size_is_tiled() {
        let f = file_with(0, vec![]);
        assert!(f.layout().tiled);
    }

    // -----------------------------------------------------------------------
    // build_plan
    // -----------------------------------------------------------------------

    #[test]
    fn the_plan_sorts_the_chunks_by_offset() {
        // The manifest is whatever the server served: the chunks come out in
        // whatever order it listed them, and nothing downstream may depend on
        // that.
        let f = SophonChunkFile {
            file: "a.dat".into(),
            chunks: vec![
                chunk("c2", 128, 64),
                chunk("c0", 0, 64),
                chunk("c1", 64, 64),
            ],
            is_folder: false,
            size: 192,
            md5: "m1".into(),
        };
        let per = vec![("http://x".to_string(), vec![f])];
        let plan = build_plan(per, &HashSet::new(), "5.0", &PathScope::default()).unwrap();
        let offsets: Vec<i64> = plan.files[0].chunks.iter().map(|c| c.offset).collect();
        assert_eq!(offsets, vec![0, 64, 128]);
    }

    #[test]
    fn the_plan_drops_empty_chunks() {
        let f = SophonChunkFile {
            file: "a.dat".into(),
            chunks: vec![chunk("c0", 0, 64), chunk("empty", 64, 0)],
            is_folder: false,
            size: 64,
            md5: "m1".into(),
        };
        let per = vec![("http://x".to_string(), vec![f])];
        let plan = build_plan(per, &HashSet::new(), "5.0", &PathScope::default()).unwrap();
        assert_eq!(plan.files[0].chunks.len(), 1);
        assert_eq!(plan.files[0].chunks[0].id, "c0");
    }

    #[test]
    fn a_plan_whose_chunks_do_not_tile_is_still_built() {
        // The layout is *reported*, not enforced: the writer can zero-fill a
        // hole, and refusing the file here would turn a repairable file into
        // a failed one.
        let f = SophonChunkFile {
            file: "a.dat".into(),
            chunks: vec![chunk("c0", 0, 64), chunk("c1", 128, 64)],
            is_folder: false,
            size: 192,
            md5: "m1".into(),
        };
        let per = vec![("http://x".to_string(), vec![f])];
        let plan = build_plan(per, &HashSet::new(), "5.0", &PathScope::default()).unwrap();
        assert_eq!(plan.files.len(), 1);
        let layout = plan.files[0].layout();
        assert!(!layout.tiled);
        assert_eq!(layout.gaps, vec![(64, 64)]);
    }

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
        let plan = build_plan(per, &bl, "5.0", &PathScope::default()).unwrap();
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
        let plan = build_plan(per, &HashSet::new(), "5.0", &PathScope::default()).unwrap();
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].rel, "a/b.dat");
        // First manifest wins the prefix.
        assert_eq!(plan.url_prefix_by_file.get("a/b.dat").unwrap(), "http://x");
        assert_eq!(plan.expected_paths().len(), 1);
    }

    /// A manifest path is server data. Anything that can leave `game_dir` is
    /// refused at plan time — exit `2`, before a directory is created or a byte
    /// is written — rather than joined and followed by the sinks.
    #[test]
    fn a_manifest_path_that_escapes_the_game_dir_is_refused() {
        for bad in [
            "../outside.dat",
            "/abs/evil.dat",
            "C:evil.dat",
            "C:/evil.dat",
            r"\\server\share\evil.dat",
        ] {
            let f = SophonChunkFile {
                file: bad.into(),
                chunks: vec![],
                is_folder: false,
                size: 1,
                md5: "m".into(),
            };
            let per = vec![("http://x".to_string(), vec![f])];
            let got = build_plan(per, &HashSet::new(), "5.0", &PathScope::default());
            assert!(got.is_err(), "{bad:?} must not reach the plan");
        }
    }

    /// Windows: two manifest spellings that differ only in case name the same
    /// file. Planning both would spawn two workers writing the same target
    /// concurrently; the scope is already case-insensitive, so the plan has to
    /// agree.
    #[cfg(windows)]
    #[test]
    fn the_plan_dedups_case_insensitively_on_windows() {
        let mut files = Vec::new();
        for (file, md5) in [("Game/Foo.dat", "m1"), ("game/foo.dat", "m2")] {
            files.push(SophonChunkFile {
                file: file.into(),
                chunks: vec![],
                is_folder: false,
                size: 1,
                md5: md5.into(),
            });
        }
        let per = vec![("http://x".to_string(), files)];
        let plan = build_plan(per, &HashSet::new(), "5.0", &PathScope::default()).unwrap();
        assert_eq!(plan.files.len(), 1, "case variants are one file");
        assert_eq!(plan.files[0].rel, "Game/Foo.dat", "the first spelling wins");
    }

    /// Server-controlled chunk sizes can overflow the `u64` sum: three chunks
    /// at `i64::MAX` do not fit (two do). Saturating keeps the estimate an
    /// over-approximation instead of panicking the plan phase.
    #[test]
    fn an_overflowing_download_estimate_saturates() {
        let huge = chunk("c", 0, i64::MAX);
        let file = PlannedFile {
            rel: "f.bin".into(),
            size: 64,
            md5: "m".into(),
            chunks: vec![huge.clone(), huge.clone(), huge],
        };
        assert_eq!(est_file_download_bytes(&file, None), u64::MAX);
    }

    /// A leading `.` component is accepted but must not survive into the
    /// plan: the purge walk reports the plain form, so a stored `./game/foo.dat`
    /// would not match `expected_paths()` and `--purge-after` would delete the
    /// file the run just repaired.
    #[test]
    fn a_leading_dot_component_is_stripped_from_the_plan() {
        let f = SophonChunkFile {
            file: "./game/foo.dat".into(),
            chunks: vec![],
            is_folder: false,
            size: 1,
            md5: "m".into(),
        };
        let per = vec![("http://x".to_string(), vec![f])];
        let plan = build_plan(per, &HashSet::new(), "5.0", &PathScope::default()).unwrap();
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].rel, "game/foo.dat");
        assert_eq!(plan.expected_paths(), ["game/foo.dat".to_string()].into());
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
        )
        .unwrap();
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].rel, "game/foo.dat");
        assert_eq!(plan.expected_paths(), ["game/foo.dat".to_string()].into());
        assert!(
            !plan.url_prefix_by_file.contains_key("game/bar.dat"),
            "a dropped path must not keep a prefix entry either"
        );
    }

    // -----------------------------------------------------------------------
    // plan-time chunk-range validation
    // -----------------------------------------------------------------------

    /// Every malformed shape a corrupt manifest can carry is a plan error
    /// (exit `2`), named with the file's rel. This is the same gate for every
    /// mode, so `--check-only`, `--dry-run` and `--print-plan` cannot disagree
    /// with the write path.
    #[test]
    fn malformed_chunk_ranges_are_refused_at_plan_time() {
        let cases = [
            (
                "negative file size",
                manifest_file("bad.dat", -1, vec![]),
                "invalid size",
            ),
            (
                "negative chunk offset",
                manifest_file("bad.dat", 64, vec![chunk("c", -1, 32)]),
                "negative offset",
            ),
            // A negative size must be refused *before* the empty-chunk drop:
            // a corrupt range is a refusal, not a missing chunk.
            (
                "negative chunk size",
                manifest_file("bad.dat", 64, vec![chunk("c", 0, -1)]),
                "negative size",
            ),
            (
                "offset plus size overflow",
                manifest_file("bad.dat", i64::MAX, vec![chunk("c", i64::MAX, 1)]),
                "overflows",
            ),
            (
                "range past the file size",
                manifest_file("bad.dat", 64, vec![chunk("c", 32, 64)]),
                "past the file's size",
            ),
        ];
        for (name, file, needle) in cases {
            let msg = plan_error(file);
            assert!(msg.contains("bad.dat"), "{name}: rel missing from {msg}");
            assert!(msg.contains(needle), "{name}: expected {needle:?} in {msg}");
        }
    }

    /// A zero-length file is legitimate only when it declares no bytes. One
    /// that declares a chunk has a range past its own size, and the same gate
    /// that refuses every other malformed range refuses it here — before the
    /// empty-chunk drop could quietly delete the evidence.
    #[test]
    fn a_zero_length_entry_declaring_a_chunk_is_refused_at_plan_time() {
        let msg = plan_error(manifest_file("empty.dat", 0, vec![chunk("c", 0, 64)]));
        assert!(msg.contains("empty.dat"), "rel missing from {msg}");
        assert!(msg.contains("past the file's size 0"), "{msg}");
    }

    /// The range gate runs *after* the blacklist/scope/dedup filters, so an
    /// entry the run would not touch cannot block the rest of the plan — even
    /// when its range is corrupt.
    #[test]
    fn a_corrupt_entry_the_plan_excludes_does_not_block_the_rest() {
        let blacklisted = manifest_file("keep/blacklisted.dat", 64, vec![chunk("b", -1, 32)]);
        let out_of_scope = manifest_file("keep/out.dat", 64, vec![chunk("o", -1, 32)]);
        // The good spelling comes first, so the corrupt duplicate is dropped by
        // the dedup key before it can be validated.
        let good_dup = manifest_file("keep/dup.dat", 32, vec![chunk("d", 0, 32)]);
        let corrupt_dup = manifest_file("keep/dup.dat", 64, vec![chunk("d2", -1, 32)]);
        let good = manifest_file("keep/good.dat", 32, vec![chunk("g", 0, 32)]);
        let per = vec![
            (
                "http://x".to_string(),
                vec![blacklisted, out_of_scope, good_dup, good],
            ),
            ("http://y".to_string(), vec![corrupt_dup]),
        ];
        let blacklist: HashSet<String> = ["keep/blacklisted.dat".to_string()].into_iter().collect();
        let exclude = vec!["keep/out.dat".to_string()];
        let scope = PathScope::new(&[], &exclude).unwrap();
        let plan = build_plan(per, &blacklist, "5.0", &scope).unwrap();
        let rels: Vec<&str> = plan.files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(rels, vec!["keep/dup.dat", "keep/good.dat"]);
    }
}
