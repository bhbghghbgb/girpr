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
use std::sync::atomic::Ordering;

use crate::report::{self, Summary};
use crate::util;

use super::plan::{PlannedFile, RepairPlan};
use crate::repair::scan::{ScanReport, Verdict};

/// Verify the whole plan; returns the number of damaged files.
///
/// When the run carried a `--scan`, the scan's verdicts are used instead of
/// reading the files again: the scan already read them, and the answer it
/// carries is the same one. That is what makes `--check-only --scan` a
/// single read of the install.
pub fn verify_plan(
    game_dir: &Path,
    root: Option<&util::PathRoot>,
    plan: &RepairPlan,
    summary: &Summary,
    output: report::OutputFormat,
    scan: Option<&ScanReport>,
) -> u64 {
    let mut bad = 0u64;
    let mut clean = 0u64;
    let mut damaged_chunks = 0u64;
    let mut last_emit = std::time::Instant::now();
    for f in &plan.files {
        // Resolve once. A containment failure is a *diagnostic*: `build_plan`
        // already refused an escaping rel, so this is defence in depth, and it
        // must not be mislabelled as a size mismatch (which is what `.ok()`
        // did here). The run's cached root saves the per-file base resolution
        // when it has one; `None` keeps the old `contained_join` fallback.
        let path = root.map_or_else(
            || util::contained_join(game_dir, &f.rel),
            |root| root.join(&f.rel),
        );
        let outcome = match scan.and_then(|s| s.files.get(&f.rel)) {
            Some(v) => check_one(f, &path, DigestSource::Scanned(v)),
            None => check_one(f, &path, DigestSource::Read),
        };
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
        // Keep the counters live so PROGRESS reflects work done so far.
        summary.files_skipped.store(clean, Ordering::Relaxed);
        summary.files_failed.store(bad, Ordering::Relaxed);
        if last_emit.elapsed().as_secs() >= report::PROGRESS_INTERVAL_SECS {
            report::emit_progress(summary, output);
            last_emit = std::time::Instant::now();
        }
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
    if actual.eq_ignore_ascii_case(&f.md5) {
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
            chunks: Vec::new(),
        }
    }

    fn verdict(file_md5: &str, dest_ok: Vec<bool>) -> Verdict {
        Verdict {
            file_md5: file_md5.into(),
            dest_ok,
            source_ok: Vec::new(),
        }
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
}
