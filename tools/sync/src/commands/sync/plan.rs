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
            // extra dirs are removed by the unknown-dirs pass, not here
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

    Plan {
        mkdir,
        copy,
        delete_files,
        fix_dirs,
    }
}

/// Print the plan as `MKDIR`/`COPY`/`DELETE`/`RMDIR-FILE` lines plus a
/// `SUMMARY`, for `--dry-run`. Writes nothing.
pub(super) fn print_dry_run(plan: &Plan, renamed: usize, missing_only: bool, keep_extra: bool) {
    for r in &plan.mkdir {
        println!("MKDIR {}", r);
    }
    for r in &plan.copy {
        println!("COPY {}", r);
    }
    for r in &plan.delete_files {
        println!("DELETE {}", r);
    }
    for r in &plan.fix_dirs {
        println!("RMDIR-FILE {}", r);
    }
    println!(
        "SUMMARY renamed={} mkdir={} copy={} delete={} missing_only={} keep_extra={} dry_run=true",
        renamed,
        plan.mkdir.len(),
        plan.copy.len(),
        plan.deletes(),
        missing_only,
        keep_extra
    );
}
