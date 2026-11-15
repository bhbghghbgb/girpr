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
use crate::util::{elapsed_s, is_cache_name};

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
///
/// **The wording is pinned, because the harnesses depend on it.** A label is
/// `record <path>` or `folder <path>` — the word for what the side is, a space,
/// and the side's own path — and `a_side_is_named_by_its_kind_and_its_own_path`
/// in `tests/helpers.rs` holds that text. A rewording is therefore a deliberate
/// change rather than a quiet one: the harnesses that build their labels
/// through here cannot drift apart without the pin saying so.
pub fn side_label(side: &Side) -> String {
    let path = match side {
        Side::Record(p) => p,
        Side::Folder(root) => root,
    };
    format!("{} {}", side_kind(side), path.display())
}

/// The folder a side's cache lives in, when it has one to repopulate.
///
/// A record that *is* a folder's `girpr-cache` lives in that folder, and that
/// folder is what `girsync update --dir` takes — so a coverage failure can
/// end with a command the user can paste instead of a placeholder.
///
/// **A record that is not a folder's cache has none.** A `girpr-cache-backup-*`
/// copy is a record as far as the tool is concerned — a comparison may be
/// pointed at one — but it is not any folder's `girpr-cache`, so no
/// repopulation would rebuild it, and naming the folder it happens to sit in
/// would send the user to rebuild a *different* file.
///
/// [`Repopulate::Nothing`] for a folder side: a folder can always answer for
/// itself, so it is never the short side, and it has no record of its own to
/// repopulate.
///
/// **Public, for the same reason [`side_label`] is**: a caller outside the
/// crate names a side the way the commands do, or it keeps a second spelling.
pub fn side_folder(side: &Side) -> Repopulate {
    match side {
        Side::Record(p) if is_cache_name(p) => Repopulate::Folder(match p.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
            // A record named on its own lives in the working directory.
            _ => PathBuf::from("."),
        }),
        _ => Repopulate::Nothing,
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::side_folder;
    use crate::effective::Side;
    use crate::planner::Repopulate;

    /// A record's folder is the one its cache lives in: the parent of
    /// `<folder>/girpr-cache`.
    #[test]
    fn a_record_names_the_folder_its_cache_lives_in() {
        let side = Side::Record(Path::new("game").join("old").join("girpr-cache"));
        assert_eq!(
            side_folder(&side),
            Repopulate::Folder(Path::new("game").join("old"))
        );
    }

    /// **A record named on its own lives in the working directory.**
    ///
    /// The edge case: a record path with no parent at all, so there is no
    /// folder to name — and the working directory is where it lives.
    #[test]
    fn a_record_named_on_its_own_lives_in_the_working_directory() {
        let side = Side::Record(PathBuf::from("girpr-cache"));
        assert_eq!(
            side_folder(&side),
            Repopulate::Folder(PathBuf::from(".")),
            "a record with no parent sits in the working directory"
        );
    }

    /// **A copy of a cache is not a cache.** A `girpr-cache-backup-*` file is a
    /// record as far as the tool is concerned — a comparison may be pointed at
    /// one — but it is not any folder's `girpr-cache`, so no repopulation would
    /// rebuild it. Naming the folder it happens to sit in would send the user
    /// to rebuild a *different* file, so no folder is named at all.
    #[test]
    fn a_record_that_is_not_a_folder_cache_has_no_folder() {
        let side = Side::Record(Path::new("game").join("old").join("girpr-cache-backup-1"));
        assert_eq!(
            side_folder(&side),
            Repopulate::Nothing,
            "no folder's cache is this file, so there is nothing to repopulate"
        );
    }

    /// A folder side is never short of coverage — it can always read the file —
    /// so it is never the side a remedy is about, and it has no record of its
    /// own to repopulate.
    #[test]
    fn a_folder_side_has_no_folder_to_repopulate() {
        let side = Side::Folder(PathBuf::from("game"));
        assert_eq!(side_folder(&side), Repopulate::Nothing);
    }
}
