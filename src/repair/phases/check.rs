//! `--check-only`: verify every planned file and report damage, write nothing
//! (docs/02 step 6.5).
//!
//! This is the one place that answers "is my install intact?" without touching
//! it: no `config.ini` bump, no purge, no `*_tmp` sweep, no audio scan write.
//! The exit status is [`RunExit::Success`](crate::RunExit::Success) when clean
//! and [`RunExit::CheckFailed`](crate::RunExit::CheckFailed) when any file
//! fails size, MD5, or is unreadable. A check task that panics is a run failure
//! ([`RunExit::Write`](crate::RunExit::Write)), not a damage verdict: the file
//! was not shown to be bad, the verifier itself broke.
//!
//! `files_skipped` counts verified-clean files and `files_failed` counts damaged
//! ones, so the `SUMMARY`/`PROGRESS` counters read the same as a repair run.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use futures_util::StreamExt;

use crate::config::RunFailure;
use crate::report::{self, Summary};
use crate::util;

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
) -> Result<u64, RunFailure> {
    verify_plan_with(
        game_dir, root, plan, summary, output, scan, jobs, None, check_one,
    )
    .await
}

/// The body of [`verify_plan`], with the per-file check injectable for tests.
///
/// `folded` receives the number of outcomes folded so far, once per completion.
/// Production passes `None`; a test uses it to wait for a specific fold instead
/// of polling the counters.
#[allow(clippy::too_many_arguments)] // the phase's inputs are one run's shared context
async fn verify_plan_with<F>(
    game_dir: &Path,
    root: Option<&util::PathRoot>,
    plan: Arc<RepairPlan>,
    summary: Arc<Summary>,
    output: report::OutputFormat,
    scan: Option<Arc<ScanReport>>,
    jobs: usize,
    folded: Option<std::sync::mpsc::Sender<usize>>,
    check: F,
) -> Result<u64, RunFailure>
where
    F: for<'a> Fn(&PlannedFile, &anyhow::Result<PathBuf>, DigestSource<'a>) -> CheckOutcome
        + Send
        + Sync
        + 'static,
{
    let timer = crate::phase::PhaseTimer::start("check");
    // One progress owner for the phase: the ticker reads the counters below as
    // they are folded. Dropping the guard on return stops it, so the check-only
    // path never leaves a ticker behind.
    let _progress = report::ProgressGuard::start(Arc::clone(&summary), output);
    // The install root is resolved once here, not once per file, and shared by
    // the blocking tasks. `None` keeps the per-file `contained_join` fallback.
    let root: Option<Arc<util::PathRoot>> = root.cloned().map(Arc::new);
    let game_dir = game_dir.to_path_buf();
    let check = Arc::new(check);
    let mut outcomes: Vec<(usize, CheckOutcome)> = Vec::with_capacity(plan.len());
    let mut checks = futures_util::stream::iter(0..plan.len())
        .map(|idx| {
            let plan = Arc::clone(&plan);
            let game_dir = game_dir.clone();
            let root = root.clone();
            let scan = scan.clone();
            let check = Arc::clone(&check);
            async move {
                let result = tokio::task::spawn_blocking(move || {
                    let f = &plan.files()[idx];
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
                    match scan.as_deref().and_then(|s| s.get(&plan, FileId::new(idx))) {
                        Some(v) => check(f, &path, DigestSource::Scanned(v)),
                        None => check(f, &path, DigestSource::Read),
                    }
                })
                .await;
                (idx, result)
            }
        })
        .buffer_unordered(jobs.max(1));
    // Fold each finished task into the live counters immediately. The ordered
    // pass below still owns the failure log, so its order stays deterministic.
    while let Some((idx, result)) = checks.next().await {
        match result {
            Ok(outcome) => {
                record_check(&summary, &outcome);
                outcomes.push((idx, outcome));
                if let Some(tx) = &folded {
                    // The count, not a bare ping: a waiter can tell which
                    // completion it observed.
                    let _ = tx.send(outcomes.len());
                }
            }
            Err(e) => {
                // A panicked or cancelled task is a bug, not a damage verdict:
                // the file was never shown to be bad. Report the same run
                // failure class the purge tasks use instead of folding it into
                // `RunExit::CheckFailed`.
                //
                // Returning drops the stream, so no further file starts. A
                // check already running on the blocking pool cannot be
                // cancelled: it finishes its read before releasing the thread.
                // That tail is bounded by `jobs` reads, and it affects only
                // teardown — the output and exit class above are already
                // decided. Cutting it short would need a cancellation check
                // threaded through the read itself, which is a wider change
                // than this failure path justifies.
                return Err(RunFailure::write(anyhow::anyhow!("check task failed: {e}")));
            }
        }
    }
    outcomes.sort_by_key(|(idx, _)| *idx);

    let mut bad = 0u64;
    let mut damaged_chunks = 0u64;
    for (idx, outcome) in outcomes {
        let f = &plan.files()[idx];
        if !outcome.ok {
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
    }
    // Keep the check's own damaged-chunk tally on the summary for callers that
    // need it, but leave the emitted `damaged_chunks` key alone: that key is
    // the plan-phase scan's figure and its presence is part of the output
    // contract. Publishing after the tally is what tells a clean check
    // (measured, zero) apart from a check that never ran.
    summary.publish_check_damaged_chunks(damaged_chunks);
    tracing::info!(
        phase = % "check",
        duration_ms = timer.elapsed_ms(),
        "check-only: total={} bad={} damaged_chunks={}",
        plan.len(),
        bad,
        damaged_chunks
    );
    timer.disarm();
    Ok(bad)
}

/// Fold one finished check into the phase counters as soon as it completes, so
/// the ticker's `done` tracks the pass instead of waiting for the ordered fold.
fn record_check(summary: &Summary, outcome: &CheckOutcome) {
    if outcome.ok {
        summary.files_skipped.fetch_add(1, Ordering::Relaxed);
    } else {
        summary.files_failed.fetch_add(1, Ordering::Relaxed);
    }
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
    let actual = match digest {
        DigestSource::Scanned(v) => v.file_md5,
        DigestSource::Read => match util::md5::md5_file_digest(path) {
            Ok(digest) => digest,
            Err(e) => return CheckOutcome::failed(CheckFailure::Read(format!("{e:#}"))),
        },
    };
    if actual.matches_hex(&f.md5) {
        return CheckOutcome::clean();
    }
    let damaged_chunks = match digest {
        DigestSource::Scanned(v) => Some(v.damaged_chunks()),
        DigestSource::Read => None,
    };
    CheckOutcome::failed(CheckFailure::Md5 {
        actual: actual.to_hex(),
        damaged_chunks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::Md5;
    use std::time::Duration;

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
        RepairPlan::from_files("9.9", files)
    }

    fn verdict(file_md5: &str, dest_ok: Vec<bool>) -> Verdict {
        Verdict {
            file_md5: Md5::parse_hex(file_md5).unwrap(),
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
    ) -> (u64, u64, u64, u64, String) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let summary = Arc::new(Summary::default());
        let mut bad = 0u64;
        let logs = crate::repair::test_support::logs_of(|| {
            bad = runtime
                .block_on(verify_plan(
                    base,
                    None,
                    Arc::clone(plan),
                    Arc::clone(&summary),
                    report::OutputFormat::Text,
                    scan.clone(),
                    jobs,
                ))
                .expect("the check tasks finish");
        });
        (
            bad,
            summary.files_skipped.load(Ordering::Relaxed),
            summary.files_failed.load(Ordering::Relaxed),
            summary
                .check_damaged_chunks()
                .expect("the check measured it"),
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
        let scan = Some(Arc::new(ScanReport::for_test(
            &plan,
            vec![
                None,
                Some(verdict(
                    "00000000000000000000000000000000",
                    vec![true, false, false],
                )),
                None,
            ],
        )));

        let one = run_verify(base, &plan, &scan, 1);
        let many = run_verify(base, &plan, &scan, 4);

        assert_eq!(
            (one.0, one.1, one.2, one.3),
            (many.0, many.1, many.2, many.3),
            "jobs must not change the verdict or the counters"
        );
        assert_eq!((one.0, one.1, one.2, one.3), (2, 1, 2, 2));
        let one_warnings = check_warnings(&one.4);
        assert_eq!(
            one_warnings,
            check_warnings(&many.4),
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
            one.4.contains("damaged_chunks=2"),
            "the scan's damaged-chunk total is folded once: {}",
            one.4
        );
    }

    /// A clean pass records a measured zero, not the same "never measured"
    /// state a default `Summary` carries.
    #[test]
    fn a_clean_check_measures_zero() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_check", "clean_zero");
        let base = &*dir;
        std::fs::write(base.join("a.dat"), b"aaaa").unwrap();
        let md5 = util::md5::md5_digest(b"aaaa").to_hex();
        let plan = Arc::new(plan_of(vec![planned("a.dat", 4, &md5)]));
        let scan = None;

        let (bad, skipped, failed, damaged_chunks, _) = run_verify(base, &plan, &scan, 2);

        assert_eq!((bad, skipped, failed), (0, 1, 0));
        assert_eq!(
            damaged_chunks, 0,
            "a clean check measured zero damaged chunks"
        );
    }

    #[test]
    fn a_panicked_check_task_fails_the_run_instead_of_reporting_damage() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_check", "task_panic");
        let base = &*dir;
        let plan = Arc::new(plan_of(vec![planned(
            "a.dat",
            0,
            "00000000000000000000000000000000",
        )]));
        let summary = Arc::new(Summary::default());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let err = runtime
            .block_on(verify_plan_with(
                base,
                None,
                plan,
                Arc::clone(&summary),
                report::OutputFormat::Text,
                None,
                1,
                None,
                |_: &PlannedFile,
                 _: &anyhow::Result<PathBuf>,
                 _: DigestSource<'_>|
                 -> CheckOutcome { panic!("injected check-task panic") },
            ))
            .expect_err("a panicked check task is a run failure");

        assert_eq!(
            err.exit_code,
            crate::RunExit::Write,
            "a broken verifier is RunExit::Write, not RunExit::CheckFailed"
        );
        assert_eq!(
            summary.files_failed.load(Ordering::Relaxed),
            0,
            "the file was never shown to be damaged"
        );
    }

    /// The ticker reads the counters while the pass is still running, so a
    /// finished check is folded before the slowest one returns. The gate keeps
    /// one task blocked without sleeping; the other task's counter must be
    /// visible while it is held.
    #[test]
    fn finished_checks_move_the_counters_before_the_pass_ends() {
        #[derive(Default)]
        struct Gate {
            slow_started: bool,
            fast_done: bool,
            release_slow: bool,
        }

        let dir = girpr_tempdir::TempDir::new("girpr_test_check", "live_progress");
        let base = dir.to_path_buf();
        let plan = Arc::new(plan_of(vec![
            planned("slow.dat", 0, "00000000000000000000000000000000"),
            planned("fast.dat", 0, "00000000000000000000000000000000"),
        ]));
        let summary = Arc::new(Summary::default());
        let gate = Arc::new((
            std::sync::Mutex::new(Gate::default()),
            std::sync::Condvar::new(),
        ));
        // The fold reports each completion on this channel. The test waits for
        // the signal instead of polling the counter, so it has no timing
        // assumption to violate on a single-core or heavily loaded host.
        let (folded_tx, folded_rx) = std::sync::mpsc::channel::<usize>();

        let thread_summary = Arc::clone(&summary);
        let thread_gate = Arc::clone(&gate);
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime
                .block_on(verify_plan_with(
                    &base,
                    None,
                    plan,
                    thread_summary,
                    report::OutputFormat::Text,
                    None,
                    2,
                    Some(folded_tx),
                    move |f: &PlannedFile,
                          _: &anyhow::Result<PathBuf>,
                          _: DigestSource<'_>|
                          -> CheckOutcome {
                        let (lock, cv) = &*thread_gate;
                        let mut state = lock.lock().unwrap();
                        if f.rel == "slow.dat" {
                            state.slow_started = true;
                            cv.notify_all();
                            while !state.release_slow {
                                state = cv.wait(state).unwrap();
                            }
                        } else {
                            state.fast_done = true;
                            cv.notify_all();
                        }
                        CheckOutcome::clean()
                    },
                ))
                .expect("the checks finish")
        });

        // Wait until both tasks have reached their gate: one is held, the
        // other has returned and is ready to be folded.
        let (lock, cv) = &*gate;
        let mut state = lock.lock().unwrap();
        while !(state.slow_started && state.fast_done) {
            state = cv.wait(state).unwrap();
        }
        drop(state);

        // The first fold is the finished task. `recv_timeout` parks the test
        // thread until the fold happens and turns a lost fold into a clear
        // failure instead of hanging the suite.
        assert_eq!(
            folded_rx.recv_timeout(Duration::from_secs(10)),
            Ok(1),
            "the finished check was not folded while the slow task was held"
        );
        assert_eq!(
            summary.files_skipped.load(Ordering::Relaxed),
            1,
            "only the finished task is folded so far"
        );
        assert_eq!(summary.files_failed.load(Ordering::Relaxed), 0);

        let (lock, cv) = &*gate;
        let mut state = lock.lock().unwrap();
        state.release_slow = true;
        cv.notify_all();
        drop(state);

        assert_eq!(thread.join().unwrap(), 0);
        assert_eq!(summary.files_skipped.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn a_scanned_mismatch_carries_the_damaged_chunks() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_check", "scanned_md5");
        let base = &*dir;
        std::fs::write(base.join("f.dat"), b"on disk").unwrap();
        let f = planned("f.dat", 7, "expected");
        let path = util::contained_join(base, &f.rel);
        let actual_hex = "00000000000000000000000000000002";

        let outcome = check_one(
            &f,
            &path,
            DigestSource::Scanned(&verdict(actual_hex, vec![true, false, false])),
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
            }) if actual == actual_hex
        ));
    }

    #[test]
    fn an_absent_file_fails_before_the_digest_is_read() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_check", "absent");
        let f = planned("missing.dat", 7, "expected");
        let path = util::contained_join(&dir, &f.rel);

        for digest in [
            DigestSource::Read,
            DigestSource::Scanned(&verdict("00000000000000000000000000000000", vec![true])),
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
