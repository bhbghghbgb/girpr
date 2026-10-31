//! The plan-time guards (`ensure_*_safe`, `spare_link_storage`).
//!
//! Every guard is a **fail-fast, plan-time** check: it refuses a plan before the
//! first filesystem change, so a dry run refuses exactly what a real run would.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;

use anyhow::{Context, Result, bail};
use tracing::warn;

use crate::diff::Diff;
use crate::effective::EffRec;
use crate::hardlink::{Identity, info};
use crate::link::link_ancestor_above;

use super::build::Plan;
use super::link::{
    LinkStorage, absolute_alias, alias_target, disk_rel, folded_keys, plan_rel, spare_alias_chain,
    spare_link_components, target_rel,
};

/// Find the storage of every dst file link the plan keeps, and refuse a plan
/// that asks one file to hold two different contents.
///
/// A link **stays** when src has a file at its path; a src directory or an
/// absent path means the plan replaces or removes it, and its target is then
/// nobody's storage. For each kept link:
///
/// - walk the `read_link` chain while the next entry is itself a link, sparing
///   each one with its ancestor directories (deleting an intermediate link
///   would dangle the one above it, and an `rmdir` above it would take it);
/// - spare every link **component** of the target path too: a target
///   `link/x` where `link` is a directory link resolves through `link`, and
///   canonicalizing the parent names only `real/x`;
/// - keep both spellings the walk reported (the alias `link/x` and the
///   canonical `real/x`), because the delete plan may name either;
/// - a final target inside dst that src does **not** have is the link's content,
///   so it is spared with its ancestor directories;
/// - a final target src **does** have is a second mirrored path on the same
///   inode. If either end is copied, refuse: one file cannot be given two
///   contents, and copying one name would silently change the other (or race it
///   under the parallel copy phase). A target the plan replaces with a directory
///   is refused for the same reason — the copy would write through a link whose
///   target is gone.
///
/// A target outside dst is the documented write-through: its bytes are not this
/// plan's business, but the links on the way still are, so they are spared.
///
/// Two kept links can resolve to the same storage even when neither's terminal
/// target is mirrored by src. [`ensure_shared_storage_safe`] judges that set as
/// a whole after every chain has been walked.
pub(crate) fn spare_link_storage(
    sm: &HashMap<String, EffRec>,
    dm: &HashMap<String, EffRec>,
    plan: &mut Plan,
    diff: &mut Diff,
    dst: &Path,
    on_disk: Option<&super::super::rename::DiskSpelling>,
) -> Result<()> {
    let mut storage = LinkStorage::default();
    // The plan is not mutated until `storage.apply` at the end, so membership
    // in its copy and type-conflict sets can be read once up front.
    let copy_rels: HashSet<&str> = plan.copy.iter().map(|item| item.rel.as_str()).collect();
    let fix_dirs: HashSet<&str> = plan.fix_dirs.iter().map(String::as_str).collect();
    // Nothing to spare when no copy or delete could meet a link target.
    let mut rels: Vec<&String> = dm.keys().collect();
    rels.sort();
    let canonical_dst = std::fs::canonicalize(dst)
        .with_context(|| format!("resolve dst root {}", dst.display()))?;
    // Every kept link's terminal storage: (link, grouping key, display name).
    let mut terminals: Vec<(String, String, String)> = Vec::new();
    let folded = folded_keys(dm);
    for rel in rels {
        let rec = &dm[rel];
        if !rec.is_link || rec.kind != "file" {
            continue;
        }
        // Only a link src mirrors as a file stays. Anything else is replaced by
        // a directory or removed as an extra, and its target is not its
        // content.
        if sm.get(rel).is_none_or(|s| s.kind != "file") {
            continue;
        }
        // Resolve under the canonical root so every prefix can be compared with
        // `canonical_dst`; the alias is carried separately, in plan spelling.
        let mut cur = canonical_dst.join(disk_rel(on_disk, rel).as_ref());
        let mut cur_alias = Some(rel.clone());
        // A chain longer than this is a loop the walk would already have
        // refused; the bound keeps a plan-time check from spinning.
        for step in 0..64 {
            if step == 63 {
                bail!(
                    "refusing to sync `{rel}`: its link chain does not terminate \
                     (remove the link or exclude it with --exclude)"
                );
            }
            let raw_target = match std::fs::read_link(&cur) {
                Ok(t) => t,
                Err(e) if e.kind() == io::ErrorKind::NotFound => break,
                Err(e) => {
                    return Err(e).with_context(|| format!("read link {}", cur.display()));
                }
            };
            let target = if raw_target.is_absolute() {
                raw_target.clone()
            } else {
                match cur.parent() {
                    Some(parent) => parent.join(&raw_target),
                    None => raw_target.clone(),
                }
            };
            let target_alias = if raw_target.is_absolute() {
                // A junction target is absolute but still keeps the spelling it
                // was created with, so `link/x` is an alias worth sparing.
                absolute_alias(&target, &canonical_dst, on_disk)
            } else {
                cur_alias
                    .as_deref()
                    .and_then(|alias| alias_target(alias, &raw_target))
            };
            spare_link_components(&target, &canonical_dst, on_disk, &mut storage)?;
            let canonical_rel =
                target_rel(&target, &canonical_dst)?.map(|disk| plan_rel(on_disk, disk));
            let target_is_link = crate::link::try_is_link(&target)
                .with_context(|| format!("check link {}", target.display()))?;
            if target_is_link {
                // An intermediate link is part of the chain the kept link
                // resolves through; removing it would break the kept link. Its
                // ancestors must be spared too.
                if let Some(tr) = &canonical_rel {
                    storage.keep_path(tr);
                }
                if let Some(alias) = &target_alias {
                    spare_alias_chain(alias, dm, &folded, &mut storage);
                }
                cur = target;
                cur_alias = target_alias;
                continue;
            }
            match canonical_rel.as_deref() {
                Some(tr) => {
                    if let Some(alias) = &target_alias
                        && alias.as_str() != tr
                    {
                        spare_alias_chain(alias, dm, &folded, &mut storage);
                    }
                    match sm.get(tr) {
                        Some(trec) if trec.kind == "file" => {
                            if copy_rels.contains(rel.as_str()) || copy_rels.contains(tr) {
                                bail!(
                                    "refusing to sync `{rel}`: dst `{rel}` is a link to \
                                     `{tr}`, which src also has; copying either path would \
                                     change the one file they share (remove the link, or \
                                     exclude it with --exclude)"
                                );
                            }
                        }
                        Some(_) => {
                            if fix_dirs.contains(tr) {
                                bail!(
                                    "refusing to sync `{rel}`: dst `{rel}` is a link to \
                                     `{tr}`, which this plan replaces with a directory; the \
                                     copy would write through a link whose target is gone \
                                     (remove the link, or exclude it with --exclude)"
                                );
                            }
                        }
                        None => {
                            // The link's content, not an extra path: deleting it
                            // (or a directory above it) is what turns the
                            // write-through into a dangling link.
                            storage.keep_path(tr);
                        }
                    }
                    terminals.push((rel.clone(), tr.to_string(), tr.to_string()));
                }
                None => {
                    // Outside dst: the bytes are write-through, but the chain
                    // above them is still storage the kept link needs.
                    if let Some(alias) = &target_alias {
                        spare_alias_chain(alias, dm, &folded, &mut storage);
                    }
                    // Record the external file too: two links to one outside
                    // inode are the same shared-storage hazard as two inside.
                    match std::fs::canonicalize(&target) {
                        Ok(p) => terminals.push((
                            rel.clone(),
                            format!("external:{}", p.display()),
                            p.display().to_string(),
                        )),
                        // Dangling: the walk would have refused it before this
                        // plan existed, and there is no inode to share.
                        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                        Err(e) => {
                            return Err(e).with_context(|| {
                                format!("resolve link target {}", target.display())
                            });
                        }
                    }
                }
            }
            break;
        }
    }
    ensure_shared_storage_safe(&terminals, plan)?;
    storage.apply(diff, plan);
    Ok(())
}

/// Refuse a plan in which two kept links resolve to one dst inode and either is
/// copied.
///
/// [`spare_link_storage`] judges each link against its own terminal target; it
/// cannot see that two links name the same file. `dst/a -> c` and `dst/b -> c`
/// (or the chained `dst/a -> b -> c`) are one inode behind two mirrored paths:
/// writing either changes the content the other resolves to, so one path ends
/// wrong at exit 0 and later runs alternate. The rule is the single-link one —
/// one file cannot be given two contents — applied to the whole set.
///
/// A group with no copy is left alone: nothing writes, so both paths keep
/// resolving to the content they already agree on.
pub(super) fn ensure_shared_storage_safe(
    terminals: &[(String, String, String)],
    plan: &Plan,
) -> Result<()> {
    let copy_rels: HashSet<&str> = plan.copy.iter().map(|item| item.rel.as_str()).collect();
    // Each group keeps the first label recorded for its key, so the refusal
    // needs no second pass over `terminals`.
    let mut by_storage: HashMap<&str, (&str, Vec<&str>)> = HashMap::new();
    for (rel, key, label) in terminals {
        by_storage
            .entry(key.as_str())
            .or_insert_with(|| (label.as_str(), Vec::new()))
            .1
            .push(rel);
    }
    let mut groups: Vec<_> = by_storage
        .into_iter()
        .filter(|(_, (_, rels))| rels.len() > 1)
        .collect();
    // Deterministic message when several groups qualify.
    groups.sort_by(|a, b| a.0.cmp(b.0));
    for (_, (label, rels)) in groups {
        if !rels.iter().any(|r| copy_rels.contains(*r)) {
            continue;
        }
        bail!(
            "refusing to sync: dst links `{}` and `{}` both resolve to the same file \
             `{label}`; copying either would give one inode two contents (remove one \
             link, or exclude it with --exclude)",
            rels[0],
            rels[1]
        );
    }
    Ok(())
}

/// Refuse to mirror through a `dst` link, before any apply phase changes
/// anything.
///
/// The planner resolves an extra descendant to the topmost link, but writes and
/// type-conflict fixes under a link are a different question: a shared link
/// (one `src` also has) is deliberately left in place, and `mkdir`/`fix_dirs`/
/// `copy` would then resolve the link as an intermediate path component. That
/// writes into the link target — possibly outside `dst`. The same applies to a rename, which
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
pub(crate) fn ensure_link_safe(
    plan: &Plan,
    dst: &Path,
    missing_only: bool,
    keep_extra: bool,
    on_disk: Option<&super::super::rename::DiskSpelling>,
) -> Result<()> {
    let fixed: HashSet<&str> = plan.fix_dirs.iter().map(String::as_str).collect();
    // A dry run names the link with its pre-rename spelling while `fix_dirs`
    // is keyed by src's casing, so the carve-out has to compare cases too.
    let fixed_lower: HashSet<String> = if on_disk.is_some() {
        plan.fix_dirs.iter().map(|f| f.to_lowercase()).collect()
    } else {
        HashSet::new()
    };
    let actions = plan
        .mkdir
        .iter()
        .chain(plan.copy.iter().map(|item| &item.rel))
        .chain(plan.fix_dirs.iter())
        .chain(plan.delete_files.iter())
        .chain(plan.rmdir.iter());
    for rel in actions {
        let disk = disk_rel(on_disk, rel);
        if let Some(link) = link_ancestor_above(dst, disk.as_ref())? {
            // A strict link ancestor that is itself a planned `fix_dirs` entry is
            // removed *before* the nested action runs (`fix_dirs` precedes
            // `mkdir`/`copy`), so the write resolves an ordinary directory. This
            // is the same carve-out `ensure_mkdir_targets_free` and
            // `ensure_dirs_applicable` grant a blocking file: the guards must
            // describe the same plan, and only a link that survives the fix is a
            // write-through. `link_ancestor_above` returns the topmost link, so a
            // link above the fixed one would have been returned first and is
            // still refused.
            if fixed.contains(link.as_str())
                || (!fixed_lower.is_empty() && fixed_lower.contains(&link.to_lowercase()))
            {
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
        warn!(
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
/// Pure, like [`build_plan`](super::build::build_plan): it reads only the plan
/// and dst's effective map, so a dry run refuses the same plan a real run would.
pub(crate) fn ensure_dirs_applicable(plan: &Plan, dm: &HashMap<String, EffRec>) -> Result<()> {
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
/// [`copy_one`](super::super::apply::copy) refuses a destination with more than one name, but
/// that check lives in the apply phase. This is the plan-time counterpart,
/// reading the same disk state both modes read, so the two agree before either
/// prints an action.
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
pub(crate) fn ensure_hard_linked_targets_safe(
    plan: &Plan,
    dm: &HashMap<String, EffRec>,
    dst: &Path,
    on_disk: Option<&super::super::rename::DiskSpelling>,
) -> Result<()> {
    // Ordinary-file names this run removes before copying, by the inode they
    // name. A link is skipped: its removal leaves the target's count unchanged.
    let mut removed: HashMap<Identity, u64> = HashMap::new();
    for rel in plan.delete_files.iter().chain(plan.fix_dirs.iter()) {
        if dm
            .get(rel)
            .is_none_or(|rec| rec.kind != "file" || rec.is_link)
        {
            continue;
        }
        let p = dst.join(disk_rel(on_disk, rel).as_ref());
        match info(&p) {
            Ok(info) => *removed.entry(info.id).or_default() += 1,
            // Gone already, so this plan entry removes no name at copy time.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e).with_context(|| format!("hard-link count {}", p.display()));
            }
        }
    }
    for item in &plan.copy {
        let d = dst.join(disk_rel(on_disk, &item.rel).as_ref());
        let info = match info(&d) {
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
/// [`build_plan`](super::build::build_plan) and to [`ensure_dirs_applicable`],
/// both of which read `dm`: the plan says `MKDIR`, the dry run prints it as
/// applicable, and `create_dir_all` then fails with `AlreadyExists` after
/// earlier phases have changed dst.
///
/// This runs before [`super::super::rename`] because a case-only rename is a filesystem
/// change, and a refusal must not follow one. It cannot read the plan — that is
/// built after the rename — so it derives the same mkdir candidates
/// [`build_plan`](super::build::build_plan) will: src directories the post-rename `dm` will
/// not hold. In insensitive mode that is a lowercase match, because the rename
/// pass collapses those pairs; a same-path dst **file** is then a planned
/// `fix_dirs` entry, not a mkdir, and is allowed.
///
/// Every strict ancestor of a src directory is itself a src directory, so the
/// blocker is normally its own candidate. A prefix that is not (a file in `dm`
/// while the disk agrees) is a planned type conflict for the same reason, and the
/// map-based [`ensure_dirs_applicable`] remains the later assertion for it.
pub(crate) fn ensure_mkdir_targets_free(
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
