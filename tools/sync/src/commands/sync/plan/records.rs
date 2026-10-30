//! The `SUMMARY` record's fields and the plan-as-records rendering.
//!
//! One struct, one field list, one [`Record`] constructor — so a dry run and a
//! real run cannot disagree about the *vocabulary*, only about the numbers and
//! `dry_run`. That is the whole of the `--dry-run` reporting contract: same
//! field names, same order, same meaning. A duplicated field list would make
//! that a property of two lists agreeing rather than of one list existing.

use crate::report::Record;

use super::super::apply::Applied;
use super::build::Plan;

/// The `SUMMARY` record's fields, in both modes.
///
/// `dry_run` is the only field that differs by design. The names describe the
/// *result*, not the plan, so a dry run reports what it would reach; the numbers
/// agree whenever a real run completes, since a failed copy aborts before the
/// summary is reached at all.
struct Summary {
    renamed: usize,
    mkdir: usize,
    copied: usize,
    deleted: usize,
    rmdir: usize,
    missing_only: bool,
    keep_extra: bool,
    dry_run: bool,
}

impl Summary {
    fn record(&self) -> Record {
        Record::keyed("SUMMARY")
            .put("renamed", self.renamed)
            .put("mkdir", self.mkdir)
            .put("copied", self.copied)
            .put("deleted", self.deleted)
            .put("rmdir", self.rmdir)
            .put("missing_only", self.missing_only)
            .put("keep_extra", self.keep_extra)
            .put("dry_run", self.dry_run)
    }
}

/// The plan as records: one per action plus the `SUMMARY`. Writes nothing.
///
/// **This is the single definition of what a dry run reports**, so the real run
/// cannot drift from it. [`a_dry_run_reports_what_a_real_run_reports`] builds
/// both and asserts the two record lists are equal field for field with only the
/// `dry_run` marker differing, so any future divergence in labels, ordering or
/// counts fails there rather than being discovered by a user.
///
/// [`a_dry_run_reports_what_a_real_run_reports`]: ../../sync_plan/
///
/// Which path gets which action is not pinned here as a transcript. It is asserted
/// per path by the fixture table in `tests/sync_plan/`, which also derives the
/// summary counts from the plan rather than stating them.
///
/// `rmdir` is exact rather than omitted: the apply phase reads [`Plan::rmdir`]
/// instead of walking dst, so the count is the same before anything is written as
/// it is after.
///
/// The records are in **apply order** — fix-dirs, mkdirs, deletes, copies,
/// rmdir — rather than grouped by plan category, so the sequence a dry run
/// reports is the sequence a real run will perform, and the two report the same
/// document. The action labels are shared verbatim too, including `FIX-DIR` for
/// the file-blocking-a-directory case, which `FIX-DIR` names in the direction the
/// action goes: the *file* is removed and a directory takes its place.
pub(crate) fn dry_run_records(
    plan: &Plan,
    renamed: usize,
    missing_only: bool,
    keep_extra: bool,
) -> Vec<Record> {
    let mut out = Vec::new();
    for r in &plan.fix_dirs {
        out.push(Record::path("FIX-DIR", r));
    }
    for r in &plan.mkdir {
        out.push(Record::path("MKDIR", r));
    }
    for r in &plan.delete_files {
        out.push(Record::path("DELETE", r));
    }
    for item in &plan.copy {
        out.push(Record::path("COPY", &item.rel).put("because", item.because.as_str()));
    }
    for r in &plan.rmdir {
        out.push(Record::path("RMDIR", r));
    }
    out.push(
        Summary {
            renamed,
            mkdir: plan.mkdir.len(),
            copied: plan.copy.len(),
            deleted: plan.delete_files.len(),
            rmdir: plan.rmdir.len(),
            missing_only,
            keep_extra,
            dry_run: true,
        }
        .record(),
    );
    out
}

/// The `SUMMARY` record for an applied run: the same fields and the same order as
/// [`dry_run_records`]'s, with the counts a real run reached rather than the ones
/// it planned.
pub(crate) fn summary_record(
    renamed: usize,
    plan: &Plan,
    applied: &Applied,
    missing_only: bool,
    keep_extra: bool,
) -> Record {
    Summary {
        renamed,
        mkdir: plan.mkdir.len(),
        copied: applied.copied,
        deleted: applied.deleted,
        rmdir: applied.removed_dirs,
        missing_only,
        keep_extra,
        dry_run: false,
    }
    .record()
}
