//! Subcommand implementations and the `Cli` -> command dispatch.

mod compare;
mod compare_self;
mod sync;
mod update;

pub use compare::cmd_compare;
pub use compare_self::cmd_compare_self;
pub use sync::cmd_sync;
pub use update::cmd_update;

pub use crate::report::verdict;

use std::time::Instant;

use anyhow::Result;
use tracing::info;

use crate::cli::{Cli, Cmd};
use crate::config::{CommonOpts, CompareOpts, CompareSelfOpts, LogCtx, SyncOpts, UpdateOpts};
use crate::diff::Diff;
use crate::report::{Report, visit_verdict};
use crate::util::elapsed_s;

/// Emit every record a diff reports — one per difference plus the `SUMMARY` — and
/// return the exit code: `0` when nothing differs, `4` when something does.
///
/// Shared by `compare` and `compare-self` so the two cannot drift apart in
/// vocabulary or exit code. The caller has already logged the span outcome.
///
/// The records come from [`visit_verdict`], which is *data* rather than text, so
/// [`Report`] alone decides whether a human reads lines or a parser reads JSON.
/// That is also why printing and asserting read the same definitions: there is one
/// definition of a `CHANGED` record, not one in the printer and another in each
/// test.
///
/// The reporting flags are threaded in and nothing else is: the exit code is computed
/// from [`Diff`] below, never from the record stream, so neither `--why` nor
/// `--show-identical` can reach it. That is the property that lets a clean tree with
/// `--show-identical` stay exit `0`, and it is why the two flags stop here rather than
/// being read off the emitted records.
pub(super) fn report_diff(diff: &Diff, report: &Report, why: bool, show_identical: bool) -> i32 {
    visit_verdict(diff, why, show_identical, |rec| report.emit(rec));
    if diff.is_empty() { 0 } else { 4 }
}

/// Report a diff and log the run's `"end"` summary line, returning the exit code.
///
/// Both `compare` and `compare-self` finish this way, so the summary fields and the
/// exit rule live in one place: a new count or a changed exit rule cannot land in
/// one command and miss the other. The code still comes from [`report_diff`], which
/// reads [`Diff::is_empty`]; the logged counters are descriptive and never decide it.
pub(super) fn report_and_finish(
    diff: &Diff,
    report: &Report,
    why: bool,
    show_identical: bool,
    t0: Instant,
) -> i32 {
    let code = report_diff(diff, report, why, show_identical);
    info!(
        missing = diff.missing.len(),
        extra = diff.extra.len(),
        changed = diff.changed.len(),
        type_conflict = diff.type_conflict.len(),
        case_mismatch = diff.case_mismatch.len(),
        total_diff = diff.total(),
        elapsed_s = elapsed_s(t0),
        exit = code,
        "end"
    );
    code
}

/// Dispatch a parsed [`Cli`] to its command, returning the process exit code.
///
/// This is where raw CLI strings are validated into [`CommonOpts`]; every later
/// layer works with compiled patterns and checked algorithm lists.
///
/// `plan` is the resolved log policy — where the log file went, and at what
/// level each sink runs. It is passed in rather than resolved here, because
/// the subscriber was installed with it and a second resolution would name a
/// second file: the default path carries a timestamp, and two timestamps are
/// two files.
pub fn run(cli: Cli, plan: &girpr_log::LogPlan) -> Result<i32> {
    let log = LogCtx {
        level: plan.console_level,
        file_level: plan.file_level,
        file: plan.file.clone(),
        output: cli.output,
    };
    match cli.cmd {
        Cmd::Update { dir, common } => cmd_update(
            UpdateOpts {
                dir,
                common: CommonOpts::try_from(common)?,
            },
            &log,
        ),
        Cmd::Compare {
            src,
            dst,
            trust,
            common,
        } => cmd_compare(
            CompareOpts {
                src,
                dst,
                trust: trust.into(),
                common: CommonOpts::try_from(common)?,
            },
            &log,
        ),
        Cmd::CompareSelf {
            dir,
            no_trust_cached_hashes,
            common,
        } => cmd_compare_self(
            CompareSelfOpts {
                dir,
                no_trust_cached_hashes,
                common: CommonOpts::try_from(common)?,
            },
            &log,
        ),
        Cmd::Sync {
            src,
            dst,
            missing_only,
            keep_extra,
            trust,
            common,
        } => cmd_sync(
            SyncOpts {
                src,
                dst,
                trust: trust.into(),
                missing_only,
                keep_extra,
                common: CommonOpts::try_from(common)?,
            },
            &log,
        ),
    }
}
