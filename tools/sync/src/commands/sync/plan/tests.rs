use super::super::rename::DiskSpelling;
#[cfg(windows)]
use super::link::without_verbatim;
use super::link::{
    LinkStorage, alias_target, folded_keys, keep_ancestors, plan_rel, spare_alias_chain,
    spare_link_components, target_rel,
};
use super::safety::ensure_shared_storage_safe;
use super::*;
use crate::diff::Diff;
use crate::effective::EffRec;
use crate::hash::DigestSet;
use girpr_tempdir::TempDir;
use std::collections::HashMap;
use std::path::Path;

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
    let t = TempDir::new("girsync_plan", "filtered_dir");
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
    let t = TempDir::new("girsync_plan", "case_pair_fix");
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
    let t = TempDir::new("girsync_plan", "prefix_blocker");
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
    let t = TempDir::new("girsync_plan", "prefix_fix");
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

    ensure_hard_linked_targets_safe(&p, &dm, &dst, None)
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
    let t = TempDir::new("girsync_plan", "hardlink_deleted");
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

    ensure_hard_linked_targets_safe(&p, &dm, &dst, None)
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

    ensure_hard_linked_targets_safe(&p, &dm, &dst, None)
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

    ensure_hard_linked_targets_safe(&p, &dm, &dst, None)
        .expect("both other names are removed before the copy");

    p.delete_files.pop();
    ensure_hard_linked_targets_safe(&p, &dm, &dst, None)
        .expect_err("one surviving name still shares the inode");
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
        alias_target("a.bin", Path::new("../../x.bin")).as_deref(),
        None,
        "a target that escapes the root has no alias under it"
    );
}

/// `canonicalize` returns a verbatim path while a junction's `read_link` keeps
/// the spelling the junction was created with, so the two are compared without
/// the prefix. Dropping it must still leave a path the other spelling matches:
/// a UNC path needs **two** leading backslashes, not one.
#[cfg(windows)]
#[test]
fn without_verbatim_rebuilds_the_prefix_the_path_needs() {
    assert_eq!(
        without_verbatim(Path::new(r"\\?\UNC\server\share\dir")).as_ref(),
        Path::new(r"\\server\share\dir"),
        "the UNC prefix is two backslashes, so the share stays a share"
    );
    assert_eq!(
        without_verbatim(Path::new(r"\\?\C:\dir")).as_ref(),
        Path::new(r"C:\dir"),
        "a local drive path only drops the prefix"
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
    let err = spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None)
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
    let err = spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None)
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
    let root = std::fs::canonicalize(&dst).unwrap();
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
    let err = spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None)
        .expect_err("a mirrored target cannot also be given a new content");
    let msg = err.to_string();
    assert!(msg.contains("`real`"), "names the target's walk key: {msg}");
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

    spare_link_storage(&sm, &dm, &mut p, &mut diff, &dst, None).unwrap();

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
