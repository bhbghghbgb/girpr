//! Turning the src/dst diff into a concrete work list.
//!
//! [`build_plan`] is pure — it only reads the two effective maps — which is
//! what makes `--dry-run` trustworthy: the plan it prints is byte-for-byte the
//! plan a real run would execute.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;

use anyhow::{Context, Result, bail};

use super::apply::Applied;
use crate::diff::Diff;
use crate::effective::EffRec;
use crate::report::Record;

/// Which difference bucket caused a path to be copied.
///
/// The three ways a path can be on both sides and still need a copy. Named rather
/// than spelled inline because the `COPY` record prints it: it is a **bucket**, not
/// the reason the bucket applies — the reason for a `CHANGED` lives on that path's
/// plan line, and repeating it on the action would put one fact in two places with
/// two vocabularies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CopyBecause {
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
pub(super) struct CopyItem {
    /// `/`-separated relative path with src's casing.
    pub rel: String,
    /// Which difference put this path on the copy list.
    pub because: CopyBecause,
}

/// The complete work list for one sync, computed before anything is written.
///
/// Every list holds `/`-separated relative paths with src's casing.
#[derive(Debug, Default)]
pub(super) struct Plan {
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
    /// the non-convergence. See [`ensure_link_safe`].
    pub shared_link_extras: Vec<(String, String)>,
}

impl Plan {
    /// Total planned removals, for the `SUMMARY` record.
    pub fn deletes(&self) -> usize {
        self.delete_files.len() + self.fix_dirs.len()
    }
}

/// The shortest prefix of `rel` (itself included) that `dm` reports as a link.
///
/// The map is the dst walk's own answer, so this needs no filesystem access and
/// the dry run's plan is computed from the same facts the real run has. A prefix
/// missing from `dm` cannot be a link the walk saw; `delete_extras` re-checks on
/// disk before it removes anything, which covers a difference between the walk
/// and the apply phase.
fn link_ancestor<'a>(rel: &str, dm: &'a HashMap<String, EffRec>) -> Option<&'a str> {
    let mut end = 0;
    loop {
        let (prefix, next) = match rel[end..].find('/') {
            Some(i) => (&rel[..end + i], Some(end + i + 1)),
            None => (rel, None),
        };
        if let Some((key, rec)) = dm.get_key_value(prefix)
            && rec.is_link
        {
            return Some(key.as_str());
        }
        end = next?;
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
                // A type conflict *at* the link replaces it (F-2), which takes
                // the followed subtree with it: those extras are not left
                // behind, so this is not the non-convergence case below.
                let replaced = diff.type_conflict.iter().any(|t| t.as_str() == link);
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
        let wiped: Vec<String> = copy
            .iter()
            .filter(|item| dst_is_dir(&item.rel))
            .map(|item| format!("{}/", item.rel))
            .collect();
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
        shared_link_extras,
    }
}

/// Refuse to mirror through a `dst` link, before any apply phase changes
/// anything.
///
/// The planner resolves an extra descendant to the topmost link, but writes and
/// type-conflict fixes under a link are a different question: a shared link
/// (one `src` also has) is deliberately left in place, and `mkdir`/`fix_dirs`/
/// `copy` would then resolve the link as an intermediate path component. That
/// writes into the link target — possibly outside `dst` — and used to delete
/// external data while reporting success. The same applies to a rename, which
/// `rename_to_src_casing` guards before its first filesystem call.
///
/// This is a **fail-fast** policy: remove the link from `dst`, or exclude it
/// (`--exclude link`), and rerun. `--keep-extra` and `--missing-only` accept
/// non-convergence for *extras* under a shared link, but they do not license a
/// write through one.
///
/// The check asks the filesystem rather than `dm` on purpose: `dm` is the
/// filtered, depth-capped view the plan was computed from, while the disk is
/// what the apply phase will actually traverse.
pub(super) fn ensure_link_safe(
    plan: &Plan,
    dst: &Path,
    missing_only: bool,
    keep_extra: bool,
) -> Result<()> {
    let fixed: HashSet<&str> = plan.fix_dirs.iter().map(String::as_str).collect();
    let actions = plan
        .mkdir
        .iter()
        .chain(plan.copy.iter().map(|item| &item.rel))
        .chain(plan.fix_dirs.iter())
        .chain(plan.delete_files.iter())
        .chain(plan.rmdir.iter());
    for rel in actions {
        if let Some(link) = crate::link::link_ancestor_above(dst, rel)? {
            // A strict link ancestor that is itself a planned `fix_dirs` entry is
            // removed *before* the nested action runs (`fix_dirs` precedes
            // `mkdir`/`copy`), so the write resolves an ordinary directory. This
            // is the same carve-out `ensure_mkdir_targets_free` and
            // `ensure_dirs_applicable` grant a blocking file: the guards must
            // describe the same plan, and only a link that survives the fix is a
            // write-through. `link_ancestor_above` returns the topmost link, so a
            // link above the fixed one would have been returned first and is
            // still refused.
            if fixed.contains(link.as_str()) {
                continue;
            }
            bail!(
                "refusing to change `{rel}` through dst link `{link}`: \
                 remove the link, or exclude it with --exclude"
            );
        }
    }
    if !missing_only && !keep_extra {
        if let Some((link, path)) = plan.shared_link_extras.first() {
            bail!(
                "dst link `{link}` is shared with src and has extra paths under it \
                 (for example `{path}`); sync cannot converge without deleting through \
                 the link: remove the link, or pass --keep-extra"
            );
        }
    } else if let Some((link, path)) = plan.shared_link_extras.first() {
        tracing::warn!(
            link = %link,
            path = %path,
            "leaving extras under a shared dst link; this run does not converge"
        );
    }
    Ok(())
}

/// Refuse a plan whose paths cannot be created after `fix_dirs` has run.
///
/// The apply order puts `fix_dirs` first, so a strict ancestor that dst holds as
/// a **file** is fine exactly when that ancestor is itself a planned `fix_dirs`
/// entry: the fix removes the file and puts a directory there, and the nested
/// `mkdir`/`copy` then has a parent. A file ancestor that is *not* in
/// `fix_dirs` would make the action fail with `AlreadyExists` after earlier
/// actions had already landed.
///
/// Today's planner cannot produce that shape: a file ancestor of a src path is
/// by construction a type conflict (src has the directory, dst has the file), so
/// it is in `fix_dirs`. This is a plan-time assertion rather than a second
/// derivation of the plan: if a later planner change stops holding that
/// property, the run must fail before the first filesystem change instead of
/// halfway through.
///
/// Pure, like [`build_plan`]: it reads only the plan and dst's effective map, so
/// a dry run refuses the same plan a real run would.
pub(super) fn ensure_dirs_applicable(plan: &Plan, dm: &HashMap<String, EffRec>) -> Result<()> {
    let fixed: HashSet<&str> = plan.fix_dirs.iter().map(String::as_str).collect();
    let actions = plan
        .mkdir
        .iter()
        .chain(plan.copy.iter().map(|item| &item.rel))
        .chain(plan.fix_dirs.iter());
    for rel in actions {
        let mut end = 0;
        while let Some(i) = rel[end..].find('/') {
            let prefix = &rel[..end + i];
            if dm.get(prefix).is_some_and(|rec| rec.kind == "file") && !fixed.contains(prefix) {
                bail!(
                    "plan cannot create `{rel}`: dst holds `{prefix}` as a file and it is \
                     not a planned type-conflict fix"
                );
            }
            end += i + 1;
        }
    }
    Ok(())
}

/// Refuse, in both modes, a planned copy whose destination would still be
/// hard-linked when the copy phase truncates it.
///
/// [`copy_one`](super::apply) refuses a destination with more than one name, but
/// that check lives in the apply phase: a dry run used to report `COPY` and exit
/// 0 for a tree the real run refused with exit 3. This is the plan-time
/// counterpart, reading the same disk state both modes read, so the two agree
/// before either prints an action.
///
/// **Only a regular file is judged.** A directory at a copy destination is a
/// type conflict that the copy phase removes before writing, so its inode is
/// not truncated; on Unix its `nlink` is `2 + subdirectories`, and reading that
/// as a shared file would refuse every file-over-directory conflict with a
/// diagnostic about a hard link that does not exist. A file link is still
/// judged through its target: `metadata` follows the final component, which is
/// exactly the inode the write-through would truncate.
///
/// A destination is allowed when names the plan removes **before** the copy
/// phase account for every extra one. `delete_files` and `fix_dirs` both run
/// first and both remove an ordinary file name, which reduces the inode's count.
/// A `delete_files` entry that is a **link** does not count: unlinking a link
/// removes the link, not the inode it points at. `rmdir` runs after the copy and
/// cannot help.
///
/// The default-run shape is exactly this: a dst file hard-linked to a dst-only
/// file is refused only when `--keep-extra` leaves that second name in place;
/// without it, the delete phase removes the other name first and the copy is
/// safe.
///
/// Conservative where it cannot tell: a stat error other than `NotFound` is
/// returned, and `NotFound` means there is no inode to truncate.
pub(super) fn ensure_hard_linked_targets_safe(
    plan: &Plan,
    dm: &HashMap<String, EffRec>,
    dst: &Path,
) -> Result<()> {
    // Ordinary-file names this run removes before copying, by the inode they
    // name. A link is skipped: its removal leaves the target's count unchanged.
    let mut removed: HashMap<crate::hardlink::Identity, u64> = HashMap::new();
    for rel in plan.delete_files.iter().chain(plan.fix_dirs.iter()) {
        if dm
            .get(rel)
            .is_none_or(|rec| rec.kind != "file" || rec.is_link)
        {
            continue;
        }
        let p = dst.join(rel);
        match crate::hardlink::info(&p) {
            Ok(info) => *removed.entry(info.id).or_default() += 1,
            // Gone already, so this plan entry removes no name at copy time.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e).with_context(|| format!("hard-link count {}", p.display()));
            }
        }
    }
    for item in &plan.copy {
        let d = dst.join(&item.rel);
        let info = match crate::hardlink::info(&d) {
            Ok(info) => info,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(e).with_context(|| format!("hard-link count {}", d.display()));
            }
        };
        // A directory here is a planned type conflict, not a shared file. On
        // Unix its link count is at least 2 by construction, so this test is
        // what keeps that from becoming a bogus refusal.
        if !info.regular_file || info.links <= 1 {
            continue;
        }
        let credited = removed.get(&info.id).copied().unwrap_or(0);
        if credited >= info.links - 1 {
            continue;
        }
        bail!(
            "refusing to copy `{}`: dst `{}` has {} hard links and the plan removes only \
             {credited} of the other names before copying; truncating it would change \
             another path that shares the file (possibly one inside src); remove the \
             extra link or replace the file, then rerun",
            item.rel,
            d.display(),
            info.links
        );
    }
    Ok(())
}

/// Refuse, before the rename pass, a src directory that dst holds on disk as
/// something other than a directory.
///
/// `--include`/`--exclude` hide entries from the effective dst map but not from
/// the disk. A filtered-out file at a `mkdir` path is therefore invisible to
/// [`build_plan`] and to [`ensure_dirs_applicable`], both of which read `dm`: the
/// plan says `MKDIR`, the dry run prints it as applicable, and `create_dir_all`
/// then fails with `AlreadyExists` after earlier phases have changed dst.
///
/// This runs before [`super::rename`] because a case-only rename is a filesystem
/// change, and a refusal must not follow one. It cannot read the plan — that is
/// built after the rename — so it derives the same mkdir candidates
/// [`build_plan`] will: src directories the post-rename `dm` will not hold. In
/// insensitive mode that is a lowercase match, because the rename pass collapses
/// those pairs; a same-path dst **file** is then a planned `fix_dirs` entry, not
/// a mkdir, and is allowed.
///
/// Every strict ancestor of a src directory is itself a src directory, so the
/// blocker is normally its own candidate. A prefix that is not (a file in `dm`
/// while the disk agrees) is a planned type conflict for the same reason, and the
/// map-based [`ensure_dirs_applicable`] remains the later assertion for it.
pub(super) fn ensure_mkdir_targets_free(
    sm: &HashMap<String, EffRec>,
    dm: &HashMap<String, EffRec>,
    dst: &Path,
    case_sensitive: bool,
) -> Result<()> {
    // The dst view the plan will see after the rename: exact keys in sensitive
    // mode, lowercase pairs otherwise.
    let dst_lower: HashMap<String, &EffRec> = if case_sensitive {
        HashMap::new()
    } else {
        dm.iter().map(|(k, v)| (k.to_lowercase(), v)).collect()
    };
    let paired = |rel: &str| dst_after_rename(rel, dm, &dst_lower, case_sensitive);
    // A src directory the plan will fix rather than mkdir, because dst holds a
    // file (or a file link) at the same path after the rename.
    let fixed: HashSet<&str> = sm
        .iter()
        .filter(|(rel, rec)| rec.kind == "dir" && paired(rel).is_some_and(|dst| dst.kind == "file"))
        .map(|(rel, _)| rel.as_str())
        .collect();
    for (rel, rec) in sm {
        if rec.kind != "dir" || paired(rel).is_some() {
            continue;
        }
        // Shortest prefix first, so the message names the entry that actually
        // blocks the path rather than the action underneath it.
        let mut end = 0;
        loop {
            let (prefix, next) = match rel[end..].find('/') {
                Some(i) => (&rel[..end + i], Some(end + i + 1)),
                None => (rel.as_str(), None),
            };
            match disk_is_dir(&dst.join(prefix))? {
                // Existing directory (including a link to one): create_dir_all
                // accepts it.
                Some(true) => {}
                // A file the plan will replace before the nested mkdir runs.
                Some(false) if fixed.contains(prefix) => {}
                Some(false) => bail!(
                    "cannot create `{rel}`: dst holds `{prefix}` as a file and it is not \
                     a planned type-conflict fix (a filter can hide it from the dst map); \
                     remove it, or include it so the conflict can be resolved"
                ),
                None => {}
            }
            match next {
                Some(next) => end = next,
                None => break,
            }
        }
    }
    Ok(())
}

/// The dst entry a src path will pair with after the rename pass.
fn dst_after_rename<'a>(
    rel: &str,
    dm: &'a HashMap<String, EffRec>,
    lower: &'a HashMap<String, &'a EffRec>,
    case_sensitive: bool,
) -> Option<&'a EffRec> {
    if case_sensitive {
        dm.get(rel)
    } else {
        lower.get(&rel.to_lowercase()).copied()
    }
}

/// `Some(is_dir)` when `path` exists, judged the way `create_dir_all` judges it.
///
/// A link is followed: a link to a directory is an existing directory, while a
/// dangling link (or one to a file) is not. `None` means there is no entry, which
/// is also fine. Every other error is returned rather than read as "not there".
fn disk_is_dir(path: &Path) -> Result<Option<bool>> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("stat {}", path.display())),
        Ok(_) => match std::fs::metadata(path) {
            Ok(md) => Ok(Some(md.is_dir())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Some(false)),
            Err(e) => Err(e).with_context(|| format!("stat {}", path.display())),
        },
    }
}

/// The `SUMMARY` record's fields, in both modes.
///
/// One struct, one field list, one [`Record`] constructor — so a dry run and a
/// real run cannot disagree about the *vocabulary*, only about the numbers and
/// `dry_run`. That is the whole of the `--dry-run` reporting contract: same field
/// names, same order, same meaning. A duplicated field list would make that a
/// property of two lists agreeing rather than of one list existing.
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
pub(super) fn dry_run_records(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::DigestSet;

    fn file() -> EffRec {
        EffRec {
            kind: "file".into(),
            size: 1,
            mtime_ns: 0,
            hashes: DigestSet::default(),
            is_link: false,
        }
    }

    fn dir() -> EffRec {
        EffRec {
            kind: "dir".into(),
            size: 0,
            mtime_ns: 0,
            hashes: DigestSet::default(),
            is_link: false,
        }
    }

    fn plan(mkdir: &[&str], copy: &[&str], fix_dirs: &[&str]) -> Plan {
        Plan {
            mkdir: mkdir.iter().map(|s| s.to_string()).collect(),
            copy: copy
                .iter()
                .map(|s| CopyItem {
                    rel: s.to_string(),
                    because: CopyBecause::Missing,
                })
                .collect(),
            fix_dirs: fix_dirs.iter().map(|s| s.to_string()).collect(),
            ..Plan::default()
        }
    }

    /// The check exists for a planner regression, so it has to be able to fail:
    /// a file ancestor that no `fix_dirs` entry replaces makes every action
    /// below it unexecutable in apply order.
    #[test]
    fn a_file_ancestor_that_is_not_a_planned_fix_refuses_the_plan() {
        let mut dm = HashMap::new();
        dm.insert("a".to_string(), file());
        let p = plan(&["a/b"], &["a/b/x.txt"], &[]);

        let err = ensure_dirs_applicable(&p, &dm)
            .expect_err("a blocking file ancestor with no fix is not applicable");
        let msg = err.to_string();
        assert!(msg.contains("a/b"), "names the action path: {msg}");
        assert!(msg.contains("`a`"), "names the blocking ancestor: {msg}");
    }

    /// The shape today's planner produces: the file ancestor is the type
    /// conflict, `fix_dirs` replaces it first, and the nested mkdir/copy then
    /// has a parent. Refusing this would break every nested type conflict.
    #[test]
    fn a_file_ancestor_that_fix_dirs_replaces_is_applicable() {
        let mut dm = HashMap::new();
        dm.insert("a".to_string(), file());
        let p = plan(&["a/b"], &["a/b/x.txt"], &["a"]);

        ensure_dirs_applicable(&p, &dm).expect("fix-dirs runs before mkdir and copy");
    }

    /// **The P6-1 shape.** `foo` is absent from `dm` because `--include` hid the
    /// dst file, so the map-based guard cannot see the blocker. The disk can.
    #[test]
    fn a_filtered_file_at_a_mkdir_path_is_refused_from_disk() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "filtered_blocker");
        let dst = t.subdir("dst");
        std::fs::write(dst.join("foo"), b"blocker").unwrap();
        let mut sm = HashMap::new();
        sm.insert("foo".to_string(), dir());
        sm.insert("foo/bar.txt".to_string(), file());
        let dm = HashMap::new();

        let err = ensure_mkdir_targets_free(&sm, &dm, &dst, false)
            .expect_err("a filtered dst file blocks the mkdir and the copy under it");
        let msg = err.to_string();
        assert!(msg.contains("`foo`"), "names the blocker: {msg}");
        assert!(
            msg.contains("cannot create `foo`"),
            "names the mkdir: {msg}"
        );
    }

    /// An existing directory is what `create_dir_all` wants, so it is not a
    /// blocker even though the filtered `dm` does not know about it.
    #[test]
    fn an_existing_filtered_directory_is_not_a_blocker() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "filtered_dir");
        let dst = t.subdir("dst");
        std::fs::create_dir_all(dst.join("foo")).unwrap();
        let mut sm = HashMap::new();
        sm.insert("foo".to_string(), dir());
        sm.insert("foo/bar.txt".to_string(), file());

        ensure_mkdir_targets_free(&sm, &HashMap::new(), &dst, false)
            .expect("create_dir_all is a no-op on an existing directory");
    }

    /// In insensitive mode the rename pass pairs dst's `foo` with src's `Foo`,
    /// and the plan turns the same-path kind difference into a `fix_dirs` entry.
    /// The preflight must use the same pairing, not the exact key.
    #[test]
    fn a_case_paired_file_is_a_planned_fix_not_a_mkdir() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "case_pair_fix");
        let dst = t.subdir("dst");
        std::fs::write(dst.join("foo"), b"blocker").unwrap();
        let mut sm = HashMap::new();
        sm.insert("Foo".to_string(), dir());
        let mut dm = HashMap::new();
        dm.insert("foo".to_string(), file());

        ensure_mkdir_targets_free(&sm, &dm, &dst, false)
            .expect("rename then fix-dirs resolves the pair");
    }

    /// The strict-prefix half: `a/b` is the candidate, and `a` is a file on disk
    /// that `dm` misreports as a directory. The plan would `mkdir a/b` and fail,
    /// because `a` is not a `fix_dirs` entry.
    #[test]
    fn a_file_at_a_strict_prefix_is_refused_from_disk() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "prefix_blocker");
        let dst = t.subdir("dst");
        std::fs::write(dst.join("a"), b"blocker").unwrap();
        let mut sm = HashMap::new();
        sm.insert("a".to_string(), dir());
        sm.insert("a/b".to_string(), dir());
        let mut dm = HashMap::new();
        dm.insert("a".to_string(), dir());

        let err = ensure_mkdir_targets_free(&sm, &dm, &dst, false)
            .expect_err("the prefix is a file on disk and is not a planned fix");
        let msg = err.to_string();
        assert!(msg.contains("`a`"), "names the blocking prefix: {msg}");
        assert!(msg.contains("a/b"), "names the action: {msg}");
    }

    /// A planned fix is exactly the licence a strict prefix gets: `fix_dirs`
    /// removes the file and creates the directory before `mkdir a/b` runs.
    #[test]
    fn a_planned_fix_at_a_strict_prefix_is_applicable() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "prefix_fix");
        let dst = t.subdir("dst");
        std::fs::write(dst.join("a"), b"blocker").unwrap();
        let mut sm = HashMap::new();
        sm.insert("a".to_string(), dir());
        sm.insert("a/b".to_string(), dir());
        let mut dm = HashMap::new();
        dm.insert("a".to_string(), file());

        ensure_mkdir_targets_free(&sm, &dm, &dst, false)
            .expect("fix-dirs replaces `a` before the nested mkdir");
    }

    /// **P8-1.** A directory at a copy destination is a planned type conflict:
    /// the copy phase removes it, so its inode is never truncated. On Unix its
    /// `nlink` is `2 + subdirectories` (the fixture here makes it 2), which the
    /// old guard read as a hard link and refused with a bogus message.
    #[test]
    fn a_directory_copy_destination_is_not_a_hard_link_hazard() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "hardlink_dir");
        let dst = t.subdir("dst");
        std::fs::create_dir_all(dst.join("node/inner")).unwrap();
        let mut dm = HashMap::new();
        dm.insert("node".to_string(), dir());
        let p = plan(&[], &["node"], &[]);

        ensure_hard_linked_targets_safe(&p, &dm, &dst)
            .expect("a directory at the copy destination is removed, not truncated");
    }

    /// Two dst names for one file, and neither is planned for removal: the copy
    /// would truncate both. `copy_one` refuses this in a real run; the preflight
    /// must refuse it in a dry run too, or the two modes answer differently.
    #[test]
    fn a_hard_linked_copy_target_with_no_planned_removal_is_refused() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "hardlink_refused");
        let dst = t.subdir("dst");
        std::fs::write(dst.join("a.txt"), b"shared").unwrap();
        std::fs::hard_link(dst.join("a.txt"), dst.join("shared.txt")).unwrap();
        let mut dm = HashMap::new();
        dm.insert("a.txt".to_string(), file());
        dm.insert("shared.txt".to_string(), file());
        let p = plan(&[], &["a.txt"], &[]);

        let err = ensure_hard_linked_targets_safe(&p, &dm, &dst)
            .expect_err("a name the run keeps makes the truncation unsafe");
        let msg = err.to_string();
        assert!(msg.contains("hard links"), "names the hazard: {msg}");
        assert!(msg.contains("a.txt"), "names the copy: {msg}");
    }

    /// The default-run shape, and the reason the preflight cannot refuse every
    /// count above one: `shared.txt` is dst-only, the delete phase removes it,
    /// and the copy then truncates the only remaining name.
    #[test]
    fn deleting_the_other_name_makes_the_hard_linked_copy_safe() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "hardlink_deleted");
        let dst = t.subdir("dst");
        std::fs::write(dst.join("a.txt"), b"shared").unwrap();
        std::fs::hard_link(dst.join("a.txt"), dst.join("shared.txt")).unwrap();
        let mut dm = HashMap::new();
        dm.insert("a.txt".to_string(), file());
        dm.insert("shared.txt".to_string(), file());
        let mut p = plan(&[], &["a.txt"], &[]);
        p.delete_files.push("shared.txt".to_string());

        ensure_hard_linked_targets_safe(&p, &dm, &dst)
            .expect("the delete phase runs before the copy and removes the other name");
    }

    /// A delete of an unrelated inode is no credit: identity, not the number of
    /// planned removals, is what makes the other name go away.
    #[test]
    fn deleting_an_unrelated_file_does_not_make_the_copy_safe() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "hardlink_unrelated");
        let dst = t.subdir("dst");
        std::fs::write(dst.join("a.txt"), b"shared").unwrap();
        std::fs::hard_link(dst.join("a.txt"), dst.join("shared.txt")).unwrap();
        std::fs::write(dst.join("other.txt"), b"other").unwrap();
        let mut dm = HashMap::new();
        dm.insert("a.txt".to_string(), file());
        dm.insert("shared.txt".to_string(), file());
        dm.insert("other.txt".to_string(), file());
        let mut p = plan(&[], &["a.txt"], &[]);
        p.delete_files.push("other.txt".to_string());

        ensure_hard_linked_targets_safe(&p, &dm, &dst)
            .expect_err("deleting a different file leaves both names of this inode");
    }

    /// `fix_dirs` runs before the copy phase too, so a blocking file it replaces
    /// reduces the shared count just as a delete does.
    #[test]
    fn a_file_replaced_by_fix_dirs_makes_the_hard_linked_copy_safe() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "hardlink_fixdir");
        let dst = t.subdir("dst");
        std::fs::write(dst.join("a.txt"), b"shared").unwrap();
        std::fs::hard_link(dst.join("a.txt"), dst.join("repl")).unwrap();
        let mut dm = HashMap::new();
        dm.insert("a.txt".to_string(), file());
        dm.insert("repl".to_string(), file());
        let p = plan(&[], &["a.txt"], &["repl"]);

        ensure_hard_linked_targets_safe(&p, &dm, &dst)
            .expect("fix-dirs removes the second name before the copy");
    }

    /// Removing both other names leaves one: the arithmetic that keeps the
    /// preflight from refusing a default run that deletes two extras first.
    #[test]
    fn removing_both_other_names_makes_a_three_way_share_safe() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "hardlink_three");
        let dst = t.subdir("dst");
        std::fs::write(dst.join("a.txt"), b"shared").unwrap();
        std::fs::hard_link(dst.join("a.txt"), dst.join("b.txt")).unwrap();
        std::fs::hard_link(dst.join("a.txt"), dst.join("c.txt")).unwrap();
        let mut dm = HashMap::new();
        for rel in ["a.txt", "b.txt", "c.txt"] {
            dm.insert(rel.to_string(), file());
        }
        let mut p = plan(&[], &["a.txt"], &[]);
        p.delete_files.push("b.txt".to_string());
        p.delete_files.push("c.txt".to_string());

        ensure_hard_linked_targets_safe(&p, &dm, &dst)
            .expect("both other names are removed before the copy");

        p.delete_files.pop();
        ensure_hard_linked_targets_safe(&p, &dm, &dst)
            .expect_err("one surviving name still shares the inode");
    }

    /// **P7-2.** A strict link ancestor that is itself a planned `fix_dirs`
    /// entry is removed before the nested action, so the link guard must skip it
    /// exactly as the two mkdir guards skip a blocking file. The same link
    /// without the fix is still a write-through and is refused.
    #[cfg(any(unix, windows))]
    #[test]
    fn a_planned_fix_link_ancestor_is_not_a_write_through() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "fixed_link");
        let dst = t.subdir("dst");
        let outside = t.subdir("outside");
        if let Err(e) = make_dir_link(&outside, &dst.join("a")) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }

        let fixed = plan(&["a/b"], &[], &["a"]);
        ensure_link_safe(&fixed, &dst, false, false)
            .expect("fix-dirs unlinks `a` before `a/b` is created");

        let unfixed = plan(&["a/b"], &[], &[]);
        let err = ensure_link_safe(&unfixed, &dst, false, false)
            .expect_err("a link that survives the plan is still a write-through");
        let msg = err.to_string();
        assert!(msg.contains("a/b"), "names the action: {msg}");
        assert!(msg.contains("`a`"), "names the link: {msg}");
    }

    /// A directory link for the test above: `symlink_dir` on Unix, and a
    /// junction on Windows because creating a symlink needs developer mode.
    #[cfg(unix)]
    fn make_dir_link(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn make_dir_link(target: &Path, link: &Path) -> std::io::Result<()> {
        let out = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()?;
        if out.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(
                String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            ))
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn make_dir_link(_target: &Path, _link: &Path) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "no link API",
        ))
    }
}
