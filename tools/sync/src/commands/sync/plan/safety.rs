//! The plan-time guards (`ensure_*_safe`, `spare_link_storage`).
//!
//! Every guard is a **fail-fast, plan-time** check: it refuses a plan before
//! the apply phases run, so a dry run refuses exactly what a real run would.
//! Only the mkdir preflight refuses before the run's first change to dst; the
//! guards that run after the rename refuse a plan that a real run is already
//! one change into. A dry run's rename pass changes nothing, so there a
//! refusal leaves dst as it was.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tracing::warn;

use crate::diff::Diff;
use crate::effective::EffRec;
use crate::hardlink::{FileId, LinkInfo, info};
use crate::link::link_ancestor_above;
use crate::util::path_key;

use super::build::Plan;
use super::link::{
    LinkStorage, absolute_alias, alias_target, disk_rel, folded_keys, plan_rel, spare_alias_chain,
    spare_link_components, target_rel,
};

/// The disk questions the plan-time guards ask.
///
/// A guard is fail-fast: a probe that fails is a refusal at plan time, so a
/// dry run refuses exactly what a real run would. The guards ask the disk
/// through this seam, so a test can make a probe fail, or count the probes,
/// without needing a filesystem that fails on its own — and those two are what
/// a plan-time refusal turns on.
///
/// **"Before the run has changed anything" is the mkdir preflight's alone.**
/// It runs before the rename pass, which is the run's first change to dst.
/// The guards that ask the disk after the rename — [`spare_link_storage`],
/// [`ensure_link_safe`] and [`ensure_hard_linked_targets_safe`] — refuse a
/// plan that a real run is already one change into, so in a real run a
/// refusal there does not leave dst as it was. A dry run's rename pass
/// changes nothing, so there a refusal leaves dst as it was.
///
/// The seam covers the questions a guard asks about the dst root and about the
/// paths a plan touches. The **link** questions are asked of the disk
/// directly, and [`DIRECT_QUESTIONS`] is the list of them: each entry
/// names the file that asks the question and the call it makes. The hard-link
/// count is not one of them — it asks how many names a file has, not whether
/// a path is a link or where a target points — and it goes through the seam
/// like the other plan questions. A test that counts probes counts the seam's
/// questions and not those.
///
/// What the bypass costs the seam is its reach: a question asked directly is
/// neither counted nor failed on demand. Nothing else is lost with it — the
/// answer still reaches the guard that asked it, and a failure the guard
/// cannot read as an answer is still a refusal at plan time.
pub(crate) trait Probes {
    /// The canonical form of `path`, with every link resolved.
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;

    /// The link count and identity of the file at `path`, following links.
    fn link_info(&self, path: &Path) -> io::Result<LinkInfo>;

    /// `Some(true)` when `path` names an existing directory — a link to one
    /// counts — `Some(false)` when it names anything else, and `None` when
    /// there is no entry at all: the answer `create_dir_all` acts on.
    fn is_dir(&self, path: &Path) -> io::Result<Option<bool>>;
}

/// The real disk.
pub(crate) struct SystemProbes;

impl Probes for SystemProbes {
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        girpr_fs_names::canonical_no_verbatim(path)
    }

    fn link_info(&self, path: &Path) -> io::Result<LinkInfo> {
        info(path)
    }

    /// `Some(is_dir)` when `path` exists, judged the way `create_dir_all`
    /// judges it.
    ///
    /// A link is followed: a link to a directory is an existing directory,
    /// while a dangling link (or one to a file) is not. `None` means there is
    /// no entry, which is also fine. Every other error is returned rather than
    /// read as "not there".
    fn is_dir(&self, path: &Path) -> io::Result<Option<bool>> {
        match std::fs::symlink_metadata(path) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        }
        match std::fs::metadata(path) {
            Ok(md) => Ok(Some(md.is_dir())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Some(false)),
            Err(e) => Err(e),
        }
    }
}

/// A disk question a plan-time guard asks directly, rather than through the
/// seam.
///
/// The file that asks it and the call it makes are what a check needs: they
/// are what the code says, so a call added without an entry in the list is
/// caught. `asks` is what the call asks the disk.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) struct DirectQuestion {
    /// The file of the guards that asks it: `safety.rs` or `link.rs`.
    pub(super) file: &'static str,
    /// The call it makes.
    pub(super) call: &'static str,
    /// What the call asks the disk.
    pub(super) asks: &'static str,
}

/// The disk questions the plan-time guards ask directly.
///
/// The link questions are asked of the disk where they arise, so the seam
/// does not reach them: a test cannot fail one on demand, and a count of
/// probes does not count it. This is the list of them, and it is kept in step
/// with the code by the test that reads the direct calls out of the two files
/// the guards live in (`safety.rs` and `plan/link.rs`) and compares them with
/// it — so a call added without an entry here fails that test.
///
/// The hard-link count is not one of them: it asks how many names a file has,
/// not whether a path is a link or where a target points, so it goes through
/// the seam like the other plan questions.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const DIRECT_QUESTIONS: &[DirectQuestion] = &[
    DirectQuestion {
        file: "safety.rs",
        call: "std::fs::read_link",
        asks: "the link text the chain walk follows",
    },
    DirectQuestion {
        file: "safety.rs",
        call: "girpr_fs_names::canonical_no_verbatim",
        asks: "where a target outside dst resolves",
    },
    DirectQuestion {
        file: "safety.rs",
        call: "crate::link::try_is_link",
        asks: "whether the chain's next hop is itself a link",
    },
    DirectQuestion {
        file: "safety.rs",
        call: "crate::link::link_ancestor_above",
        asks: "whether a path above the action is a link",
    },
    DirectQuestion {
        file: "link.rs",
        call: "std::fs::read_dir",
        asks: "which entry of a directory the OS would open, for a target's final component (a Windows-only probe)",
    },
    DirectQuestion {
        file: "link.rs",
        call: "girpr_fs_names::canonical_no_verbatim",
        asks: "the parent a link target resolves through",
    },
    DirectQuestion {
        file: "link.rs",
        call: "crate::link::try_is_link",
        asks: "whether a component of the target path is a link",
    },
];

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
///
/// A tree without a kept link is left alone: the pass returns before it walks
/// a chain, because there is no storage it could spare and no decision the
/// walk would feed. The dst root is still resolved — a root that cannot be
/// resolved leaves every path under it unanswerable, and that failure is a
/// plan-time one.
pub(crate) fn spare_link_storage(
    sm: &HashMap<String, EffRec>,
    dm: &HashMap<String, EffRec>,
    plan: &mut Plan,
    diff: &mut Diff,
    dst: &Path,
    on_disk: Option<&super::super::rename::DiskSpelling>,
    probes: &dyn Probes,
) -> Result<()> {
    // The dst root must resolve before the pass decides anything: every path
    // under it is compared with the canonical root, and a root that cannot be
    // resolved leaves the whole plan unanswerable. The failure belongs here,
    // at plan time — a run that found it later would act on a plan it could
    // not check.
    let canonical_dst = probes
        .canonicalize(dst)
        .with_context(|| format!("resolve dst root {}", dst.display()))?;
    // The links this plan keeps: a dst file link src mirrors as a file stays,
    // and its storage is then the plan's business. A tree without one has
    // nothing to spare, so the folded key map — and the chain walk below —
    // are work no decision depends on.
    let mut rels: Vec<&String> = Vec::new();
    for (rel, rec) in dm {
        if !rec.is_link || rec.kind != "file" {
            continue;
        }
        // Only a link src mirrors as a file stays. Anything else is replaced by
        // a directory or removed as an extra, and its target is not its
        // content.
        if sm.get(rel).is_none_or(|s| s.kind != "file") {
            continue;
        }
        rels.push(rel);
    }
    if rels.is_empty() {
        return Ok(());
    }
    rels.sort();
    let mut storage = LinkStorage::default();
    // The plan is not mutated until `storage.apply` at the end, so membership
    // in its copy and fix-dir sets can be read once up front.
    let copy_rels: HashSet<&str> = plan.copy.iter().map(|item| item.rel.as_str()).collect();
    let fix_dirs: HashSet<&str> = plan.fix_dirs.iter().map(String::as_str).collect();
    // Every kept link's terminal storage: (link, grouping key, display name).
    let mut terminals: Vec<(String, String, String)> = Vec::new();
    let folded = folded_keys(dm);
    for rel in rels {
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
                    match girpr_fs_names::canonical_no_verbatim(&target) {
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
    let fixed_folded: HashSet<String> = if on_disk.is_some() {
        plan.fix_dirs.iter().map(|f| path_key(f)).collect()
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
                || (!fixed_folded.is_empty() && fixed_folded.contains(&path_key(&link)))
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
/// `copy_one` refuses a destination with more than one name, but that check
/// lives in the apply phase. This is the plan-time counterpart, reading the
/// same disk state both modes read, so the two agree before either prints an
/// action.
///
/// **The link count is a seam probe.** It is asked through
/// [`Probes::link_info`], like the other plan questions, so a test can make
/// it fail on demand and count it. It asks how many names a file has, not
/// whether a path is a link or where a target points, which is why it is not
/// one of the link questions the guards ask directly ([`DIRECT_QUESTIONS`]).
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
///
/// A run with no copy has nothing to protect: it cannot truncate an inode, so
/// the preflight has no decision to make. The removals it would credit are
/// this guard's own accounting, not a separate concern — they only matter to
/// a copy that is going to happen. They are still read, because a removal the
/// disk cannot answer is a plan-time failure, and the delete phase would only
/// find it after the rename had changed dst.
pub(crate) fn ensure_hard_linked_targets_safe(
    plan: &Plan,
    dm: &HashMap<String, EffRec>,
    dst: &Path,
    on_disk: Option<&super::super::rename::DiskSpelling>,
    probes: &dyn Probes,
) -> Result<()> {
    // A copy-free run cannot truncate an inode, so the preflight has nothing
    // to protect. This is not "the plan is empty": the removals below are
    // read either way, because a removal the disk cannot answer is a
    // plan-time failure — but only a copy credits them.
    let copy_free = plan.copy.is_empty();
    // Ordinary-file names this run removes before copying, by the inode they
    // name. A link is skipped: its removal leaves the target's count unchanged.
    let mut removed: HashMap<FileId, u64> = HashMap::new();
    for rel in plan.delete_files.iter().chain(plan.fix_dirs.iter()) {
        if dm
            .get(rel)
            .is_none_or(|rec| rec.kind != "file" || rec.is_link)
        {
            continue;
        }
        let p = dst.join(disk_rel(on_disk, rel).as_ref());
        match probes.link_info(&p) {
            Ok(info) => {
                // The credit is only read by a copy, so a copy-free run does
                // not build the map — but it still asks the disk, because the
                // answer it would not use can still be an error.
                if !copy_free {
                    *removed.entry(info.id).or_default() += 1;
                }
            }
            // Gone already, so this plan entry removes no name at copy time.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e).with_context(|| format!("hard-link count {}", p.display()));
            }
        }
    }
    if copy_free {
        return Ok(());
    }
    for item in &plan.copy {
        let d = dst.join(disk_rel(on_disk, &item.rel).as_ref());
        let info = match probes.link_info(&d) {
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
/// not hold. In insensitive mode that is a match by the folded key, because the
/// rename pass collapses those pairs; a same-path dst **file** is then a planned
/// `fix_dirs` entry, not a mkdir, and is allowed.
///
/// Every strict ancestor of a src directory is itself a src directory, so the
/// blocker is normally its own candidate. A prefix that is not (a file in `dm`
/// while the disk agrees) is a planned type conflict for the same reason, and the
/// map-based [`ensure_dirs_applicable`] remains the later assertion for it.
///
/// Each candidate asks the disk about itself and about every strict prefix, and
/// the prefixes are shared: a deep tree would ask about the same ancestor once
/// per descendant. [`PrefixStats`] keeps one answer per prefix, so the disk is
/// asked once — without changing which prefix the refusal names.
pub(crate) fn ensure_mkdir_targets_free(
    sm: &HashMap<String, EffRec>,
    dm: &HashMap<String, EffRec>,
    dst: &Path,
    case_sensitive: bool,
    probes: &dyn Probes,
) -> Result<()> {
    // The dst view the plan will see after the rename: exact keys in sensitive
    // mode, folded pairs otherwise. This index folds to a *value* rather than
    // to a key, so it does not share the key-shaped `util::folded_keys` index,
    // and it is named for what it holds: the dst's records, each under the
    // folded key of its path.
    let dst_folded_values: HashMap<String, &EffRec> = if case_sensitive {
        HashMap::new()
    } else {
        dm.iter().map(|(k, v)| (path_key(k), v)).collect()
    };
    let paired = |rel: &str| dst_after_rename(rel, dm, &dst_folded_values, case_sensitive);
    // A src directory the plan will fix rather than mkdir, because dst holds a
    // file (or a file link) at the same path after the rename.
    let fixed: HashSet<&str> = sm
        .iter()
        .filter(|(rel, rec)| rec.kind == "dir" && paired(rel).is_some_and(|dst| dst.kind == "file"))
        .map(|(rel, _)| rel.as_str())
        .collect();
    // The disk's answer about a path, remembered for the rest of the call: a
    // deep tree asks about the same ancestor once per descendant otherwise.
    let mut stats = PrefixStats::default();
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
            match stats.get(prefix, || {
                let path = dst.join(prefix);
                probes
                    .is_dir(&path)
                    .with_context(|| format!("stat {}", path.display()))
            })? {
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

/// The disk's answer about a path, remembered for the rest of one call.
///
/// The mkdir preflight asks about every strict prefix of every candidate, and
/// the prefixes are shared: without a memo the same ancestor is stat'd once per
/// descendant. One answer per prefix keeps the disk to one question.
#[derive(Default)]
pub(super) struct PrefixStats {
    answers: HashMap<String, Option<bool>>,
}

impl PrefixStats {
    /// The answer for `prefix`, asking the disk the first time only.
    ///
    /// The probe is a closure so the memo can be asked about a path without
    /// this type knowing how a path is stat'd.
    pub(super) fn get(
        &mut self,
        prefix: &str,
        probe: impl FnOnce() -> Result<Option<bool>>,
    ) -> Result<Option<bool>> {
        if let Some(answer) = self.answers.get(prefix) {
            return Ok(*answer);
        }
        let answer = probe()?;
        self.answers.insert(prefix.to_string(), answer);
        Ok(answer)
    }
}

/// The dst entry a src path will pair with after the rename pass.
///
/// `folded_values` holds the dst's records under the folded key of their
/// path: the value-shaped index, not the key-shaped one.
fn dst_after_rename<'a>(
    rel: &str,
    dm: &'a HashMap<String, EffRec>,
    folded_values: &'a HashMap<String, &'a EffRec>,
    case_sensitive: bool,
) -> Option<&'a EffRec> {
    if case_sensitive {
        dm.get(rel)
    } else {
        folded_values.get(&path_key(rel)).copied()
    }
}

/// The check that keeps the seam's list of direct questions in step with the
/// code.
///
/// The list is a fact about the source, so the check reads the source: the
/// calls the two files make of the disk themselves are found in the text and
/// compared with the list. It reads the code and not the doc, so a rewording
/// of the doc cannot move the list, and a call added without an entry in the
/// list fails here.
#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::DIRECT_QUESTIONS;

    /// The modules whose functions reach the disk. `girpr_fs_names::canonical_no_verbatim`
    /// is a path canonicalizer (dunce-backed) like `std::fs`'s, so its calls are
    /// direct disk questions too.
    const DISK_MODULES: [&str; 5] = [
        "std::fs",
        "girpr_fs_names::canonical_no_verbatim",
        "crate::link",
        "crate::hardlink",
        "dunce::canonicalize",
    ];

    /// The `Path` methods that reach the disk.
    const DISK_METHODS: [&str; 7] = [
        "canonicalize",
        "exists",
        "is_dir",
        "is_file",
        "metadata",
        "symlink_metadata",
        "try_exists",
    ];

    /// The direct disk questions the guards ask, as the code asks them.
    ///
    /// The list is the seam's, and this is what keeps it in step with the
    /// code: the calls the two files make of the disk themselves are read out
    /// of the source and compared with the list. A call added without an
    /// entry in the list fails here, and so does an entry the code no longer
    /// backs.
    #[test]
    fn the_direct_disk_questions_are_the_ones_the_code_asks() {
        let files = [
            ("safety.rs", include_str!("safety.rs")),
            ("link.rs", include_str!("link.rs")),
        ];
        let mut asked = Vec::new();
        for (file, src) in files {
            let globs = glob_imports(&without_comments(src));
            assert!(
                globs.is_empty(),
                "{file} imports a disk module by a glob, which brings names the scan \
                 cannot resolve, so a call through one would be invisible: {globs:?}",
            );
            asked.extend(direct_disk_calls(file, src));
        }
        asked.sort();
        let mut listed = DIRECT_QUESTIONS
            .iter()
            .map(|q| (q.file.to_string(), leaf(q.call)))
            .collect::<Vec<_>>();
        listed.sort();
        assert_eq!(
            asked,
            listed,
            "the guards ask a disk question the list does not name, or the list names \
             a question they no longer ask\nasked: {asked:?}\nlisted: {listed:?}\n{}",
            described(),
        );
    }

    /// The disk modules a file imports by a glob.
    ///
    /// A glob brings names the scan cannot resolve — it does not know what a
    /// module holds — so a call through one would be invisible to it. The
    /// scan refuses that shape rather than pretend to cover it.
    fn glob_imports(code: &str) -> Vec<String> {
        let mut globs = Vec::new();
        for statement in use_statements(code) {
            let body = statement
                .trim_start_matches("use ")
                .trim_end_matches(';')
                .trim();
            let Some(path) = body.strip_suffix("::*") else {
                continue;
            };
            let segments: Vec<&str> = path.split("::").collect();
            if segments.len() >= 2 {
                let module = format!("{}::{}", segments[0], segments[1]);
                if DISK_MODULES.contains(&module.as_str()) {
                    globs.push(body.to_string());
                }
            }
        }
        globs
    }

    /// The list as it reads: each entry and what it asks the disk.
    fn described() -> String {
        DIRECT_QUESTIONS
            .iter()
            .map(|q| format!("{}: {} — {}", q.file, q.call, q.asks))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The disk calls a file makes directly, as `(file, call)` pairs.
    ///
    /// A call inside an `impl Probes for` block is the seam's own answer, a
    /// call on the seam itself is a question the seam answers, and a call in
    /// the file's own tests asks nothing on a run's behalf; none of them is a
    /// question asked around the seam. Everything else that reaches the disk
    /// is asked directly, and so belongs in the list.
    fn direct_disk_calls(file: &str, src: &str) -> Vec<(String, String)> {
        let code = without_comments(src);
        let exempt: Vec<(usize, usize)> = seam_blocks(&code)
            .into_iter()
            .chain(test_blocks(&code))
            .collect();
        let seam_names = seam_names(&code);
        let (imported, aliases) = imported_disk_names(&code);
        let mut calls = Vec::new();
        for call in call_sites(&code) {
            if exempt
                .iter()
                .any(|(start, end)| call.at >= *start && call.at < *end)
            {
                continue;
            }
            let name = match &call.how {
                How::Path(path) => {
                    if !reaches_the_disk(path, &aliases) {
                        continue;
                    }
                    leaf(path)
                }
                How::Method(receiver) => {
                    if !DISK_METHODS.contains(&call.name.as_str()) {
                        continue;
                    }
                    if seam_names.contains(receiver) {
                        continue;
                    }
                    call.name.clone()
                }
                How::Free => {
                    if !imported.contains(&call.name) {
                        continue;
                    }
                    call.name.clone()
                }
            };
            calls.push((file.to_string(), name));
        }
        calls
    }

    /// How a call was reached.
    enum How {
        /// A path: `std::fs::read_link`.
        Path(String),
        /// A method: `probes.canonicalize`.
        Method(String),
        /// A plain name: `link_ancestor_above`.
        Free,
    }

    /// One call the scan found.
    struct Call {
        /// Where it is in the code.
        at: usize,
        /// The callee's own name.
        name: String,
        /// How it was reached.
        how: How,
    }

    /// The calls the code makes, with the name and how each was reached.
    fn call_sites(code: &str) -> Vec<Call> {
        let bytes = code.as_bytes();
        let mut calls = Vec::new();
        for at in 0..bytes.len() {
            if bytes[at] != b'(' {
                continue;
            }
            // The callee's name: the identifier just before the '('.
            let mut end = at;
            while end > 0 && bytes[end - 1].is_ascii_whitespace() {
                end -= 1;
            }
            let mut start = end;
            while start > 0 && is_ident(bytes[start - 1]) {
                start -= 1;
            }
            if start == end {
                continue;
            }
            let name = code[start..end].to_string();
            // What reaches it: a path, a receiver, or nothing.
            let mut before = start;
            while before > 0 && bytes[before - 1].is_ascii_whitespace() {
                before -= 1;
            }
            let how = if before >= 2 && &code[before - 2..before] == "::" {
                let mut path_start = before - 2;
                while path_start > 0
                    && (is_ident(bytes[path_start - 1]) || bytes[path_start - 1] == b':')
                {
                    path_start -= 1;
                }
                How::Path(code[path_start..end].trim().to_string())
            } else if before > 0 && bytes[before - 1] == b'.' {
                // The receiver is the name before the '.', across a line break
                // if the chain is written on two lines.
                let mut recv_end = before - 1;
                while recv_end > 0 && bytes[recv_end - 1].is_ascii_whitespace() {
                    recv_end -= 1;
                }
                let mut recv_start = recv_end;
                while recv_start > 0 && is_ident(bytes[recv_start - 1]) {
                    recv_start -= 1;
                }
                How::Method(code[recv_start..recv_end].to_string())
            } else {
                How::Free
            };
            calls.push(Call { at, name, how });
        }
        calls
    }

    /// The blocks that implement the seam: the `impl Probes for` blocks.
    fn seam_blocks(code: &str) -> Vec<(usize, usize)> {
        let mut blocks = Vec::new();
        let mut from = 0;
        while let Some(found) = code[from..].find("impl Probes for") {
            let start = from + found;
            let Some(block) = block_at(code, start) else {
                break;
            };
            blocks.push(block);
            from = block.1.max(start + 1);
        }
        blocks
    }

    /// The blocks the file keeps for its own tests: the code behind a
    /// `#[cfg(test)]`.
    ///
    /// A test may ask the disk as it likes: it asks no question on a run's
    /// behalf, so it is not one of the guards' questions.
    fn test_blocks(code: &str) -> Vec<(usize, usize)> {
        let mut blocks = Vec::new();
        let mut from = 0;
        while let Some(found) = code[from..].find("#[cfg(test)]") {
            let start = from + found;
            let Some(block) = block_at(code, start) else {
                break;
            };
            blocks.push(block);
            from = block.1.max(start + 1);
        }
        blocks
    }

    /// The block that opens after `from`, as a range of the code.
    ///
    /// The braces are matched, so a nested block does not end it.
    fn block_at(code: &str, from: usize) -> Option<(usize, usize)> {
        let open = code[from..].find('{')? + from;
        let mut depth = 0i32;
        for (i, c) in code[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((from, open + i + 1));
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The names the seam answers to: the parameter a guard takes the seam
    /// as, and the seam's own type.
    fn seam_names(code: &str) -> HashSet<String> {
        let mut names = HashSet::new();
        // The `&dyn Probes` parameters.
        let mut from = 0;
        while let Some(found) = code[from..].find("&dyn Probes") {
            let at = from + found;
            // The parameter's name is the identifier before the ':'.
            let mut colon = at;
            while colon > 0 && code.as_bytes()[colon - 1].is_ascii_whitespace() {
                colon -= 1;
            }
            if colon > 0 && code.as_bytes()[colon - 1] == b':' {
                colon -= 1;
            }
            let mut start = colon;
            while start > 0 && is_ident(code.as_bytes()[start - 1]) {
                start -= 1;
            }
            if start < colon {
                names.insert(code[start..colon].to_string());
            }
            from = at + 1;
        }
        // The seam's own type: the `impl Probes for` blocks name it.
        let mut from = 0;
        while let Some(found) = code[from..].find("impl Probes for") {
            let at = from + found;
            let rest = &code[at + "impl Probes for".len()..];
            let end = rest.find('{').unwrap_or(rest.len());
            let name = rest[..end].trim().to_string();
            if !name.is_empty() {
                names.insert(name);
            }
            from = at + 1;
        }
        names
    }

    /// The names the file imports from the modules that reach the disk, and
    /// the aliases those modules are imported under.
    ///
    /// A module can be reached under another name (`use std::fs as filesystem`
    /// or `use std::fs::{self as fs, ..}`), and a call reaches it by that
    /// name, so the alias is what the scan has to know: a module reached by
    /// a name the scan does not hold would make every call through it
    /// invisible.
    fn imported_disk_names(code: &str) -> (HashSet<String>, HashSet<String>) {
        let mut names = HashSet::new();
        let mut aliases = HashSet::new();
        for statement in use_statements(code) {
            let body = statement
                .trim_start_matches("use ")
                .trim_end_matches(';')
                .trim();
            // `use std::fs as filesystem;` names the module under another
            // name. The `as` inside a brace list belongs to a list item, so
            // only an `as` outside the braces renames the module.
            let (body, alias) = match body.split_once(" as ") {
                Some((path, alias)) if !path.contains('{') && !alias.contains('{') => {
                    (path.trim(), Some(alias.trim()))
                }
                _ => (body, None),
            };
            let segments: Vec<&str> = body.split("::").collect();
            if segments.len() < 2 {
                continue;
            }
            let module = format!("{}::{}", segments[0], segments[1]);
            if !DISK_MODULES.contains(&module.as_str()) {
                continue;
            }
            let last = segments[segments.len() - 1];
            match last.strip_suffix('}').and_then(|s| s.strip_prefix('{')) {
                Some(inner) => {
                    for item in inner.split(',') {
                        let item = item.trim();
                        if item.is_empty() {
                            continue;
                        }
                        let (item, alias) = match item.split_once(" as ") {
                            Some((item, alias)) => (item.trim(), Some(alias.trim())),
                            None => (item, None),
                        };
                        if item == "self" {
                            // `self` is the module itself, under its own name
                            // or under the alias the item gives it.
                            aliases.insert(alias.unwrap_or(segments[1]).to_string());
                        } else if item != "*" {
                            names.insert(alias.unwrap_or(item).to_string());
                        }
                    }
                }
                None if segments.len() == 2 => {
                    aliases.insert(alias.unwrap_or(last).to_string());
                }
                None => {
                    names.insert(alias.unwrap_or(last).to_string());
                }
            }
        }
        (names, aliases)
    }

    /// The `use` statements of the code.
    fn use_statements(code: &str) -> Vec<String> {
        let mut statements = Vec::new();
        let mut from = 0;
        while let Some(found) = code[from..].find("use ") {
            let at = from + found;
            // A statement runs to its ';', and the word starts one.
            let starts = at == 0 || !is_ident(code.as_bytes()[at - 1]);
            let Some(end) = code[at..].find(';') else {
                break;
            };
            if starts {
                statements.push(code[at..at + end].to_string());
            }
            from = at + end;
        }
        statements
    }

    /// Whether a path reaches the disk.
    fn reaches_the_disk(path: &str, aliases: &HashSet<String>) -> bool {
        // A path may be written with a leading `::`; the module it names is
        // the same one either way.
        let path = path.trim_start_matches("::");
        let segments: Vec<&str> = path.split("::").collect();
        if segments.len() >= 2 {
            let module = format!("{}::{}", segments[0], segments[1]);
            if DISK_MODULES.contains(&module.as_str()) {
                return true;
            }
        }
        aliases.contains(segments.first().copied().unwrap_or_default())
    }

    /// The last segment of a path: the call's own name.
    fn leaf(path: &str) -> String {
        path.rsplit("::").next().unwrap_or(path).to_string()
    }

    /// Whether the byte is part of an identifier.
    fn is_ident(byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || byte == b'_'
    }

    /// The code of a file, with the comments and the string contents removed.
    ///
    /// A doc comment names calls — the seam's doc names the questions it does
    /// not cover — so the scan must not read a mention as a call.
    fn without_comments(src: &str) -> String {
        let bytes = src.as_bytes();
        let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'/' if bytes.get(i + 1) == Some(&b'/') => {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    i += 2;
                    while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                        i += 1;
                    }
                    i = (i + 2).min(bytes.len());
                }
                b'"' => {
                    // A raw string's backslash is not an escape, so its closing
                    // quote is found by scanning for it rather than by
                    // following the escapes.
                    let mut hashes = 0;
                    let mut before = i;
                    while before > 0 && bytes[before - 1] == b'#' {
                        hashes += 1;
                        before -= 1;
                    }
                    let raw = before > 0
                        && bytes[before - 1] == b'r'
                        && (before == 1 || !is_ident(bytes[before - 2]));
                    out.push(b'"');
                    i += 1;
                    if raw {
                        while i < bytes.len() {
                            let closes = bytes[i] == b'"'
                                && (0..hashes).all(|k| bytes.get(i + 1 + k) == Some(&b'#'));
                            if closes {
                                break;
                            }
                            i += 1;
                        }
                    } else {
                        while i < bytes.len() {
                            match bytes[i] {
                                b'\\' => i += 2,
                                b'"' => break,
                                _ => i += 1,
                            }
                        }
                    }
                    out.push(b'"');
                    i += 1 + hashes;
                }
                b'\'' => {
                    // A char literal or a lifetime: `'x'` is a literal, `'a` is not.
                    let escaped = bytes.get(i + 1) == Some(&b'\\');
                    let literal = if escaped {
                        bytes.get(i + 3) == Some(&b'\'')
                    } else {
                        bytes.get(i + 2) == Some(&b'\'')
                    };
                    if literal {
                        i += if escaped { 4 } else { 3 };
                    } else {
                        out.push(b'\'');
                        i += 1;
                    }
                }
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}
