use super::super::rename::DiskSpelling;
#[cfg(windows)]
use super::link::on_disk_name;
use super::link::{
    LinkStorage, absolute_alias, alias_target, folded_keys, keep_ancestors, plan_rel,
    spare_alias_chain, spare_link_components, target_rel,
};
use super::safety::{PrefixStats, Probes, SystemProbes, ensure_shared_storage_safe};
use super::*;
use crate::diff::Diff;
use crate::effective::EffRec;
use crate::hardlink::{LinkInfo, info};
use crate::hash::DigestSet;
use girpr_tempdir::TempDir;
use std::cell::RefCell;
use std::collections::HashMap;
#[cfg(windows)]
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};

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

/// The disk the guards ask, under the test's control.
///
/// It answers like the real disk, records the paths it was asked about, and
/// can be told to fail one of them. A probe that fails and a probe that is
/// asked twice are the two things a real filesystem cannot be asked for on
/// demand, and a plan-time refusal turns on both.
struct TestProbes {
    asked: RefCell<Vec<PathBuf>>,
    fail: Option<PathBuf>,
}

impl TestProbes {
    /// A probe that answers like the real disk.
    fn new() -> Self {
        Self {
            asked: RefCell::new(Vec::new()),
            fail: None,
        }
    }

    /// A probe that fails for `path`, and answers like the real disk otherwise.
    fn failing(path: &Path) -> Self {
        Self {
            asked: RefCell::new(Vec::new()),
            fail: Some(path.to_path_buf()),
        }
    }

    /// The paths the guard asked about, in the order it asked.
    fn asked(&self) -> Vec<PathBuf> {
        self.asked.borrow().clone()
    }

    fn record(&self, path: &Path) -> io::Result<()> {
        self.asked.borrow_mut().push(path.to_path_buf());
        match &self.fail {
            Some(fail) if fail == path => Err(io::Error::other("the disk refused the probe")),
            _ => Ok(()),
        }
    }
}

impl Probes for TestProbes {
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.record(path)?;
        dunce::canonicalize(path)
    }

    fn link_info(&self, path: &Path) -> io::Result<LinkInfo> {
        self.record(path)?;
        info(path)
    }

    fn is_dir(&self, path: &Path) -> io::Result<Option<bool>> {
        self.record(path)?;
        SystemProbes.is_dir(path)
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

/// `foo` is absent from `dm` because `--include` hid the dst file, so the
/// map-based guard cannot see the blocker. The disk can.
#[test]
fn a_filtered_file_at_a_mkdir_path_is_refused_from_disk() {
    let t = TempDir::new("girsync_plan", "filtered_blocker");
    let dst = t.subdir("dst");
    std::fs::write(dst.join("foo"), b"blocker").unwrap();
    let mut sm = HashMap::new();
    sm.insert("foo".to_string(), dir());
    sm.insert("foo/bar.txt".to_string(), file());
    let dm = HashMap::new();

    let err = ensure_mkdir_targets_free(&sm, &dm, &dst, false, &SystemProbes)
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
    let t = TempDir::new("girsync_plan", "filtered_dir");
    let dst = t.subdir("dst");
    std::fs::create_dir_all(dst.join("foo")).unwrap();
    let mut sm = HashMap::new();
    sm.insert("foo".to_string(), dir());
    sm.insert("foo/bar.txt".to_string(), file());

    ensure_mkdir_targets_free(&sm, &HashMap::new(), &dst, false, &SystemProbes)
        .expect("create_dir_all is a no-op on an existing directory");
}

/// In insensitive mode the rename pass pairs dst's `foo` with src's `Foo`,
/// and the plan turns the same-path kind difference into a `fix_dirs` entry.
/// The preflight must use the same pairing, not the exact key.
#[test]
fn a_case_paired_file_is_a_planned_fix_not_a_mkdir() {
    let t = TempDir::new("girsync_plan", "case_pair_fix");
    let dst = t.subdir("dst");
    std::fs::write(dst.join("foo"), b"blocker").unwrap();
    let mut sm = HashMap::new();
    sm.insert("Foo".to_string(), dir());
    let mut dm = HashMap::new();
    dm.insert("foo".to_string(), file());

    ensure_mkdir_targets_free(&sm, &dm, &dst, false, &SystemProbes)
        .expect("rename then fix-dirs resolves the pair");
}

/// The strict-prefix half: `a/b` is the candidate, and `a` is a file on disk
/// that `dm` misreports as a directory. The plan would `mkdir a/b` and fail,
/// because `a` is not a `fix_dirs` entry.
#[test]
fn a_file_at_a_strict_prefix_is_refused_from_disk() {
    let t = TempDir::new("girsync_plan", "prefix_blocker");
    let dst = t.subdir("dst");
    std::fs::write(dst.join("a"), b"blocker").unwrap();
    let mut sm = HashMap::new();
    sm.insert("a".to_string(), dir());
    sm.insert("a/b".to_string(), dir());
    let mut dm = HashMap::new();
    dm.insert("a".to_string(), dir());

    let err = ensure_mkdir_targets_free(&sm, &dm, &dst, false, &SystemProbes)
        .expect_err("the prefix is a file on disk and is not a planned fix");
    let msg = err.to_string();
    assert!(msg.contains("`a`"), "names the blocking prefix: {msg}");
    assert!(msg.contains("a/b"), "names the action: {msg}");
}

/// A planned fix is exactly the licence a strict prefix gets: `fix_dirs`
/// removes the file and creates the directory before `mkdir a/b` runs.
#[test]
fn a_planned_fix_at_a_strict_prefix_is_applicable() {
    let t = TempDir::new("girsync_plan", "prefix_fix");
    let dst = t.subdir("dst");
    std::fs::write(dst.join("a"), b"blocker").unwrap();
    let mut sm = HashMap::new();
    sm.insert("a".to_string(), dir());
    sm.insert("a/b".to_string(), dir());
    let mut dm = HashMap::new();
    dm.insert("a".to_string(), file());

    ensure_mkdir_targets_free(&sm, &dm, &dst, false, &SystemProbes)
        .expect("fix-dirs replaces `a` before the nested mkdir");
}

/// A deep candidate asks the disk about every strict prefix, and the prefixes
/// are shared between the candidates below them. The refusal must still name
/// the entry that blocks the path — the shortest prefix — and not a deeper
/// action.
#[test]
fn a_deep_mkdir_path_names_the_blocking_prefix() {
    let t = TempDir::new("girsync_plan", "deep_blocker");
    let dst = t.subdir("dst");
    std::fs::write(dst.join("a"), b"blocker").unwrap();
    let mut sm = HashMap::new();
    for rel in ["a", "a/b", "a/b/c", "a/b/c/d"] {
        sm.insert(rel.to_string(), dir());
    }
    let mut dm = HashMap::new();
    for rel in ["a", "a/b", "a/b/c"] {
        dm.insert(rel.to_string(), dir());
    }

    let err = ensure_mkdir_targets_free(&sm, &dm, &dst, false, &SystemProbes)
        .expect_err("the blocked prefix refuses the nested mkdir");
    let msg = err.to_string();
    assert!(
        msg.contains("dst holds `a` as a file"),
        "names the blocker: {msg}"
    );
    assert!(
        !msg.contains("dst holds `a/b"),
        "the blocker is the shortest prefix, not a deeper one: {msg}"
    );
    assert!(msg.contains("`a/b/c/d`"), "names the mkdir: {msg}");
}

/// The memo itself: one answer per prefix, so the disk is asked once.
#[test]
fn the_prefix_memo_answers_a_prefix_once() {
    let mut stats = PrefixStats::default();
    let mut probes = 0;
    for _ in 0..3 {
        let answer = stats
            .get("a/b", || {
                probes += 1;
                Ok(Some(true))
            })
            .expect("the probe answers");
        assert_eq!(answer, Some(true));
    }
    assert_eq!(probes, 1, "the disk is asked once per prefix");
}

/// The preflight asks the disk about every strict prefix of every candidate,
/// and the prefixes are shared between the candidates below them: without the
/// memo the same ancestor is stat'd once per descendant. The guard asks
/// through a probe, so this counts the questions the **guard** made rather
/// than the memo's own bookkeeping.
#[test]
fn a_prefix_is_asked_about_once_per_call() {
    let t = TempDir::new("girsync_plan", "prefix_probe");
    let dst = t.subdir("dst");
    let mut sm = HashMap::new();
    for rel in ["p", "p/q", "p/q/r", "p/q/s"] {
        sm.insert(rel.to_string(), dir());
    }
    let probes = TestProbes::new();

    ensure_mkdir_targets_free(&sm, &HashMap::new(), &dst, false, &probes)
        .expect("every prefix of every candidate is free");

    let asked = probes.asked();
    assert_eq!(
        asked.len(),
        4,
        "one question per distinct prefix, not one per descendant: {asked:?}"
    );
    for rel in ["p", "p/q", "p/q/r", "p/q/s"] {
        let path = dst.join(rel);
        assert_eq!(
            asked.iter().filter(|asked| *asked == &path).count(),
            1,
            "`{rel}` is asked about once: {asked:?}"
        );
    }
}

/// A directory at a copy destination is a planned type conflict: the copy
/// phase removes it, so its inode is never truncated. On Unix its `nlink` is
/// `2 + subdirectories` (the fixture here makes it 2), which the old guard
/// read as a hard link and refused with a bogus message.
#[test]
fn a_directory_copy_destination_is_not_a_hard_link_hazard() {
    let t = TempDir::new("girsync_plan", "hardlink_dir");
    let dst = t.subdir("dst");
    std::fs::create_dir_all(dst.join("node/inner")).unwrap();
    let mut dm = HashMap::new();
    dm.insert("node".to_string(), dir());
    let p = plan(&[], &["node"], &[]);

    ensure_hard_linked_targets_safe(&p, &dm, &dst, None, &SystemProbes)
        .expect("a directory at the copy destination is removed, not truncated");
}

/// Two dst names for one file, and neither is planned for removal: the copy
/// would truncate both. `copy_one` refuses this in a real run; the preflight
/// must refuse it in a dry run too, or the two modes answer differently.
#[test]
fn a_hard_linked_copy_target_with_no_planned_removal_is_refused() {
    let t = TempDir::new("girsync_plan", "hardlink_refused");
    let dst = t.subdir("dst");
    std::fs::write(dst.join("a.txt"), b"shared").unwrap();
    std::fs::hard_link(dst.join("a.txt"), dst.join("shared.txt")).unwrap();
    let mut dm = HashMap::new();
    dm.insert("a.txt".to_string(), file());
    dm.insert("shared.txt".to_string(), file());
    let p = plan(&[], &["a.txt"], &[]);

    let err = ensure_hard_linked_targets_safe(&p, &dm, &dst, None, &SystemProbes)
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
    let t = TempDir::new("girsync_plan", "hardlink_deleted");
    let dst = t.subdir("dst");
    std::fs::write(dst.join("a.txt"), b"shared").unwrap();
    std::fs::hard_link(dst.join("a.txt"), dst.join("shared.txt")).unwrap();
    let mut dm = HashMap::new();
    dm.insert("a.txt".to_string(), file());
    dm.insert("shared.txt".to_string(), file());
    let mut p = plan(&[], &["a.txt"], &[]);
    p.delete_files.push("shared.txt".to_string());

    ensure_hard_linked_targets_safe(&p, &dm, &dst, None, &SystemProbes)
        .expect("the delete phase runs before the copy and removes the other name");
}

/// A delete of an unrelated inode is no credit: identity, not the number of
/// planned removals, is what makes the other name go away.
#[test]
fn deleting_an_unrelated_file_does_not_make_the_copy_safe() {
    let t = TempDir::new("girsync_plan", "hardlink_unrelated");
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

    ensure_hard_linked_targets_safe(&p, &dm, &dst, None, &SystemProbes)
        .expect_err("deleting a different file leaves both names of this inode");
}

/// `fix_dirs` runs before the copy phase too, so a blocking file it replaces
/// reduces the shared count just as a delete does.
#[test]
fn a_file_replaced_by_fix_dirs_makes_the_hard_linked_copy_safe() {
    let t = TempDir::new("girsync_plan", "hardlink_fixdir");
    let dst = t.subdir("dst");
    std::fs::write(dst.join("a.txt"), b"shared").unwrap();
    std::fs::hard_link(dst.join("a.txt"), dst.join("repl")).unwrap();
    let mut dm = HashMap::new();
    dm.insert("a.txt".to_string(), file());
    dm.insert("repl".to_string(), file());
    let p = plan(&[], &["a.txt"], &["repl"]);

    ensure_hard_linked_targets_safe(&p, &dm, &dst, None, &SystemProbes)
        .expect("fix-dirs removes the second name before the copy");
}

/// Removing both other names leaves one: the arithmetic that keeps the
/// preflight from refusing a default run that deletes two extras first.
#[test]
fn removing_both_other_names_makes_a_three_way_share_safe() {
    let t = TempDir::new("girsync_plan", "hardlink_three");
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

    ensure_hard_linked_targets_safe(&p, &dm, &dst, None, &SystemProbes)
        .expect("both other names are removed before the copy");

    p.delete_files.pop();
    ensure_hard_linked_targets_safe(&p, &dm, &dst, None, &SystemProbes)
        .expect_err("one surviving name still shares the inode");
}

/// A run with no copy cannot truncate an inode, so the preflight has nothing
/// to protect and answers without reading a link count. The same tree with a
/// copy planned is the case the guard exists for, so the answer must still
/// depend on the copy and not on the tree.
#[test]
fn a_copy_free_plan_with_hard_linked_extras_is_accepted() {
    let t = TempDir::new("girsync_plan", "hardlink_copy_free");
    let dst = t.subdir("dst");
    std::fs::write(dst.join("a.txt"), b"shared").unwrap();
    std::fs::hard_link(dst.join("a.txt"), dst.join("shared.txt")).unwrap();
    let mut dm = HashMap::new();
    dm.insert("a.txt".to_string(), file());
    dm.insert("shared.txt".to_string(), file());

    let copying = plan(&[], &["a.txt"], &[]);
    ensure_hard_linked_targets_safe(&copying, &dm, &dst, None, &SystemProbes)
        .expect_err("a copy would truncate the inode the second name shares");

    let mut copy_free = plan(&[], &[], &[]);
    copy_free.delete_files.push("shared.txt".to_string());
    ensure_hard_linked_targets_safe(&copy_free, &dm, &dst, None, &SystemProbes)
        .expect("a copy-free run cannot truncate an inode");
}

/// A copy-free run cannot truncate an inode, but it still removes names, and
/// a removal the disk cannot answer is a failure the run used to surface at
/// plan time. The delete phase would only find it later — after the rename
/// had already changed dst, and after other removals had already landed.
#[test]
fn an_unreadable_planned_removal_is_refused_at_plan_time() {
    let t = TempDir::new("girsync_plan", "unreadable_removal");
    let dst = t.subdir("dst");
    std::fs::write(dst.join("gone.txt"), b"x").unwrap();
    let mut dm = HashMap::new();
    dm.insert("gone.txt".to_string(), file());
    let mut p = plan(&[], &[], &[]);
    p.delete_files.push("gone.txt".to_string());
    let probes = TestProbes::failing(&dst.join("gone.txt"));

    let err = ensure_hard_linked_targets_safe(&p, &dm, &dst, None, &probes)
        .expect_err("a removal the disk cannot answer is refused at plan time");
    let msg = err.to_string();
    assert!(msg.contains("hard-link count"), "names the probe: {msg}");
    assert!(msg.contains("gone.txt"), "names the removal: {msg}");
}

/// A strict link ancestor that is itself a planned `fix_dirs`
/// entry is removed before the nested action, so the link guard must skip it
/// exactly as the two mkdir guards skip a blocking file. The same link
/// without the fix is still a write-through and is refused.
#[cfg(any(unix, windows))]
#[test]
fn a_planned_fix_link_ancestor_is_not_a_write_through() {
    let t = TempDir::new("girsync_plan", "fixed_link");
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

/// A kept link's content is not an extra path. Sparing it has to drop the
/// `EXTRA` record, the planned delete, and the `rmdir` of every directory
/// above it — otherwise the delete phase removes the file the write-through
/// is about to need.
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
    let t = TempDir::new("girsync_plan", "target_rel");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    std::fs::create_dir_all(dst.join("d")).unwrap();
    std::fs::write(dst.join("d/t.bin"), b"x").unwrap();
    std::fs::write(outside.join("x.bin"), b"x").unwrap();
    let root = dunce::canonicalize(&dst).unwrap();

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
        alias_target("a.bin", Path::new("../../x.bin")).as_deref(),
        None,
        "a target that escapes the root has no alias under it"
    );
}

/// Direct `..`-escape cases for the relative-target walk. `Path::clean()` keeps
/// a leading `..` on a relative path, so the refusal is checked on the cleaned
/// result rather than by popping while walking.
#[test]
fn a_relative_target_that_escapes_is_refused() {
    // Exactly enough `..` to climb back to the root, then a name: stays in.
    assert_eq!(
        alias_target("a/b/c", Path::new("../../d")).as_deref(),
        Some("d"),
        "two `..` cancel the two parent components and stay under the root"
    );
    // One more `..` than there are parents: escapes, even though a later name
    // would otherwise land back under the root. `clean()` keeps the leading
    // `..`, so it is refused rather than silently recovered.
    assert_eq!(
        alias_target("a/b/c", Path::new("../../../d")).as_deref(),
        None,
        "the third `..` escapes; a following name does not bring it back"
    );
    // A target that collapses to the link's own directory is the root itself.
    assert_eq!(
        alias_target("d/a.bin", Path::new("..")).as_deref(),
        None,
        "a target that cleans to `.` names no entry under the root"
    );
    assert_eq!(
        alias_target("d/a.bin", Path::new(".")).as_deref(),
        Some("d"),
        "an explicit `.` target names the link's own directory"
    );
}

/// Absolute targets are not aliases under the root: the old walk refused a
/// `Prefix`/`RootDir` component, and `PathBuf::push` would otherwise replace the
/// base rather than append to it.
#[cfg(unix)]
#[test]
fn an_absolute_relative_target_is_refused() {
    assert_eq!(alias_target("a", Path::new("/abs")).as_deref(), None);
}

/// Windows counterpart of the absolute-target refusal, including a drive-relative
/// prefix (`C:foo`) that has no root but is still not an in-root alias.
#[cfg(windows)]
#[test]
fn an_absolute_relative_target_is_refused() {
    assert_eq!(alias_target("a", Path::new(r"C:\abs")).as_deref(), None);
    assert_eq!(alias_target("a", Path::new(r"\abs")).as_deref(), None);
    assert_eq!(alias_target("a", Path::new("C:foo")).as_deref(), None);
}

/// Direct `..`-escape cases for an absolute junction target stripped against
/// its canonical root. Uses relative synthetic paths so it runs on both platforms.
#[test]
fn an_absolute_alias_that_escapes_is_refused() {
    // Inside: `clean()` cancels the `..`, giving the same spelling as before.
    assert_eq!(
        absolute_alias(Path::new("dst/a/../b"), Path::new("dst"), None).as_deref(),
        Some("b"),
        "a `..` that stays inside the root is resolved lexically"
    );
    // Escape: more `..` than names, so a leading `..` survives `clean()`.
    assert_eq!(
        absolute_alias(Path::new("dst/a/../../b"), Path::new("dst"), None).as_deref(),
        None,
        "the second `..` escapes the root and is refused"
    );
    assert_eq!(
        absolute_alias(Path::new("dst/../../b"), Path::new("dst"), None).as_deref(),
        None,
        "a leading escape stays leading after cleaning"
    );
    // A target that cleans to the root itself is not an alias under it.
    assert_eq!(
        absolute_alias(Path::new("dst/a/.."), Path::new("dst"), None).as_deref(),
        None,
        "a path that cleans to `.` names no entry under the root"
    );
}

/// Both canonicalization (`dunce`) and a junction's `read_link` spell a UNC
/// root with its **two** leading backslashes — `\\server\share\dir` — and no
/// verbatim prefix, so the alias comparison strips the root directly. The
/// hand-rolled prefix rebuild is gone; this pins the UNC behaviour it existed
/// to protect at the call site that relies on it.
#[cfg(windows)]
#[test]
fn without_verbatim_rebuilds_the_prefix_the_path_needs() {
    let root = Path::new(r"\\server\share\dir");
    let target = Path::new(r"\\server\share\dir\sub\x.bin");
    assert_eq!(
        absolute_alias(target, root, None).as_deref(),
        Some("sub/x.bin"),
        "the share keeps its two backslashes and the target strips under the root"
    );
}

/// `dst/a -> link/x.bin` with `link` a directory link. The parent
/// canonicalization names only `real/x.bin`, so the walk's alias `link` is
/// spared with the target: both spellings and every link prefix are spared.
#[cfg(any(unix, windows))]
#[test]
fn a_kept_link_through_a_directory_link_spares_the_alias_and_the_target() {
    let t = TempDir::new("girsync_plan", "link_component");
    let dst = t.subdir("dst");
    std::fs::create_dir_all(dst.join("real")).unwrap();
    std::fs::write(dst.join("real/x.bin"), b"x").unwrap();
    if let Err(e) = make_dir_link(&dst.join("real"), &dst.join("link")) {
        eprintln!("skip: this platform refused a directory link: {e}");
        return;
    }
    let root = dunce::canonicalize(&dst).unwrap();
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
    spare_alias_chain("link/x.bin", &dm, &folded_keys(&dm), &mut storage);
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

/// A spared **intermediate** link keeps its ancestors through `keep_path`,
/// the single insertion point, so `remove_dir_all` cannot take the link
/// with a dst-only directory.
#[test]
fn a_spared_intermediate_link_keeps_its_ancestor_directories() {
    let mut dm = HashMap::new();
    dm.insert("extra/b".to_string(), link_file());
    let mut storage = LinkStorage::default();
    spare_alias_chain("extra/b", &dm, &folded_keys(&dm), &mut storage);
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
    let t = TempDir::new("girsync_plan", "spare_component");
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

    spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None, &SystemProbes).unwrap();

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
    let t = TempDir::new("girsync_plan", "spare_intermediate");
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

    spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None, &SystemProbes).unwrap();

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
    let t = TempDir::new("girsync_plan", "spare_shared");
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
    let err = spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None, &SystemProbes)
        .expect_err("two kept links on one inode cannot both be copied");
    let msg = err.to_string();
    assert!(
        msg.contains("`a`") && msg.contains("`b`"),
        "names both links: {msg}"
    );

    // Chained: `a -> b -> c`; the same terminal groups the two kept links.
    let t = TempDir::new("girsync_plan", "spare_shared_chain");
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
    let err = spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None, &SystemProbes)
        .expect_err("the chained pair shares the same terminal storage");
    let msg = err.to_string();
    assert!(
        msg.contains("`a`") && msg.contains("`b`"),
        "names both links: {msg}"
    );
}

/// The final component comes back in the link text's spelling, so on a
/// case-insensitive filesystem `REAL/X.BIN` must name the walk's `real/x.bin`
/// key in `sm`/`dm`, the spare's `delete_files` match and the shared-storage
/// grouping.
/// The resolver names the entry the walk reported.
#[test]
fn a_case_differing_final_component_names_the_walk_entry() {
    let t = TempDir::new("girsync_plan", "target_case");
    let dst = t.subdir("dst");
    std::fs::create_dir_all(dst.join("real")).unwrap();
    std::fs::write(dst.join("real/x.bin"), b"x").unwrap();
    let root = dunce::canonicalize(&dst).unwrap();
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

/// **The identity rule and the on-disk lookup agree on a non-ASCII
/// spelling.**
///
/// The two sites ask different questions — one asks whether two spellings name
/// the same place on disk, the other asks which directory entry the operating
/// system would open — but they compare with one Unicode lowercase mapping, so
/// they must not answer differently about the same pair of spellings.
///
/// The pair is the one the crate's other rule keeps apart: the ASCII-only path
/// key folds `Ärger` and `ärger` into two keys, while these two sites say the
/// two spellings are one place. That is where a second copy of the mapping
/// would show up as a disagreement, so it is where the two sites are compared
/// with each other.
#[cfg(windows)]
#[test]
fn the_identity_rule_and_the_on_disk_lookup_agree_on_a_non_ascii_spelling() {
    let t = TempDir::new("girsync_plan", "non_ascii_case");
    let dir = t.subdir("dst");
    std::fs::write(dir.join("Ärger.txt"), b"x").unwrap();

    // The identity rule: two spellings, one place.
    assert!(
        crate::util::same_path(&dir.join("Ärger.txt"), &dir.join("ärger.txt")),
        "Windows opens either spelling as the same file"
    );

    // The on-disk lookup: the entry the OS would open, asked for under the
    // other spelling.
    assert_eq!(
        on_disk_name(&dir, OsStr::new("ärger.txt")).unwrap(),
        Some(OsString::from("Ärger.txt")),
        "the lookup names the entry the identity rule calls the same place"
    );
    assert_eq!(
        on_disk_name(&dir, OsStr::new("Ärger.txt")).unwrap(),
        Some(OsString::from("Ärger.txt")),
        "and the walk's own spelling is unchanged"
    );
    assert_eq!(
        on_disk_name(&dir, OsStr::new("other.txt")).unwrap(),
        None,
        "and a name the directory does not hold is still not found"
    );

    // The key keeps the two spellings apart: the disagreement these two
    // sites are not part of.
    assert_ne!(
        crate::util::path_key("Ärger.txt"),
        crate::util::path_key("ärger.txt"),
        "the key folds ASCII case only, so a non-ASCII variant is a distinct key"
    );
}

/// Dry-run parity: a case-only rename of the target is routine for a mirror. A real run moves the entry to src's casing before the plan,
/// while a dry run leaves it on disk and maps the spelling forward. Both
/// must hand `sm`/`dm`, the spare and the shared-storage grouping the same
/// key.
#[test]
fn a_case_only_rename_of_the_target_is_one_spare_in_both_modes() {
    let t = TempDir::new("girsync_plan", "target_case_rename");
    let dst = t.subdir("dst");
    std::fs::create_dir_all(dst.join("real")).unwrap();
    std::fs::write(dst.join("real/x.bin"), b"x").unwrap();
    let root = dunce::canonicalize(&dst).unwrap();
    if std::fs::metadata(dst.join("real/X.BIN")).is_err() {
        eprintln!("skip: case-sensitive filesystem; the rename is not a pair");
        return;
    }
    let spelling = DiskSpelling::new(&[("real/x.bin".to_string(), "real/X.BIN".to_string())], &[]);

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

/// A target whose intermediate link is spelled with another case (`LINK`
/// on disk is `link`) must spare the walk's spelling — sparing the link
/// text's spelling would leave `link` in `delete_files` for the run to
/// unlink.
#[cfg(any(unix, windows))]
#[test]
fn a_case_differing_link_component_is_spared_under_the_walk_spelling() {
    let t = TempDir::new("girsync_plan", "link_component_case");
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
    let root = dunce::canonicalize(&dst).unwrap();
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

/// The same spelling feeds the `sm.get` key, so a case-differing target
/// names the walk's record and the mirrored-target guard refuses it. The
/// terminal's map record is the guard's input; the junction stands in for
/// the file link.
#[cfg(any(unix, windows))]
#[test]
fn a_case_differing_mirrored_target_is_refused() {
    let t = TempDir::new("girsync_plan", "mirrored_case");
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
    let err = spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None, &SystemProbes)
        .expect_err("a mirrored target cannot also be given a new content");
    let msg = err.to_string();
    assert!(msg.contains("`real`"), "names the target's walk key: {msg}");
}

/// The same rule without the case difference: a kept link whose target src
/// also has is one inode carrying two mirrored paths, so the pass must refuse
/// it. A pass that skipped a tree it took to be link-free would let the copy
/// through.
#[cfg(any(unix, windows))]
#[test]
fn a_kept_link_with_a_mirrored_target_is_still_refused() {
    let t = TempDir::new("girsync_plan", "kept_mirrored");
    let dst = t.subdir("dst");
    std::fs::create_dir_all(dst.join("real")).unwrap();
    if let Err(e) = make_dir_link(&dst.join("real"), &dst.join("a")) {
        eprintln!("skip: this platform refused a directory link: {e}");
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
    let err = spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None, &SystemProbes)
        .expect_err("one file cannot hold two contents");
    let msg = err.to_string();
    assert!(msg.contains("`a`"), "names the kept link: {msg}");
    assert!(msg.contains("`real`"), "names the mirrored target: {msg}");
}

/// A tree without a kept link has nothing to spare, so the pass walks no
/// chain. The plan and the record it would have adjusted must come out the
/// same.
#[test]
fn a_link_free_tree_leaves_the_plan_unchanged() {
    let t = TempDir::new("girsync_plan", "link_free");
    let dst = t.subdir("dst");
    std::fs::write(dst.join("extra.bin"), b"x").unwrap();
    let mut sm = HashMap::new();
    sm.insert("a.txt".to_string(), file());
    let mut dm = HashMap::new();
    dm.insert("extra.bin".to_string(), file());
    let mut diff = Diff {
        extra: vec!["extra.bin".to_string()],
        ..Diff::default()
    };
    let mut p = plan(&[], &[], &[]);
    p.delete_files = vec!["extra.bin".to_string()];

    spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None, &SystemProbes)
        .expect("a tree without a kept link has nothing to spare");

    assert_eq!(p.delete_files, ["extra.bin"], "the plan is unchanged");
    assert_eq!(diff.extra, ["extra.bin"], "and so is the record");
}

/// The dst root has to resolve before a plan exists: every path under it is
/// compared with the canonical root, and a root that cannot be resolved leaves
/// the whole plan unanswerable. The failure belongs at plan time — a run that
/// found it later would act on a plan it could not check, and the rename has
/// already changed dst by then.
#[test]
fn an_unresolvable_dst_root_is_refused_at_plan_time() {
    let t = TempDir::new("girsync_plan", "unresolvable_root");
    let dst = t.subdir("dst");
    let sm = HashMap::new();
    let dm = HashMap::new();
    let mut diff = Diff::default();
    let mut p = plan(&[], &[], &[]);
    let probes = TestProbes::failing(&dst);

    let err = spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None, &probes)
        .expect_err("the dst root must resolve before the plan exists");
    let msg = err.to_string();
    assert!(msg.contains("resolve dst root"), "names the root: {msg}");

    // The same refusal without the probe: a root that is not there cannot be
    // resolved either.
    let missing = t.join("not-there");
    let err = spare_link_storage(&sm, &dm, &mut p, &mut diff, &missing, None, &SystemProbes)
        .expect_err("a root that is not there cannot be resolved");
    assert!(
        err.to_string().contains("resolve dst root"),
        "names the root: {err}"
    );
}

/// The alias half: a link text can spell a chain component with
/// another case. The alias spare must then name the walk's key, not the link
/// text's spelling, or `diff.extra` keeps a path the plan deliberately keeps.
#[cfg(windows)]
#[test]
fn a_case_differing_alias_is_spared_under_the_walk_key() {
    let mut dm = HashMap::new();
    dm.insert("link".to_string(), link_dir());
    dm.insert("link/x.bin".to_string(), file());
    let folded = folded_keys(&dm);
    let mut storage = LinkStorage::default();

    spare_alias_chain("LINK/x.bin", &dm, &folded, &mut storage);

    assert!(storage.keep.contains("link"), "{:?}", storage.keep);
    assert!(
        storage.keep.contains("link/x.bin"),
        "the walk's key is spared: {:?}",
        storage.keep
    );
    assert!(
        !storage.keep.contains("LINK/x.bin"),
        "the link text's spelling is no record's key: {:?}",
        storage.keep
    );
}

/// A junction's `read_link` target keeps the spelling it was created with,
/// so a case-differing terminal (`REAL` on disk is `real`) must spare the
/// key the removal lists hold: the directory stays out of `rmdir` and
/// `remove_dir_all` cannot take the link's storage. The junction stands in
/// for the file link this host refuses.
#[cfg(any(unix, windows))]
#[test]
fn a_case_differing_terminal_is_spared_by_the_full_pass() {
    let t = TempDir::new("girsync_plan", "spare_terminal_case");
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

    spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None, &SystemProbes).unwrap();

    assert!(
        p.rmdir.is_empty(),
        "the terminal's owning directory is spared: {:?}",
        p.rmdir
    );
    assert!(diff.extra.is_empty(), "{:?}", diff.extra);
}

/// Two kept links share one dst-only file. Neither terminal target is
/// mirrored by src, so the per-link check never fires; copying both wrote
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

/// Only a copy whose dst is a **directory** wipes a subtree: `copy_one` removes
/// a blocking directory recursively, while a file at the same path is merely
/// overwritten and the paths the walk reported under it are still there.
#[test]
fn a_copy_wipes_a_subtree_only_when_its_dst_is_a_directory() {
    let mut dm = HashMap::new();
    dm.insert("blocked".to_string(), file());
    dm.insert("tree".to_string(), dir());
    let p = plan(&[], &["blocked", "tree"], &[]);

    let wiped = DirPrefixes::wiped_by_copy(&p.copy, &dm);
    assert!(
        !wiped.has_ancestor("blocked/x.txt"),
        "a file-kind dst is overwritten, not wiped: {:?}",
        wiped
    );
    assert!(
        wiped.has_ancestor("tree/x.txt"),
        "a directory-kind dst is removed recursively: {:?}",
        wiped
    );
    assert!(
        !wiped.has_ancestor("tree"),
        "the directory at the root of the wipe is not under itself"
    );
}

/// Wiped subtrees are matched strictly and by whole component: the directory at
/// the root of a wipe is not under itself, and a shared name prefix is not an
/// ancestor.
#[test]
fn dir_prefixes_match_strict_descendants_on_a_component_boundary() {
    let mut dirs = DirPrefixes::default();
    assert!(dirs.is_empty());
    dirs.insert("a");
    assert!(!dirs.is_empty());
    assert!(!dirs.has_ancestor("a"), "a directory is not under itself");
    assert!(dirs.has_ancestor("a/f"));
    assert!(dirs.has_ancestor("a/b/f"));
    assert!(
        !dirs.has_ancestor("ab/f"),
        "a name prefix is not an ancestor"
    );
    assert!(!dirs.has_ancestor("x/a"));
}
