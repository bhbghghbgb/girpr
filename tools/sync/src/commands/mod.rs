//! Subcommand implementations and the `Cli` -> command dispatch.

mod compare;
mod sync;
mod update;

pub use compare::cmd_compare;
pub use sync::cmd_sync;
pub use update::cmd_update;

use anyhow::Result;

use crate::cli::{Cli, Cmd};
use crate::config::{CommonOpts, CompareOpts, LogCtx, SyncOpts, UpdateOpts};

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
            no_fast,
            common,
        } => cmd_compare(
            CompareOpts {
                src,
                dst,
                fast: !no_fast,
                common: CommonOpts::try_from(common)?,
            },
            &log,
        ),
        Cmd::Sync {
            src,
            dst,
            no_fast,
            missing_only,
            keep_extra,
            dry_run,
            jobs,
            common,
        } => cmd_sync(
            SyncOpts {
                src,
                dst,
                fast: !no_fast,
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
