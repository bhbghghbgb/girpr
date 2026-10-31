//! The [`Plan`] work list and [`build_plan`], pure over the two effective maps.

use std::collections::{HashMap, HashSet};

use crate::diff::Diff;
use crate::effective::EffRec;

use super::link::link_ancestor;

/// Which difference bucket caused a path to be copied.
///
/// The three ways a path can be on both sides and still need a copy. Named rather
/// than spelled inline because the `COPY` record prints it: it is a **bucket**, not
/// the reason the bucket applies — the reason for a `CHANGED` lives on that path's
/// plan line, and repeating it on the action would put one fact in two places with
/// two vocabularies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyBecause {
    /// dst does not have it.
    Missing,
    /// Both have it and the content differs.
    Changed,
    /// Both have it and the **kind** differs, so the copy is over a blocking entry.
    TypeConflict,
}

impl CopyBecause {
    /// The value as it appears in `because=`, which is also the plan record's event
    /// name — the two must match for the join to work.
    pub fn as_str(&self) -> &'static str {
        match self {
            CopyBecause::Missing => "missing",
            CopyBecause::Changed => "changed",
            CopyBecause::TypeConflict => "type-conflict",
        }
    }
}

/// One planned copy: the path, and the bucket that caused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CopyItem {
    /// `/`-separated relative path with src's casing.
    pub rel: String,
    /// Which difference put this path on the copy list.
    pub because: CopyBecause,
}

/// A set of directories whose subtrees a run destroys, matched by ancestor.
///
/// Holds `/`-separated relative directory paths **without** a trailing slash.
/// Membership is by whole component, so `a` covers `a/b` but neither `a` itself
/// nor `ab/b`. The strictness is load-bearing: callers handle the directory at
/// the root of a wipe by name — a copy keeps its own row, a removal drops its
/// own row — so the test must never match that path itself.
#[derive(Debug, Default)]
pub(crate) struct DirPrefixes {
    dirs: HashSet<Box<str>>,
}

impl DirPrefixes {
    pub(crate) fn from_dirs<'a>(dirs: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            dirs: dirs.into_iter().map(Into::into).collect(),
        }
    }

    /// The dst directories this run's copies clear out of the way.
    ///
    /// `copy_one` removes a blocking dst directory *recursively*, so a copy
    /// whose dst is a directory takes that directory's whole subtree with it.
    pub(crate) fn wiped_by_copy(copy: &[CopyItem], dm: &HashMap<String, EffRec>) -> Self {
        Self::from_dirs(
            copy.iter()
                .filter(|item| dm.get(&item.rel).is_some_and(|rec| rec.kind == "dir"))
                .map(|item| item.rel.as_str()),
        )
    }

    pub(crate) fn insert(&mut self, dir: &str) {
        self.dirs.insert(dir.into());
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.dirs.is_empty()
    }

    /// True only for a strict descendant, never the directory itself.
    pub(crate) fn has_ancestor(&self, rel: &str) -> bool {
        rel.match_indices('/')
            .any(|(i, _)| self.dirs.contains(&rel[..i]))
    }
}

/// The complete work list for one sync, computed before anything is written.
///
/// Every list holds `/`-separated relative paths with src's casing.
#[derive(Debug, Default)]
pub(crate) struct Plan {
    /// Src directories absent from dst.
    pub mkdir: Vec<String>,
    /// Src files to copy over: missing, changed, or type-conflicting — each
    /// carrying the bucket that put it there.
    pub copy: Vec<CopyItem>,
    /// Dst-only files to delete (empty when `keep_extra`).
    ///
    /// A path under a dst directory link is replaced by the **link** here: the
    /// link is unlinked, and its target's files are never named. A link src also
    /// has is not listed at all. See `link_ancestor`.
    pub delete_files: Vec<String>,
    /// Type-conflicts where src is a dir and dst is a file, so the file is
    /// removed and a directory put in its place.
    pub fix_dirs: Vec<String>,
    /// Dst directories src does not have, deepest first (empty when
    /// `keep_extra`).
    ///
    /// Never a link or a directory under one: those are `delete_files`' business
    /// (or are left alone), because a recursive removal through a link would
    /// delete in its target.
    ///
    /// Computed here rather than by walking dst during the apply phase, so that
    /// the number is the same before anything is written as it is after. That is
    /// what lets the `--dry-run` summary carry a real `rmdir` count instead of
    /// omitting it — and it is the reason the apply phase reads this list rather
    /// than deriving one: two derivations of the same set is the drift this
    /// whole plan exists to avoid.
    pub rmdir: Vec<String>,
    /// Extra paths under a dst link that `src` also has, as `(link, example)`.
    ///
    /// Such a link is deliberately left in place (unlinking it would remove a
    /// path `src` mirrors into), but its extra descendants cannot be removed
    /// without reaching through it. A default run therefore refuses to call
    /// itself a mirror; `--keep-extra` and `--missing-only` explicitly accept
    /// the non-convergence. See [`ensure_link_safe`](super::safety::ensure_link_safe).
    pub shared_link_extras: Vec<(String, String)>,
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
pub(crate) fn build_plan(
    sm: &HashMap<String, EffRec>,
    dm: &HashMap<String, EffRec>,
    diff: &Diff,
    missing_only: bool,
    keep_extra: bool,
) -> Plan {
    let src_is_file = |rel: &String| sm.get(rel).map(|e| e.kind == "file").unwrap_or(false);
    let src_is_dir = |rel: &String| sm.get(rel).map(|e| e.kind == "dir").unwrap_or(false);
    let dst_is_file = |rel: &String| dm.get(rel).map(|e| e.kind == "file").unwrap_or(false);

    // Each bucket is pushed with itself attached, so the `COPY` record can say what
    // caused it without the reporter having to re-derive the origin from a path that
    // by now exists in three lists.
    let mut copy: Vec<CopyItem> = Vec::new();
    for r in &diff.missing {
        if src_is_file(r) {
            copy.push(CopyItem {
                rel: r.clone(),
                because: CopyBecause::Missing,
            });
        }
    }
    if !missing_only {
        for r in &diff.changed {
            if src_is_file(r) {
                copy.push(CopyItem {
                    rel: r.clone(),
                    because: CopyBecause::Changed,
                });
            }
        }
        // Where src is a file, copy_one clears a blocking dst dir out of the way.
        for r in &diff.type_conflict {
            if src_is_file(r) {
                copy.push(CopyItem {
                    rel: r.clone(),
                    because: CopyBecause::TypeConflict,
                });
            }
        }
    }
    // Sorted by path, then deduped **by path alone**.
    //
    // Correct because `missing`, `changed` and `type_conflict` are disjoint buckets
    // — a path cannot be on two of them at once — so no path ever arrives here
    // twice carrying two different reasons. Deduplicating on the whole item
    // instead would be equivalent *today* and would stop being equivalent the day
    // two buckets could overlap, which is exactly the kind of quiet assumption
    // that becomes a bug nobody can reproduce.
    copy.sort_by(|a, b| a.rel.cmp(&b.rel));
    copy.dedup_by(|a, b| a.rel == b.rel);

    let mut delete_files: Vec<String> = Vec::new();
    let mut shared_link_extras: Vec<(String, String)> = Vec::new();
    // Every link-extra path asks the same question of the same list, so answer it
    // once for the whole loop.
    let type_conflicts: HashSet<&str> = diff.type_conflict.iter().map(String::as_str).collect();
    if !keep_extra {
        for r in &diff.extra {
            // A path *under* a dst link resolves through it: `remove_file` on
            // `dst/link/x` removes the target's `x`, which may be outside `dst`
            // and may be a file src also has. The plan therefore names the
            // **topmost link** instead, and `delete_extras` unlinks that; the
            // link's descendants go with it.
            //
            // A link src also has is left alone. Both sides agree that path
            // exists, and removing dst's link would delete a directory src is
            // mirroring into. Its extra descendants are the target's files, and
            // deleting them is exactly the reach-through this guard exists to
            // stop - so they stay. A default run refuses to call that a mirror
            // (`shared_link_extras`); `--keep-extra`/`--missing-only` accept it.
            if let Some(link) = link_ancestor(r, dm) {
                // A type conflict *at* the link replaces it, which takes
                // the followed subtree with it: those extras are not left
                // behind, so this is not the non-convergence case below.
                let replaced = type_conflicts.contains(link);
                if sm.contains_key(link) && !replaced {
                    // Shared: removing the link would delete a directory `src`
                    // is mirroring into, and removing the descendant would reach
                    // through it. Record the non-convergence for
                    // `ensure_link_safe`; the apply passes never see this path.
                    shared_link_extras.push((link.to_string(), r.clone()));
                } else if !sm.contains_key(link) {
                    delete_files.push(link.to_string());
                }
                continue;
            }
            if dst_is_file(r) {
                delete_files.push(r.clone());
            }
            // extra dirs are handled by `rmdir` below, not here
        }
        // Two descendants of one link both name that link, so dedupe by path.
        delete_files.sort();
        delete_files.dedup();
    }

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
    // src has, and `delete_files` holds files and links (a link takes its
    // followed subtree, not a directory row). And `dm` is the post-rename map,
    // walked with the same depth cap and filters as a live walk would be, so
    // reading it instead of the filesystem yields the same paths.
    let mut rmdir: Vec<String> = Vec::new();
    if !keep_extra {
        let wiped = DirPrefixes::wiped_by_copy(&copy, dm);
        for (rel, rec) in dm {
            if rec.kind != "dir" || sm.contains_key(rel) {
                continue;
            }
            // A link, or any directory under one, is removed (or deliberately
            // kept) through `delete_files`: `remove_dir_all(dst/link/sub)` would
            // delete in the link target. See the delete-files loop above.
            if link_ancestor(rel, dm).is_some() {
                continue;
            }
            if wiped.has_ancestor(rel) {
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
        shared_link_extras,
    }
}
