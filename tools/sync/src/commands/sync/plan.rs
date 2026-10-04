//! Turning the src/dst diff into a concrete work list.
//!
//! [`build_plan`] is pure — it only reads the two effective maps — which is
//! what makes `--dry-run` trustworthy: the plan it prints is byte-for-byte the
//! plan a real run would execute.

use std::collections::HashMap;

use super::apply::Applied;
use crate::diff::Diff;
use crate::effective::EffRec;
use crate::report::Record;

/// The complete work list for one sync, computed before anything is written.
///
/// Every list holds `/`-separated relative paths with src's casing.
#[derive(Debug, Default)]
pub(super) struct Plan {
    /// Src directories absent from dst.
    pub mkdir: Vec<String>,
    /// Src files to copy over: missing, changed, or type-conflicting.
    pub copy: Vec<String>,
    /// Dst-only files to delete (empty when `keep_extra`).
    pub delete_files: Vec<String>,
    /// Type-conflicts where src is a dir and dst is a file, so the file is
    /// removed and a directory put in its place.
    pub fix_dirs: Vec<String>,
    /// Dst directories src does not have, deepest first (empty when
    /// `keep_extra`).
    ///
    /// Computed here rather than by walking dst during the apply phase, so that
    /// the number is the same before anything is written as it is after. That is
    /// what lets the `--dry-run` summary carry a real `rmdir` count instead of
    /// omitting it — and it is the reason the apply phase reads this list rather
    /// than deriving one: two derivations of the same set is the drift this
    /// whole plan exists to avoid.
    pub rmdir: Vec<String>,
}

impl Plan {
    /// Total planned removals, for the `SUMMARY` record.
    pub fn deletes(&self) -> usize {
        self.delete_files.len() + self.fix_dirs.len()
    }
}

/// Decide what to do about each difference between `sm` (src) and `dm` (dst).
///
/// `diff` must have been computed case-*sensitively*: the rename pass runs
/// first so the two maps share exact keys by this point.
pub(super) fn build_plan(
    sm: &HashMap<String, EffRec>,
    dm: &HashMap<String, EffRec>,
    diff: &Diff,
    missing_only: bool,
    keep_extra: bool,
) -> Plan {
    let src_is_file = |rel: &String| sm.get(rel).map(|e| e.kind == "file").unwrap_or(false);
    let src_is_dir = |rel: &String| sm.get(rel).map(|e| e.kind == "dir").unwrap_or(false);
    let dst_is_file = |rel: &String| dm.get(rel).map(|e| e.kind == "file").unwrap_or(false);
    let dst_is_dir = |rel: &String| dm.get(rel).map(|e| e.kind == "dir").unwrap_or(false);

    let mut copy: Vec<String> = Vec::new();
    for r in &diff.missing {
        if src_is_file(r) {
            copy.push(r.clone());
        }
    }
    if !missing_only {
        for r in &diff.changed {
            if src_is_file(r) {
                copy.push(r.clone());
            }
        }
        // Where src is a file, copy_one clears a blocking dst dir out of the way.
        for r in &diff.type_conflict {
            if src_is_file(r) {
                copy.push(r.clone());
            }
        }
    }
    copy.sort();
    copy.dedup();

    let mut delete_files: Vec<String> = Vec::new();
    if !keep_extra {
        for r in &diff.extra {
            if dst_is_file(r) {
                delete_files.push(r.clone());
            }
            // extra dirs are handled by `rmdir` below, not here
        }
    }
    // `delete_files` and `fix_dirs` need no explicit sort: both are derived
    // from a single pass over an already-sorted, duplicate-free diff bucket.

    let mut fix_dirs: Vec<String> = Vec::new();
    for r in &diff.type_conflict {
        if src_is_dir(r) {
            fix_dirs.push(r.clone());
        }
    }

    let mut mkdir: Vec<String> = Vec::new();
    for (rel, rec) in sm {
        if rec.kind == "dir" && !dm.contains_key(rel) {
            mkdir.push(rel.clone());
        }
    }
    mkdir.sort();

    // Dst directories src does not have. `keep_extra` spares dirs as well as
    // files — the flag's name is not a promise about files only, and a
    // `--keep-extra` run that quietly collected empty directories was deleting
    // the trees the user asked it to keep.
    //
    // The one thing that changes dst's directory set before this pass runs is
    // `copy_one` clearing a blocking directory out of the way, which it does
    // *recursively*. So a planned copy landing on a dst directory takes that
    // directory's whole subtree with it, and those directories are not here at
    // apply time however much they were going to be. Counting them would
    // promise removals that cannot happen, and would make the dry run report an
    // `rmdir` the real run cannot reach.
    //
    // Nothing else moves the set: `mkdir` and `fix_dirs` both create directories
    // src has, and `delete_files` holds files only. And `dm` is the post-rename
    // map, walked with the same depth cap and filters as a live walk would be,
    // so reading it instead of the filesystem yields the same paths.
    let mut rmdir: Vec<String> = Vec::new();
    if !keep_extra {
        let wiped: Vec<String> = copy
            .iter()
            .filter(|r| dst_is_dir(r))
            .map(|r| format!("{r}/"))
            .collect();
        for (rel, rec) in dm {
            if rec.kind != "dir" || sm.contains_key(rel) {
                continue;
            }
            if wiped.iter().any(|w| rel.starts_with(w.as_str())) {
                continue;
            }
            rmdir.push(rel.clone());
        }
        // Deepest first: the removal pass must take a child before its parent.
        rmdir.sort_by_key(|s| std::cmp::Reverse(s.len()));
    }

    Plan {
        mkdir,
        copy,
        delete_files,
        fix_dirs,
        rmdir,
    }
}

/// The `SUMMARY` record's fields, in both modes.
///
/// One struct, one field list, one [`Record`] constructor — so a dry run and a
/// real run cannot disagree about the *vocabulary*, only about the numbers and
/// `dry_run`. That is the whole of the `--dry-run` reporting contract: same field
/// names, same order, same meaning. It was previously written out twice, and the
/// two lists had in fact drifted (`copy`/`delete` against `copied`/`deleted`,
/// with `rmdir` missing from one side), which is exactly the class of bug a
/// duplicated field list invites.
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
/// [`a_dry_run_reports_what_a_real_run_reports`]: ../../sync_plan.rs
///
/// Which path gets which action is not pinned here as a transcript. It is asserted
/// per path by the fixture table in `tests/sync_plan.rs`, which also derives the
/// summary counts from the plan rather than stating them.
///
/// `rmdir` is exact rather than omitted: the apply phase reads [`Plan::rmdir`]
/// instead of walking dst, so the count is the same before anything is written as
/// it is after.
///
/// The records are in **apply order** — mkdirs, fix-dirs, deletes, copies,
/// rmdir — rather than grouped by plan category, so the sequence a dry run
/// reports is the sequence a real run will perform, and the two report the same
/// document. The action labels are shared verbatim too, including `FIX-DIR` for
/// the file-blocking-a-directory case, which this used to spell `RMDIR-FILE`:
/// that name reads as "remove a directory that is a file", which is backwards —
/// the *file* goes and a directory takes its place.
pub(super) fn dry_run_records(
    plan: &Plan,
    renamed: usize,
    missing_only: bool,
    keep_extra: bool,
) -> Vec<Record> {
    let mut out = Vec::new();
    for r in &plan.mkdir {
        out.push(Record::path("MKDIR", r));
    }
    for r in &plan.fix_dirs {
        out.push(Record::path("FIX-DIR", r));
    }
    for r in &plan.delete_files {
        out.push(Record::path("DELETE", r));
    }
    for r in &plan.copy {
        out.push(Record::path("COPY", r));
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
pub(super) fn summary_record(
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
