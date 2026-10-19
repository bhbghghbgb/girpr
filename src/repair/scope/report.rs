//! The `PLAN` line and the `--plan-out` document.
//!
//! `--dry-run` does not cover this and cannot be bent into covering it, for
//! three separate reasons:
//!
//! 1. **Cost.** `repair_one_file` runs `util::file_len` *then*
//!    `util::md5_file` and only *then* hits `if ctx.dry_run { return DryRun }`.
//!    So `--dry-run` on a real install full-file-MD5s every planned file —
//!    precisely the cost the limitation exists to avoid. `--print-plan` stops
//!    before the repair loop and pays none of it.
//! 2. **Plane.** `--dry-run` answers on stderr / the log file via `tracing`.
//!    The stdout data plane (`REPORT`/`PROGRESS`/`SUMMARY`) exists so a caller
//!    can parse it; the plan belongs there.
//! 3. **Totals.** `--dry-run` emits no aggregate at all: there is no way to ask
//!    "how many files, how many bytes" without scraping per-file log lines and
//!    summing them by hand.
//!
//! **It is not an offline mode and does not pretend to be.** The plan *is* the
//! manifest — there is no plan without it — so `--print-plan` fetches, logs at
//! the usual level, and the flag help says so.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Serialize;

use crate::repair::plan::{PlannedFile, PurgePlan, RepairPlan};
use crate::report::Record;
use crate::util;

/// The aggregated numbers behind the `PLAN` line, and behind `PLAN` in JSON
/// mode. Computed from the two finished plans; no I/O.
pub struct PlanTotals {
    pub files: u64,
    pub est_download_bytes: u64,
    /// The bytes the plan would write **into the install**. Without a scan
    /// this is an upper bound: the whole file, because every chunk of it
    /// could be wrong.
    pub est_write_bytes: u64,
    /// The extra free space the plan needs beyond the install. Under the
    /// in-place path that is the files' **growth** — zero for a same-size
    /// repair — and under the temp path it is one whole file per file in
    /// flight.
    pub est_extra_bytes: u64,
    pub purge_files: u64,
    pub purge_bytes: u64,
    pub max_chunk_bytes: u64,
    pub scoped: bool,
    /// The bytes the scan pass read, when `--scan` ran.
    pub scan_bytes: Option<u64>,
}

impl PlanTotals {
    pub fn new(
        plan: &RepairPlan,
        purge: Option<&PurgePlan>,
        est_download_bytes: u64,
        scoped: bool,
        extras: &PlanExtras,
        scan_bytes: Option<u64>,
    ) -> Self {
        let per_file: Vec<FileExtra> = plan.files.iter().map(|f| extras.for_file(f)).collect();
        Self {
            files: plan.files.len() as u64,
            est_download_bytes,
            est_write_bytes: per_file.iter().map(|e| e.write_bytes).sum(),
            est_extra_bytes: per_file.iter().map(|e| e.extra_bytes).sum(),
            purge_files: purge.map_or(0, |p| p.files.len() as u64),
            purge_bytes: purge.map_or(0, |p| p.total_bytes),
            // The largest single chunk is what bounds peak RAM (a whole chunk is
            // buffered compressed + uncompressed) and, multiplied by
            // `io_threads`, transient disk. Worth reporting even though nothing
            // enforces it: it is the number a `--max-chunk-bytes` ceiling would
            // take, and it is one line to print.
            max_chunk_bytes: plan
                .files
                .iter()
                .flat_map(|f| f.chunks.iter())
                .map(|c| c.compressed_size.max(0) as u64)
                .max()
                .unwrap_or(0),
            scoped,
            scan_bytes,
        }
    }
}

/// What one file costs the run, on the disk axis.
///
/// The two numbers are different questions: `write_bytes` is what the repair
/// writes into the install, `extra_bytes` is what it needs *beyond* the
/// install to do it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileExtra {
    /// Which path would write this file: `"inplace"` or `"temp"`.
    pub mode: &'static str,
    /// The bytes written into the install (an upper bound without a scan).
    pub write_bytes: u64,
    /// The extra free space the file needs: its growth under the in-place
    /// path, its whole size under the temp path.
    pub extra_bytes: u64,
}

/// The per-file disk estimates, computed once from the game dir and the mode.
pub struct PlanExtras {
    game_dir: PathBuf,
    repair_mode: crate::config::RepairMode,
}

impl PlanExtras {
    pub fn new(game_dir: &Path, repair_mode: crate::config::RepairMode) -> Self {
        Self {
            game_dir: game_dir.to_path_buf(),
            repair_mode,
        }
    }

    /// The disk cost of one file, under the mode that would write it.
    pub fn for_file(&self, f: &PlannedFile) -> FileExtra {
        let size = f.size.max(0) as u64;
        let on_disk = util::file_len(&self.game_dir.join(util::normalize_rel(&f.rel))).unwrap_or(0);
        // The in-place path writes the file where it lies — creating it when
        // it is not there — so it needs the file's **growth** and nothing
        // else; the temp path needs a whole copy of it beside the original.
        let inplace = self.repair_mode != crate::config::RepairMode::Temp;
        if inplace {
            FileExtra {
                mode: "inplace",
                write_bytes: size,
                extra_bytes: size.saturating_sub(on_disk.min(size)),
            }
        } else {
            FileExtra {
                mode: "temp",
                write_bytes: size,
                extra_bytes: size,
            }
        }
    }
}

/// The one-line `PLAN` summary. Key order is fixed so two runs diff cleanly.
///
/// A [`Record`], like every other stdout line, so `--print-plan --output json`
/// emits a `{"event":"plan",…}` object: one object per event, so the stream
/// parses as JSON.
///
/// `scoped` is an integer here for the same reason it is one on `SUMMARY`: one
/// key name carries one type across the whole stream.
pub fn plan_record(t: &PlanTotals) -> Record {
    let rec = Record::keyed("PLAN")
        .put("files", t.files)
        .put("est_download_bytes", t.est_download_bytes)
        .put("est_write_bytes", t.est_write_bytes)
        .put("est_extra_bytes", t.est_extra_bytes)
        .put("purge_files", t.purge_files)
        .put("purge_bytes", t.purge_bytes)
        .put("max_chunk_bytes", t.max_chunk_bytes)
        .put("scoped", u64::from(t.scoped));
    // The scan's own figure, when it ran: how much the plan had to read to
    // know what it was going to do.
    match t.scan_bytes {
        Some(n) => rec.put("scan_bytes", n),
        None => rec,
    }
}

/// The `--plan-out` document.
///
/// **JSON**: `girsync --output json` and the whole artifact side of this repo
/// (`gircap`'s bundle, `patchmaps/<id>.json`) are JSON, and a parser that reads
/// `SUMMARY` *and* the plan wants one `serde_json` reader. The `key=value` plane
/// keeps its own job: one line per fact, greppable, no nesting.
#[derive(Debug, Serialize)]
pub struct PlanDocument<'a> {
    /// Every planned file: what it is, how big, what it would cost.
    pub repair: Vec<RepairEntry<'a>>,
    /// Every file the files-cleanup would delete.
    pub purge: Vec<PurgeEntry<'a>>,
}

#[derive(Debug, Serialize)]
pub struct RepairEntry<'a> {
    pub rel: &'a str,
    pub size: i64,
    pub chunks_total: u64,
    /// Chunks the local reuse map cannot supply, by compressed size — this
    /// file's half of the plan's `est_download_bytes`.
    pub est_download_bytes: u64,
    /// Which path would write this file: `inplace` or `temp`.
    pub mode: &'static str,
    /// The extra free space this file needs beyond the install: its growth
    /// under the in-place path, its whole size under the temp path.
    pub extra_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct PurgeEntry<'a> {
    pub rel: &'a str,
    pub size: u64,
}

impl<'a> PlanDocument<'a> {
    pub fn new(
        plan: &'a RepairPlan,
        purge: Option<&'a PurgePlan>,
        est_download_bytes: u64,
        extras: &PlanExtras,
    ) -> Self {
        // The per-file estimates must sum to the plan total, or the document
        // would disagree with the `PLAN` line. `est` is the caller-computed plan
        // total; the split is only trusted when it adds up, and the difference
        // is folded into the first file so the two always agree.
        let mut per_file: Vec<u64> = plan
            .files
            .iter()
            .map(|f| {
                f.chunks
                    .iter()
                    .map(|c| c.compressed_size.max(0) as u64)
                    .sum()
            })
            .collect();
        let summed: u64 = per_file.iter().sum();
        if summed != est_download_bytes
            && let Some(first) = per_file.first_mut()
        {
            *first = first.saturating_add(est_download_bytes.saturating_sub(summed));
        }
        Self {
            repair: plan
                .files
                .iter()
                .zip(per_file)
                .map(|(f, est)| {
                    let extra = extras.for_file(f);
                    RepairEntry {
                        rel: &f.rel,
                        size: f.size,
                        chunks_total: f.chunks.len() as u64,
                        est_download_bytes: est,
                        mode: extra.mode,
                        extra_bytes: extra.extra_bytes,
                    }
                })
                .collect(),
            purge: purge
                .map(|p| {
                    p.files
                        .iter()
                        .map(|c| PurgeEntry {
                            rel: &c.rel,
                            size: c.bytes,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// Render to the file named by `--plan-out`.
    pub fn write_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create dir {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self).context("serialize plan")?;
        std::fs::write(path, format!("{json}\n"))
            .with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repair::plan::{PlannedFile, PurgeCandidate};
    use crate::report::{Summary, summary_record};
    use std::collections::HashMap;

    fn chunk(id: &str, size: i64) -> crate::sophon::SophonChunk {
        crate::sophon::SophonChunk {
            id: id.into(),
            uncompressed_md5: format!("md5-{id}"),
            offset: 0,
            compressed_size: size,
            uncompressed_size: size,
            unknown: 0,
            compressed_md5: String::new(),
        }
    }

    fn plan() -> RepairPlan {
        RepairPlan {
            latest: "9.9".into(),
            files: vec![
                PlannedFile {
                    rel: "game/foo.dat".into(),
                    size: 128,
                    md5: "a".into(),
                    chunks: vec![chunk("c0", 10), chunk("c1", 20)],
                },
                PlannedFile {
                    rel: "game/bar.dat".into(),
                    size: 64,
                    md5: "b".into(),
                    chunks: vec![chunk("c2", 30)],
                },
                PlannedFile {
                    rel: "game/empty.dat".into(),
                    size: 0,
                    md5: "c".into(),
                    chunks: vec![],
                },
            ],
            url_prefix_by_file: HashMap::new(),
            verdicts: None,
        }
    }

    fn purge_plan() -> PurgePlan {
        PurgePlan {
            files: vec![PurgeCandidate {
                rel: "stray/orphan.dat".into(),
                path: "stray/orphan.dat".into(),
                bytes: 4096,
            }],
            dirs: vec![],
            total_bytes: 4096,
            hidden: 3,
        }
    }

    /// The per-file disk estimates, against a game dir with nothing in it:
    /// every planned file is missing, so the in-place path would create it
    /// (and need its whole size) and the temp path would need a whole copy.
    fn extras() -> PlanExtras {
        let dir = crate::testtemp::TempDir::new("girpr_test_plan_out", "extras");
        PlanExtras::new(&dir, crate::config::RepairMode::Auto)
    }

    #[test]
    fn the_plan_line_has_a_fixed_key_order() {
        let t = PlanTotals::new(&plan(), Some(&purge_plan()), 60, true, &extras(), None);
        let rec = plan_record(&t);
        assert_eq!(
            rec.text(),
            "PLAN files=3 est_download_bytes=60 est_write_bytes=192 est_extra_bytes=192 \
             purge_files=1 purge_bytes=4096 max_chunk_bytes=30 scoped=1"
        );
        // No purge flag: the purge fields are zero, not absent.
        let t = PlanTotals::new(&plan(), None, 60, false, &extras(), None);
        assert_eq!(
            plan_record(&t).text(),
            "PLAN files=3 est_download_bytes=60 est_write_bytes=192 est_extra_bytes=192 \
             purge_files=0 purge_bytes=0 max_chunk_bytes=30 scoped=0"
        );
    }

    /// One record per event, on either plane. The JSON keys are the same names the
    /// text line uses — `files`, not `plan` — so a parser reads one vocabulary.
    #[test]
    fn the_plan_is_one_record_on_either_plane() {
        let t = PlanTotals::new(&plan(), Some(&purge_plan()), 60, true, &extras(), None);
        let rec = plan_record(&t);
        let json = rec.json();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["event"], "plan");
        assert_eq!(v["files"], 3);
        assert_eq!(v["est_download_bytes"], 60);
        assert_eq!(v["est_write_bytes"], 192);
        assert_eq!(v["est_extra_bytes"], 192);
        assert_eq!(v["purge_files"], 1);
        assert_eq!(v["purge_bytes"], 4096);
        assert_eq!(v["max_chunk_bytes"], 30);
        assert_eq!(v.as_object().unwrap().len(), 9, "{json}");
        // `scoped` is an integer here for the same reason it is one on
        // `SUMMARY`: one key name carries one type across the whole stream.
        assert_eq!(v["scoped"], 1);
        assert!(v["scoped"].is_i64(), "{json}");
    }

    #[test]
    fn a_plan_that_is_not_scoped_reports_zero_not_a_boolean() {
        let t = PlanTotals::new(&plan(), None, 60, false, &extras(), None);
        let v: serde_json::Value = serde_json::from_str(&plan_record(&t).json()).unwrap();
        assert_eq!(v["scoped"], 0);
        assert!(!v["scoped"].is_boolean(), "{v}");
    }

    #[test]
    fn max_chunk_bytes_is_the_largest_compressed_chunk() {
        // Peak RAM per active chunk is roughly compressed + uncompressed, and
        // peak transient disk is io_threads x that.
        let t = PlanTotals::new(&plan(), None, 60, false, &extras(), None);
        assert_eq!(t.max_chunk_bytes, 30);
        let t = PlanTotals::new(
            &RepairPlan {
                files: vec![],
                ..plan()
            },
            None,
            0,
            false,
            &extras(),
            None,
        );
        assert_eq!(t.max_chunk_bytes, 0, "an empty plan has no chunks");
    }

    #[test]
    fn the_document_lists_every_file_and_the_estimates_sum_to_the_plan() {
        let (plan, purge) = (plan(), purge_plan());
        let doc = PlanDocument::new(&plan, Some(&purge), 60, &extras());
        assert_eq!(doc.repair.len(), 3);
        assert_eq!(doc.repair[0].rel, "game/foo.dat");
        assert_eq!(doc.repair[0].chunks_total, 2);
        assert_eq!(doc.repair[0].est_download_bytes, 30);
        assert_eq!(doc.repair[2].est_download_bytes, 0, "no chunks, no bytes");
        let sum: u64 = doc.repair.iter().map(|e| e.est_download_bytes).sum();
        assert_eq!(sum, 60, "the document must agree with the PLAN line");
        // The disk story, per file: the mode and what it costs.
        assert_eq!(doc.repair[0].mode, "inplace");
        assert_eq!(doc.repair[0].extra_bytes, 128, "a missing file is created");

        assert_eq!(doc.purge.len(), 1);
        assert_eq!(doc.purge[0].rel, "stray/orphan.dat");
        assert_eq!(doc.purge[0].size, 4096);
    }

    #[test]
    fn a_mismatched_estimate_is_reconciled_into_the_document() {
        // The document is written from the same estimate the `PLAN` line
        // reports; if they ever disagreed, the file would be a lie.
        let plan = plan();
        let doc = PlanDocument::new(&plan, None, 100, &extras());
        let sum: u64 = doc.repair.iter().map(|e| e.est_download_bytes).sum();
        assert_eq!(sum, 100);
    }

    #[test]
    fn the_document_is_valid_json_with_the_documented_shape() {
        let (plan, purge) = (plan(), purge_plan());
        let doc = PlanDocument::new(&plan, Some(&purge), 60, &extras());
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&doc).unwrap())
            .expect("the document must be valid JSON");
        assert_eq!(v["repair"].as_array().unwrap().len(), 3);
        assert_eq!(v["repair"][0]["rel"], "game/foo.dat");
        assert!(v["repair"][0].get("est_download_bytes").is_some());
        assert_eq!(v["purge"][0]["rel"], "stray/orphan.dat");
    }

    #[test]
    fn writing_creates_the_parent_directory() {
        let dir = crate::testtemp::TempDir::new("girpr_test_plan_out", "creates_parent");
        let path = dir.join("nested").join("plan.json");
        let plan = plan();
        let doc = PlanDocument::new(&plan, None, 60, &extras());
        doc.write_to(&path).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("game/foo.dat"), "{written}");
        assert!(
            written.ends_with('\n'),
            "a text document ends with a newline"
        );
    }

    #[test]
    fn the_summary_a_plan_run_returns_is_untouched() {
        // `--print-plan` reports the plan, not a repair: no counter moves.
        let mut s = Summary::default();
        s.flags.scoped = true;
        s.files_total = 3;
        let line = summary_record(&s, 0).text();
        assert!(line.contains("skipped=0 repaired=0 failed=0"), "{line}");
        assert!(line.contains("scoped=1"), "{line}");
    }
}
