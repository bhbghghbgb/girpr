//! Link storage, alias resolution, and the on-disk/dst spellings.
//!
//! Most of the helpers here are pure: they answer from what the run already
//! knows — the effective maps, the disk-spelling map, the paths themselves —
//! and touch nothing. Two of them ask the disk as well, because the plan
//! needs what only the disk can say — where a link's target sits inside dst
//! ([`target_rel`]) and which of a target's prefixes are links
//! ([`spare_link_components`], which asks the link primitive
//! [`try_is_link`]). On Windows a third does: [`on_disk_name`], which lists a
//! directory to learn which spelling the OS would open. On POSIX the name
//! already is the spelling, so that lookup asks nothing at all, and the two
//! above are the whole of the disk-asking helpers there.
//!
//! They are plan-time only: no filesystem write happens here, only the
//! link probes [`DIRECT_QUESTIONS`](super::safety::DIRECT_QUESTIONS)
//! names, and they are asked to learn which paths a kept dst file link
//! depends on. The probes are made directly rather than through the guard
//! seam.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use path_clean::PathClean;

use super::super::rename::DiskSpelling;
use crate::effective::EffRec;
use crate::link::{path_prefixes, strict_path_prefixes, try_is_link};
#[cfg(windows)]
use crate::util::unicode_case_eq;
use crate::util::{path_key, slash_rel};

/// The shortest prefix of `rel` (itself included) that `dm` reports as a link.
///
/// The map is the dst walk's own answer, so this needs no filesystem access and
/// the dry run's plan is computed from the same facts the real run has. A prefix
/// missing from `dm` cannot be a link the walk saw; `delete_extras` re-checks on
/// disk before it removes anything, which covers a difference between the walk
/// and the apply phase.
pub(super) fn link_ancestor<'a>(rel: &str, dm: &'a HashMap<String, EffRec>) -> Option<&'a str> {
    for prefix in path_prefixes(rel) {
        // The map is the question here: no disk stat, and the map's own key is
        // returned so a case-only spelling is spared as the walk reported it.
        if let Some((key, rec)) = dm.get_key_value(prefix)
            && rec.is_link
        {
            return Some(key.as_str());
        }
    }
    None
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
    for prefix in strict_path_prefixes(rel) {
        keep_dirs.insert(prefix.to_string());
    }
}

/// Turn a lexically cleaned, root-relative `Path` into a slash-joined rel string.
///
/// [`PathClean::clean`] collapses `.` and `..` without asking the filesystem, but
/// it deliberately keeps a `..` that has no preceding component to cancel on a
/// relative path. That leading `..` is exactly the escape the hand-written
/// component walk refused by popping an empty stack, so it is checked here —
/// once, for both callers — and nowhere else. A cleaned path of `.` means the
/// candidate denoted the root itself, which is likewise not an alias under it.
fn cleaned_rel(cleaned: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for comp in cleaned.components() {
        match comp {
            Component::Normal(name) => parts.push(name.to_str()?),
            // The only `CurDir` `clean()` leaves is the sole component of `.`.
            Component::CurDir => {}
            // A leading `..` (or any prefix/root) escaped the root. Refuse it.
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
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
    // Canonicalization goes through the shared `girpr_fs_names::canonical_no_verbatim`
    // helper (dunce-backed), which — like a junction's `read_link` — spells a
    // UNC root with two backslashes and a local drive path without a verbatim
    // prefix, so the two compare directly.
    let rel = target.strip_prefix(canonical_dst).ok()?;
    // `clean()` is purely lexical; a leading `..` it cannot resolve is kept and
    // refused by `cleaned_rel`, preserving the old pop-on-empty escape refusal.
    let rel = cleaned_rel(&rel.clean())?;
    Some(plan_rel(on_disk, rel))
}

/// The plan-relative spelling of a link target as the link text names it.
///
/// This is the **alias** spelling the walk reported (`link/x`), which can differ
/// from the canonical spelling (`real/x`) that [`target_rel`] returns. Both are
/// real paths in `dm` and the delete plan may name either; sparing only one
/// leaves the other free to remove the storage.
pub(super) fn alias_target(cur_alias: &str, target: &Path) -> Option<String> {
    // An absolute target is refused, as the old component walk refused a
    // `Prefix`/`RootDir` component. `PathBuf::push` would otherwise replace the
    // base with it, turning an escape into an alias under a different root.
    if target
        .components()
        .any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
    {
        return None;
    }
    let mut parts: Vec<&str> = cur_alias.split('/').filter(|s| !s.is_empty()).collect();
    // The target is relative to the directory holding the current link. The
    // base is a walk-reported rel path, so it has no `.` or `..` of its own for
    // `clean()` to reinterpret; only the target's spelling is normalized.
    parts.pop()?;
    let base = parts.join("/");
    cleaned_rel(&Path::new(&base).join(target).clean())
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
/// **Neither of the other two rules.** The path key
/// ([`crate::util::path_key`]) folds ASCII case only and answers which bucket
/// of a run's map a path belongs to; the identity rule
/// ([`crate::util::same_path`]) keeps a Unicode fallback and answers whether
/// two spellings open the same place on disk. This lookup asks a third
/// question — which directory entry the OS would open — and the OS's own table
/// is a Unicode one, so the comparison folds Unicode case.
///
/// The comparison is [`crate::util::unicode_case_eq`], the comparison the
/// identity rule's fallback uses as well, over the shared mapping
/// [`crate::util::unicode_lowercase`]: one implementation for two
/// questions, not one rule.
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
        if unicode_case_eq(&candidate, name) {
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
    let parent = match girpr_fs_names::canonical_no_verbatim(parent) {
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
    let rel = slash_rel(&rel.to_string_lossy());
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

/// Folded key → the map's spelling, for a Windows filesystem that matches
/// names case-insensitively.
///
/// The fold is the path key's — ASCII case only — so this index finds a map
/// key whose spelling differs from the wanted one in ASCII case alone.
///
/// Two `dm` keys cannot differ only by case (the walk's mixed-case check is
/// fatal and the rename pass collapses them), so the mapping is unambiguous.
/// On POSIX two spellings are two paths and folding would join them, so the
/// index is empty there.
///
/// Not the shared `util::folded_keys` index: this one owns its spellings
/// instead of borrowing them from `dm`, and it answers only for a
/// case-insensitive filesystem.
pub(super) fn folded_keys(dm: &HashMap<String, EffRec>) -> HashMap<String, String> {
    #[cfg(windows)]
    {
        dm.keys().map(|k| (path_key(k), k.clone())).collect()
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
    folded.get(&path_key(rel)).map(String::as_str)
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
