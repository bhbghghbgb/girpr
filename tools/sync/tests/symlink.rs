//! Links in the walk and in `copy`.
//!
//! The walk follows links, so a directory link is reported as the directory it
//! points at. That is the documented comparison behaviour and it is what makes
//! the *deletion* side dangerous: a relative path under the link resolves
//! through it, so `DELETE`/`RMDIR` can reach a file that is not in `dst` at all.
//! These cases pin the walk (F-3) and the copy-side type conflict (F-2); the
//! deletion cases build on them.
//!
//! ## Skipping
//!
//! Windows without developer mode refuses `symlink_file` and `symlink_dir`;
//! `mklink /J` creates a directory junction without elevation, so the directory
//! cases still run there. A case that needs a refused primitive prints why and
//! returns - a silent pass would read as coverage it never had. The file-link
//! cases are skipped on such a host, and the note says so.

mod common;

use common::{dir_link, file_link, is_link, log, rfile, sync_mem, temp_root, wfile};
use girsync::cmd_sync;
use girsync::scan::walk_live;

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
