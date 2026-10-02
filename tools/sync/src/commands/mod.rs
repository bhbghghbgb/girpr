//! Subcommand implementations and the `Cli` -> command dispatch.

mod compare;
mod compare_self;
mod sync;
mod update;

pub use compare::cmd_compare;
pub use compare_self::cmd_compare_self;
pub use sync::cmd_sync;
pub use update::cmd_update;

use anyhow::Result;
use tracing::debug;

use crate::cli::{Cli, Cmd};
use crate::config::{CommonOpts, CompareOpts, CompareSelfOpts, LogCtx, SyncOpts, UpdateOpts};
use crate::diff::Diff;

/// Print one line per difference plus the `SUMMARY` line, and return the exit
/// code: `0` when nothing differs, `4` when something does.
///
/// Shared by `compare` and `compare-self` so the two cannot drift apart in
/// vocabulary or exit code. The caller has already logged the span outcome.
/// The lines a diff prints, in report order, each with the log kind it is
/// reported under.
///
/// Printing and asserting read the same list, so what a test states and what a
/// user sees cannot drift apart: there is one definition of a `CHANGED` line,
/// not one in the printer and another in each test.
pub fn verdict(diff: &Diff) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    for r in &diff.missing {
        out.push(("missing", format!("MISSING {r}")));
    }
    for r in &diff.extra {
        out.push(("extra", format!("EXTRA {r}")));
    }
    for r in &diff.changed {
        out.push(("changed", format!("CHANGED {r}")));
    }
    for r in &diff.type_conflict {
        out.push(("type-conflict", format!("TYPE-CONFLICT {r}")));
    }
    for (a, b) in &diff.case_mismatch {
        out.push(("case-mismatch", format!("CASE-MISMATCH {a} <=> {b}")));
    }
    out.push((
        "summary",
        format!(
            "SUMMARY missing={} extra={} changed={} type_conflict={} case_mismatch={} total_diff={}",
            diff.missing.len(),
            diff.extra.len(),
            diff.changed.len(),
            diff.type_conflict.len(),
            diff.case_mismatch.len(),
            diff.total()
        ),
    ));
    out
}

/// Print one line per difference plus the `SUMMARY` line, and return the exit
/// code: `0` when nothing differs, `4` when something does.
///
/// Shared by `compare` and `compare-self` so the two cannot drift apart in
/// vocabulary or exit code. The caller has already logged the span outcome.
pub(super) fn report_diff(diff: &Diff) -> i32 {
    for (kind, line) in verdict(diff) {
        println!("{line}");
        debug!(kind, "diff");
    }
    if diff.is_empty() { 0 } else { 4 }
}

/// Dispatch a parsed [`Cli`] to its command, returning the process exit code.
///
/// This is where raw CLI strings are validated into [`CommonOpts`]; every later
/// layer works with compiled patterns and checked algorithm lists.
pub fn run(cli: Cli) -> Result<i32> {
    let log = LogCtx {
        level: cli.log_level,
        file: cli.log_file,
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
            dry_run,
            trust,
            common,
        } => cmd_compare(
            CompareOpts {
                src,
                dst,
                trust: trust.into(),
                dry_run,
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
            dry_run,
            jobs,
            trust,
            common,
        } => cmd_sync(
            SyncOpts {
                src,
                dst,
                trust: trust.into(),
                missing_only,
                keep_extra,
                dry_run,
                jobs,
                common: CommonOpts::try_from(common)?,
            },
            &log,
        ),
    }
}
