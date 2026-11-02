//! The plan-phase scan: what is actually wrong with the install.
//!
//! `--scan` reads every planned file **once**, in the plan phase, and
//! answers the questions the plan needs exact answers to:
//!
//! - which chunks are wrong where they belong — so the download estimate can
//!   be the figure the repair will really spend, instead of an upper bound;
//! - which reuse sources are intact — so a chunk that can be copied is not
//!   counted as a download;
//! - how many bytes the scan read — so `--print-plan` can report them.
//!
//! It is **opt-in**, and it has to be: the scan is a full read of the
//! install, and an unflagged run must not pay for it. The cost of the flag
//! is the one thing it buys — a run that refuses a ceiling now reads the
//! install before it refuses, because the estimate the ceiling is checked
//! against is only exact after the scan.
//!
//! Under `--scan`, the plan's verdict is also available to the in-place
//! writer, guarded by a length and modification-time check that falls back to
//! a fresh read when the file changed since it was scanned. The plan scan
//! feeds the in-place writer only: the temp path's gate is a single digest
//! over the finished temp file, not a verdict about the target, so it keeps
//! its own read.

use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use futures_util::StreamExt;

use super::file::Mode;
use crate::config::RepairMode;
use crate::repair::manifest::LocalChunkMap;
use crate::repair::phases::plan::{FileId, PlannedFile, RepairPlan};
use crate::repair::reuse;
use crate::sophon::SophonChunk;
use crate::util;

/// What one file's scan found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// The whole-file MD5 of what was on disk.
    pub file_md5: String,
    /// Per chunk, in the plan's order: is it correct where it belongs?
    pub dest_ok: Vec<bool>,
    /// Per chunk, in the plan's order: is its reuse source intact?
    pub source_ok: Vec<bool>,
    /// The file length the scan read. The in-place writer compares this
    /// against its open handle before reusing the verdict.
    pub file_len: u64,
    /// The file's modification time when the scan was taken, from one
    /// `metadata` call. `None` when that stat failed, which leaves the
    /// verdict unusable for reuse.
    pub modified: Option<SystemTime>,
}

impl Verdict {
    /// The chunks that are wrong where they belong.
    pub fn damaged_chunks(&self) -> u64 {
        self.dest_ok.iter().filter(|ok| !**ok).count() as u64
    }
}

/// The scan of the whole plan.
#[derive(Debug, Clone, Default)]
pub struct ScanReport {
    /// Per planned file, in `plan.files` order. `None` is a file the scan did
    /// not reach: a task that panicked or was cancelled, or a plan this report
    /// was not built from.
    pub files: Vec<Option<Verdict>>,
    /// The bytes the scan read.
    pub bytes: u64,
    /// The chunks that are wrong where they belong, over the whole plan.
    pub damaged_chunks: u64,
}

impl ScanReport {
    /// The verdict for one planned file, when the scan produced one.
    pub fn get(&self, id: FileId) -> Option<&Verdict> {
        self.files.get(id.index()).and_then(Option::as_ref)
    }

    /// The bytes the repair will really download, per file, in `plan.files`
    /// order.
    ///
    /// A chunk is downloaded when it is **wrong where it belongs** and has
    /// **no source the chosen write order keeps alive** to copy from.
    /// "Alive" is the writer's own rule ([`reuse::salvage_order`]), not "the
    /// source bytes verified" alone: the ordered tier can only reuse one
    /// direction's worth, and a source the direction overwrites is downloaded
    /// even though the scan found it intact. The estimate therefore takes the
    /// ordered tier's (larger) figure, which is also the staged tier's
    /// over-approximation — the RAM budget that picks between them is a
    /// runtime decision this phase cannot make.
    ///
    /// The write path is part of the estimate, not assumed:
    ///
    /// - **temp** (requested, or the target is not a regular file) writes every
    ///   chunk of the file, so it downloads every chunk whose reuse source is
    ///   not intact — including chunks already correct where they belong.
    /// - **discard** deletes the target first, so every chunk is downloaded.
    /// - **in place** leaves the correct chunks alone and reuses only what the
    ///   chosen direction keeps.
    ///
    /// The one thing this cannot see is an in-place open that fails at runtime
    /// and falls back to temp (`file/mod.rs`); that file then downloads its
    /// correct chunks too. It is bounded by the runtime `--max-download-bytes`
    /// hard stop, which is what the preflight estimate sits beside.
    ///
    /// The `PLAN` total and the `--plan-out` document are fed from this one
    /// vector, so the document cannot disagree with the line.
    ///
    /// `root` is the run's cached install root. `None` means the base itself
    /// could not be resolved, and each file falls back to [`util::contained_join`].
    pub fn est_download_bytes_by_file(
        &self,
        plan: &RepairPlan,
        game_dir: &Path,
        root: Option<&util::PathRoot>,
        requested: RepairMode,
        local_map: &LocalChunkMap,
    ) -> Vec<u64> {
        plan.iter()
            .map(|(id, f)| {
                let sources = reuse::sources_for(&f.chunks, local_map.get(&f.rel));
                // The run's cached root when it has one; otherwise resolve the
                // base here, exactly as `contained_join` always did.
                let joined = root.map_or_else(
                    || util::contained_join(game_dir, &f.rel),
                    |root| root.join(&f.rel),
                );
                let resolved = match joined {
                    Ok(path) => super::file::resolve_mode(requested, &path),
                    // Defence in depth, matching the scan itself: a path that
                    // escapes is not readable, so nothing is reusable.
                    Err(_) => return all_chunks_download_bytes(&f.chunks),
                };
                est_file_download_bytes(&f.chunks, self.get(id), &sources, requested, resolved)
            })
            .collect()
    }
}

/// `Σ compressed_size` over every chunk: the temp-after-discard and
/// unreachable-file figure. Saturated, because the sizes are server data.
fn all_chunks_download_bytes(chunks: &[SophonChunk]) -> u64 {
    chunks.iter().fold(0u64, |n, c| {
        n.saturating_add(c.compressed_size.max(0) as u64)
    })
}

/// One file's download figure, for the write path it will really take.
///
/// See [`ScanReport::est_download_bytes_by_file`] for the rules. `sources[i]`
/// is where chunk `i`'s bytes live, from `reuse::sources_for`; it is only
/// consulted when the scan found those bytes intact.
fn est_file_download_bytes(
    chunks: &[SophonChunk],
    verdict: Option<&Verdict>,
    sources: &[Option<u64>],
    requested: RepairMode,
    resolved: Mode,
) -> u64 {
    // `discard` deletes the target before writing, so no source survives and
    // every chunk is fetched.
    if requested == RepairMode::Discard {
        return all_chunks_download_bytes(chunks);
    }
    // A file the scan did not reach: the whole thing is unknown, so the whole
    // thing is estimated.
    let Some(v) = verdict else {
        return all_chunks_download_bytes(chunks);
    };
    match resolved {
        // The temp path writes every chunk; only the reuse sources save a
        // download, and `source_ok` is exactly the scan's verdict on them.
        Mode::Temp => chunks
            .iter()
            .zip(v.source_ok.iter())
            .filter(|(_, ok)| !**ok)
            .fold(0u64, |n, (c, _)| {
                n.saturating_add(c.compressed_size.max(0) as u64)
            }),
        Mode::Inplace => {
            // One pass builds the writer's two inputs: the damaged chunks, in
            // plan order, and their sources. The writer's rule is the same
            // one, so the borrowed form avoids cloning the chunks.
            let mut bad: Vec<&SophonChunk> = Vec::new();
            let mut srcs: Vec<Option<u64>> = Vec::new();
            for (i, c) in chunks.iter().enumerate() {
                if v.dest_ok[i] {
                    continue;
                }
                bad.push(c);
                // `usable_len = u64::MAX` is sound because `source_ok`
                // already proved the source range was read from inside the
                // file.
                srcs.push(if v.source_ok[i] {
                    sources.get(i).copied().flatten()
                } else {
                    None
                });
            }
            let order = reuse::salvage_order_refs(&bad, &srcs, u64::MAX);
            bad.iter()
                .enumerate()
                .filter(|(k, _)| !order.live[*k])
                .fold(0u64, |n, (_, c)| {
                    n.saturating_add(c.compressed_size.max(0) as u64)
                })
        }
    }
}

/// One file's scan, before it is folded into the report.
struct Scanned {
    verdict: Verdict,
    bytes: u64,
}

/// Scan every planned file, in parallel.
///
/// The scan is blocking I/O, so each file is scanned on the blocking pool
/// and the results are collected in order of completion. `jobs` bounds the
/// concurrency, exactly as it bounds the repair loop.
pub async fn scan_plan(
    game_dir: &Path,
    root: Option<&util::PathRoot>,
    plan: Arc<RepairPlan>,
    local_map: Arc<LocalChunkMap>,
    jobs: usize,
) -> ScanReport {
    let timer = crate::phase::PhaseTimer::start("scan");
    // NOTE: the plan is shared by `Arc` and each task is handed an index, not a
    // file. A `PlannedFile` owns its rel, MD5 and every chunk, so cloning one
    // per task would deep-clone the whole plan — the same reason
    // `file::repair_all` shares its `FileCtx` instead of the plan's files.
    //
    // The install root is resolved once here, not once per file, and shared by
    // the blocking tasks. `None` keeps the per-file `contained_join` fallback.
    let root: Option<Arc<util::PathRoot>> = root.cloned().map(Arc::new);
    let total_files = plan.files.len();
    let mut stream = futures_util::stream::iter(0..total_files)
        .map(move |idx| {
            let plan = Arc::clone(&plan);
            let game_dir = game_dir.to_path_buf();
            let local_map = Arc::clone(&local_map);
            let root = root.clone();
            async move {
                let result = tokio::task::spawn_blocking(move || {
                    scan_one(&game_dir, root.as_deref(), &plan.files[idx], &local_map)
                })
                .await;
                (idx, result)
            }
        })
        .buffer_unordered(jobs.max(1));

    let mut out = ScanReport::default();
    out.files.resize(total_files, None);
    while let Some((idx, res)) = stream.next().await {
        let scanned = match res {
            Ok(scanned) => scanned,
            Err(e) => {
                // `scan_one` catches unreadable files itself and answers
                // "nothing is correct", so the only error here is a panic or a
                // cancellation. Dropping it silently would undercount the scan
                // with no trace of why; the estimate stays safe (an unreached
                // file is estimated in full, see `est_download_bytes`).
                tracing::warn!(
                    error = %format!("{e:#}"),
                    "scan task failed; unreached files are estimated in full"
                );
                continue;
            }
        };
        out.bytes += scanned.bytes;
        out.damaged_chunks += scanned.verdict.damaged_chunks();
        out.files[idx] = Some(scanned.verdict);
    }
    tracing::info!(
        phase = % "scan",
        duration_ms = timer.elapsed_ms(),
        files = out.files.iter().filter(|verdict| verdict.is_some()).count(),
        bytes = out.bytes,
        damaged_chunks = out.damaged_chunks,
        "scan"
    );
    timer.disarm();
    out
}

/// The scan's two range lists for one file, plus the chunk each source range
/// belongs to.
///
/// `dest` has one range per chunk, in plan order. `source` has a range only
/// for the chunks that **have** a source: coercing the absent ones to offset
/// 0 would ask the scan to verify the chunk's bytes at the head of the file,
/// so unrelated bytes that happen to match (the file reflowed, or the head
/// holds the content under a different layout) would mark a non-existent
/// source intact while the writer, which gates on `sources[i].is_some()`,
/// downloads the chunk anyway. `source_owner[k]` is the chunk `source[k]`
/// belongs to, so the scan's per-source verdicts can be mapped back to one
/// verdict per chunk.
pub(crate) fn scan_ranges<'a>(
    chunks: &'a [SophonChunk],
    sources: &[Option<u64>],
) -> (
    Vec<util::ScanRange<'a>>,
    Vec<util::ScanRange<'a>>,
    Vec<usize>,
) {
    let dest_ranges = chunks
        .iter()
        .map(|c| {
            (
                c.offset.max(0) as u64,
                c.uncompressed_size.max(0) as u64,
                c.uncompressed_md5.as_str(),
            )
        })
        .collect();
    let mut source_ranges: Vec<util::ScanRange<'_>> = Vec::new();
    let mut source_owner: Vec<usize> = Vec::new();
    for (i, (c, src)) in chunks.iter().zip(sources.iter()).enumerate() {
        let Some(offset) = src else { continue };
        source_owner.push(i);
        source_ranges.push((
            *offset,
            c.uncompressed_size.max(0) as u64,
            c.uncompressed_md5.as_str(),
        ));
    }
    (dest_ranges, source_ranges, source_owner)
}

/// Map the scan's per-source-range verdicts back onto one verdict per chunk.
///
/// `scan_source_ok` is indexed by the range list [`scan_ranges`] built, which
/// has one entry per chunk **with a source**; `source_owner[k]` is the chunk
/// that range `k` belongs to. A chunk with no source has no intact source, by
/// definition, so it stays `false`. A source verdict that is missing keeps
/// that same conservative answer rather than panicking.
///
/// The discard path replaces a real scan with [`util::Scan::absent`], whose
/// tuple is `(dest ranges, source ranges)`; this remap is what keeps that
/// per-range shape from being read as per-chunk. Both the scan's estimate and
/// the in-place writer use this one definition.
pub(crate) fn source_ok_by_chunk(
    chunks_len: usize,
    source_owner: &[usize],
    scan_source_ok: &[bool],
) -> Vec<bool> {
    let mut source_ok = vec![false; chunks_len];
    for (k, &i) in source_owner.iter().enumerate() {
        source_ok[i] = scan_source_ok.get(k).copied().unwrap_or(false);
    }
    source_ok
}

/// Scan one file: one read, every answer.
fn scan_one(
    game_dir: &Path,
    root: Option<&util::PathRoot>,
    file: &PlannedFile,
    local_map: &LocalChunkMap,
) -> Scanned {
    let joined = root.map_or_else(
        || util::contained_join(game_dir, &file.rel),
        |root| root.join(&file.rel),
    );
    let path = match joined {
        Ok(p) => p,
        Err(e) => {
            // `build_plan` already refused an escaping rel, so this is defence
            // in depth. Treat it like an unreadable file: nothing is correct.
            tracing::warn!(
                file = %file.rel,
                error = %format!("{e:#}"),
                "scan: path escapes the game dir; nothing is correct"
            );
            let n = file.chunks.len();
            return Scanned {
                bytes: 0,
                verdict: Verdict {
                    file_md5: util::EMPTY_MD5.to_string(),
                    dest_ok: vec![false; n],
                    source_ok: vec![false; n],
                    file_len: 0,
                    modified: None,
                },
            };
        }
    };
    // The guard the writer can use later: the modification time from one stat
    // taken before the read, plus the length the read itself observes. A
    // failed stat leaves the time absent, so the verdict is never reusable.
    let modified = std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok());
    let sources = reuse::sources_for(&file.chunks, local_map.get(&file.rel));
    let (dest_ranges, source_ranges, source_owner) = scan_ranges(&file.chunks, &sources);
    let scan = util::scan_file(&path, &dest_ranges, &source_ranges).unwrap_or_else(|e| {
        // An unreadable file is not a failed scan: the verdict is "nothing
        // is correct", which is what the repair would find too.
        tracing::debug!(file = %file.rel, error = %format!("{e:#}"), "scan: unreadable");
        util::Scan::absent(dest_ranges.len(), source_ranges.len())
    });
    // Back to one verdict per chunk, in the plan's order: a chunk with no
    // source has no intact source, by definition.
    let source_ok = source_ok_by_chunk(file.chunks.len(), &source_owner, &scan.source_ok);
    Scanned {
        bytes: scan.file_len,
        verdict: Verdict {
            file_md5: scan.file_md5,
            dest_ok: scan.dest_ok,
            source_ok,
            file_len: scan.file_len,
            modified,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sophon::SophonChunk;

    fn chunk(id: &str, md5: &str, size: i64) -> SophonChunk {
        chunk_at(id, md5, 0, size)
    }

    fn chunk_at(id: &str, md5: &str, offset: i64, size: i64) -> SophonChunk {
        SophonChunk {
            id: id.into(),
            uncompressed_md5: md5.into(),
            offset,
            compressed_size: size,
            uncompressed_size: size,
            unknown: 0,
            compressed_md5: String::new(),
        }
    }

    /// The estimate is the figure the repair will really spend: a chunk that
    /// is wrong and whose source the write order does not keep.
    #[test]
    fn the_exact_estimate_counts_the_chunks_that_must_be_downloaded() {
        let chunks = vec![chunk_at("c0", "m0", 0, 10), chunk_at("c1", "m1", 10, 20)];
        // c0 is wrong but has an intact source the ascending order keeps; c1
        // is wrong and has none.
        let verdict = Verdict {
            file_md5: "x".into(),
            dest_ok: vec![false, false],
            source_ok: vec![true, false],
            file_len: 0,
            modified: None,
        };
        let sources = vec![Some(20u64), None];
        assert_eq!(
            est_file_download_bytes(
                &chunks,
                Some(&verdict),
                &sources,
                RepairMode::Auto,
                Mode::Inplace
            ),
            20,
            "only c1 is fetched"
        );
        // A file the scan never reached is estimated in full.
        assert_eq!(
            est_file_download_bytes(&chunks, None, &sources, RepairMode::Auto, Mode::Inplace),
            30
        );
    }

    /// N-17: the scan's `source_ok` alone is not the writer's decision. Two
    /// damaged chunks whose intact sources point at each other (a swap): the
    /// ordered tier keeps one direction's worth, so one chunk is downloaded
    /// even though the scan found both sources intact.
    #[test]
    fn the_ordered_tier_can_download_a_chunk_whose_source_the_scan_found_intact() {
        let chunks = vec![chunk_at("c0", "m0", 0, 10), chunk_at("c1", "m1", 10, 20)];
        let verdict = Verdict {
            file_md5: "x".into(),
            dest_ok: vec![false, false],
            source_ok: vec![true, true],
            file_len: 0,
            modified: None,
        };
        // Each chunk's bytes live where the other lands.
        let sources = vec![Some(20u64), Some(0)];
        // In place (ordered): one direction survives, one chunk downloads.
        assert_eq!(
            est_file_download_bytes(
                &chunks,
                Some(&verdict),
                &sources,
                RepairMode::Auto,
                Mode::Inplace
            ),
            20,
            "the direction that loses a source must be estimated as a download"
        );
        // The temp path can still reuse both: it copies each chunk before the
        // target is replaced, so no order issue exists there.
        assert_eq!(
            est_file_download_bytes(
                &chunks,
                Some(&verdict),
                &sources,
                RepairMode::Auto,
                Mode::Temp
            ),
            0
        );
    }

    /// The temp path writes every chunk, so a chunk that is already correct
    /// where it belongs still costs a download when its source is not intact.
    #[test]
    fn the_temp_estimate_counts_the_correct_chunks_the_in_place_path_keeps() {
        let chunks = vec![chunk_at("c0", "m0", 0, 10), chunk_at("c1", "m1", 10, 20)];
        let verdict = Verdict {
            file_md5: "x".into(),
            dest_ok: vec![true, false],
            source_ok: vec![false, false],
            file_len: 0,
            modified: None,
        };
        let sources = vec![None, None];
        assert_eq!(
            est_file_download_bytes(
                &chunks,
                Some(&verdict),
                &sources,
                RepairMode::Auto,
                Mode::Inplace
            ),
            20,
            "c0 is correct where it belongs; the in-place path keeps it"
        );
        assert_eq!(
            est_file_download_bytes(
                &chunks,
                Some(&verdict),
                &sources,
                RepairMode::Auto,
                Mode::Temp
            ),
            30,
            "the temp path rewrites c0 and has no source for it"
        );
        assert_eq!(
            est_file_download_bytes(
                &chunks,
                Some(&verdict),
                &sources,
                RepairMode::Discard,
                Mode::Inplace
            ),
            30,
            "discard deletes the target, so every chunk downloads"
        );
    }

    #[test]
    fn an_overflowing_estimate_saturates_instead_of_panicking() {
        // Three chunks at `i64::MAX` do not fit in a `u64` (two do). The
        // manifest is server data, so the estimate must not panic the plan
        // phase in a debug build; saturating keeps it an over-approximation.
        let huge = i64::MAX;
        let chunks = vec![
            chunk("c0", "m", huge),
            chunk("c1", "m", huge),
            chunk("c2", "m", huge),
        ];
        assert_eq!(
            est_file_download_bytes(&chunks, None, &[], RepairMode::Auto, Mode::Inplace),
            u64::MAX
        );

        let all_wrong = Verdict {
            file_md5: "x".into(),
            dest_ok: vec![false; 3],
            source_ok: vec![false; 3],
            file_len: 0,
            modified: None,
        };
        assert_eq!(
            est_file_download_bytes(
                &chunks,
                Some(&all_wrong),
                &[None, None, None],
                RepairMode::Auto,
                Mode::Inplace
            ),
            u64::MAX
        );
    }

    #[test]
    fn a_chunk_with_no_source_is_never_scanned_at_offset_zero() {
        // The chunk's bytes sit at offset 10, but the same content also sits
        // at the head of the file under a different layout, and the local map
        // has no entry for it. Scanning "no source" as offset 0 would call the
        // non-existent source intact and hide the download from the estimate.
        let x = vec![0x58u8; 10];
        let y = vec![0x59u8; 10];
        let md5_x = util::md5::md5_digest(&x).to_hex();
        let mut bytes = x.clone();
        bytes.extend_from_slice(&y);
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "no_source");
        std::fs::write(dir.join("a.dat"), &bytes).unwrap();
        let file = PlannedFile {
            rel: "a.dat".into(),
            size: 20,
            md5: util::md5::md5_digest(&bytes).to_hex(),
            url_prefix: Arc::from(""),
            chunks: vec![chunk_at("c0", &md5_x, 10, 10)],
        };

        // The seam is real: the bytes at offset 0 do match the chunk.
        let buggy =
            util::scan_file(&dir.join("a.dat"), &[(10, 10, &md5_x)], &[(0, 10, &md5_x)]).unwrap();
        assert!(!buggy.dest_ok[0], "the destination is the other copy");
        assert!(
            buggy.source_ok[0],
            "the head copy really would verify at offset 0"
        );

        let scanned = scan_one(&dir, None, &file, &LocalChunkMap::new());
        assert_eq!(scanned.verdict.dest_ok, vec![false]);
        assert_eq!(
            scanned.verdict.source_ok,
            vec![false],
            "a chunk with no source has no intact source"
        );
        assert_eq!(
            est_file_download_bytes(
                &file.chunks,
                Some(&scanned.verdict),
                &[None],
                RepairMode::Auto,
                Mode::Inplace
            ),
            10,
            "the estimate must count the chunk it cannot copy"
        );
    }

    /// One remap pins the ownership rule: only chunks with a source have an
    /// entry in `scan_source_ok`, and a chunk without one stays `false`. A
    /// verdict list that is shorter than the owner list is a missing answer,
    /// not a panic.
    #[test]
    fn the_source_verdicts_land_on_the_chunk_that_owns_the_range() {
        assert_eq!(
            source_ok_by_chunk(3, &[0, 2], &[true, true]),
            vec![true, false, true],
            "chunk 1 has no source range, so it has no intact source"
        );
        assert_eq!(
            source_ok_by_chunk(3, &[0, 2], &[true]),
            vec![true, false, false],
            "a missing per-range verdict is false, not a panic"
        );
    }

    /// A no-source chunk stays `false` even when the scan's source list carries
    /// an entry the owner list does not claim.
    #[test]
    fn a_stray_source_verdict_never_marks_a_no_source_chunk() {
        assert_eq!(
            source_ok_by_chunk(3, &[2], &[true, true]),
            vec![false, false, true],
            "the extra answer belongs to no chunk, and chunk 1 has no source"
        );
    }

    /// The scan's estimate and the in-place writer read the same remapped
    /// per-chunk verdict: the chunk without a source is the one both name as a
    /// download.
    #[test]
    fn the_scan_and_inplace_estimates_agree_after_the_remap() {
        let chunks = vec![
            chunk_at("c0", "m0", 0, 10),
            chunk_at("c1", "m1", 10, 20),
            chunk_at("c2", "m2", 30, 30),
        ];
        let sources = vec![Some(100u64), None, Some(300)];
        let (_, _, owner) = scan_ranges(&chunks, &sources);
        assert_eq!(owner, vec![0, 2]);
        let source_ok = source_ok_by_chunk(chunks.len(), &owner, &[true, true]);
        let verdict = Verdict {
            file_md5: "x".into(),
            dest_ok: vec![false; 3],
            source_ok,
            file_len: 0,
            modified: None,
        };
        assert_eq!(
            est_file_download_bytes(
                &chunks,
                Some(&verdict),
                &sources,
                RepairMode::Auto,
                Mode::Inplace
            ),
            20,
            "only the source-less chunk must be fetched"
        );

        // The writer builds its patch list from the same verdict; the chunks
        // it downloads are the same ones the estimate counted.
        let bad: Vec<usize> = (0..chunks.len()).filter(|&i| !verdict.dest_ok[i]).collect();
        let srcs: Vec<Option<u64>> = bad
            .iter()
            .map(|&i| {
                if verdict.source_ok[i] {
                    sources[i]
                } else {
                    None
                }
            })
            .collect();
        let kept: Vec<&SophonChunk> = bad.iter().map(|&i| &chunks[i]).collect();
        let order = reuse::salvage_order_refs(&kept, &srcs, u64::MAX);
        let downloaded: u64 = kept
            .iter()
            .enumerate()
            .filter(|(k, _)| !order.live[*k])
            .fold(0u64, |n, (_, c)| {
                n.saturating_add(c.compressed_size.max(0) as u64)
            });
        assert_eq!(
            downloaded, 20,
            "the writer's patch list and the estimate must agree"
        );
    }

    #[test]
    fn the_damaged_chunk_count_is_the_wrong_chunks() {
        let v = Verdict {
            file_md5: "x".into(),
            dest_ok: vec![true, false, false, true],
            source_ok: vec![true; 4],
            file_len: 0,
            modified: None,
        };
        assert_eq!(v.damaged_chunks(), 2);
    }

    #[test]
    fn the_report_returns_verdicts_by_file_id() {
        let report = ScanReport {
            files: vec![
                None,
                Some(Verdict {
                    file_md5: "present".into(),
                    dest_ok: vec![],
                    source_ok: vec![],
                    file_len: 0,
                    modified: None,
                }),
            ],
            bytes: 0,
            damaged_chunks: 0,
        };

        assert!(report.get(FileId::new(0)).is_none());
        assert_eq!(report.get(FileId::new(1)).unwrap().file_md5, "present");
        assert!(
            report.get(FileId::new(2)).is_none(),
            "an id outside the report is not a verdict"
        );
    }
}
