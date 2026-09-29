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
pub(super) fn report_diff(diff: &Diff) -> i32 {
    for r in &diff.missing {
        println!("MISSING {}", r);
        debug!(kind = "missing", rel = %r, "diff");
    }
    for r in &diff.extra {
        println!("EXTRA {}", r);
        debug!(kind = "extra", rel = %r, "diff");
    }
    for r in &diff.changed {
        println!("CHANGED {}", r);
        debug!(kind = "changed", rel = %r, "diff");
    }
    for r in &diff.type_conflict {
        println!("TYPE-CONFLICT {}", r);
        debug!(kind = "type-conflict", rel = %r, "diff");
    }
    for (a, b) in &diff.case_mismatch {
        println!("CASE-MISMATCH {} <=> {}", a, b);
        debug!(kind = "case-mismatch", src_rel = %a, dst_rel = %b, "diff");
    }
    println!(
        "SUMMARY missing={} extra={} changed={} type_conflict={} case_mismatch={} total_diff={}",
        diff.missing.len(),
        diff.extra.len(),
        diff.changed.len(),
        diff.type_conflict.len(),
        diff.case_mismatch.len(),
        diff.total()
    );
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
