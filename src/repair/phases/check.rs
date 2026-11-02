//! `--check-only`: verify every planned file and report damage, write nothing
//! (docs/02 step 6.5).
//!
//! This is the one place that answers "is my install intact?" without touching
//! it: no `config.ini` bump, no purge, no `*_tmp` sweep, no audio scan write.
//! Exit code is `0` when clean, `4` when any file fails size, MD5, or is
//! unreadable.
//!
//! `files_skipped` counts verified-clean files and `files_failed` counts damaged
//! ones, so the `SUMMARY`/`PROGRESS` counters read the same as a repair run.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use futures_util::StreamExt;

use crate::report::{self, Summary};
use crate::util::{self, Md5};

use super::plan::{FileId, PlannedFile, RepairPlan};
use crate::repair::scan::{ScanReport, Verdict};

/// Verify the whole plan; returns the number of damaged files.
///
/// When the run carried a `--scan`, the scan's verdicts are used instead of
/// reading the files again: the scan already read them, and the answer it
/// carries is the same one. That is what makes `--check-only --scan` a
/// single read of the install.
///
/// The checks are blocking filesystem reads, so each file is checked on the
/// blocking pool and `jobs` bounds how many run at once. The results are folded
/// in plan order even when the tasks finish out of order, so the failure log
/// and the counters stay deterministic.
pub async fn verify_plan(
    game_dir: &Path,
    root: Option<&util::PathRoot>,
    plan: Arc<RepairPlan>,
    summary: Arc<Summary>,
    output: report::OutputFormat,
    scan: Option<Arc<ScanReport>>,
    jobs: usize,
) -> u64 {
    // One progress owner for the phase: the ticker reads the counters below as
    // they are folded. Dropping the guard on return stops it, so the check-only
    // path never leaves a ticker behind.
    let _progress = report::ProgressGuard::start(Arc::clone(&summary), output);
    // The install root is resolved once here, not once per file, and shared by
    // the blocking tasks. `None` keeps the per-file `contained_join` fallback.
    let root: Option<Arc<util::PathRoot>> = root.cloned().map(Arc::new);
    let game_dir = game_dir.to_path_buf();
    let mut outcomes: Vec<(usize, CheckOutcome)> = futures_util::stream::iter(0..plan.files.len())
        .map(|idx| {
            let plan = Arc::clone(&plan);
            let game_dir = game_dir.clone();
            let root = root.clone();
            let scan = scan.clone();
            async move {
                match tokio::task::spawn_blocking(move || {
                    let f = &plan.files[idx];
                    // Resolve once. A containment failure is a *diagnostic*:
                    // `build_plan` already refused an escaping rel, so this is
                    // defence in depth, and it must not be mislabelled as a
                    // size mismatch (which is what `.ok()` did here). The run's
                    // cached root saves the per-file base resolution when it has
                    // one; `None` keeps the old `contained_join` fallback.
                    let path = root.map_or_else(
                        || util::contained_join(&game_dir, &f.rel),
                        |root| root.join(&f.rel),
                    );
                    let outcome = match scan.as_deref().and_then(|s| s.get(FileId::new(idx))) {
                        Some(v) => check_one(f, &path, DigestSource::Scanned(v)),
                        None => check_one(f, &path, DigestSource::Read),
                    };
                    (idx, outcome)
                })
                .await
                {
                    Ok(outcome) => outcome,
                    Err(e) => {
                        // A panicked or cancelled task is a bug, not a clean
                        // verdict. Keep the file counted as damaged instead of
                        // dropping it from the total without a trace.
                        (
                            idx,
                            CheckOutcome::failed(CheckFailure::Read(format!(
                                "check task failed: {e}"
                            ))),
                        )
                    }
                }
            }
        })
        .buffer_unordered(jobs.max(1))
        .collect()
        .await;
    outcomes.sort_by_key(|(idx, _)| *idx);

    let mut bad = 0u64;
    let mut clean = 0u64;
    let mut damaged_chunks = 0u64;
    for (idx, outcome) in outcomes {
        let f = &plan.files[idx];
        if outcome.ok {
            clean += 1;
        } else {
            bad += 1;
            damaged_chunks += outcome.damaged_chunks.unwrap_or(0);
            // The one warn site: every failure shape is logged here, in the
            // same order and with the same fields as before the branches were
            // merged.
            if let Some(failure) = outcome.failure {
                match failure {
                    CheckFailure::PathEscaped(e) => {
                        tracing::warn!(
                            file = %f.rel,
                            error = %e,
                            "CHECK fail: path escapes the game dir"
                        );
                    }
                    CheckFailure::Unreadable(e) => {
                        tracing::warn!(file = %f.rel, error = %e, "CHECK fail: unreadable");
                    }
                    CheckFailure::Absent => {
                        tracing::warn!(file = %f.rel, expect_size = f.size, actual_size = ?Option::<u64>::None, "CHECK fail: size");
                    }
                    CheckFailure::Size(n) => {
                        tracing::warn!(file = %f.rel, expect_size = f.size, actual_size = ?Some(n), "CHECK fail: size");
                    }
                    CheckFailure::Md5 {
                        actual,
                        damaged_chunks: Some(n),
                    } => {
                        tracing::warn!(
                            file = %f.rel,
                            expect_md5 = %f.md5,
                            actual_md5 = %actual,
                            damaged_chunks = n,
                            "CHECK fail: md5"
                        );
                    }
                    CheckFailure::Md5 {
                        actual,
                        damaged_chunks: None,
                    } => {
                        tracing::warn!(file = %f.rel, expect_md5 = %f.md5, actual_md5 = %actual, "CHECK fail: md5");
                    }
                    CheckFailure::Read(e) => {
                        tracing::warn!(file = %f.rel, "CHECK fail: unreadable: {e}");
                    }
                }
            }
        }
        // Keep the counters live so the phase's ticker reflects work done so
        // far. Only the ticker emits `PROGRESS`.
        summary.files_skipped.store(clean, Ordering::Relaxed);
        summary.files_failed.store(bad, Ordering::Relaxed);
    }
    tracing::info!(
        "check-only: total={} bad={} damaged_chunks={}",
        plan.files.len(),
        bad,
        damaged_chunks
    );
    bad
}

/// Where the digest a check compares against the manifest came from.
#[derive(Clone, Copy)]
enum DigestSource<'a> {
    /// The plan-phase scan already hashed the file; its verdict also knows
    /// which chunks are wrong where they belong.
    Scanned(&'a Verdict),
    /// The check hashes the file itself, and only after the presence and
    /// length checks pass, so an absent or wrongly sized file is never read.
    Read,
}

/// Why one checked file is damaged. Each shape has its own fields, and the
/// caller has the one warn site for all of them.
enum CheckFailure {
    /// The rel escaped the game dir (defence in depth; `build_plan` already
    /// refused an escaping rel).
    PathEscaped(String),
    /// The presence probe could not stat the file.
    Unreadable(String),
    /// The file is not there.
    Absent,
    /// The file exists with another length.
    Size(u64),
    /// The length matched but the digest did not. `damaged_chunks` is the
    /// scan's count, and `None` when the digest was read here.
    Md5 {
        actual: String,
        damaged_chunks: Option<u64>,
    },
    /// Reading the file for its digest failed.
    Read(String),
}

/// What checking one planned file found.
struct CheckOutcome {
    /// The file matched the manifest: present, the declared length, the
    /// declared digest.
    ok: bool,
    /// The chunks the scan found wrong where they belong, on a scanned
    /// mismatch.
    damaged_chunks: Option<u64>,
    /// The failure shape, for the caller's one warn site. `None` when `ok`.
    failure: Option<CheckFailure>,
}

impl CheckOutcome {
    fn clean() -> Self {
        Self {
            ok: true,
            damaged_chunks: None,
            failure: None,
        }
    }

    fn failed(failure: CheckFailure) -> Self {
        let damaged_chunks = match &failure {
            CheckFailure::Md5 { damaged_chunks, .. } => *damaged_chunks,
            _ => None,
        };
        Self {
            ok: false,
            damaged_chunks,
            failure: Some(failure),
        }
    }
}

/// Check one planned file against the manifest.
///
/// Presence and length are checked for both digest sources: the scan is a
/// shortcut for the *read*, not for the "is it there" answer. An absent or
/// unreadable file's scan verdict carries the empty digest, and a legitimate
/// empty file's manifest entry has exactly that digest, so the digest alone
/// cannot tell them apart.
fn check_one(
    f: &PlannedFile,
    path: &anyhow::Result<PathBuf>,
    digest: DigestSource<'_>,
) -> CheckOutcome {
    let path = match path {
        Ok(path) => path,
        Err(e) => {
            return CheckOutcome::failed(CheckFailure::PathEscaped(format!("{e:#}")));
        }
    };
    let actual_size = match util::file_len(path) {
        util::FileLen::Present(n) => n,
        util::FileLen::Unreadable(e) => {
            return CheckOutcome::failed(CheckFailure::Unreadable(e.to_string()));
        }
        util::FileLen::Absent => return CheckOutcome::failed(CheckFailure::Absent),
    };
    if actual_size != f.size as u64 {
        return CheckOutcome::failed(CheckFailure::Size(actual_size));
    }
    let actual: Cow<'_, str> = match digest {
        DigestSource::Scanned(v) => Cow::Borrowed(v.file_md5.as_str()),
        DigestSource::Read => match util::md5_file(path) {
            Ok(h) => Cow::Owned(h),
            Err(e) => return CheckOutcome::failed(CheckFailure::Read(format!("{e:#}"))),
        },
    };
    if Md5::parse_hex(&actual).is_ok_and(|actual| actual.matches_hex(&f.md5)) {
        return CheckOutcome::clean();
    }
    let damaged_chunks = match digest {
        DigestSource::Scanned(v) => Some(v.damaged_chunks()),
        DigestSource::Read => None,
    };
    CheckOutcome::failed(CheckFailure::Md5 {
        actual: actual.into_owned(),
        damaged_chunks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planned(rel: &str, size: i64, md5: &str) -> PlannedFile {
        PlannedFile {
            rel: rel.into(),
            size,
            md5: md5.into(),
            url_prefix: Arc::from(""),
            chunks: Vec::new(),
        }
    }

    fn plan_of(files: Vec<PlannedFile>) -> RepairPlan {
        RepairPlan {
            latest: "9.9".into(),
            files,
        }
    }

    fn verdict(file_md5: &str, dest_ok: Vec<bool>) -> Verdict {
        Verdict {
            file_md5: file_md5.into(),
            dest_ok,
            source_ok: Vec::new(),
            file_len: 0,
            modified: None,
        }
    }

    /// The failure lines from a captured log, without the timestamp prefix, so
    /// two runs can be compared for the same warning set and order.
    fn check_warnings(logs: &str) -> Vec<String> {
        logs.lines()
            .filter_map(|line| line.find("CHECK fail").map(|at| line[at..].to_string()))
            .collect()
    }

    /// Run one verify on a current-thread runtime and return its counters and
    /// captured log. The check tasks use the blocking pool; only the ordered
    /// fold logs, so the captured lines stay deterministic.
    fn run_verify(
        base: &Path,
        plan: &Arc<RepairPlan>,
        scan: &Option<Arc<ScanReport>>,
        jobs: usize,
    ) -> (u64, u64, u64, String) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let summary = Arc::new(Summary::default());
        let mut bad = 0u64;
        let logs = crate::repair::test_support::logs_of(|| {
            bad = runtime.block_on(verify_plan(
                base,
                None,
                Arc::clone(plan),
                Arc::clone(&summary),
                report::OutputFormat::Text,
                scan.clone(),
                jobs,
            ));
        });
        (
            bad,
            summary.files_skipped.load(Ordering::Relaxed),
            summary.files_failed.load(Ordering::Relaxed),
            logs,
        )
    }

    /// The jobs flag bounds the checks without changing their answer: same
    /// clean/failed counts, same damaged-chunk total, same warnings in the same
    /// plan order, whether one file runs at a time or all of them do.
    #[test]
    fn parallel_checks_match_one_job_and_stay_in_plan_order() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_check", "parallel_parity");
        let base = &*dir;
        std::fs::write(base.join("a.dat"), b"aaaa").unwrap();
        std::fs::write(base.join("b.dat"), b"bbbb").unwrap();
        let clean_md5 = util::md5::md5_digest(b"aaaa").to_hex();
        let plan = Arc::new(plan_of(vec![
            planned("a.dat", 4, &clean_md5),
            planned("b.dat", 4, "ffffffffffffffffffffffffffffffff"),
            planned("c.dat", 7, "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"),
        ]));
        let scan = Some(Arc::new(ScanReport {
            files: vec![
                None,
                Some(verdict(
                    "00000000000000000000000000000000",
                    vec![true, false, false],
                )),
                None,
            ],
            bytes: 0,
            damaged_chunks: 0,
        }));

        let one = run_verify(base, &plan, &scan, 1);
        let many = run_verify(base, &plan, &scan, 4);

        assert_eq!(
            (one.0, one.1, one.2),
            (many.0, many.1, many.2),
            "jobs must not change the verdict or the counters"
        );
        assert_eq!((one.0, one.1, one.2), (2, 1, 2));
        let one_warnings = check_warnings(&one.3);
        assert_eq!(
            one_warnings,
            check_warnings(&many.3),
            "the warning set and its plan order are jobs-independent"
        );
        assert_eq!(one_warnings.len(), 2, "{:?}", one_warnings);
        assert!(
            one_warnings[0].contains("b.dat") && one_warnings[0].contains("CHECK fail: md5"),
            "{:?}",
            one_warnings
        );
        assert!(
            one_warnings[1].contains("c.dat") && one_warnings[1].contains("CHECK fail: size"),
            "{:?}",
            one_warnings
        );
        assert!(
            one.3.contains("damaged_chunks=2"),
            "the scan's damaged-chunk total is folded once: {}",
            one.3
        );
    }

    #[test]
    fn a_scanned_mismatch_carries_the_damaged_chunks() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_check", "scanned_md5");
        let base = &*dir;
        std::fs::write(base.join("f.dat"), b"on disk").unwrap();
        let f = planned("f.dat", 7, "expected");
        let path = util::contained_join(base, &f.rel);

        let outcome = check_one(
            &f,
            &path,
            DigestSource::Scanned(&verdict("actual", vec![true, false, false])),
        );
        assert!(!outcome.ok);
        assert_eq!(
            outcome.damaged_chunks,
            Some(2),
            "the scan's count is carried once"
        );
        assert!(matches!(
            outcome.failure,
            Some(CheckFailure::Md5 {
                ref actual,
                damaged_chunks: Some(2)
            }) if actual == "actual"
        ));
    }

    #[test]
    fn an_absent_file_fails_before_the_digest_is_read() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_check", "absent");
        let f = planned("missing.dat", 7, "expected");
        let path = util::contained_join(&dir, &f.rel);

        for digest in [
            DigestSource::Read,
            DigestSource::Scanned(&verdict("actual", vec![true])),
        ] {
            let outcome = check_one(&f, &path, digest);
            assert!(!outcome.ok);
            assert!(
                matches!(outcome.failure, Some(CheckFailure::Absent)),
                "presence is checked even when the scan has a verdict"
            );
            assert_eq!(outcome.damaged_chunks, None);
        }
    }

    /// A scanned check trusts the scan's digest — it must not read the file a
    /// second time — but the presence and length checks still run against the
    /// disk, so a file that changed after the scan is still caught where those
    /// checks can see it.
    #[test]
    fn a_scanned_check_uses_the_scan_digest_but_still_checks_the_length() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_check", "scanned_trusts_digest");
        let base = &*dir;
        std::fs::write(base.join("f.dat"), b"on disk").unwrap();
        let expected = "0123456789abcdef0123456789abcdef";
        let scan = verdict(expected, vec![true]);

        // The on-disk bytes do not hash to `expected`, but the scan says they
        // did: using the scan's answer means no second read.
        let same_size = planned("f.dat", 7, expected);
        let path = util::contained_join(base, &same_size.rel);
        assert!(
            check_one(&same_size, &path, DigestSource::Scanned(&scan)).ok,
            "the scan's digest is the digest"
        );

        // The length check is not delegated to the scan.
        let changed_length = planned("f.dat", 8, expected);
        let outcome = check_one(&changed_length, &path, DigestSource::Scanned(&scan));
        assert!(!outcome.ok);
        assert!(
            matches!(outcome.failure, Some(CheckFailure::Size(7))),
            "a length change after the scan is still damage"
        );
    }
}
