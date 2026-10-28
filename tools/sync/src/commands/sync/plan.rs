//! Turning the src/dst diff into a concrete work list.
//!
//! [`build_plan`] is pure — it only reads the two effective maps — which is
//! what makes `--dry-run` trustworthy: the plan it prints is byte-for-byte the
//! plan a real run would execute.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::apply::Applied;
use super::rename::DiskSpelling;
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

/// The spelling `rel` has on disk now.
///
/// A real run has already renamed the entries the plan names, so `rel` is the
/// disk spelling. A dry run only re-keyed the map: its guards must stat the
/// pre-rename spelling, or a case-only link (or hard link) is invisible and the
/// preview answers differently from the run it previews. See
/// [`DiskSpelling`] and [`rename_to_src_casing`](super::rename).
fn disk_rel<'a>(on_disk: Option<&'a DiskSpelling>, rel: &'a str) -> Cow<'a, str> {
    match on_disk {
        Some(spelling) => spelling.disk_rel(rel),
        None => Cow::Borrowed(rel),
    }
}

/// The post-rename `rel` of a path canonicalized from disk.
fn plan_rel(on_disk: Option<&DiskSpelling>, disk: String) -> String {
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
struct LinkStorage {
    /// Paths whose removal would take the content of a kept link with it: the
    /// final target, plus any link on the resolution chain.
    keep: HashSet<String>,
    /// Their ancestor directories, which must not be `rmdir`'d.
    keep_dirs: HashSet<String>,
}

impl LinkStorage {
    /// Spare one path and every directory above it.
    ///
    /// Every entry that goes into `keep` must go through here: a spared
    /// **intermediate** link is just as load-bearing as the terminal target,
    /// and its ancestor directory can otherwise be in `plan.rmdir`, whose
    /// `remove_dir_all` would take the link with it.
    fn keep_path(&mut self, rel: &str) {
        keep_ancestors(rel, &mut self.keep_dirs);
        self.keep.insert(rel.to_string());
    }

    /// Drop the spared paths from the diff and the plan.
    ///
    /// The diff is adjusted too, so a dry run does not print `EXTRA target` for
    /// a path it will deliberately keep — the record and the action must name the
    /// same set, which is the whole reason `build_plan` reads the diff.
    fn apply(&self, diff: &mut Diff, plan: &mut Plan) {
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
fn keep_ancestors(rel: &str, keep_dirs: &mut HashSet<String>) {
    for (i, _) in rel.match_indices('/') {
        keep_dirs.insert(rel[..i].to_string());
    }
}

#[cfg(windows)]
fn without_verbatim(path: &Path) -> Cow<'_, Path> {
    let Some(text) = path.to_str() else {
        return Cow::Borrowed(path);
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        Cow::Owned(PathBuf::from(format!(r"\\{rest}")))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        Cow::Owned(PathBuf::from(rest))
    } else {
        Cow::Borrowed(path)
    }
}

#[cfg(not(windows))]
fn without_verbatim(path: &Path) -> Cow<'_, Path> {
    Cow::Borrowed(path)
}

/// The plan-relative spelling of an absolute link target under `canonical_dst`.
///
/// A junction's `read_link` target is absolute and keeps the spelling the
/// junction was created with (`dst/link/x`), so it can name an alias the
/// canonical spelling (`dst/real/x`) does not. Both are real paths in `dm` and
/// the delete plan may name either. `None` when the target is outside the root
/// or escapes it lexically.
fn absolute_alias(
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
fn alias_target(cur_alias: &str, target: &Path) -> Option<String> {
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
/// the R9-3 grouping — reports `real/x.bin`. The final component of a target is
/// deliberately not resolved (it may be the chain's next hop), so its spelling
/// has to come from the directory listing: an exact entry wins, otherwise the
/// unique case-insensitive match is the one the OS would open.
///
/// POSIX names are case-sensitive, so `name` already is the entry's spelling and
/// no lookup is needed (N-5: case folding is Windows-only). `None` means the
/// directory holds no entry the OS would match; the caller keeps the lexical
/// spelling, as it did before this lookup existed.
#[cfg(windows)]
fn on_disk_name(parent: &Path, name: &OsStr) -> Result<Option<OsString>> {
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
fn on_disk_name(_parent: &Path, name: &OsStr) -> Result<Option<OsString>> {
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
fn target_rel(target: &Path, canonical_dst: &Path) -> Result<Option<String>> {
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
fn spare_link_components(
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
        if !crate::link::try_is_link(&prefix)
            .with_context(|| format!("check link {}", prefix.display()))?
        {
            continue;
        }
        if let Some(disk) = target_rel(&prefix, canonical_dst)? {
            storage.keep_path(&plan_rel(on_disk, disk));
        }
    }
    Ok(())
}

/// Spare `alias` and every link on its way to the root.
///
/// `alias` is the spelling the walk reported for the target (`link/x`), so the
/// `dm` lookup can name a link the canonical parent resolved away. The alias
/// itself is spared even when it is not a link: it is the same storage, and
/// dropping it from `diff.extra` keeps the printed plan and the applied plan
/// naming the same set.
fn spare_alias_chain(alias: &str, dm: &HashMap<String, EffRec>, storage: &mut LinkStorage) {
    let mut end = 0;
    loop {
        let (prefix, next) = match alias[end..].find('/') {
            Some(i) => (&alias[..end + i], Some(end + i + 1)),
            None => (alias, None),
        };
        if dm.get(prefix).is_some_and(|rec| rec.is_link) {
            storage.keep_path(prefix);
        }
        match next {
            Some(next) => end = next,
            None => break,
        }
    }
    storage.keep_path(alias);
}

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
pub(super) fn spare_link_storage(
    sm: &HashMap<String, EffRec>,
    dm: &HashMap<String, EffRec>,
    plan: &mut Plan,
    diff: &mut Diff,
    dst: &Path,
    on_disk: Option<&DiskSpelling>,
) -> Result<()> {
    let mut storage = LinkStorage::default();
    // Nothing to spare when no copy or delete could meet a link target.
    let mut rels: Vec<&String> = dm.keys().collect();
    rels.sort();
    let canonical_dst = std::fs::canonicalize(dst)
        .with_context(|| format!("resolve dst root {}", dst.display()))?;
    // Every kept link's terminal storage: (link, grouping key, display name).
    let mut terminals: Vec<(String, String, String)> = Vec::new();
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
                // ancestors must be spared too (R9-2).
                if let Some(tr) = &canonical_rel {
                    storage.keep_path(tr);
                }
                if let Some(alias) = &target_alias {
                    spare_alias_chain(alias, dm, &mut storage);
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
                        spare_alias_chain(alias, dm, &mut storage);
                    }
                    match sm.get(tr) {
                        Some(trec) if trec.kind == "file" => {
                            if plan
                                .copy
                                .iter()
                                .any(|item| item.rel == *rel || item.rel == tr)
                            {
                                bail!(
                                    "refusing to sync `{rel}`: dst `{rel}` is a link to \
                                     `{tr}`, which src also has; copying either path would \
                                     change the one file they share (remove the link, or \
                                     exclude it with --exclude)"
                                );
                            }
                        }
                        Some(_) => {
                            if plan.fix_dirs.iter().any(|f| f == tr) {
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
                        spare_alias_chain(alias, dm, &mut storage);
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
fn ensure_shared_storage_safe(terminals: &[(String, String, String)], plan: &Plan) -> Result<()> {
    let mut by_storage: HashMap<&str, Vec<&str>> = HashMap::new();
    for (rel, key, _) in terminals {
        by_storage.entry(key.as_str()).or_default().push(rel);
    }
    let mut groups: Vec<_> = by_storage
        .into_iter()
        .filter(|(_, rels)| rels.len() > 1)
        .collect();
    // Deterministic message when several groups qualify.
    groups.sort_by(|a, b| a.0.cmp(b.0));
    for (key, rels) in groups {
        if !plan
            .copy
            .iter()
            .any(|item| rels.contains(&item.rel.as_str()))
        {
            continue;
        }
        let label = terminals
            .iter()
            .find(|(_, k, _)| k.as_str() == key)
            .map(|(_, _, label)| label.as_str())
            .unwrap_or(key);
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
    on_disk: Option<&DiskSpelling>,
) -> Result<()> {
    let fixed: HashSet<&str> = plan.fix_dirs.iter().map(String::as_str).collect();
    // A dry run names the link with its pre-rename spelling while `fix_dirs`
    // is keyed by src's casing, so the P7-2 carve-out has to compare cases too.
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
        if let Some(link) = crate::link::link_ancestor_above(dst, disk.as_ref())? {
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
    on_disk: Option<&DiskSpelling>,
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
        let p = dst.join(disk_rel(on_disk, rel).as_ref());
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
        let d = dst.join(disk_rel(on_disk, &item.rel).as_ref());
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

    fn link_file() -> EffRec {
        EffRec {
            is_link: true,
            ..file()
        }
    }

    fn link_dir() -> EffRec {
        EffRec {
            is_link: true,
            ..dir()
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

        ensure_hard_linked_targets_safe(&p, &dm, &dst, None)
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

        let err = ensure_hard_linked_targets_safe(&p, &dm, &dst, None)
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

        ensure_hard_linked_targets_safe(&p, &dm, &dst, None)
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

        ensure_hard_linked_targets_safe(&p, &dm, &dst, None)
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

        ensure_hard_linked_targets_safe(&p, &dm, &dst, None)
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

        ensure_hard_linked_targets_safe(&p, &dm, &dst, None)
            .expect("both other names are removed before the copy");

        p.delete_files.pop();
        ensure_hard_linked_targets_safe(&p, &dm, &dst, None)
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
        ensure_link_safe(&fixed, &dst, false, false, None)
            .expect("fix-dirs unlinks `a` before `a/b` is created");

        let unfixed = plan(&["a/b"], &[], &[]);
        let err = ensure_link_safe(&unfixed, &dst, false, false, None)
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
        // `cmd` reads a `/` in an argument as a switch; `Path::join("a/b")`
        // keeps the separator as written, so normalize before handing it over.
        let link = link.to_string_lossy().replace('/', "\\");
        let target = target.to_string_lossy().replace('/', "\\");
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

    /// **P8-2.** A kept link's content is not an extra path. Sparing it has to
    /// drop the `EXTRA` record, the planned delete, and the `rmdir` of every
    /// directory above it — otherwise the delete phase removes the file the
    /// write-through is about to need.
    #[test]
    fn sparing_a_links_content_drops_it_from_every_removal_list() {
        let mut diff = Diff {
            extra: vec![
                "d".to_string(),
                "d/target.bin".to_string(),
                "other.bin".to_string(),
            ],
            ..Diff::default()
        };
        let mut p = plan(&[], &[], &[]);
        p.delete_files = vec!["d/target.bin".to_string(), "other.bin".to_string()];
        p.rmdir = vec!["d".to_string(), "gone".to_string()];

        let mut storage = LinkStorage::default();
        storage.keep.insert("d/target.bin".to_string());
        keep_ancestors("d/target.bin", &mut storage.keep_dirs);
        storage.apply(&mut diff, &mut p);

        assert_eq!(
            diff.extra,
            ["other.bin"],
            "the record and the action name the same set"
        );
        assert_eq!(p.delete_files, ["other.bin"], "the content is spared");
        assert_eq!(p.rmdir, ["gone"], "and so is the directory holding it");
    }

    /// A `read_link` target can be relative or carry `..`; the plan needs the
    /// dst-relative spelling of the entry, not of the link text.
    #[test]
    fn a_link_target_is_named_relative_to_the_dst_root() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "target_rel");
        let dst = t.subdir("dst");
        let outside = t.subdir("outside");
        std::fs::create_dir_all(dst.join("d")).unwrap();
        std::fs::write(dst.join("d/t.bin"), b"x").unwrap();
        std::fs::write(outside.join("x.bin"), b"x").unwrap();
        let root = std::fs::canonicalize(&dst).unwrap();

        assert_eq!(
            target_rel(&dst.join("d").join("..").join("d").join("t.bin"), &root).unwrap(),
            Some("d/t.bin".to_string()),
            "the resolved parent plus the final name, not the link text"
        );
        assert_eq!(
            target_rel(&outside.join("x.bin"), &root).unwrap(),
            None,
            "a target outside dst is the documented write-through, not storage"
        );
    }

    /// A relative target is named from the directory holding its link, so the
    /// alias spelling survives a `..` that stays inside the root.
    #[test]
    fn a_relative_target_keeps_its_alias_spelling() {
        assert_eq!(
            alias_target("a.bin", Path::new("link/x.bin")).as_deref(),
            Some("link/x.bin"),
            "a target one level down keeps the walk's spelling"
        );
        assert_eq!(
            alias_target("d/a.bin", Path::new("../x.bin")).as_deref(),
            Some("x.bin"),
            "`..` is resolved against the link's parent, not dst"
        );
        assert_eq!(
            alias_target("a.bin", Path::new("../../x.bin")),
            None,
            "a target that escapes the root has no alias under it"
        );
    }

    /// **R9-1.** `dst/a -> link/x.bin` with `link` a directory link. The parent
    /// canonicalization names only `real/x.bin`, so the alias `link` the walk
    /// reported used to stay in `delete_files`; the run unlinked it and left `a`
    /// dangling. Both spellings, and every link prefix, are spared.
    #[cfg(any(unix, windows))]
    #[test]
    fn a_kept_link_through_a_directory_link_spares_the_alias_and_the_target() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "link_component");
        let dst = t.subdir("dst");
        std::fs::create_dir_all(dst.join("real")).unwrap();
        std::fs::write(dst.join("real/x.bin"), b"x").unwrap();
        if let Err(e) = make_dir_link(&dst.join("real"), &dst.join("link")) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }
        let root = std::fs::canonicalize(&dst).unwrap();
        let target = dst.join("link/x.bin");

        let mut storage = LinkStorage::default();
        spare_link_components(&target, &root, None, &mut storage).unwrap();
        assert!(
            storage.keep.contains("link"),
            "the walk's alias for the intermediate link is spared: {:?}",
            storage.keep
        );
        assert_eq!(
            target_rel(&target, &root).unwrap().as_deref(),
            Some("real/x.bin"),
            "the canonical spelling names the same content"
        );

        let mut dm = HashMap::new();
        dm.insert("link".to_string(), link_dir());
        dm.insert("link/x.bin".to_string(), link_file());
        spare_alias_chain("link/x.bin", &dm, &mut storage);
        storage.keep_path("real/x.bin");

        let mut diff = Diff {
            extra: vec![
                "link".to_string(),
                "link/x.bin".to_string(),
                "real".to_string(),
                "real/x.bin".to_string(),
            ],
            ..Diff::default()
        };
        let mut p = plan(&[], &[], &[]);
        p.delete_files = vec!["link".to_string(), "real/x.bin".to_string()];
        p.rmdir = vec!["real".to_string()];
        storage.apply(&mut diff, &mut p);

        assert!(
            p.delete_files.is_empty(),
            "the delete plan cannot unlink the alias: {:?}",
            p.delete_files
        );
        assert!(
            p.rmdir.is_empty(),
            "nor rmdir the target's directory: {:?}",
            p.rmdir
        );
        assert!(
            diff.extra.is_empty(),
            "the record and the action name the same set: {:?}",
            diff.extra
        );
    }

    /// **R9-2.** A spared **intermediate** link used to skip `keep_ancestors`, so
    /// its dst-only directory stayed in `plan.rmdir` and `remove_dir_all` took
    /// the link with it. `keep_path` is the single insertion point now.
    #[test]
    fn a_spared_intermediate_link_keeps_its_ancestor_directories() {
        let mut dm = HashMap::new();
        dm.insert("extra/b".to_string(), link_file());
        let mut storage = LinkStorage::default();
        spare_alias_chain("extra/b", &dm, &mut storage);
        assert!(storage.keep.contains("extra/b"), "{:?}", storage.keep);
        assert!(
            storage.keep_dirs.contains("extra"),
            "the directory above the kept link is spared: {:?}",
            storage.keep_dirs
        );

        let mut diff = Diff {
            extra: vec!["extra".to_string(), "extra/b".to_string()],
            ..Diff::default()
        };
        let mut p = plan(&[], &[], &[]);
        p.delete_files = vec!["extra/b".to_string()];
        p.rmdir = vec!["extra".to_string()];
        storage.apply(&mut diff, &mut p);

        assert!(p.delete_files.is_empty(), "{:?}", p.delete_files);
        assert!(
            p.rmdir.is_empty(),
            "rmdir cannot take the kept link's directory: {:?}",
            p.rmdir
        );
        assert!(diff.extra.is_empty(), "{:?}", diff.extra);
    }

    /// The full spare pass over a chain that passes through a dst directory
    /// link. This host refuses file symlinks, so the kept link is a directory
    /// junction and `dm` describes it as a file link — which is exactly the
    /// input the resolver reads; only `read_link` matters for the chain.
    #[cfg(any(unix, windows))]
    #[test]
    fn the_spare_pass_walks_through_a_directory_link_component() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "spare_component");
        let dst = t.subdir("dst");
        std::fs::create_dir_all(dst.join("real/x")).unwrap();
        if let Err(e) = make_dir_link(&dst.join("real"), &dst.join("link")) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }
        if let Err(e) = make_dir_link(&dst.join("link/x"), &dst.join("a")) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }
        let mut sm = HashMap::new();
        sm.insert("a".to_string(), file());
        let mut dm = HashMap::new();
        dm.insert("a".to_string(), link_file());
        dm.insert("link".to_string(), link_dir());
        dm.insert("link/x".to_string(), dir());
        dm.insert("real".to_string(), dir());
        dm.insert("real/x".to_string(), dir());

        let mut diff = Diff {
            extra: vec![
                "link".to_string(),
                "link/x".to_string(),
                "real".to_string(),
                "real/x".to_string(),
            ],
            ..Diff::default()
        };
        let mut p = plan(&[], &[], &[]);
        p.delete_files = vec!["link".to_string(), "real/x".to_string()];
        p.rmdir = vec!["real".to_string()];

        spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None).unwrap();

        assert!(
            p.delete_files.is_empty(),
            "the intermediate link and its canonical target are spared: {:?}",
            p.delete_files
        );
        assert!(p.rmdir.is_empty(), "{:?}", p.rmdir);
        assert!(diff.extra.is_empty(), "{:?}", diff.extra);
    }

    /// The full spare pass over an intermediate link under a dst-only
    /// directory. Junctions stand in for file links as above; the ancestor is
    /// the point.
    #[cfg(any(unix, windows))]
    #[test]
    fn the_spare_pass_keeps_an_intermediate_links_ancestors() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "spare_intermediate");
        let dst = t.subdir("dst");
        let outside = t.subdir("outside");
        std::fs::create_dir_all(dst.join("extra")).unwrap();
        if let Err(e) = make_dir_link(&outside, &dst.join("extra/b")) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }
        if let Err(e) = make_dir_link(&dst.join("extra/b"), &dst.join("a")) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }
        let mut sm = HashMap::new();
        sm.insert("a".to_string(), file());
        let mut dm = HashMap::new();
        dm.insert("a".to_string(), link_file());
        dm.insert("extra".to_string(), dir());
        dm.insert("extra/b".to_string(), link_dir());

        let mut diff = Diff {
            extra: vec!["extra".to_string(), "extra/b".to_string()],
            ..Diff::default()
        };
        let mut p = plan(&[], &[], &[]);
        p.delete_files = vec!["extra/b".to_string()];
        p.rmdir = vec!["extra".to_string()];

        spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None).unwrap();

        assert!(p.delete_files.is_empty(), "{:?}", p.delete_files);
        assert!(
            p.rmdir.is_empty(),
            "the directory above the intermediate link is spared: {:?}",
            p.rmdir
        );
        assert!(diff.extra.is_empty(), "{:?}", diff.extra);
    }

    /// The full spare pass over two kept links sharing one dst-only storage,
    /// including the chained shape. Junctions stand in for file links; both
    /// links resolve through `read_link` exactly as the real shape would.
    #[cfg(any(unix, windows))]
    #[test]
    fn the_spare_pass_refuses_two_links_on_one_storage() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "spare_shared");
        let dst = t.subdir("dst");
        std::fs::create_dir_all(dst.join("c")).unwrap();
        if let Err(e) = make_dir_link(&dst.join("c"), &dst.join("a")) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }
        if let Err(e) = make_dir_link(&dst.join("c"), &dst.join("b")) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }
        let mut sm = HashMap::new();
        sm.insert("a".to_string(), file());
        sm.insert("b".to_string(), file());
        let mut dm = HashMap::new();
        dm.insert("a".to_string(), link_file());
        dm.insert("b".to_string(), link_file());
        dm.insert("c".to_string(), dir());

        let mut p = plan(&[], &["a", "b"], &[]);
        let mut diff = Diff::default();
        let err = spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None)
            .expect_err("two kept links on one inode cannot both be copied");
        let msg = err.to_string();
        assert!(
            msg.contains("`a`") && msg.contains("`b`"),
            "names both links: {msg}"
        );

        // Chained: `a -> b -> c`; the same terminal groups the two kept links.
        let t = girpr_tempdir::TempDir::new("girsync_plan", "spare_shared_chain");
        let dst = t.subdir("dst");
        std::fs::create_dir_all(dst.join("c")).unwrap();
        if let Err(e) = make_dir_link(&dst.join("c"), &dst.join("b")) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }
        if let Err(e) = make_dir_link(&dst.join("b"), &dst.join("a")) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }
        let mut dm = HashMap::new();
        dm.insert("a".to_string(), link_file());
        dm.insert("b".to_string(), link_file());
        dm.insert("c".to_string(), dir());
        let mut p = plan(&[], &["a", "b"], &[]);
        let err = spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None)
            .expect_err("the chained pair shares the same terminal storage");
        let msg = err.to_string();
        assert!(
            msg.contains("`a`") && msg.contains("`b`"),
            "names both links: {msg}"
        );
    }

    /// **R10-1.** The final component came back in the link text's spelling, so
    /// on a case-insensitive filesystem `REAL/X.BIN` missed the walk's
    /// `real/x.bin` key in `sm`/`dm`, in the spare's `delete_files` match and in
    /// the R9-3 grouping. The resolver must name the entry the walk reported.
    #[test]
    fn a_case_differing_final_component_names_the_walk_entry() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "target_case");
        let dst = t.subdir("dst");
        std::fs::create_dir_all(dst.join("real")).unwrap();
        std::fs::write(dst.join("real/x.bin"), b"x").unwrap();
        let root = std::fs::canonicalize(&dst).unwrap();
        if std::fs::metadata(dst.join("REAL/X.BIN")).is_err() {
            eprintln!("skip: case-sensitive filesystem; `REAL/X.BIN` is another entry");
            return;
        }

        assert_eq!(
            target_rel(&dst.join("REAL/X.BIN"), &root).unwrap(),
            Some("real/x.bin".to_string()),
            "both spellings name the one entry the walk reported"
        );
        assert_eq!(
            target_rel(&dst.join("real/x.bin"), &root).unwrap(),
            Some("real/x.bin".to_string()),
            "the walk's own spelling is unchanged"
        );
    }

    /// **R10-1, dry-run parity.** A case-only rename of the target is routine
    /// for a mirror. A real run moves the entry to src's casing before the plan,
    /// while a dry run leaves it on disk and maps the spelling forward. Both
    /// must hand `sm`/`dm`, the spare and the R9-3 grouping the same key.
    #[test]
    fn a_case_only_rename_of_the_target_is_one_spare_in_both_modes() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "target_case_rename");
        let dst = t.subdir("dst");
        std::fs::create_dir_all(dst.join("real")).unwrap();
        std::fs::write(dst.join("real/x.bin"), b"x").unwrap();
        let root = std::fs::canonicalize(&dst).unwrap();
        if std::fs::metadata(dst.join("real/X.BIN")).is_err() {
            eprintln!("skip: case-sensitive filesystem; the rename is not a pair");
            return;
        }
        let spelling =
            DiskSpelling::new(&[("real/x.bin".to_string(), "real/X.BIN".to_string())], &[]);

        // Dry run: the entry is still `x.bin` on disk; the guard maps forward.
        let dry = target_rel(&dst.join("real/X.BIN"), &root)
            .unwrap()
            .map(|disk| plan_rel(Some(&spelling), disk));
        std::fs::rename(dst.join("real/x.bin"), dst.join("real/X.BIN")).unwrap();
        // Real run: the rename already happened, so the disk answers directly.
        let real = target_rel(&dst.join("real/X.BIN"), &root)
            .unwrap()
            .map(|disk| plan_rel(None, disk));

        assert_eq!(
            dry.as_deref(),
            Some("real/X.BIN"),
            "dry run names the plan key"
        );
        assert_eq!(real, dry, "the two modes agree on the spare's key");
    }

    /// **R10-1, R9-1's half.** A target whose intermediate link is spelled with
    /// another case (`LINK` on disk is `link`) spared the link-text spelling, so
    /// the walk's `link` stayed in `delete_files` and the run unlinked it.
    #[cfg(any(unix, windows))]
    #[test]
    fn a_case_differing_link_component_is_spared_under_the_walk_spelling() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "link_component_case");
        let dst = t.subdir("dst");
        std::fs::create_dir_all(dst.join("real")).unwrap();
        std::fs::write(dst.join("real/x.bin"), b"x").unwrap();
        if let Err(e) = make_dir_link(&dst.join("real"), &dst.join("link")) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }
        if std::fs::metadata(dst.join("LINK")).is_err() {
            eprintln!("skip: case-sensitive filesystem; `LINK` is another entry");
            return;
        }
        let root = std::fs::canonicalize(&dst).unwrap();
        let target = dst.join("LINK/x.bin");

        let mut storage = LinkStorage::default();
        spare_link_components(&target, &root, None, &mut storage).unwrap();
        assert!(
            storage.keep.contains("link"),
            "the walk's alias for the link component is spared: {:?}",
            storage.keep
        );
        assert!(
            !storage.keep.contains("LINK"),
            "the link text's spelling is not a key the walk reported: {:?}",
            storage.keep
        );
    }

    /// **R10-1, the mirrored-target refusal.** The same spelling feeds the
    /// `sm.get` key, so a case-differing target used to look absent and the
    /// plan copied through the link instead of refusing it. The terminal's map
    /// record is the guard's input; the junction stands in for the file link.
    #[cfg(any(unix, windows))]
    #[test]
    fn a_case_differing_mirrored_target_is_refused() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "mirrored_case");
        let dst = t.subdir("dst");
        std::fs::create_dir_all(dst.join("real")).unwrap();
        if let Err(e) = make_dir_link(&dst.join("REAL"), &dst.join("a")) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }
        if std::fs::metadata(dst.join("REAL")).is_err() {
            eprintln!("skip: case-sensitive filesystem; `REAL` is another entry");
            return;
        }
        let mut sm = HashMap::new();
        sm.insert("a".to_string(), file());
        sm.insert("real".to_string(), file());
        let mut dm = HashMap::new();
        dm.insert("a".to_string(), link_file());
        dm.insert("real".to_string(), file());

        let mut diff = Diff::default();
        let mut p = plan(&[], &["a"], &[]);
        let err = spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None)
            .expect_err("a mirrored target cannot also be given a new content");
        let msg = err.to_string();
        assert!(msg.contains("`real`"), "names the target's walk key: {msg}");
    }

    /// **R10-1 through the full spare pass.** A junction's `read_link` target
    /// keeps the spelling it was created with, so a case-differing terminal
    /// (`REAL` on disk is `real`) spared a key no removal list holds: the
    /// directory stayed in `rmdir` and `remove_dir_all` took the link's storage.
    /// The junction stands in for the file link this host refuses.
    #[cfg(any(unix, windows))]
    #[test]
    fn a_case_differing_terminal_is_spared_by_the_full_pass() {
        let t = girpr_tempdir::TempDir::new("girsync_plan", "spare_terminal_case");
        let dst = t.subdir("dst");
        std::fs::create_dir_all(dst.join("real")).unwrap();
        if let Err(e) = make_dir_link(&dst.join("REAL"), &dst.join("a")) {
            eprintln!("skip: this platform refused a directory link: {e}");
            return;
        }
        if std::fs::metadata(dst.join("REAL")).is_err() {
            eprintln!("skip: case-sensitive filesystem; `REAL` is another entry");
            return;
        }
        let mut sm = HashMap::new();
        sm.insert("a".to_string(), file());
        let mut dm = HashMap::new();
        dm.insert("a".to_string(), link_file());
        dm.insert("real".to_string(), dir());

        let mut diff = Diff {
            extra: vec!["real".to_string()],
            ..Diff::default()
        };
        let mut p = plan(&[], &[], &[]);
        p.rmdir = vec!["real".to_string()];

        spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None).unwrap();

        assert!(
            p.rmdir.is_empty(),
            "the terminal's owning directory is spared: {:?}",
            p.rmdir
        );
        assert!(diff.extra.is_empty(), "{:?}", diff.extra);
    }

    /// **R9-3.** Two kept links share one dst-only file. Neither terminal target
    /// is mirrored by src, so the per-link check never fires; copying both wrote
    /// two contents through one inode and one mirrored path ended wrong at exit
    /// 0. The set is judged as a whole, including the chained shape.
    #[test]
    fn two_kept_links_on_one_storage_refuse_when_either_is_copied() {
        let terminals = vec![
            ("a".to_string(), "c".to_string(), "c".to_string()),
            ("b".to_string(), "c".to_string(), "c".to_string()),
        ];
        let mut p = plan(&[], &["a", "b"], &[]);
        let err = ensure_shared_storage_safe(&terminals, &p)
            .expect_err("one inode cannot be given two contents");
        let msg = err.to_string();
        assert!(
            msg.contains("`a`") && msg.contains("`b`"),
            "names both links: {msg}"
        );
        assert!(msg.contains("`c`"), "names the shared file: {msg}");

        // The chained `a -> b -> c` group is the same terminal; copying only
        // `a` already changes what `b` resolves to.
        p.copy.truncate(1);
        assert!(
            ensure_shared_storage_safe(&terminals, &p).is_err(),
            "copying one end breaks the other"
        );

        // Nothing copied: both paths already agree with the one file.
        ensure_shared_storage_safe(&terminals, &Plan::default()).expect("no write, no conflict");
    }
}
