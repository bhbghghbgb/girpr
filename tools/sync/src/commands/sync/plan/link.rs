//! Link storage, alias resolution, and the on-disk/dst spellings.
//!
//! These helpers are pure over the effective maps plus the link-resolution
//! primitives in [`crate::link`]. They are plan-time only: no filesystem write
//! happens here, only `read_link`/`canonicalize` probes to learn which paths a
//! kept dst file link depends on.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use super::super::rename::DiskSpelling;
use crate::effective::EffRec;
use crate::link::try_is_link;

/// The shortest prefix of `rel` (itself included) that `dm` reports as a link.
///
/// The map is the dst walk's own answer, so this needs no filesystem access and
/// the dry run's plan is computed from the same facts the real run has. A prefix
/// missing from `dm` cannot be a link the walk saw; `delete_extras` re-checks on
/// disk before it removes anything, which covers a difference between the walk
/// and the apply phase.
pub(super) fn link_ancestor<'a>(rel: &str, dm: &'a HashMap<String, EffRec>) -> Option<&'a str> {
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

/// The spelling `rel` has on disk now.
///
/// A real run has already renamed the entries the plan names, so `rel` is the
/// disk spelling. A dry run only re-keyed the map: its guards must stat the
/// pre-rename spelling, or a case-only link (or hard link) is invisible and the
/// preview answers differently from the run it previews. See
/// [`DiskSpelling`] and [`rename_to_src_casing`](super::super::rename).
pub(super) fn disk_rel<'a>(on_disk: Option<&'a DiskSpelling>, rel: &'a str) -> Cow<'a, str> {
    match on_disk {
        Some(spelling) => spelling.disk_rel(rel),
        None => Cow::Borrowed(rel),
    }
}

/// The post-rename `rel` of a path canonicalized from disk.
pub(super) fn plan_rel(on_disk: Option<&DiskSpelling>, disk: String) -> String {
    match on_disk {
        Some(spelling) => spelling.plan_rel(&disk).into_owned(),
        None => disk,
    }
}

/// Dst paths that are the storage of a dst file link this plan keeps.
///
/// A dst file link is followed by the walk, so `dm` holds its path as an
/// ordinary file and its **target** as a second path when the target is inside
/// dst. The two names are one file: writing through the link changes the target,
/// so the target must not be in `delete_files` or under a directory in `rmdir`,
/// and the plan cannot also copy the target to a different content.
///
/// This is a **plan-time** answer in both modes. The disk is asked because that
/// is what `copy_one` writes through; the mode only changes the spelling it must
/// ask about ([`disk_rel`]).
#[derive(Debug, Default)]
pub(super) struct LinkStorage {
    /// Paths whose removal would take the content of a kept link with it: the
    /// final target, plus any link on the resolution chain.
    pub(super) keep: HashSet<String>,
    /// Their ancestor directories, which must not be `rmdir`'d.
    pub(super) keep_dirs: HashSet<String>,
}

impl LinkStorage {
    /// Spare one path and every directory above it.
    ///
    /// Every entry that goes into `keep` must go through here: a spared
    /// **intermediate** link is just as load-bearing as the terminal target,
    /// and its ancestor directory can otherwise be in `plan.rmdir`, whose
    /// `remove_dir_all` would take the link with it.
    pub(super) fn keep_path(&mut self, rel: &str) {
        keep_ancestors(rel, &mut self.keep_dirs);
        self.keep.insert(rel.to_string());
    }

    /// Drop the spared paths from the diff and the plan.
    ///
    /// The diff is adjusted too, so a dry run does not print `EXTRA target` for
    /// a path it will deliberately keep — the record and the action must name the
    /// same set, which is the whole reason `build_plan` reads the diff.
    pub(super) fn apply(&self, diff: &mut crate::diff::Diff, plan: &mut super::build::Plan) {
        if self.keep.is_empty() && self.keep_dirs.is_empty() {
            return;
        }
        diff.extra
            .retain(|p| !self.keep.contains(p) && !self.keep_dirs.contains(p));
        plan.delete_files.retain(|p| !self.keep.contains(p));
        plan.rmdir
            .retain(|p| !self.keep.contains(p) && !self.keep_dirs.contains(p));
    }
}

/// Add an ancestor directory and every directory above it to `keep_dirs`.
pub(super) fn keep_ancestors(rel: &str, keep_dirs: &mut HashSet<String>) {
    for (i, _) in rel.match_indices('/') {
        keep_dirs.insert(rel[..i].to_string());
    }
}

#[cfg(windows)]
pub(super) fn without_verbatim(path: &Path) -> Cow<'_, Path> {
    let Some(text) = path.to_str() else {
        return Cow::Borrowed(path);
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        Cow::Owned(PathBuf::from(format!(r"\{rest}")))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        Cow::Owned(PathBuf::from(rest))
    } else {
        Cow::Borrowed(path)
    }
}

#[cfg(not(windows))]
pub(super) fn without_verbatim(path: &Path) -> Cow<'_, Path> {
    Cow::Borrowed(path)
}

/// The plan-relative spelling of an absolute link target under `canonical_dst`.
///
/// A junction's `read_link` target is absolute and keeps the spelling the
/// junction was created with (`dst/link/x`), so it can name an alias the
/// canonical spelling (`dst/real/x`) does not. Both are real paths in `dm` and
/// the delete plan may name either. `None` when the target is outside the root
/// or escapes it lexically.
pub(super) fn absolute_alias(
    target: &Path,
    canonical_dst: &Path,
    on_disk: Option<&DiskSpelling>,
) -> Option<String> {
    // `canonicalize` and a junction's `read_link` need not agree on the
    // verbatim prefix (`\\?\C:\...` vs `C:\...`); compare without it.
    let target = without_verbatim(target);
    let root = without_verbatim(canonical_dst);
    let rel = target.strip_prefix(root.as_ref()).ok()?;
    let mut parts: Vec<&str> = Vec::new();
    for comp in rel.components() {
        match comp {
            Component::Normal(name) => parts.push(name.to_str()?),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::Prefix(_) | Component::RootDir => return None,
        }
    }
    (!parts.is_empty()).then(|| plan_rel(on_disk, parts.join("/")))
}

/// The plan-relative spelling of a link target as the link text names it.
///
/// This is the **alias** spelling the walk reported (`link/x`), which can differ
/// from the canonical spelling (`real/x`) that [`target_rel`] returns. Both are
/// real paths in `dm` and the delete plan may name either; sparing only one
/// leaves the other free to remove the storage.
pub(super) fn alias_target(cur_alias: &str, target: &Path) -> Option<String> {
    let mut parts: Vec<&str> = cur_alias.split('/').filter(|s| !s.is_empty()).collect();
    // The target is relative to the directory holding the current link.
    parts.pop()?;
    for comp in target.components() {
        match comp {
            Component::Normal(name) => parts.push(name.to_str()?),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::Prefix(_) | Component::RootDir => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The on-disk spelling of the entry `name` in the canonical directory `parent`.
///
/// On Windows the filesystem matches names case-insensitively, so a link text
/// may say `real\X.BIN` while the walk — and therefore `sm`/`dm`, the spare and
/// the shared-storage grouping — reports `real/x.bin`. The final component of a target is
/// deliberately not resolved (it may be the chain's next hop), so its spelling
/// has to come from the directory listing: an exact entry wins, otherwise the
/// unique case-insensitive match is the one the OS would open.
///
/// POSIX names are case-sensitive, so `name` already is the entry's spelling and
/// no lookup is needed; case folding stays Windows-only, as in
/// [`util::same_path`](crate::util::same_path). `None` means the directory holds
/// no entry the OS would match; the caller keeps the lexical spelling.
#[cfg(windows)]
pub(super) fn on_disk_name(parent: &Path, name: &OsStr) -> Result<Option<OsString>> {
    let mut folded = None;
    let entries =
        std::fs::read_dir(parent).with_context(|| format!("list {}", parent.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("list {}", parent.display()))?;
        let candidate = entry.file_name();
        if candidate == name {
            return Ok(Some(candidate));
        }
        if candidate.to_string_lossy().to_lowercase() == name.to_string_lossy().to_lowercase() {
            // A directory can be case-sensitive per directory on Windows; two
            // case-only names then make "the" entry ambiguous, and the name the
            // OS would open is the exact one that is not there.
            if folded.is_some() {
                return Ok(None);
            }
            folded = Some(candidate);
        }
    }
    Ok(folded)
}

#[cfg(not(windows))]
pub(super) fn on_disk_name(_parent: &Path, name: &OsStr) -> Result<Option<OsString>> {
    Ok(Some(name.to_owned()))
}

/// The dst-relative spelling of a link target, if the target is inside `dst`.
///
/// The **final component** is deliberately not resolved: an intermediate link
/// must stay in the chain so it can be spared as well, and a `read_link` target
/// may be relative or contain `..`. Resolving the parent and naming the entry
/// gives the spelling the walk reported at that path — including the final
/// component, whose spelling a link text need not share on a case-insensitive
/// filesystem.
///
/// Only `NotFound` is read as "not inside dst". Any other `canonicalize` error
/// is returned: a permission or I/O failure must not silently disable the spare
/// and leave the delete plan free to remove the link's content.
pub(super) fn target_rel(target: &Path, canonical_dst: &Path) -> Result<Option<String>> {
    let Some(parent) = target.parent() else {
        return Ok(None);
    };
    let parent = match parent.canonicalize() {
        Ok(parent) => parent,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(e).with_context(|| format!("resolve {}", target.display()));
        }
    };
    // A parent outside dst is the documented write-through: the final name
    // cannot bring it back inside. Asking before the listing also keeps an
    // unreadable outside directory from turning a write-through into an error.
    let Ok(parent_rel) = parent.strip_prefix(canonical_dst) else {
        return Ok(None);
    };
    let Some(name) = target.file_name() else {
        return Ok(None);
    };
    let name = on_disk_name(&parent, name)?.unwrap_or_else(|| name.to_owned());
    let rel = parent_rel.join(name);
    let rel = rel.to_string_lossy().replace('\\', "/");
    Ok((!rel.is_empty()).then_some(rel))
}

/// Spare every link component the path `target` resolves through.
///
/// [`target_rel`] deliberately leaves the final component unresolved, but an
/// intermediate **directory** link is also part of the chain a kept link needs:
/// `dst/a -> link/x` with `link -> real` resolves through `link`. Canonicalizing
/// the parent names only `real/x`, so the alias `link` the walk reported would
/// stay in `delete_files` and the run would unlink it. Every strict prefix is
/// therefore asked of the filesystem, in the spelling the resolution actually
/// uses, and each link prefix is spared.
pub(super) fn spare_link_components(
    target: &Path,
    canonical_dst: &Path,
    on_disk: Option<&DiskSpelling>,
    storage: &mut LinkStorage,
) -> Result<()> {
    let comps: Vec<_> = target.components().collect();
    let mut prefix = PathBuf::new();
    for (i, comp) in comps.iter().enumerate() {
        prefix.push(comp.as_os_str());
        // The final component is the caller's decision: a link there is the
        // next hop of the chain, a non-link there is the terminal storage.
        if i + 1 == comps.len() || !matches!(comp, Component::Normal(_)) {
            continue;
        }
        if !try_is_link(&prefix).with_context(|| format!("check link {}", prefix.display()))? {
            continue;
        }
        if let Some(disk) = target_rel(&prefix, canonical_dst)? {
            storage.keep_path(&plan_rel(on_disk, disk));
        }
    }
    Ok(())
}

/// Lowercase key → the map's spelling, for a Windows filesystem that matches
/// names case-insensitively.
///
/// Two `dm` keys cannot differ only by case (the walk's mixed-case check is
/// fatal and the rename pass collapses them), so the mapping is unambiguous.
/// On POSIX two spellings are two paths and folding would join them, so the
/// index is empty there.
pub(super) fn folded_keys(dm: &HashMap<String, EffRec>) -> HashMap<String, String> {
    #[cfg(windows)]
    {
        dm.keys().map(|k| (k.to_lowercase(), k.clone())).collect()
    }
    #[cfg(not(windows))]
    {
        let _ = dm;
        HashMap::new()
    }
}

/// The map's spelling of `rel`: exact, else — on Windows — its case-insensitive
/// twin. `None` when the map holds no such path under any spelling.
pub(super) fn map_spelling<'a>(
    rel: &str,
    dm: &'a HashMap<String, EffRec>,
    folded: &'a HashMap<String, String>,
) -> Option<&'a str> {
    if let Some((key, _)) = dm.get_key_value(rel) {
        return Some(key.as_str());
    }
    folded.get(&rel.to_lowercase()).map(String::as_str)
}

/// Spare `alias` and every link on its way to the root.
///
/// `alias` is the spelling the walk reported for the target (`link/x`), so the
/// `dm` lookup can name a link the canonical parent resolved away. It comes from
/// a link text, which need not share the walk's casing (`LINK/x` on a
/// case-insensitive filesystem), so each lookup answers with the map's own
/// spelling; sparing the link text's spelling instead would leave the walk's key
/// in `diff.extra`. The alias itself is spared even when it is not a link: it is
/// the same storage, and dropping it from `diff.extra` keeps the printed plan
/// and the applied plan naming the same set.
pub(super) fn spare_alias_chain(
    alias: &str,
    dm: &HashMap<String, EffRec>,
    folded: &HashMap<String, String>,
    storage: &mut LinkStorage,
) {
    let mut end = 0;
    loop {
        let (prefix, next) = match alias[end..].find('/') {
            Some(i) => (&alias[..end + i], Some(end + i + 1)),
            None => (alias, None),
        };
        if let Some(key) = map_spelling(prefix, dm, folded)
            && dm[key].is_link
        {
            storage.keep_path(key);
        }
        match next {
            Some(next) => end = next,
            None => break,
        }
    }
    storage.keep_path(map_spelling(alias, dm, folded).unwrap_or(alias));
}
