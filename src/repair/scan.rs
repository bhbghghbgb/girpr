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
//! The scan does not change what the repair does. The repair re-reads each
//! file before it writes it, because a file can have changed since the plan
//! phase, and the repair needs its verdicts at the moment it writes. The
//! scan is for the *plan*, not for the writer.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use futures::StreamExt;

use crate::repair::manifest::LocalChunkMap;
use crate::repair::plan::{PlannedFile, RepairPlan};
use crate::repair::reuse;
use crate::util;

/// What one file's scan found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// The whole-file MD5 of what was on disk.
    pub file_md5: String,
    /// The on-disk length.
    pub len: u64,
    /// Per chunk, in the plan's order: is it correct where it belongs?
    pub dest_ok: Vec<bool>,
    /// Per chunk, in the plan's order: is its reuse source intact?
    pub source_ok: Vec<bool>,
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
    /// Per planned file, by rel path.
    pub files: HashMap<String, Verdict>,
    /// The bytes the scan read.
    pub bytes: u64,
    /// The chunks that are wrong where they belong, over the whole plan.
    pub damaged_chunks: u64,
}

impl ScanReport {
    /// The bytes the repair will really download.
    ///
    /// A chunk is downloaded when it is **wrong where it belongs** and has
    /// **no intact source** to copy from. Everything else is either already
    /// correct — the in-place path leaves it alone — or copied from inside
    /// the file.
    ///
    /// This is the figure the temp path's estimate over-counts: the temp path
    /// writes every chunk of a file it repairs, so it downloads every chunk
    /// the reuse map cannot supply, even the ones that are already correct
    /// where they belong.
    pub fn est_download_bytes(&self, plan: &RepairPlan) -> u64 {
        plan.files
            .iter()
            .map(|f| match self.files.get(&f.rel) {
                Some(v) => f
                    .chunks
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !v.dest_ok[*i] && !v.source_ok[*i])
                    .map(|(_, c)| c.compressed_size.max(0) as u64)
                    .sum::<u64>(),
                // A file the scan did not reach: the whole thing is unknown,
                // so the whole thing is estimated.
                None => f
                    .chunks
                    .iter()
                    .map(|c| c.compressed_size.max(0) as u64)
                    .sum(),
            })
            .sum()
    }
}

/// One file's scan, before it is folded into the report.
struct Scanned {
    rel: String,
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
    plan: Arc<RepairPlan>,
    local_map: Arc<LocalChunkMap>,
    jobs: usize,
) -> ScanReport {
    let mut stream = futures::stream::iter(plan.files.iter().cloned().map(|f| {
        let game_dir = game_dir.to_path_buf();
        let local_map = Arc::clone(&local_map);
        async move {
            tokio::task::spawn_blocking(move || scan_one(&game_dir, &f, &local_map)).await
        }
    }))
    .buffer_unordered(jobs.max(1));

    let mut out = ScanReport::default();
    while let Some(res) = stream.next().await {
        let Ok(scanned) = res else { continue };
        out.bytes += scanned.bytes;
        out.damaged_chunks += scanned.verdict.damaged_chunks();
        out.files.insert(scanned.rel, scanned.verdict);
    }
    tracing::info!(
        files = out.files.len(),
        bytes = out.bytes,
        damaged_chunks = out.damaged_chunks,
        "scan"
    );
    out
}

/// Scan one file: one read, every answer.
fn scan_one(game_dir: &Path, file: &PlannedFile, local_map: &LocalChunkMap) -> Scanned {
    let path = game_dir.join(util::normalize_rel(&file.rel));
    let entries = local_map.get(&file.rel);
    let sources = reuse::sources_for(
        &file.chunks,
        entries.map(|v| v.as_slice()).unwrap_or(&[]),
    );
    let dest_ranges: Vec<util::ScanRange<'_>> = file
        .chunks
        .iter()
        .map(|c| {
            (
                c.offset.max(0) as u64,
                c.uncompressed_size.max(0) as u64,
                c.uncompressed_md5.as_str(),
            )
        })
        .collect();
    let source_ranges: Vec<util::ScanRange<'_>> = file
        .chunks
        .iter()
        .zip(sources.iter())
        .map(|(c, src)| {
            (
                src.unwrap_or(0),
                c.uncompressed_size.max(0) as u64,
                c.uncompressed_md5.as_str(),
            )
        })
        .collect();
    let scan = util::scan_file(&path, &dest_ranges, &source_ranges).unwrap_or_else(|e| {
        // An unreadable file is not a failed scan: the verdict is "nothing
        // is correct", which is what the repair would find too.
        tracing::debug!(file = %file.rel, error = %format!("{e:#}"), "scan: unreadable");
        util::Scan::absent(dest_ranges.len(), source_ranges.len())
    });
    Scanned {
        rel: file.rel.clone(),
        bytes: scan.file_len,
        verdict: Verdict {
            file_md5: scan.file_md5,
            len: scan.file_len,
            dest_ok: scan.dest_ok,
            source_ok: scan.source_ok,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sophon::SophonChunk;

    fn chunk(id: &str, md5: &str, size: i64) -> SophonChunk {
        SophonChunk {
            id: id.into(),
            uncompressed_md5: md5.into(),
            offset: 0,
            compressed_size: size,
            uncompressed_size: size,
            unknown: 0,
            compressed_md5: String::new(),
        }
    }

    fn file(rel: &str, chunks: Vec<SophonChunk>) -> PlannedFile {
        PlannedFile {
            rel: rel.into(),
            size: 128,
            md5: "m".into(),
            chunks,
        }
    }

    /// The estimate is the figure the repair will really spend: a chunk that
    /// is wrong **and** has no intact source.
    #[test]
    fn the_exact_estimate_counts_the_chunks_that_must_be_downloaded() {
        let plan = RepairPlan {
            latest: "9.9".into(),
            files: vec![file(
                "a.dat",
                vec![chunk("c0", "m0", 10), chunk("c1", "m1", 20)],
            )],
            url_prefix_by_file: HashMap::new(),
        };
        let mut report = ScanReport::default();
        // c0 is wrong but has an intact source; c1 is wrong and has none.
        report.files.insert(
            "a.dat".to_string(),
            Verdict {
                file_md5: "x".into(),
                len: 128,
                dest_ok: vec![false, false],
                source_ok: vec![true, false],
            },
        );
        assert_eq!(report.est_download_bytes(&plan), 20, "only c1 is fetched");
        // And a file the scan never reached is estimated in full.
        let report = ScanReport::default();
        assert_eq!(report.est_download_bytes(&plan), 30);
    }

    #[test]
    fn the_damaged_chunk_count_is_the_wrong_chunks() {
        let v = Verdict {
            file_md5: "x".into(),
            len: 128,
            dest_ok: vec![true, false, false, true],
            source_ok: vec![true; 4],
        };
        assert_eq!(v.damaged_chunks(), 2);
    }
}
