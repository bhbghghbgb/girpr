//! The case-fixing rename pass.
//!
//! In insensitive mode a path that exists on both sides under different casing
//! must be renamed on dst *before* the diff runs, so the two maps line up on
//! exact keys. After this pass the diff can be taken case-sensitively.

use anyhow::{Context, Result, bail};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::info;

use crate::report::{Record, Report};

/// The point of a case-fix step a test hook asks to fail at.
///
/// `GIRSYNC_TEST_FAIL_RENAME` keeps its P7-3 meaning: fail before the temporary
/// is claimed. `GIRSYNC_TEST_FAIL_RENAME_STEP1` fails after the claim (so the
/// file placeholder exists) and `GIRSYNC_TEST_FAIL_RENAME_STEP2` after step 1
/// (so the entry is in the temporary). The three are separately named because
/// they exercise three different cleanup paths: nothing to clean, placeholder
/// removal, and rollback.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    BeforeClaim,
    Step1,
    Step2,
}

/// Whether the test-only hook for `stage` names `rel`.
///
/// Default **off**: an unset variable matches nothing. A test needs the failure
/// because a real failed `fs::rename` (a lock, a permission, a volume that went
/// away) cannot be arranged portably; naming one relative path exercises the
/// same error path the production call takes, including the rule that a failed
/// rename publishes no `RENAME` record.
fn test_fail_rename(rel: &str, stage: Stage) -> bool {
    static BEFORE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    static STEP1: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    static STEP2: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let (var, slot) = match stage {
        Stage::BeforeClaim => ("GIRSYNC_TEST_FAIL_RENAME", &BEFORE),
        Stage::Step1 => ("GIRSYNC_TEST_FAIL_RENAME_STEP1", &STEP1),
        Stage::Step2 => ("GIRSYNC_TEST_FAIL_RENAME_STEP2", &STEP2),
    };
    slot.get_or_init(|| std::env::var(var).ok()).as_deref() == Some(rel)
}

/// The dst paths that must move to src's casing, as `(dst rel, src rel)` pairs.
///
/// The decision, with no filesystem or cache involved, so it can be stated as a
/// set. **A dst path is renamed exactly when src holds the same path under
/// different casing** — the pair whose lowercase key matches and whose exact key
/// does not.
///
/// Both sides are indexed by lowercase key and every src key is looked up, rather
/// than every src key scanning every dst key and lowercasing as it goes. The result
/// is the same set, and it is the same set because a lowercase key maps to exactly
/// one path per side: `check_mixed_case` has already made a mixed-case collision
/// within one side fatal, and this pass runs only in the insensitive mode that check
/// guards. Without that invariant the two forms could disagree on *which* of two
/// colliding keys they pick, which is why this is not a free substitution.
///
/// Ordered by lowercase src key, because these become `RENAME` records in report
/// order and `HashMap` iteration is seeded per process — an unsorted pass would
/// reshuffle between two runs over the same tree.
fn case_renames<'a>(
    src: &'a HashMap<String, crate::effective::EffRec>,
    dm: &'a HashMap<String, crate::effective::EffRec>,
) -> Vec<(&'a String, &'a String)> {
    // lower -> rel, per side.
    let mut slow: HashMap<String, &String> = HashMap::new();
    for k in src.keys() {
        slow.insert(k.to_lowercase(), k);
    }
    let mut dlow: HashMap<String, &String> = HashMap::new();
    for k in dm.keys() {
        dlow.insert(k.to_lowercase(), k);
    }
    let mut by_lower: Vec<(String, &String)> = slow.into_iter().collect();
    by_lower.sort();
    let mut out: Vec<(&String, &String)> = Vec::new();
    for (lk, srel) in &by_lower {
        // The same path on both sides is not a rename: it is already correct.
        if let Some(drel) = dlow.get(lk).copied().filter(|d| *d != *srel) {
            out.push((drel, srel));
        }
    }
    out
}

/// One rename the pass will perform.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RenameStep {
    /// Where the entry is when this step runs, after ancestor renames.
    from: String,
    /// The path src gives it.
    to: String,
    /// The dst path before the pass, which is the cache row's key.
    original_from: String,
    /// Whether this step moves a directory, so later paths under it move too.
    is_dir: bool,
    /// False when an ancestor's rename already put this path on src's casing;
    /// the step then has no filesystem or record work.
    perform: bool,
}

/// Turn the rename pairs into steps in a parent-first order, rewriting each
/// `from` through the directory renames already planned.
///
/// `case_renames` sorts by lowercase src key, so a directory always precedes
/// everything beneath it. Renaming the parent moves its whole subtree, so a
/// child whose only difference was the parent's casing is already correct when
/// its turn comes: it is kept as a no-op step so its cache row and map key can
/// still follow, but it must not emit a `RENAME` or call `fs::rename` on a path
/// that no longer exists. A child whose own leaf casing also differs is renamed
/// at its new parent path.
fn plan_steps(
    renames: &[(String, String)],
    dm: &HashMap<String, crate::effective::EffRec>,
) -> Vec<RenameStep> {
    let mut dir_moves: Vec<(String, String)> = Vec::new();
    let mut steps = Vec::with_capacity(renames.len());
    for (from, to) in renames {
        let effective_from = rewritten_from(from, &dir_moves);
        let is_dir = dm.get(from).is_some_and(|r| r.kind == "dir");
        let perform = effective_from != *to;
        steps.push(RenameStep {
            from: effective_from,
            to: to.clone(),
            original_from: from.clone(),
            is_dir,
            perform,
        });
        if perform && is_dir {
            // Keyed by the original path: later `from`s are still spelled the
            // way the walk found them.
            dir_moves.push((from.clone(), to.clone()));
        }
    }
    steps
}

/// Rewrite `from` through the directory renames already performed, choosing the
/// longest matching prefix.
///
/// The longest prefix matters for nested renames: with `dir` -> `Dir` and
/// `dir/sub` -> `Dir/Sub`, a path `dir/sub/f` must become `Dir/Sub/f`, not
/// `Dir/sub/f`. A mapping matches only on a component boundary, so `dir` does
/// not rewrite `dir2/f`.
fn rewritten_from(from: &str, dir_moves: &[(String, String)]) -> String {
    let mut best: Option<&(String, String)> = None;
    for mv in dir_moves {
        let old = &mv.0;
        let boundary = from == old
            || from
                .strip_prefix(old)
                .is_some_and(|rest| rest.starts_with('/'));
        if boundary && best.is_none_or(|b| old.len() > b.0.len()) {
            best = Some(mv);
        }
    }
    match best {
        Some((old, new)) => format!("{new}{}", &from[old.len()..]),
        None => from.to_string(),
    }
}

/// The key a dst map/cache entry has after the pass: an exact rename wins,
/// otherwise the path follows its renamed ancestor directories.
fn final_key(key: &str, renames: &[(String, String)], dir_moves: &[(String, String)]) -> String {
    match renames.iter().find(|(from, _)| from == key) {
        Some((_, to)) => to.clone(),
        None => rewritten_from(key, dir_moves),
    }
}

/// A unique sibling path for the two-step rename, claimed before it is used.
///
/// The candidate is a **fixed short** name, `.girsync_case_tmp.<n>`, in the
/// source's parent. It is deliberately not derived from the source name: the
/// first version replaced the extension (`data.girsync_case_tmp`, silently
/// overwriting a user file) and P7-1's full-name suffix
/// (`<name>.girsync_case_tmp.<n>`) grew past `NAME_MAX` for a source whose own
/// name was already close to the limit, so a legal case-only rename failed with
/// `ENAMETOOLONG`. A fixed name cannot do that, whatever the source is called.
///
/// The name is still in the user's namespace, so `create_new`/`create_dir` is
/// what claims it: an occupied candidate is stepped over and left exactly as it
/// was, and a leftover from an interrupted run occupies one number and is
/// skipped by the next run. Two runs over the same tree therefore cannot pick
/// the same temporary.
///
/// The placeholder is removed for a directory because Windows cannot rename a
/// directory onto an existing entry; a file placeholder is left in place and the
/// step1 rename replaces it (it is ours, empty, and never user data). A failed
/// step1 removes the file placeholder again.
fn claim_case_tmp(from: &Path, is_dir: bool) -> Result<PathBuf> {
    let parent = from.parent().context("case-fix path has no parent")?;
    for n in 0..1000 {
        let cand = parent.join(format!(".girsync_case_tmp.{n}"));
        // Ask first whether the name is taken: `create_new` on an existing
        // *directory* reports `Access is denied` rather than `AlreadyExists`, so
        // the error kind alone cannot distinguish "occupied" from "cannot write
        // here".
        match std::fs::symlink_metadata(&cand) {
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e).with_context(|| format!("stat {}", cand.display()));
            }
        }
        let claimed = if is_dir {
            std::fs::create_dir(&cand)
        } else {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&cand)
                .map(drop)
        };
        match claimed {
            Ok(()) => {
                if is_dir {
                    std::fs::remove_dir(&cand).with_context(|| {
                        format!("remove case-fix placeholder {}", cand.display())
                    })?;
                }
                return Ok(cand);
            }
            Err(e) => {
                // A racing creator, or a candidate that appeared between the
                // stat and the claim: the name is ours only if we created it.
                match std::fs::symlink_metadata(&cand) {
                    Ok(_) => continue,
                    Err(_) => {
                        return Err(e).with_context(|| format!("claim {}", cand.display()));
                    }
                }
            }
        }
    }
    bail!("no free case-fix temporary beside {}", from.display())
}

/// The error a failed step 1 leaves behind, after removing the file placeholder.
///
/// A file claim leaves an empty file that step 1 replaces only on success. It is
/// the tool's own residue inside dst, so it must not outlive the failure — and a
/// cleanup that fails is part of the failure the user needs to see, not a result
/// to drop. (The step-2 rollback reports its own secondary failure beside the
/// primary one in the same way.) A directory claim has no placeholder left:
/// Windows cannot rename a directory onto an existing entry, so it is removed
/// before it is returned.
fn fail_step1(step1: std::io::Error, from: &Path, tmp: &Path, is_dir: bool) -> anyhow::Error {
    let primary = || format!("rename {} (case fix step1)", from.display());
    if is_dir {
        return anyhow::Error::new(step1).context(primary());
    }
    match std::fs::remove_file(tmp) {
        Ok(()) => anyhow::Error::new(step1).context(primary()),
        Err(cleanup) => anyhow::Error::new(step1).context(format!(
            "{}; removing the placeholder {} also failed: {cleanup}",
            primary(),
            tmp.display()
        )),
    }
}

/// The error a failed step 2 leaves behind, after putting the entry back.
///
/// The first error is the reason; a rollback's own error is reported beside it
/// rather than replacing it.
fn fail_step2(step2: std::io::Error, from: &Path, tmp: &Path, to: &Path) -> anyhow::Error {
    match std::fs::rename(tmp, from) {
        Ok(()) => anyhow::Error::new(step2)
            .context(format!("rename to {} (case fix step2)", to.display())),
        Err(rollback) => anyhow::Error::new(step2).context(format!(
            "rename to {} (case fix step2); rolling back to {} also failed: {rollback}",
            to.display(),
            from.display()
        )),
    }
}

/// What the pass did, and — in a dry run — where the entries still are.
#[derive(Debug, Default)]
pub(super) struct RenameOutcome {
    /// Renames performed, or that a dry run would perform.
    pub renamed: usize,
    /// `Some` in a dry run — empty when no rename would happen: a real run
    /// moves the filesystem entries onto src's casing, so afterwards the
    /// post-rename spelling *is* the disk spelling. The plan-time guards run
    /// after this pass, and a dry run has re-keyed only the map; they must stat
    /// the old spelling or they answer a different question than the real run.
    pub on_disk: Option<DiskSpelling>,
}

/// Maps a post-rename relative path to the spelling still on disk after a dry
/// run. Longest prefix wins, so a nested directory move rewrites its whole
/// subtree and a component boundary is respected.
#[derive(Debug, Default)]
pub(super) struct DiskSpelling {
    /// `(post-rename, on-disk)` moves. The same path can carry two entries when
    /// a directory move and its own rename both name it; the longest prefix is
    /// identical, so either answer is right.
    moves: Vec<(String, String)>,
}

impl DiskSpelling {
    pub(super) fn new(renames: &[(String, String)], dir_moves: &[(String, String)]) -> Self {
        let mut moves = Vec::with_capacity(renames.len() + dir_moves.len());
        for (old, new) in renames {
            moves.push((new.clone(), old.clone()));
        }
        for (old, new) in dir_moves {
            moves.push((new.clone(), old.clone()));
        }
        Self { moves }
    }

    /// The on-disk spelling of `rel`, which the pass reports under `rel`.
    pub(super) fn disk_rel<'a>(&'a self, rel: &'a str) -> Cow<'a, str> {
        let mut best: Option<&(String, String)> = None;
        for mov in &self.moves {
            let boundary = rel == mov.0
                || rel
                    .strip_prefix(&mov.0)
                    .is_some_and(|rest| rest.starts_with('/'));
            if boundary && best.is_none_or(|b| mov.0.len() > b.0.len()) {
                best = Some(mov);
            }
        }
        match best {
            Some((new, old)) => Cow::Owned(format!("{old}{}", &rel[new.len()..])),
            None => Cow::Borrowed(rel),
        }
    }

    /// The inverse of [`Self::disk_rel`]: the post-rename spelling of an
    /// on-disk path.
    ///
    /// The disk-resolving plan preflight ([`super::plan`]'s link-storage check)
    /// canonicalizes a link target, which answers in the *old* spelling, and must
    /// then look the path up in the post-rename plan. Longest old prefix wins, the
    /// same boundary rule in the other direction.
    pub(super) fn plan_rel<'a>(&'a self, rel: &'a str) -> Cow<'a, str> {
        let mut best: Option<&(String, String)> = None;
        for mov in &self.moves {
            let old = &mov.1;
            let boundary = rel == old
                || rel
                    .strip_prefix(old)
                    .is_some_and(|rest| rest.starts_with('/'));
            if boundary && best.is_none_or(|b| old.len() > b.1.len()) {
                best = Some(mov);
            }
        }
        match best {
            Some((new, old)) => Cow::Owned(format!("{new}{}", &rel[old.len()..])),
            None => Cow::Borrowed(rel),
        }
    }
}

/// Rename dst paths to src's casing, updating the dst cache and `dm` in step.
///
/// Returns what the pass did. `dm` is always re-keyed to src's casing, including
/// under `dry_run` — the plan computed after this pass reads `dm`, so a map left
/// on dst's own casing would make a dry run report a case-only difference as
/// both a copy and a delete. `dry_run` therefore suppresses only the filesystem
/// and cache writes; [`RenameOutcome::on_disk`] carries the spelling the guards
/// must stat when those writes were suppressed.
pub(super) fn rename_to_src_casing(
    src: &HashMap<String, crate::effective::EffRec>,
    dst_root: &Path,
    dst_db: &crate::cache::CacheDb,
    dm: &mut HashMap<String, crate::effective::EffRec>,
    dry_run: bool,
    report: &Report,
) -> Result<RenameOutcome> {
    let renames: Vec<(String, String)> = case_renames(src, dm)
        .into_iter()
        .map(|(drel, srel)| (drel.clone(), srel.clone()))
        .collect();
    info!(pending = renames.len(), "rename pass");
    // Validate the whole pass before the first filesystem call: a rename under a
    // dst link resolves the link as an intermediate component and moves a file
    // in its target. A partial pass would leave dst half-renamed, so this is a
    // pre-loop check rather than a guard inside the loop.
    for (drel, srel) in &renames {
        if let Some(link) = crate::link::link_ancestor_above(dst_root, drel)? {
            bail!(
                "refusing to rename `{drel}` -> `{srel}` through dst link `{link}`: \
                 remove the link, or exclude it with --exclude"
            );
        }
    }
    let steps = plan_steps(&renames, dm);
    let mut renamed = 0usize;
    for step in &steps {
        if !step.perform {
            // An ancestor rename already moved this path to src's casing. The
            // map and cache row still have to follow; the record must not claim
            // a rename that did not happen.
            continue;
        }
        let record = || Record::pair("RENAME", "->", "from", &step.from, "to", &step.to);
        // A dry run reports the rename it *would* make. That is the one place a
        // record names an operation this process does not perform: the plan
        // afterwards must describe dst in src's casing, and the summary's
        // `renamed` counts the steps a real run would take.
        if dry_run {
            report.emit(record());
            renamed += 1;
            continue;
        }
        if test_fail_rename(&step.from, Stage::BeforeClaim) {
            bail!("test: injected rename failure for {}", step.from);
        }

        let from = dst_root.join(&step.from);
        let to = dst_root.join(&step.to);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("mkdir {}", parent.display()))?;
        }
        // Two-step for the Windows case-only rename. The temporary is a unique
        // sibling claimed before use: the old fixed `with_extension(...)` name was
        // a path in the user's namespace, so a file there was replaced by step1
        // (silent loss) and a directory made step1 fail after the record had
        // already been published.
        let tmp = claim_case_tmp(&from, step.is_dir)?;
        if test_fail_rename(&step.from, Stage::Step1) {
            return Err(fail_step1(
                std::io::Error::other(format!(
                    "test: injected rename failure at step1 for {}",
                    step.from
                )),
                &from,
                &tmp,
                step.is_dir,
            ));
        }
        if let Err(step1) = std::fs::rename(&from, &tmp) {
            return Err(fail_step1(step1, &from, &tmp, step.is_dir));
        }
        if test_fail_rename(&step.from, Stage::Step2) {
            return Err(fail_step2(
                std::io::Error::other(format!(
                    "test: injected rename failure at step2 for {}",
                    step.from
                )),
                &from,
                &tmp,
                &to,
            ));
        }
        if let Err(step2) = std::fs::rename(&tmp, &to) {
            return Err(fail_step2(step2, &from, &tmp, &to));
        }
        // **After** both renames: a record describes an action that happened. A
        // failed step1/step2 above leaves no record and no count.
        report.emit(record());
        renamed += 1;
    }
    // The directory moves actually performed, for descendants that were not
    // themselves renamed (dst-only paths under a renamed parent included).
    let dir_moves: Vec<(String, String)> = steps
        .iter()
        .filter(|s| s.perform && s.is_dir)
        .map(|s| (s.original_from.clone(), s.to.clone()))
        .collect();
    // Re-key the in-memory map, including under `dry_run`: the plan computed
    // after this pass reads `dm`. `final_key` handles both exact file renames
    // and dst-only descendants of a renamed directory.
    let old = std::mem::take(dm);
    for (key, rec) in old {
        dm.insert(final_key(&key, &renames, &dir_moves), rec);
    }
    if !dry_run {
        // Move the cache rows with the files. A row under a renamed directory
        // moves even when that path is dst-only, or `compare-self` would report
        // the path it just moved as missing.
        let keys = dst_db.load_keys()?;
        let mut moves = Vec::new();
        for key in keys {
            let to = final_key(&key, &renames, &dir_moves);
            if to != key
                && let Some(rec) = dst_db.get(&key)?
            {
                moves.push((key, to, rec));
            }
        }
        if !moves.is_empty() {
            let mut w = dst_db.begin_write()?;
            for (from, to, rec) in &moves {
                w.remove(from)?;
                w.put(to, rec)?;
            }
            w.commit()?;
        }
    }
    let on_disk = dry_run.then(|| DiskSpelling::new(&renames, &dir_moves));
    Ok(RenameOutcome { renamed, on_disk })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effective::EffRec;
    use crate::report::OutputFormat;

    fn file_rec() -> EffRec {
        EffRec {
            kind: "file".into(),
            size: 4,
            mtime_ns: 0,
            hashes: crate::hash::DigestSet::default(),
            is_link: false,
        }
    }

    fn dir_rec() -> EffRec {
        EffRec {
            kind: "dir".into(),
            size: 0,
            mtime_ns: 0,
            hashes: crate::hash::DigestSet::default(),
            is_link: false,
        }
    }

    fn side(rels: &[&str]) -> HashMap<String, EffRec> {
        rels.iter().map(|r| (r.to_string(), file_rec())).collect()
    }

    /// The rename set, as sorted `(dst, src)` string pairs.
    fn renames(src: &[&str], dst: &[&str]) -> Vec<(String, String)> {
        let (s, d) = (side(src), side(dst));
        let mut out: Vec<(String, String)> = case_renames(&s, &d)
            .into_iter()
            .map(|(d, s)| (d.clone(), s.clone()))
            .collect();
        out.sort();
        out
    }

    /// The set is exactly "dst holds it under different casing", stated as a set
    /// rather than as a transcript of the records — the ordering is a consequence
    /// of the sort, not the property being pinned.
    ///
    /// Three shapes in one tree: a pure case-only difference, a path already in
    /// step (which must *not* be renamed to itself), and a pair of paths that
    /// differ by more than case and so are not a rename at all.
    #[test]
    fn the_rename_set_is_exactly_the_case_only_differences() {
        assert_eq!(
            renames(
                &[
                    "Data.txt",
                    "same.txt",
                    "Alpha/Beta.txt",
                    "only_src.txt",
                    "only_dst.txt"
                ],
                &["data.txt", "same.txt", "alpha/beta.txt"],
            ),
            vec![
                ("alpha/beta.txt".to_string(), "Alpha/Beta.txt".to_string()),
                ("data.txt".to_string(), "Data.txt".to_string()),
            ],
        );
    }

    /// Zero renames is the common case and the one a lookup can get wrong: the
    /// dst path *is* the src path, so the lowercase key matches and only the
    /// exact-key check stands between the two and a spurious self-rename.
    #[test]
    fn a_tree_already_in_step_renames_nothing() {
        assert_eq!(
            renames(
                &["a.txt", "sub/B.txt", "c.txt"],
                &["a.txt", "sub/B.txt", "c.txt"],
            ),
            Vec::<(String, String)>::new(),
        );
    }

    /// Several renames at once, and the one-sided paths among them stay out of
    /// the set: a path only one side holds has no casing to align.
    #[test]
    fn several_case_only_differences_rename_independently() {
        assert_eq!(
            renames(
                &["One.txt", "Two.txt", "Three.txt", "src_only.txt"],
                &["one.txt", "two.txt", "three.txt", "dst_only.txt"],
            ),
            vec![
                ("one.txt".to_string(), "One.txt".to_string()),
                ("three.txt".to_string(), "Three.txt".to_string()),
                ("two.txt".to_string(), "Two.txt".to_string()),
            ],
        );
    }

    /// A path src does not hold is never renamed, whatever dst calls it — the
    /// pass moves dst onto *src's* casing, so a dst-only path keeps its own.
    #[test]
    fn a_dst_only_path_is_never_a_rename() {
        assert_eq!(
            renames(&["a.txt"], &["a.txt", "Orphan.txt"]),
            Vec::<(String, String)>::new(),
        );
    }

    /// The order is by lowercase src key, not `HashMap` order. These become
    /// `RENAME` records in both modes, and the seed is per process, so unsorted
    /// order would reshuffle between two runs over the same tree.
    #[test]
    fn the_rename_order_is_by_lowercase_key() {
        let s = side(&["Zeta.txt", "Alpha.txt", "Mid.txt"]);
        let d = side(&["zeta.txt", "alpha.txt", "mid.txt"]);
        let ordered: Vec<String> = case_renames(&s, &d)
            .into_iter()
            .map(|(_, s)| s.to_lowercase())
            .collect();
        assert_eq!(ordered, vec!["alpha.txt", "mid.txt", "zeta.txt"]);
    }

    /// A dry run must re-key the dst map to src's casing, not just report that it
    /// would: the plan computed afterwards reads `dm`, so a map left on dst's own
    /// casing reports a case-only difference as both `COPY` and `DELETE`.
    #[test]
    fn dry_run_rekeys_the_dst_map_without_touching_disk() {
        let root = girpr_tempdir::TempDir::new("girsync_rename_dry", "rekeys_map");
        let dst = root.join("dst");
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(dst.join("data.txt"), b"data").unwrap();

        let mut sm = HashMap::new();
        sm.insert("Data.txt".to_string(), file_rec());
        let mut dm = HashMap::new();
        dm.insert("data.txt".to_string(), file_rec());
        let db = crate::cache::CacheDb::open_temp(false).unwrap();

        let report = Report::new(OutputFormat::Text);
        let out = rename_to_src_casing(&sm, &dst, &db, &mut dm, true, &report).unwrap();
        assert_eq!(out.renamed, 1);
        assert_eq!(
            out.on_disk
                .as_ref()
                .expect("a dry run leaves the files where they were")
                .disk_rel("Data.txt"),
            "data.txt",
            "the guards must stat the spelling still on disk"
        );
        assert!(
            dm.contains_key("Data.txt"),
            "dst map follows src casing after a dry run"
        );
        assert!(
            !dm.contains_key("data.txt"),
            "stale casing dropped from the dst map"
        );
        assert!(
            dst.join("data.txt").is_file(),
            "a dry run still renames nothing on disk"
        );
    }

    /// The dry-run spelling map must follow the same longest-prefix rule as the
    /// rename plan: a nested directory move rewrites its subtree, an exact file
    /// rename wins over an ancestor, and a textual prefix is not a boundary.
    #[test]
    fn the_dry_run_spelling_map_undoes_the_planned_renames() {
        let renames = vec![
            ("dir".to_string(), "Dir".to_string()),
            ("dir/sub".to_string(), "Dir/Sub".to_string()),
            ("dir/sub/f.txt".to_string(), "Dir/Sub/f.txt".to_string()),
            ("other.txt".to_string(), "Other.txt".to_string()),
        ];
        let dir_moves = vec![
            ("dir".to_string(), "Dir".to_string()),
            ("dir/sub".to_string(), "Dir/Sub".to_string()),
        ];
        let spelling = DiskSpelling::new(&renames, &dir_moves);

        assert_eq!(spelling.disk_rel("Dir"), "dir");
        assert_eq!(spelling.disk_rel("Dir/Sub/f.txt"), "dir/sub/f.txt");
        assert_eq!(
            spelling.disk_rel("Dir/orphan.txt"),
            "dir/orphan.txt",
            "a dst-only descendant follows its renamed parent"
        );
        assert_eq!(spelling.disk_rel("Other.txt"), "other.txt");
        assert_eq!(
            spelling.disk_rel("Dir2/f.txt"),
            "Dir2/f.txt",
            "a prefix match must end at a component boundary"
        );

        // The inverse, used after canonicalizing a link target: the same
        // longest-prefix rule against the old spelling.
        assert_eq!(spelling.plan_rel("dir/sub/f.txt"), "Dir/Sub/f.txt");
        assert_eq!(
            spelling.plan_rel("dir/orphan.txt"),
            "Dir/orphan.txt",
            "a dst-only descendant follows its renamed parent"
        );
        assert_eq!(spelling.plan_rel("other.txt"), "Other.txt");
        assert_eq!(
            spelling.plan_rel("dir2/f.txt"),
            "dir2/f.txt",
            "a textual prefix is not a component boundary here either"
        );
    }

    /// A real run moved the files, so there is nothing to undo and the guards
    /// stat the post-rename spelling directly.
    #[test]
    fn a_real_run_has_no_disk_spelling_map() {
        let root = girpr_tempdir::TempDir::new("girsync_rename_real", "no_map");
        let dst = root.join("dst");
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(dst.join("data.txt"), b"data").unwrap();

        let mut sm = HashMap::new();
        sm.insert("Data.txt".to_string(), file_rec());
        let mut dm = HashMap::new();
        dm.insert("data.txt".to_string(), file_rec());
        let db = crate::cache::CacheDb::open_temp(false).unwrap();

        let report = Report::new(OutputFormat::Text);
        let out = rename_to_src_casing(&sm, &dst, &db, &mut dm, false, &report).unwrap();
        assert_eq!(out.renamed, 1);
        assert!(out.on_disk.is_none());
        assert!(dst.join("Data.txt").is_file(), "the real rename happened");
    }

    /// A directory rename moves its whole subtree, so a child that differed only
    /// by the parent's casing must not be renamed again at the old path. On a
    /// case-sensitive host that path no longer exists; the step is a no-op that
    /// still lets the map and cache row follow.
    #[test]
    fn a_child_covered_by_its_parents_rename_is_a_no_op() {
        let mut dm = HashMap::new();
        dm.insert("dir".to_string(), dir_rec());
        dm.insert("dir/file.txt".to_string(), file_rec());
        let renames = vec![
            ("dir".to_string(), "Dir".to_string()),
            ("dir/file.txt".to_string(), "Dir/file.txt".to_string()),
        ];

        let steps = plan_steps(&renames, &dm);
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].from, "dir");
        assert!(steps[0].perform, "the directory itself moves");
        assert_eq!(steps[1].from, "Dir/file.txt");
        assert!(
            !steps[1].perform,
            "the parent move already put the child on src's casing"
        );
    }

    /// A nested directory whose own leaf casing differs is renamed at the path
    /// its parent's move left it on, and the longest prefix wins for its
    /// children. A textual prefix is not a component boundary.
    #[test]
    fn nested_directory_renames_rewrite_through_the_longest_prefix() {
        let mut dm = HashMap::new();
        dm.insert("dir".to_string(), dir_rec());
        dm.insert("dir/sub".to_string(), dir_rec());
        dm.insert("dir/sub/f.txt".to_string(), file_rec());
        let renames = vec![
            ("dir".to_string(), "Dir".to_string()),
            ("dir/sub".to_string(), "Dir/Sub".to_string()),
            ("dir/sub/f.txt".to_string(), "Dir/Sub/f.txt".to_string()),
        ];

        let steps = plan_steps(&renames, &dm);
        assert_eq!(steps[1].from, "Dir/sub");
        assert!(steps[1].perform, "the child directory still moves");
        assert_eq!(steps[2].from, "Dir/Sub/f.txt");
        assert!(
            !steps[2].perform,
            "the parent directory's move covered the file"
        );

        let moves = vec![
            ("dir".to_string(), "Dir".to_string()),
            ("dir/sub".to_string(), "Dir/Sub".to_string()),
        ];
        assert_eq!(rewritten_from("dir/sub/f.txt", &moves), "Dir/Sub/f.txt");
        assert_eq!(
            rewritten_from("dir2/f.txt", &moves),
            "dir2/f.txt",
            "a prefix match must end at a component boundary"
        );
    }

    /// A dst-only path under a renamed directory follows the directory in the
    /// map and the cache, even though it has no rename pair of its own.
    #[test]
    fn a_dst_only_descendant_follows_its_renamed_parent() {
        let renames = vec![("dir".to_string(), "Dir".to_string())];
        let dir_moves = vec![("dir".to_string(), "Dir".to_string())];

        assert_eq!(final_key("dir", &renames, &dir_moves), "Dir");
        assert_eq!(
            final_key("dir/orphan.txt", &renames, &dir_moves),
            "Dir/orphan.txt"
        );
        assert_eq!(
            final_key("other/x.txt", &renames, &dir_moves),
            "other/x.txt"
        );
    }
}
