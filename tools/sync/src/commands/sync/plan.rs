//! Turning the src/dst diff into a concrete work list.
//!
//! [`build_plan`] is pure — it only reads the two effective maps — which is
//! what makes `--dry-run` trustworthy: the plan it prints is byte-for-byte the
//! plan a real run would execute.

use std::collections::HashMap;

use crate::diff::Diff;
use crate::effective::EffRec;

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
    /// Total planned removals, for the `SUMMARY` line.
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

/// Print the plan as `MKDIR`/`FIX-DIR`/`DELETE`/`COPY`/`RMDIR` lines plus a
/// `SUMMARY`, for `--dry-run`. Writes nothing.
///
/// The `SUMMARY` uses the **same field names as a real run**, so the two differ
/// only by the trailing `dry_run=true`. That is the whole point of the flag: it
/// promises the same observable result, and a caller reading either should not
/// have to know which one it got. The names had been `copy`/`delete` against a
/// real run's `copied`/`deleted`, which described the *plan* rather than the
/// result — the numbers agree whenever a real run completes, since a failed copy
/// aborts before the summary is printed, so the distinction bought nothing and
/// cost a parser.
///
/// `rmdir` is likewise present in both, and is exact rather than omitted: the
/// apply phase reads [`Plan::rmdir`] instead of walking dst, so the count is the
/// same before anything is written as it is after.
///
/// The lines are printed in **apply order** — mkdirs, fix-dirs, deletes, copies,
/// rmdir — rather than grouped by plan category, so the sequence a dry run
/// reports is the sequence a real run will perform, and the two print the same
/// document. The action labels are shared verbatim too, including `FIX-DIR` for
/// the file-blocking-a-directory case, which this printer used to spell
/// `RMDIR-FILE`: that name reads as "remove a directory that is a file", which is
/// backwards — the *file* goes and a directory takes its place.
///
/// `run_sync_dry_run_summary_matches_a_real_run` compares the whole stdout of the
/// two modes with only the `dry_run=true` marker normalised away, so any future
/// divergence in labels, ordering or counts fails there rather than being
/// discovered by a user.
pub(super) fn print_dry_run(plan: &Plan, renamed: usize, missing_only: bool, keep_extra: bool) {
    for r in &plan.mkdir {
        println!("MKDIR {}", r);
    }
    for r in &plan.fix_dirs {
        println!("FIX-DIR {}", r);
    }
    for r in &plan.delete_files {
        println!("DELETE {}", r);
    }
    for r in &plan.copy {
        println!("COPY {}", r);
    }
    for r in &plan.rmdir {
        println!("RMDIR {}", r);
    }
    println!(
        "SUMMARY renamed={} mkdir={} copied={} deleted={} rmdir={} missing_only={} keep_extra={} dry_run=true",
        renamed,
        plan.mkdir.len(),
        plan.copy.len(),
        plan.delete_files.len(),
        plan.rmdir.len(),
        missing_only,
        keep_extra
    );
}
