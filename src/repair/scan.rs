//! The scan pass: the plan's per-chunk verdicts, computed before the repair
//! starts (docs/07 §7, option I).
//!
//! The download estimate the plan carries is a **lower bound**: it is
//! `Σ compressed_size` over the chunks the local reuse map cannot supply,
//! and a reuse slice that fails its MD5 falls back to a download. The
//! in-place repair makes that bound looser still, because a chunk that is
//! already correct *where it belongs* is not fetched at all — and the
//! estimate cannot know that without looking.
//!
//! `--scan` looks. It reads every planned file once, records the verdict
//! per chunk ("is it already correct where it belongs?", "is its reuse
//! source intact?"), and the estimate becomes **exact**:
//!
//! - `--max-download-bytes` then refuses on the real figure,
//! - `--print-plan` can report `scan_bytes` (what the scan read) and the
//!   exact estimate beside it,
//! - `--check-only` can report how many chunks are damaged, not just how
//!   many files.
//!
//! It is **opt-in** because it costs a full read pass before the repair
//! starts: the run reads every planned file twice (the scan, then the
//! repair's own verification) instead of once.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;

use crate::repair::manifest::LocalChunkMap;
use crate::repair::plan::PlannedFile;
use crate::repair::reuse;
use crate::util;

/// What the scan learned about one file.
#[derive(Debug, Clone)]
pub struct FileVerdict {
    /// The whole-file MD5 of the bytes on disk.
    pub file_md5: String,
    /// Per chunk: is it already correct where it belongs?
    pub dest_ok: Vec<bool>,
    /// Per chunk: is its reuse source intact? `false` for a chunk with no
    /// candidate at all.
    pub source_ok: Vec<bool>,
    /// The bytes the scan read for this file.
    pub bytes: u64,
}

impl FileVerdict {
    /// The chunks that are not correct where they belong.
    pub fn damaged_chunks(&self) -> u64 {
        self.dest_ok.iter().filter(|ok| !**ok).count() as u64
    }
}

/// The scan pass over the whole plan.
#[derive(Debug, Default)]
pub struct ScanReport {
    /// Per planned file, by rel path.
    pub verdicts: HashMap<String, FileVerdict>,
    /// The bytes the scan read, in total.
    pub bytes: u64,
    /// The chunks that are not correct where they belong, in total.
    pub damaged_chunks: u64,
    /// The files that are not correct as a whole.
    pub damaged_files: u64,
}

/// Scan every planned file, in parallel over `jobs`.
///
/// The scan is **blocking** I/O, and the plan phase is not latency-critical
/// (the repair that follows is network-bound), so it runs on plain worker
/// threads rather than on the async runtime's: a scan that blocked a
/// runtime worker would idle the other workers' progress reporting for no
/// reason.
pub fn scan_plan(
    game_dir: &Path,
    files: &[PlannedFile],
    local_map: &LocalChunkMap,
    jobs: usize,
) -> ScanReport {
    let next = AtomicUsize::new(0);
    let results: std::sync::Mutex<Vec<(usize, FileVerdict)>> = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..jobs.max(1) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= files.len() {
                    break;
                }
                let f = &files[i];
                let entries = local_map.get(&f.rel);
                let path = game_dir.join(crate::util::normalize_rel(&f.rel));
                match scan_one(&path, f, entries) {
                    Ok(v) => results.lock().unwrap().push((i, v)),
                    Err(e) => {
                        tracing::warn!(file = %f.rel, "scan failed: {e:#}");
                    }
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|(i, _)| *i);
    let mut report = ScanReport::default();
    for (i, v) in results {
        report.damaged_chunks += v.damaged_chunks();
        report.bytes += v.bytes;
        if v.damaged_chunks() > 0 {
            report.damaged_files += 1;
        }
        report.verdicts.insert(files[i].rel.clone(), v);
    }
    report
}

/// Scan one file: the whole-file MD5 and the per-chunk verdicts.
fn scan_one(
    path: &Path,
    file: &PlannedFile,
    entries: Option<&Vec<(String, i64, i64)>>,
) -> Result<FileVerdict> {
    let dest: Vec<(u64, u64, &str)> = file
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
    // The source ranges: one per chunk that has a candidate, and the map
    // back from a chunk to its source.
    let mut source_refs: Vec<(u64, u64, &str)> = Vec::new();
    let mut source_index: Vec<Option<usize>> = Vec::with_capacity(file.chunks.len());
    for c in &file.chunks {
        match entries
            .map(|list| reuse::find_candidate(c, list))
            .unwrap_or(None)
        {
            Some((_, _, off)) => {
                source_index.push(Some(source_refs.len()));
                source_refs.push((
                    (*off).max(0) as u64,
                    c.uncompressed_size.max(0) as u64,
                    c.uncompressed_md5.as_str(),
                ));
            }
            None => source_index.push(None),
        }
    }
    let scan = util::scan_file(path, &dest, &source_refs)?;
    let bytes = util::file_len(path).unwrap_or(0);
    Ok(FileVerdict {
        file_md5: scan.file_md5,
        dest_ok: scan.dest_ok,
        source_ok: source_index
            .iter()
            .map(|i| i.map(|k| scan.source_ok[k]).unwrap_or(false))
            .collect(),
        bytes,
    })
}

/// The chunks of one file that the in-place repair would **download**.
///
/// A chunk is downloaded when it is not correct where it belongs *and* it
/// has no salvageable source: a source elsewhere in the file, intact, whose
/// bytes the write order preserves. Under the temp path the same file needs
/// more, because the temp path cannot keep the bytes that are already in
/// the right place — which is why the scanned estimate is the in-place
/// figure, and the temp path's own ceiling check stays a lower bound.
pub fn chunks_to_download<'a>(
    file: &'a PlannedFile,
    verdict: &FileVerdict,
    entries: Option<&Vec<(String, i64, i64)>>,
) -> Vec<&'a crate::sophon::SophonChunk> {
    file.chunks
        .iter()
        .enumerate()
        .filter(|(i, c)| {
            if verdict.dest_ok.get(*i).copied().unwrap_or(false) {
                // Already correct where it belongs: kept, not fetched.
                return false;
            }
            if verdict.source_ok.get(*i).copied().unwrap_or(false) {
                // Its source is intact. Is it somewhere else? A source at
                // the chunk's own place is the bad bytes themselves.
                let o = c.offset.max(0) as u64;
                let elsewhere = entries
                    .and_then(|list| reuse::find_candidate(c, list))
                    .map(|(_, _, off)| (*off).max(0) as u64 != o)
                    .unwrap_or(false);
                if elsewhere {
                    return false;
                }
            }
            true
        })
        .map(|(_, c)| c)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sophon::SophonChunk;

    fn chunk(id: &str, md5: &str, offset: i64, size: i64) -> SophonChunk {
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

    fn file(chunks: Vec<SophonChunk>) -> PlannedFile {
        PlannedFile {
            rel: "f.dat".into(),
            size: 128,
            md5: "m".into(),
            chunks,
        }
    }

    #[test]
    fn a_chunk_that_is_already_right_is_not_downloaded() {
        let f = file(vec![chunk("c0", "md5-a", 0, 64), chunk("c1", "md5-b", 64, 64)]);
        let verdict = FileVerdict {
            file_md5: "x".into(),
            dest_ok: vec![true, false],
            source_ok: vec![false, false],
            bytes: 128,
        };
        let to_download = chunks_to_download(&f, &verdict, None);
        assert_eq!(to_download.len(), 1);
        assert_eq!(to_download[0].id, "c1");
    }

    #[test]
    fn a_chunk_with_an_intact_source_elsewhere_is_salvaged_not_downloaded() {
        let f = file(vec![chunk("c0", "md5-a", 0, 64)]);
        let entries = vec![("md5-a".to_string(), 64i64, 64i64)];
        // The chunk is wrong where it belongs, but its source is at 64 —
        // elsewhere in the file — and intact.
        let verdict = FileVerdict {
            file_md5: "x".into(),
            dest_ok: vec![false],
            source_ok: vec![true],
            bytes: 128,
        };
        assert!(
            chunks_to_download(&f, &verdict, Some(&entries)).is_empty(),
            "the chunk is salvaged from the file's own bytes"
        );
        // A source at the chunk's *own* place is the bad bytes: it is
        // downloaded.
        let own = vec![("md5-a".to_string(), 64i64, 0i64)];
        assert_eq!(
            chunks_to_download(&f, &verdict, Some(&own)).len(),
            1,
            "a source at the chunk's own place is not a salvage"
        );
    }
}
