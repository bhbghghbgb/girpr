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

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use tracing::info;

use crate::cli::{Cli, Cmd};
use crate::config::{CommonOpts, CompareOpts, CompareSelfOpts, LogCtx, SyncOpts, UpdateOpts};
use crate::diff::Diff;
use crate::effective::Side;
use crate::planner::Repopulate;
use crate::report::{Report, visit_verdict};
use crate::util::elapsed_s;

/// What a side is called, in a message and in a log field.
///
/// "record" or "folder" — the word a label is built from and the word a
/// structured log field carries.
pub(super) fn side_kind(side: &Side) -> &'static str {
    match side {
        Side::Record(_) => "record",
        Side::Folder(_) => "folder",
    }
}

/// How a side is named in a coverage error.
///
/// **One spelling for every command**, so the same side is never named two
/// ways: a side is named by what it *is* — a record or a folder — and by its
/// path.
///
/// The planner knows what a side *can do* but not what it *is*, so this is the
/// caller's one chance to put that into the message, and "the dst side cannot
/// supply…" is not something a user can act on where "the record
/// `D:\game\old\girpr-cache` cannot supply…" is.
///
/// The path says the rest. A record's path is `<folder>/girpr-cache`, so the
/// folder it describes is its parent, and that folder is what the remedy the
/// message ends with repopulates. A side is therefore named by its path and
/// nothing else, and two commands reading the same record name it the same
/// way.
///
/// Every command names its sides through here, including `sync`: its two sides
/// are always folders, and the word beside a folder's path is `folder` there
/// too.
///
/// **Public, so that a caller outside the crate names a side the same way.**
/// A test harness that builds the label itself keeps a second spelling of a
/// rule the commands share, and the two drift the moment the wording changes.
pub fn side_label(side: &Side) -> String {
    let path = match side {
        Side::Record(p) => p,
        Side::Folder(root) => root,
    };
    format!("{} {}", side_kind(side), path.display())
}

/// The folder a side's cache lives in, when it has one to repopulate.
///
/// A record's path is `<folder>/girpr-cache`, so the folder it describes is
/// its parent — and that folder is what `girsync update --dir` takes, so a
/// coverage failure can end with a command the user can paste instead of a
/// placeholder.
///
/// [`Repopulate::Nothing`] for a folder side: a folder can always answer for
/// itself, so it is never the short side, and it has no record of its own to
/// repopulate.
///
/// **Public, for the same reason [`side_label`] is**: a caller outside the
/// crate names a side the way the commands do, or it keeps a second spelling.
pub fn side_folder(side: &Side) -> Repopulate {
    match side {
        Side::Record(p) => Repopulate::Folder(match p.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
            // A record named on its own lives in the working directory.
            _ => PathBuf::from("."),
        }),
        Side::Folder(_) => Repopulate::Nothing,
    }
}

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
