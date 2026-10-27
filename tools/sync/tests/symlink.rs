//! Links in the walk, in `copy`, and in the deletion plan.
//!
//! The walk follows links, so a directory link is reported as the directory it
//! points at. That is the documented comparison behaviour and it is what makes
//! the *deletion* side dangerous: a relative path under the link resolves
//! through it, so `DELETE`/`RMDIR` can reach a file that is not in `dst` at all.
//! These cases pin the walk (F-3), the copy-side type conflict (F-2) and the
//! no-reach-through guarantee (F-1).
//!
//! ## Skipping
//!
//! Windows without developer mode refuses `symlink_file` and `symlink_dir`;
//! `mklink /J` creates a directory junction without elevation, so the directory
//! cases still run there. A case that needs a refused primitive prints why and
//! returns - a silent pass would read as coverage it never had. The file-link
//! cases are skipped on such a host, and the note says so.

mod common;

use std::path::Path;

use common::{
    dir_link, entry_names, file_link, is_link, log, parse_ndjson, rfile, sync_mem, temp_root, wfile,
};
use girsync::scan::walk_live;
use girsync::{cmd_compare_self, cmd_sync};
use serde_json::Value;

/// Run the real binary, returning its exit status and records.
///
/// The in-process `cmd_sync` cannot show what was *reported* - stdout belongs to
/// the binary - and the F-1 case has to name the path a `DELETE` removed.
fn sync_bin(src: &Path, dst: &Path, extra: &[&str]) -> (std::process::ExitStatus, Vec<Value>) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .arg("sync")
        .arg("--src")
        .arg(src)
        .arg("--dst")
        .arg(dst)
        .arg("--output")
        .arg("json")
        .args(extra)
        // The case asserts the tree and the records, not the cache file; an
        // in-memory cache keeps the fixture off the disk entirely.
        .env("GIRSYNC_TEST_CACHE", "memory")
        .output()
        .expect("spawn girsync");
    (out.status, parse_ndjson(&out.stdout))
}

/// The `path` of every record with `event` `name`, in order.
fn paths<'a>(recs: &'a [Value], name: &str) -> Vec<&'a str> {
    recs.iter()
        .filter(|r| r["event"] == name)
        .map(|r| r["path"].as_str().expect("a path record has a path"))
        .collect()
}

// ---------------------------------------------------------------------------
// The walk (F-3)
// ---------------------------------------------------------------------------

/// A link to a file is walked as the file it names: its stat is the target's,
/// which is what lets the comparison treat a link and a plain file as the same
/// kind of entry.
#[test]
fn walk_follows_a_file_link() {
    let t = temp_root("walk_file_link");
    let root = t.subdir("root");
    let target = t.subdir("target");
    wfile(&target, "real.bin", b"twelve bytes");
    skip_if_no_link!(
        "file",
        file_link(&target.join("real.bin"), &root.join("link.bin"))
    );

    let live = walk_live(&root, 10).unwrap();
    let ent = live
        .iter()
        .find(|e| e.rel == "link.bin")
        .expect("the link is an entry");
    assert!(!ent.is_dir, "a file link is a file");
    assert_eq!(ent.size, 12, "the size is the target's");
    assert!(ent.is_link, "and it is still marked as a link");
}

/// A link to a directory is walked as a directory, and its target's children are
/// walked under the link's relative path - including a target outside the root.
/// The link itself is marked so the deletion plan can see it.
#[test]
fn walk_follows_a_directory_link_and_its_subtree() {
    let t = temp_root("walk_dir_link");
    let root = t.subdir("root");
    let outside = t.subdir("outside");
    wfile(&outside, "x.txt", b"x");
    wfile(&outside, "sub/y.txt", b"yy");
    skip_if_no_link!("directory", dir_link(&outside, &root.join("link")));

    let mut live = walk_live(&root, 10).unwrap();
    assert!(
        live.iter()
            .any(|e| e.rel == "link" && e.is_dir && e.is_link),
        "the link is a directory entry and is marked as a link: {live:?}"
    );
    let mut rels: Vec<String> = live.drain(..).map(|e| e.rel).collect();
    rels.sort();
    assert_eq!(
        rels,
        ["link", "link/sub", "link/sub/y.txt", "link/x.txt"],
        "the target's subtree appears under the link's relative path"
    );
}

/// A dangling link aborts the walk. The alternative - reporting what can be
/// statted and carrying on - would leave a mirror that claims to match a tree it
/// could not read.
#[test]
fn walk_aborts_on_a_dangling_link() {
    let t = temp_root("walk_dangling");
    let root = t.subdir("root");
    std::fs::create_dir_all(&root).unwrap();
    skip_if_no_link!(
        "file",
        file_link(&root.join("missing.bin"), &root.join("dangling.bin"))
    );

    let err = walk_live(&root, 10).expect_err("a dangling link is not a walkable tree");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("dangling.bin"),
        "the error names the link: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Copying a src link: materialize, do not recreate the link
// ---------------------------------------------------------------------------

/// `src` file link -> a plain file in `dst`, with the target's bytes.
#[test]
fn sync_materializes_a_src_file_link() {
    let t = temp_root("sync_src_file_link");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    wfile(&outside, "real.bin", b"payload");
    skip_if_no_link!(
        "file",
        file_link(&outside.join("real.bin"), &src.join("a.bin"))
    );

    assert_eq!(
        cmd_sync(sync_mem(src.clone(), dst.clone()), &log()).unwrap(),
        0
    );
    assert!(
        !is_link(&dst.join("a.bin")),
        "the mirror materializes the target as a plain file"
    );
    assert_eq!(rfile(&dst, "a.bin"), b"payload");
}

/// `src` directory link -> a plain directory and its files in `dst`.
#[test]
fn sync_materializes_a_src_directory_link() {
    let t = temp_root("sync_src_dir_link");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    wfile(&outside, "x.txt", b"x");
    wfile(&outside, "sub/y.txt", b"yy");
    skip_if_no_link!("directory", dir_link(&outside, &src.join("link")));

    assert_eq!(
        cmd_sync(sync_mem(src.clone(), dst.clone()), &log()).unwrap(),
        0
    );
    assert!(
        !is_link(&dst.join("link")),
        "the mirror materializes a real directory"
    );
    assert!(dst.join("link").is_dir());
    assert_eq!(rfile(&dst, "link/x.txt"), b"x");
    assert_eq!(rfile(&dst, "link/sub/y.txt"), b"yy");
}

/// A `dst` link to a *file* is written through, by design: `File::create`
/// truncates the target. This is the documented write-through, and it must keep
/// working when a `dst` link to a *directory* is unlinked instead (F-2).
#[test]
fn sync_writes_through_a_dst_file_link() {
    let t = temp_root("sync_dst_file_link");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.bin", b"the new, longer content");
    wfile(&dst, "target.bin", b"old");
    skip_if_no_link!(
        "file",
        file_link(&dst.join("target.bin"), &dst.join("a.bin"))
    );

    assert_eq!(
        cmd_sync(sync_mem(src.clone(), dst.clone()), &log()).unwrap(),
        0
    );
    assert!(
        is_link(&dst.join("a.bin")),
        "the link is still a link: write-through does not replace it"
    );
    assert_eq!(
        rfile(&dst, "target.bin"),
        b"the new, longer content",
        "the bytes went to the target"
    );
}

// ---------------------------------------------------------------------------
// F-2: src file, dst link-to-directory
// ---------------------------------------------------------------------------

/// A `dst` directory link blocking a file copy is unlinked, and its target is
/// left intact. Before the fix the guard skipped the link (`is_dir() &&
/// !is_symlink()`), then `File::create` failed and aborted the whole run.
#[test]
fn sync_replaces_a_dst_directory_link_at_a_file_path() {
    let t = temp_root("dst_dir_link_conflict");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    wfile(&src, "a.bin", b"file now");
    wfile(&outside, "keep.txt", b"outside content");
    skip_if_no_link!("directory", dir_link(&outside, &dst.join("a.bin")));

    assert_eq!(
        cmd_sync(sync_mem(src.clone(), dst.clone()), &log()).unwrap(),
        0
    );
    assert!(
        !is_link(&dst.join("a.bin")),
        "the type conflict resolved toward src: a real file, not the link"
    );
    assert_eq!(rfile(&dst, "a.bin"), b"file now");
    assert_eq!(
        rfile(&outside, "keep.txt"),
        b"outside content",
        "the link target was not touched"
    );
}

// ---------------------------------------------------------------------------
// F-1: a dst directory link must never be deleted through
// ---------------------------------------------------------------------------

/// The link points into `dst` itself. `link/x` is extra, and removing it
/// naively would remove `real/x`, a file `src` also has and whose copy plan is
/// empty (the two compare equal). The link must be removed instead.
#[test]
fn sync_removes_a_dst_directory_link_without_deleting_its_inside_target() {
    let t = temp_root("dst_link_inside");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "real/x.txt", b"shared content");
    wfile(&dst, "real/x.txt", b"shared content");
    common::sync_mtime(&src.join("real/x.txt"), &dst.join("real/x.txt"));
    skip_if_no_link!("directory", dir_link(&dst.join("real"), &dst.join("link")));

    let (status, recs) = sync_bin(&src, &dst, &[]);
    assert!(status.success(), "sync failed: {recs:?}");

    assert_eq!(
        rfile(&dst, "real/x.txt"),
        b"shared content",
        "the target's file survives: it was deleted through the link before the fix"
    );
    assert!(!dst.join("link").exists(), "the link itself is gone");
    let deleted = paths(&recs, "delete");
    assert_eq!(
        deleted,
        ["link"],
        "the DELETE names the link, never a path under it: {recs:?}"
    );
    assert!(
        !deleted.iter().any(|p| p.starts_with("link/")),
        "no descendant of the link is named: {deleted:?}"
    );
}

/// The link points outside `dst`. Removing `link/x` would delete a file in a
/// tree the mirror does not own at all.
#[test]
fn sync_removes_a_dst_directory_link_without_deleting_an_external_target() {
    let t = temp_root("dst_link_external");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    wfile(&src, "a.txt", b"a");
    wfile(&outside, "x.txt", b"external content");
    skip_if_no_link!("directory", dir_link(&outside, &dst.join("link")));

    let (status, recs) = sync_bin(&src, &dst, &[]);
    assert!(status.success(), "sync failed: {recs:?}");

    assert_eq!(
        rfile(&outside, "x.txt"),
        b"external content",
        "the external target is untouched"
    );
    assert!(!dst.join("link").exists(), "the link is removed");
    assert_eq!(paths(&recs, "delete"), ["link"]);
}

/// `--keep-extra` is the escape hatch, and it has to spare the link and its
/// target alike. The flag already spares directories; a link is a directory in
/// the dst map, so this pins that the new removal path did not bypass the flag.
#[test]
fn keep_extra_spares_a_dst_directory_link_and_its_target() {
    let t = temp_root("dst_link_keep");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    wfile(&src, "a.txt", b"a");
    wfile(&outside, "x.txt", b"external content");
    skip_if_no_link!("directory", dir_link(&outside, &dst.join("link")));

    let (status, recs) = sync_bin(&src, &dst, &["--keep-extra"]);
    assert!(status.success(), "sync failed: {recs:?}");

    assert_eq!(rfile(&outside, "x.txt"), b"external content");
    assert!(is_link(&dst.join("link")), "the link is kept");
    assert!(
        paths(&recs, "delete").is_empty(),
        "keep-extra deletes nothing: {recs:?}"
    );
}

// ---------------------------------------------------------------------------
// N-1: a shared link is never a write-through path
// ---------------------------------------------------------------------------

/// The `fix_dirs` shape: `src/link/x` is a directory, the shared `dst/link`
/// junction resolves `x` to an external **file**, and the planner asks to
/// replace it with a directory. Before the guard this removed the external file
/// and exited 0; now the run refuses before touching it.
#[test]
fn sync_refuses_to_fix_a_directory_through_a_shared_dst_link() {
    let t = temp_root("shared_link_fix_dir");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    wfile(&src, "link/x/y.txt", b"SRC");
    wfile(&outside, "x", b"EXTERNAL-DATA");
    skip_if_no_link!("directory", dir_link(&outside, &dst.join("link")));

    let err = cmd_sync(sync_mem(src.clone(), dst.clone()), &log())
        .expect_err("writing through a shared dst link must fail");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("link") && msg.contains("refusing"),
        "the error names the link and the refusal: {msg}"
    );
    assert_eq!(
        rfile(&outside, "x"),
        b"EXTERNAL-DATA",
        "the external file is untouched"
    );
    assert!(outside.join("x").is_file(), "and still a file");
    assert!(is_link(&dst.join("link")), "the link is left in place");
}

/// The `copy_one` shape: `src/link/x` is a file, the shared `dst/link` junction
/// resolves `x` to an external **directory**, and a recursive removal would
/// delete its subtree. Before the guard that subtree was gone and the run exited
/// 0; now it refuses before the first filesystem change.
#[test]
fn sync_refuses_to_copy_through_a_shared_dst_link() {
    let t = temp_root("shared_link_copy");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    wfile(&src, "link/x", b"SRC-FILE");
    wfile(&outside, "x/y.txt", b"EXTERNAL");
    skip_if_no_link!("directory", dir_link(&outside, &dst.join("link")));

    let err = cmd_sync(sync_mem(src.clone(), dst.clone()), &log())
        .expect_err("copying through a shared dst link must fail");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("link") && msg.contains("refusing"),
        "the error names the link and the refusal: {msg}"
    );
    assert_eq!(rfile(&outside, "x/y.txt"), b"EXTERNAL");
    assert!(outside.join("x").is_dir(), "the external subtree survives");
    assert!(is_link(&dst.join("link")), "the link is left in place");
}

// ---------------------------------------------------------------------------
// N-4: a shared link with extras is not a converged sync
// ---------------------------------------------------------------------------

/// A default run must not exit 0 with extras it deliberately left under a
/// shared link: `compare` immediately reports them. `--keep-extra` is the
/// explicit opt-in to that non-convergence.
#[test]
fn a_shared_link_with_extras_fails_unless_keep_extra() {
    let t = temp_root("shared_link_extras");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    wfile(&src, "link/x.txt", b"shared");
    wfile(&outside, "x.txt", b"shared");
    wfile(&outside, "y.txt", b"extra");
    common::sync_mtime(&src.join("link/x.txt"), &outside.join("x.txt"));
    skip_if_no_link!("directory", dir_link(&outside, &dst.join("link")));

    let err = cmd_sync(sync_mem(src.clone(), dst.clone()), &log())
        .expect_err("a default run cannot converge while extras remain");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("shared") && msg.contains("--keep-extra"),
        "the error explains the non-convergence and the escape hatch: {msg}"
    );

    let mut keep = sync_mem(src.clone(), dst.clone());
    keep.keep_extra = true;
    assert_eq!(
        cmd_sync(keep, &log()).unwrap(),
        0,
        "keep-extra accepts the non-convergence"
    );
    assert_eq!(rfile(&outside, "y.txt"), b"extra");
    assert!(is_link(&dst.join("link")), "the shared link is kept");
}

// ---------------------------------------------------------------------------
// N-5: a case-only rename is not a way through a shared link
// ---------------------------------------------------------------------------

/// `rename_to_src_casing` would otherwise move the target's `x.txt` to
/// `X.txt`. The pass validates every pair before its first filesystem call.
#[test]
fn sync_refuses_to_rename_through_a_shared_dst_link() {
    let t = temp_root("shared_link_rename");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    wfile(&src, "link/X.txt", b"payload");
    wfile(&outside, "x.txt", b"payload");
    skip_if_no_link!("directory", dir_link(&outside, &dst.join("link")));

    let mut opts = sync_mem(src.clone(), dst.clone());
    opts.common.case_sensitive = false;
    let err = cmd_sync(opts, &log()).expect_err("renaming through a shared dst link must fail");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("rename") && msg.contains("link"),
        "the error names the rename and the link: {msg}"
    );
    assert_eq!(
        entry_names(&outside),
        ["x.txt"],
        "the target's entry name is unchanged"
    );
    assert!(is_link(&dst.join("link")), "the link is left in place");
}

// ---------------------------------------------------------------------------
// N-2: a successful sync leaves the cache in step
// ---------------------------------------------------------------------------

/// Removing a link removes the followed descendants too. Their cache rows used
/// to survive, so `compare-self` reported the link's target paths as missing
/// immediately after a successful sync.
#[test]
fn a_sync_that_removes_a_link_leaves_compare_self_clean() {
    let t = temp_root("link_cache_prune");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "real/x.txt", b"shared content");
    wfile(&dst, "real/x.txt", b"shared content");
    common::sync_mtime(&src.join("real/x.txt"), &dst.join("real/x.txt"));
    skip_if_no_link!("directory", dir_link(&dst.join("real"), &dst.join("link")));

    assert_eq!(
        cmd_sync(common::sync(src.clone(), dst.clone()), &log()).unwrap(),
        0
    );
    assert!(!dst.join("link").exists(), "the link is removed");
    assert_eq!(rfile(&dst, "real/x.txt"), b"shared content");
    assert_eq!(
        cmd_compare_self(common::compare_self_opts(dst.clone()), &log()).unwrap(),
        0,
        "the dst cache no longer claims the link's descendants"
    );
}

/// `fix_dirs` creates a directory but used to leave no row for it, so
/// `compare-self` reported the directory the sync just created as extra. This
/// is the non-link half of the same cache-drift bug.
#[test]
fn a_type_conflict_fix_leaves_compare_self_clean() {
    let t = temp_root("fix_dir_cache_row");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "dir/x.txt", b"x");
    wfile(&dst, "dir", b"blocking file");

    assert_eq!(
        cmd_sync(common::sync(src.clone(), dst.clone()), &log()).unwrap(),
        0
    );
    assert!(dst.join("dir").is_dir());
    assert_eq!(rfile(&dst, "dir/x.txt"), b"x");
    assert_eq!(
        cmd_compare_self(common::compare_self_opts(dst.clone()), &log()).unwrap(),
        0,
        "the directory this run created has a cache row"
    );
}

// ---------------------------------------------------------------------------
// N-3: an excluded link is left alone, and the dry run says so
// ---------------------------------------------------------------------------

/// The exclude is a subtree, so neither the link nor its followed descendants
/// enter the plan. The dry run and the real run therefore name the same
/// (empty) set of removals, and the link survives both.
#[test]
fn excluding_a_dst_link_leaves_it_and_its_target_alone() {
    let t = temp_root("exclude_link");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    wfile(&src, "a.txt", b"a");
    wfile(&outside, "x.txt", b"external");
    skip_if_no_link!("directory", dir_link(&outside, &dst.join("link")));

    let (dry_status, dry) = sync_bin(&src, &dst, &["--exclude", "link", "--dry-run"]);
    assert!(dry_status.success(), "dry run failed: {dry:?}");
    assert!(is_link(&dst.join("link")), "the dry run touches nothing");
    assert!(
        paths(&dry, "delete").is_empty(),
        "the excluded link is not deleted: {dry:?}"
    );

    let (status, recs) = sync_bin(&src, &dst, &["--exclude", "link"]);
    assert!(status.success(), "sync failed: {recs:?}");
    assert!(is_link(&dst.join("link")), "the excluded link survives");
    assert_eq!(rfile(&outside, "x.txt"), b"external");
    assert_eq!(
        paths(&dry, "delete"),
        paths(&recs, "delete"),
        "the dry run names the same removals as the real run"
    );
    assert_eq!(paths(&dry, "rmdir"), paths(&recs, "rmdir"));
    assert!(paths(&recs, "delete").is_empty(), "{recs:?}");
}
