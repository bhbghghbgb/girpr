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

use girpr_chunk_cost::projected_download_bytes;
use girpr_chunk_verify::{Verdict, scan_ranges, source_ok_by_chunk};
use girpr_repair_policy::{Mode, RepairMode, resolve_mode};

use crate::repair::reuse;
use crate::sophon::SophonChunk;
use crate::util;
use crate::util::Md5;
use girpr_repair_plan::{FileId, LocalChunkMap, PlannedFile, RepairPlan};

/// The scan of the whole plan.
#[derive(Debug, Clone, Default)]
pub struct ScanReport {
    /// Per planned file, in `plan.files` order. `None` is a file the scan did
    /// not reach: a task that panicked or was cancelled, or a plan this report
    /// was not built from.
    files: Vec<Option<Verdict>>,
    /// The bytes the scan read.
    pub bytes: u64,
    /// The chunks that are wrong where they belong, over the whole plan.
    pub damaged_chunks: u64,
    /// Files this scan could not answer for, as `(plan index, reason)` pairs:
    /// a task that panicked or was cancelled, or a file the scan could not
    /// read or resolve. The estimate for such a file stays conservative — an
    /// unreached file is estimated in full and an unreadable one as all chunks
    /// wrong — so a failure is never turned into a silent "intact" verdict.
    pub failures: Vec<(usize, String)>,
    /// When the scan started. The in-place writer reports how old the plan
    /// scan is when it reuses a verdict; `None` for a report built by hand.
    pub scanned_at: Option<SystemTime>,
    /// The [`RepairPlan::fingerprint`] of the plan this scan read.
    ///
    /// A scan's `files` are indexed by plan position, so this is the identity
    /// that makes those positions meaningful. `None` is a report built by
    /// hand; it answers for no plan rather than accidentally matching one.
    ///
    /// This is an in-memory identity only: it comes from
    /// [`RepairPlan::fingerprint`], which uses the standard library's
    /// `DefaultHasher` and is not stable across processes or Rust releases.
    /// A `ScanReport` is never serialized, persisted, or sent to another
    /// process. If it ever has to cross such a boundary, the fingerprint must
    /// become a stable fold first; comparing this value elsewhere is unsafe.
    plan_fingerprint: Option<u64>,
}

impl ScanReport {
    /// Whether this report was taken from `plan`.
    ///
    /// The comparison is exactly `self.plan_fingerprint == Some(plan.fingerprint())`:
    /// the same ordered file list, file metadata and chunk layout. A report
    /// from another plan is not an answer, even at an in-range index.
    pub(crate) fn matches_plan(&self, plan: &RepairPlan) -> bool {
        self.plan_fingerprint == Some(plan.fingerprint())
    }

    /// How many files the scan could not answer for.
    ///
    /// Each one is either estimated in full (no verdict) or recorded as all
    /// chunks wrong; the count is surfaced beside the scan's phase figures so
    /// a partial scan is visible instead of silently reading as a clean one.
    pub fn scan_failed(&self) -> usize {
        self.failures.len()
    }

    /// A report for `files`, bound to `plan` — the test-only constructor for
    /// the private index fields.
    #[cfg(test)]
    pub(crate) fn for_test(plan: &RepairPlan, files: Vec<Option<Verdict>>) -> Self {
        let damaged_chunks = files.iter().flatten().map(Verdict::damaged_chunks).sum();
        Self {
            files,
            damaged_chunks,
            plan_fingerprint: Some(plan.fingerprint()),
            ..Default::default()
        }
    }

    /// [`Self::for_test`] with a plan-scan start time, so a test can exercise
    /// the reuse guard's age rule instead of always reading the file again.
    #[cfg(test)]
    pub(crate) fn for_test_at(
        plan: &RepairPlan,
        files: Vec<Option<Verdict>>,
        scanned_at: SystemTime,
    ) -> Self {
        Self {
            scanned_at: Some(scanned_at),
            ..Self::for_test(plan, files)
        }
    }

    /// The verdict for one planned file, when this report is from `plan` and
    /// the scan produced one.
    ///
    /// A report from a different plan returns `None` for every id: the caller
    /// then re-reads the file or estimates it in full, which is safe. The scan
    /// is never silently applied to a plan it does not describe.
    pub fn get(&self, plan: &RepairPlan, id: FileId) -> Option<&Verdict> {
        if !self.matches_plan(plan) {
            return None;
        }
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
                    |root| root.join_validated(&f.rel),
                );
                let resolved = match joined {
                    Ok(path) => resolve_mode(requested, &path),
                    // Defence in depth, matching the scan itself: a path that
                    // escapes is not readable, so nothing is reusable.
                    Err(_) => return all_chunks_download_bytes(&f.chunks),
                };
                est_file_download_bytes(
                    &f.chunks,
                    self.get(plan, id),
                    &sources,
                    requested,
                    resolved,
                )
            })
            .collect()
    }
}

/// `Σ compressed_size` over every chunk: the temp-after-discard and
/// unreachable-file figure. Every chunk is marked as a download; the shared
/// fold clamps and saturates the server-controlled sizes.
fn all_chunks_download_bytes(chunks: &[SophonChunk]) -> u64 {
    projected_download_bytes(chunks.iter().map(|c| (c.compressed_size, true)))
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
        Mode::Temp => projected_download_bytes(
            chunks
                .iter()
                .zip(v.source_ok.iter())
                .map(|(c, ok)| (c.compressed_size, !*ok)),
        ),
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
            projected_download_bytes(
                bad.iter()
                    .enumerate()
                    .map(|(k, c)| (c.compressed_size, !order.live[k])),
            )
        }
    }
}

/// One file's scan, before it is folded into the report.
struct Scanned {
    verdict: Verdict,
    bytes: u64,
    /// A complete, operator-facing reason when the scan could not answer for
    /// this file (bad path, unreadable file). `None` for a normal scan, even
    /// one that found the file damaged: damage is an answer, not a failure.
    failure: Option<String>,
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
    let scanned_at = Some(SystemTime::now());
    // NOTE: the plan is shared by `Arc` and each task is handed an index, not a
    // file. A `PlannedFile` owns its rel, MD5 and every chunk, so cloning one
    // per task would deep-clone the whole plan — the same reason
    // `file::repair_all` shares its `FileCtx` instead of the plan's files.
    //
    // The install root is resolved once here, not once per file, and shared by
    // the blocking tasks. `None` keeps the per-file `contained_join` fallback.
    let root: Option<Arc<util::PathRoot>> = root.cloned().map(Arc::new);
    let total_files = plan.len();
    let plan_fingerprint = plan.fingerprint();
    // The bounded window and the lost-task payload are the shared leaf's job;
    // this phase keeps its own policy: scan failures are collected on the
    // report, never turned into a damage verdict.
    let mut stream = girpr_async_join::join_iter(
        0..total_files,
        jobs,
        girpr_async_join::Wording::new("scan", girpr_async_join::DEFAULT_FALLBACK),
        move |idx| {
            let plan = Arc::clone(&plan);
            let game_dir = game_dir.to_path_buf();
            let local_map = Arc::clone(&local_map);
            let root = root.clone();
            async move {
                // Raw `spawn_blocking` on purpose: `scan_one`'s per-file lines
                // are best-effort diagnostics, and no test asserts them through
                // a scoped subscriber. A production run has the global
                // dispatcher anyway. A per-file log assertion would have to use
                // `spawn_blocking_traced` to carry the caller's dispatcher onto
                // this pool thread.
                let result = tokio::task::spawn_blocking(move || {
                    scan_one(&game_dir, root.as_deref(), &plan.files()[idx], &local_map)
                })
                .await;
                (idx, result)
            }
        },
    );

    let mut out = ScanReport {
        scanned_at,
        plan_fingerprint: Some(plan_fingerprint),
        ..Default::default()
    };
    out.files.resize(total_files, None);
    while let Some((idx, res)) = stream.next().await {
        let scanned = match res {
            Ok(scanned) => scanned,
            Err(girpr_async_join::JoinOutcome::Failed(e)) => {
                // `scan_one` catches unreadable files itself and answers
                // "nothing is correct", so the only error here is a panic or a
                // cancellation. The estimate stays safe (an unreached file is
                // estimated in full, see `est_download_bytes`), but the reason
                // must be collected and surfaced: a partial scan is not a
                // clean one.
                out.failures.push((idx, format!("scan task failed: {e:#}")));
                continue;
            }
            Err(girpr_async_join::JoinOutcome::Lost { why }) => {
                // A task that never reported still owes the report a reason.
                // The leaf recovered the panic payload when there was one.
                let why = why.unwrap_or_else(|| "the scan task did not report".to_string());
                out.failures.push((idx, format!("scan task failed: {why}")));
                continue;
            }
        };
        if let Some(reason) = scanned.failure {
            out.failures.push((idx, reason));
        }
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
        scan_failed = out.scan_failed(),
        "scan"
    );
    // The count above is visible in the phase line. The reasons stay on the
    // report and are also emitted here, at the default level, so a file the
    // scan could not answer for is never silent. The conservative estimate is
    // unchanged: a failure never becomes an "intact" verdict.
    for (idx, reason) in &out.failures {
        tracing::warn!(
            file_index = idx,
            reason = %reason,
            "scan failed for a file; estimated conservatively"
        );
    }
    timer.disarm();
    out
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
        |root| root.join_validated(&file.rel),
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
                    file_md5: Md5::EMPTY,
                    dest_ok: vec![false; n],
                    source_ok: vec![false; n],
                    file_len: 0,
                    modified: None,
                },
                failure: Some(format!("{}: path escapes the game dir: {e:#}", file.rel)),
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
    let (scan, failure) = match util::scan_file(&path, &dest_ranges, &source_ranges) {
        Ok(scan) => (scan, None),
        Err(e) => {
            // An unreadable file still gets the conservative "nothing is
            // correct" verdict — that is what the repair would find too — but
            // the scan did not answer for it, so keep the reason for the
            // report and the phase line instead of leaving it at debug level.
            // `%format!("{e:#}")` would build the whole error chain even when
            // debug is off; render it only for a listening subscriber.
            if tracing::enabled!(tracing::Level::DEBUG) {
                tracing::debug!(file = %file.rel, error = %format!("{e:#}"), "scan: unreadable");
            }
            (
                util::Scan::absent(dest_ranges.len(), source_ranges.len()),
                Some(format!("{}: unreadable: {e:#}", file.rel)),
            )
        }
    };
    // Back to one verdict per chunk, in the plan's order: a chunk with no
    // source has no intact source, by definition. The scan answers one verdict
    // per source range, so the two lists are the same length here.
    debug_assert_eq!(
        source_owner.len(),
        scan.source_ok.len(),
        "the scan answers one source verdict per source range"
    );
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
        failure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sophon::SophonChunk;
    use girpr_chunk_cost::saturating_nonneg_sum;

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
            unknown_field6: 0,
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
            file_md5: Md5::EMPTY,
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
            file_md5: Md5::EMPTY,
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
            file_md5: Md5::EMPTY,
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
            file_md5: Md5::EMPTY,
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
            file_md5: Md5::EMPTY,
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
        let downloaded: u64 = saturating_nonneg_sum(
            kept.iter()
                .enumerate()
                .filter(|(k, _)| !order.live[*k])
                .map(|(_, c)| c.compressed_size),
        );
        assert_eq!(
            downloaded, 20,
            "the writer's patch list and the estimate must agree"
        );
    }

    #[test]
    fn the_damaged_chunk_count_is_the_wrong_chunks() {
        let v = Verdict {
            file_md5: Md5::EMPTY,
            dest_ok: vec![true, false, false, true],
            source_ok: vec![true; 4],
            file_len: 0,
            modified: None,
        };
        assert_eq!(v.damaged_chunks(), 2);
    }

    #[test]
    fn the_report_returns_verdicts_by_file_id() {
        let plan = RepairPlan::from_files(
            "1.0",
            vec![PlannedFile {
                rel: "a.dat".into(),
                size: 0,
                md5: "m".into(),
                url_prefix: Arc::from(""),
                chunks: vec![],
            }],
        );
        let present = Md5::parse_hex("00000000000000000000000000000001").unwrap();
        let report = ScanReport {
            files: vec![
                None,
                Some(Verdict {
                    file_md5: present,
                    dest_ok: vec![],
                    source_ok: vec![],
                    file_len: 0,
                    modified: None,
                }),
            ],
            bytes: 0,
            damaged_chunks: 0,
            failures: Vec::new(),
            scanned_at: None,
            plan_fingerprint: Some(plan.fingerprint()),
        };

        assert!(report.get(&plan, FileId::new(0)).is_none());
        assert_eq!(report.get(&plan, FileId::new(1)).unwrap().file_md5, present);
        assert!(
            report.get(&plan, FileId::new(2)).is_none(),
            "an id outside the report is not a verdict"
        );
    }

    /// The report's file positions belong to the plan it read. A report paired
    /// with a different plan answers `None` for every id instead of handing
    /// back a verdict at a position that means something else there.
    #[test]
    fn a_report_from_another_plan_is_rejected() {
        let plan_a = RepairPlan::from_files(
            "1.0",
            vec![PlannedFile {
                rel: "a.dat".into(),
                size: 1,
                md5: "a".into(),
                url_prefix: Arc::from(""),
                chunks: vec![chunk_at("c0", "m0", 0, 1)],
            }],
        );
        let plan_b = RepairPlan::from_files(
            "1.0",
            vec![PlannedFile {
                rel: "b.dat".into(),
                size: 2,
                md5: "b".into(),
                url_prefix: Arc::from(""),
                chunks: vec![chunk_at("c0", "m1", 0, 2)],
            }],
        );
        let report = ScanReport {
            files: vec![Some(Verdict {
                file_md5: Md5::EMPTY,
                dest_ok: vec![true],
                source_ok: vec![true],
                file_len: 1,
                modified: None,
            })],
            bytes: 1,
            damaged_chunks: 0,
            failures: Vec::new(),
            scanned_at: None,
            plan_fingerprint: Some(plan_a.fingerprint()),
        };

        assert!(report.matches_plan(&plan_a));
        assert!(!report.matches_plan(&plan_b));
        assert!(
            report.get(&plan_b, FileId::new(0)).is_none(),
            "a verdict from another plan must not be applied"
        );
    }

    /// A file the scan could not resolve is recorded with its reason and still
    /// gets the conservative verdict: a failure is never turned into a silent
    /// "intact" answer, and both the count and the reason reach the log.
    #[tokio::test(flavor = "current_thread")]
    async fn an_unresolvable_file_is_recorded_as_a_scan_failure() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "scan_failure");
        let plan = Arc::new(RepairPlan::from_files(
            "1.0",
            vec![
                PlannedFile {
                    rel: "..\\outside.dat".into(),
                    size: 4,
                    md5: "m".into(),
                    url_prefix: Arc::from(""),
                    chunks: vec![chunk_at("c0", "m0", 0, 4)],
                },
                PlannedFile {
                    rel: "present.dat".into(),
                    size: 4,
                    md5: util::md5::md5_digest(b"aaaa").to_hex(),
                    url_prefix: Arc::from(""),
                    chunks: vec![chunk_at("c1", "m1", 0, 4)],
                },
            ],
        ));
        std::fs::write(dir.join("present.dat"), b"aaaa").unwrap();

        let (report, logs) = girpr_log::test_support::logs_of_async(scan_plan(
            &dir,
            None,
            Arc::clone(&plan),
            Arc::new(LocalChunkMap::new()),
            2,
        ))
        .await;

        assert_eq!(report.scan_failed(), 1, "one file could not be resolved");
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].0, 0, "the escaping file is index 0");
        assert!(
            report.failures[0].1.contains("escapes"),
            "the reason names the escape: {:?}",
            report.failures[0].1
        );
        // The unreached file still gets the conservative all-wrong verdict:
        // its chunk is a download, not a silent "intact".
        let v = report
            .get(&plan, FileId::new(0))
            .expect("a conservative verdict exists");
        assert_eq!(v.dest_ok, vec![false]);
        assert_eq!(v.source_ok, vec![false]);
        assert_eq!(
            est_file_download_bytes(
                &plan.files()[0].chunks,
                Some(v),
                &[None],
                RepairMode::Auto,
                Mode::Inplace
            ),
            4,
            "an unreached file is estimated in full"
        );
        assert!(
            logs.contains("scan_failed=1"),
            "the phase line surfaces the count: {logs}"
        );
        assert!(
            logs.contains("scan failed for a file"),
            "the reason is surfaced at the default level: {logs}"
        );
    }

    /// A clean scan records no failures, so the new count is zero and no
    /// failure reason is emitted.
    #[tokio::test(flavor = "current_thread")]
    async fn a_clean_scan_records_no_failures() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "scan_clean");
        let bytes = b"aaaa";
        std::fs::write(dir.join("a.dat"), bytes).unwrap();
        let plan = Arc::new(RepairPlan::from_files(
            "1.0",
            vec![PlannedFile {
                rel: "a.dat".into(),
                size: 4,
                md5: util::md5::md5_digest(bytes).to_hex(),
                url_prefix: Arc::from(""),
                chunks: vec![chunk_at("c0", "m0", 0, 4)],
            }],
        ));

        let (report, logs) = girpr_log::test_support::logs_of_async(scan_plan(
            &dir,
            None,
            Arc::clone(&plan),
            Arc::new(LocalChunkMap::new()),
            2,
        ))
        .await;

        assert_eq!(report.scan_failed(), 0);
        assert!(report.failures.is_empty());
        assert!(report.get(&plan, FileId::new(0)).is_some());
        assert!(
            logs.contains("scan_failed=0"),
            "the count is present even when zero: {logs}"
        );
        assert!(
            !logs.contains("scan failed for a file"),
            "a clean scan emits no failure reason: {logs}"
        );
    }
}
