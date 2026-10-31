//! Links in the walk, in `copy`, and in the deletion plan.
//!
//! The walk follows links, so a directory link is reported as the directory it
//! points at. That is the documented comparison behaviour and it is what makes
//! the *deletion* side dangerous: a relative path under the link resolves
//! through it, so `DELETE`/`RMDIR` can reach a file that is not in `dst` at all.
//! These cases pin the walk behaviour, the copy-side type conflict and the
//! no-reach-through guarantee.
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
    dir_link, entry_names, file_link, is_link, log, parse_ndjson, recs_of, rfile, sync_mem,
    temp_root, wfile,
};
use girsync::scan::walk_live;
use girsync::{cmd_compare_self, cmd_sync};
use serde_json::Value;

/// Run the real binary and hand back its whole output.
///
/// The in-process `cmd_sync` cannot show what was *reported* — stdout belongs to
/// the binary. The refusal cases also need stderr, because that is where the
/// error goes.
fn sync_bin_raw(src: &Path, dst: &Path, extra: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
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
        .expect("spawn girsync")
}

/// Run the real binary, returning its exit status and records.
fn sync_bin(src: &Path, dst: &Path, extra: &[&str]) -> (std::process::ExitStatus, Vec<Value>) {
    let out = sync_bin_raw(src, dst, extra);
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
// The walk
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
/// working when a `dst` link to a *directory* is unlinked instead.
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

/// The target of a kept dst file link is that link's content, not an extra
/// path: with no copy planned the delete phase must not remove the target and
/// leave `a.bin` dangling. The target is spared in both runs, so the pair is
/// stable.
#[test]
fn sync_keeps_the_target_of_a_dst_file_link_when_no_copy_is_needed() {
    let t = temp_root("sync_dst_file_link_equal");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.bin", b"same");
    wfile(&dst, "target.bin", b"same");
    skip_if_no_link!(
        "file",
        file_link(&dst.join("target.bin"), &dst.join("a.bin"))
    );
    common::sync_mtime(&src.join("a.bin"), &dst.join("target.bin"));

    for run in 1..=2 {
        assert_eq!(
            cmd_sync(sync_mem(src.clone(), dst.clone()), &log()).unwrap(),
            0,
            "run {run}"
        );
        assert!(
            is_link(&dst.join("a.bin")),
            "the link survives run {run}: its target was not deleted"
        );
        assert_eq!(
            rfile(&dst, "target.bin"),
            b"same",
            "and so does the content it names, after run {run}"
        );
    }
}

/// The target can sit under a dst-only directory. Both the target and that
/// directory are in the removal plan; the `rmdir` pass runs *after* the copy,
/// so leaving either in the plan removes the content the write-through landed
/// in.
#[test]
fn sync_spares_a_link_target_inside_a_dst_only_directory() {
    let t = temp_root("sync_dst_file_link_dir");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.bin", b"NEW-CONTENT");
    wfile(&dst, "d/target.bin", b"old");
    skip_if_no_link!(
        "file",
        file_link(&dst.join("d/target.bin"), &dst.join("a.bin"))
    );

    assert_eq!(
        cmd_sync(sync_mem(src.clone(), dst.clone()), &log()).unwrap(),
        0
    );
    assert!(
        dst.join("d").is_dir(),
        "the directory holding the target stays"
    );
    assert_eq!(rfile(&dst, "d/target.bin"), b"NEW-CONTENT");
    assert!(is_link(&dst.join("a.bin")), "the link is still a link");
}

/// When src also mirrors the target, the link and the target are one file that
/// cannot hold two contents: a copy at either end would change the other path,
/// so the run refuses before touching the tree.
#[test]
fn sync_refuses_a_dst_file_link_whose_target_is_also_mirrored() {
    let t = temp_root("sync_dst_file_link_mirrored");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.bin", b"AAAA");
    wfile(&src, "target.bin", b"BBBBBBBB");
    wfile(&dst, "target.bin", b"old");
    skip_if_no_link!(
        "file",
        file_link(&dst.join("target.bin"), &dst.join("a.bin"))
    );

    let err = cmd_sync(sync_mem(src.clone(), dst.clone()), &log())
        .expect_err("one inode cannot be given two contents");
    let msg = format!("{err:#}");
    assert!(msg.contains("refusing"), "names the refusal: {msg}");
    assert!(msg.contains("link"), "names the shape: {msg}");
    assert!(msg.contains("target.bin"), "names the target: {msg}");
    assert_eq!(
        rfile(&dst, "target.bin"),
        b"old",
        "the shared file is untouched"
    );
    assert!(is_link(&dst.join("a.bin")), "and the link is left in place");
}

// ---------------------------------------------------------------------------
// A dst directory link blocking a file copy is unlinked
// ---------------------------------------------------------------------------

/// A `dst` directory link blocking a file copy is unlinked, and its target is
/// left intact.
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
// A dst directory link must never be deleted through
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
        "the target's file survives: it was not deleted through the link"
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
// A link inside src that reaches dst is the same self-overlap
// ---------------------------------------------------------------------------

/// Assert a run refused the src-link/dst overlap before changing the mirror,
/// and that the destination file is still there with its bytes.
///
/// `link`, `relation` and `advice` are asserted as whole sentences: the refusal
/// is where a user learns what the link would have reached, so a reworded
/// consequence has to fail a test rather than ship unnoticed.
fn assert_refused_src_link_into_dst(
    out: &std::process::Output,
    dst: &Path,
    rel: &str,
    link: &str,
    relation: &str,
    advice: &str,
) {
    assert_eq!(
        out.status.code(),
        Some(3),
        "a src link into dst is a fatal configuration; stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let context = format!("src and dst must be disjoint: src link `{link}` resolves to");
    assert!(
        stderr.contains(&context),
        "the error names the overlap and the src link: {stderr}"
    );
    assert!(
        stderr.contains(relation),
        "the error names where the link leads: {stderr}"
    );
    assert!(
        stderr.contains(advice),
        "the error says what the link would do: {stderr}"
    );
    assert_eq!(
        rfile(dst, rel),
        b"PRECIOUS",
        "the destination file the link reached must survive"
    );
}

/// `src/link` resolves to `dst` itself. The walk exposes every dst file under
/// `link/...`; planning them as extras would delete the content the copy needs.
/// The run must refuse before touching the mirror.
#[test]
fn sync_refuses_a_src_directory_link_to_dst() {
    let t = temp_root("src_link_to_dst");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&dst, "x.txt", b"PRECIOUS");
    skip_if_no_link!("directory", dir_link(&dst, &src.join("link")));

    let out = sync_bin_raw(&src, &dst, &[]);
    assert_refused_src_link_into_dst(
        &out,
        &dst,
        "x.txt",
        "link",
        "which is inside dst",
        "a mirror may not read its own destination through a link",
    );
    assert!(!dst.join("link").exists(), "nothing was created in dst");
    assert!(is_link(&src.join("link")), "the source link is left alone");
}

/// The same overlap one level down: `src/link` resolves to `dst/sub`, so
/// `sub/x.txt` looks extra at its real path while `link/x.txt` looks missing.
/// The delete pass would remove the only copy before the copy ran.
#[test]
fn sync_refuses_a_src_directory_link_inside_dst() {
    let t = temp_root("src_link_inside_dst");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let dst_sub = t.subdir("dst/sub");
    wfile(&dst, "sub/x.txt", b"PRECIOUS");
    skip_if_no_link!("directory", dir_link(&dst_sub, &src.join("link")));

    let out = sync_bin_raw(&src, &dst, &[]);
    assert_refused_src_link_into_dst(
        &out,
        &dst,
        "sub/x.txt",
        "link",
        "which is inside dst",
        "a mirror may not read its own destination through a link",
    );
    assert!(!dst.join("link").exists(), "nothing was created in dst");
}

/// The other direction: `src/link` resolves to a directory that **contains**
/// dst, so walking the link reaches dst from above.
#[test]
fn sync_refuses_a_src_directory_link_containing_dst() {
    let t = temp_root("src_link_contains_dst");
    let src = t.subdir("src");
    let outer = t.subdir("outer");
    let dst = t.subdir("outer/dst");
    wfile(&dst, "x.txt", b"PRECIOUS");
    wfile(&outer, "sibling.txt", b"SIBLING");
    skip_if_no_link!("directory", dir_link(&outer, &src.join("link")));

    let out = sync_bin_raw(&src, &dst, &[]);
    assert_refused_src_link_into_dst(
        &out,
        &dst,
        "x.txt",
        "link",
        "which contains dst",
        "a mirror may not reach its own destination through a link",
    );
    assert_eq!(
        rfile(&outer, "sibling.txt"),
        b"SIBLING",
        "the target's other content is untouched too"
    );
}

/// A file link is the same hazard: `src/link.txt` -> `dst/x.txt` makes `x.txt`
/// look extra while `link.txt` looks missing, so the delete runs first. This
/// host refuses `symlink_file` without developer mode, so the case may skip;
/// the directory cases above run here through a junction.
#[test]
fn sync_refuses_a_src_file_link_into_dst() {
    let t = temp_root("src_file_link_to_dst");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&dst, "x.txt", b"PRECIOUS");
    skip_if_no_link!("file", file_link(&dst.join("x.txt"), &src.join("link.txt")));

    let out = sync_bin_raw(&src, &dst, &[]);
    assert_refused_src_link_into_dst(
        &out,
        &dst,
        "x.txt",
        "link.txt",
        "which is inside dst",
        "a mirror may not read its own destination through a link",
    );
    assert!(
        is_link(&src.join("link.txt")),
        "the source link is left alone"
    );
}

/// A src link outside dst is ordinary mirrored content, but a cache inside its
/// target is still tool state: a nested root keeps its own `girpr-cache`, and
/// copying it would duplicate a database or fail on its live lock.
#[test]
fn a_src_link_does_not_mirror_a_nested_cache() {
    let t = temp_root("src_link_nested_cache");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    wfile(&outside, "x.txt", b"x");
    wfile(&outside, "girpr-cache", b"tool state");
    wfile(&outside, "sub/girpr-cache-backup-1/rows", b"tool state");
    skip_if_no_link!("directory", dir_link(&outside, &src.join("link")));

    let (status, recs) = sync_bin(&src, &dst, &[]);
    assert!(status.success(), "sync failed: {recs:?}");
    assert_eq!(rfile(&dst, "link/x.txt"), b"x");
    assert!(
        !dst.join("link/girpr-cache").exists(),
        "the target's live cache is not mirror content"
    );
    assert!(
        !dst.join("link/sub/girpr-cache-backup-1").exists(),
        "nor are its backups"
    );
}

// ---------------------------------------------------------------------------
// A dst link into src is the same self-overlap
// ---------------------------------------------------------------------------

/// A `dst` file link whose target is a *different* source file. `File::create`
/// follows the final component, so copying through the link would verify
/// against the truncated target and exit 0 when `b.bin` was excluded from the
/// copy set. The run must refuse before touching either tree. This host refuses `symlink_file` without developer mode, so the case
/// may skip; the junction case below runs here.
#[test]
fn sync_refuses_a_dst_file_link_into_src() {
    let t = temp_root("dst_file_link_to_src");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.bin", b"AAAA");
    wfile(&src, "b.bin", b"BBBBBBBBBB");
    skip_if_no_link!("file", file_link(&src.join("b.bin"), &dst.join("a.bin")));

    // Excluding b.bin makes the pre-fix shape deterministic: only a.bin is
    // copied, through the link, and the run exited 0 with the source truncated.
    let out = sync_bin_raw(&src, &dst, &["--exclude", "b.bin"]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "a dst link into src is a fatal configuration; stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("src and dst must be disjoint: dst link `a.bin` resolves to"),
        "the error names the overlap and the dst link: {stderr}"
    );
    assert!(
        stderr.contains("which is inside src"),
        "the error names where the link leads: {stderr}"
    );
    assert!(
        stderr.contains("a mirror may not write into its own source through a link"),
        "the error says what the link would do: {stderr}"
    );
    assert_eq!(
        rfile(&src, "b.bin"),
        b"BBBBBBBBBB",
        "the source file the link reached must survive"
    );
    assert!(
        is_link(&dst.join("a.bin")),
        "the link is left in place, not replaced"
    );
}

/// The same overlap through a directory link, which this host can create as a
/// junction without elevation. `dst/a.bin` resolves to `src`, so the dst walk
/// sees a directory where src has a file; unlinking the link and writing the
/// file would be a write through a path that belongs to the source tree. The
/// pre-flight refuses it before any change.
#[test]
fn sync_refuses_a_dst_directory_link_into_src() {
    let t = temp_root("dst_dir_link_to_src");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.bin", b"SOURCE");
    skip_if_no_link!("directory", dir_link(&src, &dst.join("a.bin")));

    let out = sync_bin_raw(&src, &dst, &[]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "a dst link into src is a fatal configuration; stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("src and dst must be disjoint: dst link `a.bin` resolves to"),
        "the error names the overlap and the dst link: {stderr}"
    );
    assert!(
        stderr.contains("which is inside src"),
        "the error names where the link leads: {stderr}"
    );
    assert!(
        stderr.contains("a mirror may not write into its own source through a link"),
        "the error says what the link would do: {stderr}"
    );
    assert_eq!(rfile(&src, "a.bin"), b"SOURCE", "the source is untouched");
    assert!(is_link(&dst.join("a.bin")), "the link is left in place");
    assert!(
        dst.join("a.bin").is_dir(),
        "and still resolves to the source directory"
    );
}

/// The other direction: `dst/link` resolves to a directory that **contains**
/// src, so writing through the link reaches src from above.
#[test]
fn sync_refuses_a_dst_directory_link_containing_src() {
    let t = temp_root("dst_dir_link_contains_src");
    let outer = t.subdir("outer");
    let src = t.subdir("outer/src");
    let dst = t.subdir("dst");
    wfile(&src, "x.txt", b"PRECIOUS");
    skip_if_no_link!("directory", dir_link(&outer, &dst.join("link")));

    let out = sync_bin_raw(&src, &dst, &[]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "a dst link containing src is a fatal configuration; stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("src and dst must be disjoint: dst link `link` resolves to"),
        "the error names the overlap and the dst link: {stderr}"
    );
    assert!(
        stderr.contains("which contains src"),
        "the error names where the link leads: {stderr}"
    );
    assert!(
        stderr.contains("a mirror may not reach its own source through a link"),
        "the error says what the link would do: {stderr}"
    );
    assert_eq!(rfile(&src, "x.txt"), b"PRECIOUS", "the source is untouched");
    assert!(is_link(&dst.join("link")), "the link is left in place");
}

// ---------------------------------------------------------------------------
// A shared link is never a write-through path
// ---------------------------------------------------------------------------

/// The `fix_dirs` shape: `src/link/x` is a directory, the shared `dst/link`
/// junction resolves `x` to an external **file**, and the planner asks to
/// replace it with a directory. Removing it would delete external data, so the
/// run refuses before touching it.
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
/// delete its subtree. The run refuses before the first filesystem change.
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
// A shared link with extras is not a converged sync
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
// A case-only rename is not a way through a shared link
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
// A case-only rename is not a way past the plan-time link guard
// ---------------------------------------------------------------------------

/// A dst directory link at a case-only spelling is refused in both modes. The
/// rename pass only re-keys `dm` in a dry run, so the plan guard stats the
/// pre-rename spelling — the one still on disk — in both modes.
///
/// Runs only where the filesystem keeps `L` and `l` apart; on an insensitive one
/// the two names are the same entry and the divergence cannot be observed.
#[test]
fn a_case_only_dst_link_is_refused_in_both_modes() {
    let t = temp_root("case_only_link");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    skip_if_case_insensitive!(&dst);
    wfile(&src, "l/x.txt", b"NEW");
    wfile(&outside, "x.txt", b"OLD");
    skip_if_no_link!("directory", dir_link(&outside, &dst.join("L")));

    for extra in [&["--dry-run"][..], &[][..]] {
        let out = sync_bin_raw(&src, &dst, extra);
        assert_eq!(
            out.status.code(),
            Some(3),
            "the shared dst link must be refused in both modes: {extra:?}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("refusing") && stderr.contains("link"),
            "the error names the refusal and the link: {stderr}"
        );
        let recs = parse_ndjson(&out.stdout);
        assert!(
            !recs.iter().any(|r| r["event"] == "copy"),
            "no COPY is reported for a write through the link: {recs:?}"
        );
    }

    // The dry run touched nothing. The real run performed the (legitimate)
    // case-only rename before the plan guard refused, which is the mutation
    // order this case pins rather than claims away.
    assert_eq!(rfile(&outside, "x.txt"), b"OLD", "the target is untouched");
    assert_eq!(
        entry_names(&dst),
        ["l"],
        "the real run renamed the link to src's casing"
    );
    assert!(is_link(&dst.join("l")), "the link itself still survives");
}

// ---------------------------------------------------------------------------
// A successful sync leaves the cache in step
// ---------------------------------------------------------------------------

/// Removing a link removes the followed descendants too, so their cache rows
/// must be pruned with them — otherwise `compare-self` reports the link's
/// target paths as missing immediately after a successful sync.
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

/// One run can remove several links, and each one takes its followed subtree
/// with it. Every descendant row has to go, whichever link carried it: two
/// sibling links, with a third link nested inside the first one's target.
#[test]
fn a_sync_that_removes_several_links_prunes_every_descendant_row() {
    let t = temp_root("links_cache_prune");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    wfile(&src, "real/x.txt", b"shared content");
    wfile(&dst, "real/x.txt", b"shared content");
    common::sync_mtime(&src.join("real/x.txt"), &dst.join("real/x.txt"));
    wfile(&outside, "a/one.txt", b"one");
    wfile(&outside, "a/sub/two.txt", b"two");
    wfile(&outside, "b/three.txt", b"three");
    wfile(&outside, "nested/four.txt", b"four");
    skip_if_no_link!(
        "directory",
        dir_link(&outside.join("a"), &dst.join("alpha"))
    );
    skip_if_no_link!("directory", dir_link(&outside.join("b"), &dst.join("beta")));
    // Inside the first link's target, so its rows sit under `alpha` and are
    // removed with the link that carries them.
    skip_if_no_link!(
        "directory",
        dir_link(&outside.join("nested"), &outside.join("a/nested"))
    );

    assert_eq!(
        cmd_sync(common::sync(src.clone(), dst.clone()), &log()).unwrap(),
        0
    );
    assert!(!dst.join("alpha").exists(), "the first link is removed");
    assert!(!dst.join("beta").exists(), "the second link is removed");

    let rows = recs_of(&dst);
    for rel in [
        "alpha",
        "alpha/one.txt",
        "alpha/sub",
        "alpha/sub/two.txt",
        "alpha/nested",
        "alpha/nested/four.txt",
        "beta",
        "beta/three.txt",
    ] {
        assert!(!rows.contains_key(rel), "row `{rel}` is gone");
    }
    assert!(
        rows.contains_key("real/x.txt"),
        "a path src mirrors keeps its row"
    );
    assert_eq!(
        cmd_compare_self(common::compare_self_opts(dst.clone()), &log()).unwrap(),
        0,
        "the dst cache no longer claims any link's descendants"
    );
}

/// `fix_dirs` creates a directory but must leave a row for it, so
/// `compare-self` does not report the directory the sync just created as extra.
/// This is the non-link half of the same cache-drift bug.
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
// An excluded link is left alone, and the dry run says so
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

// ---------------------------------------------------------------------------
// The whole link chain, and shared storage
// ---------------------------------------------------------------------------

/// A kept dst file link whose target passes through a dst **directory link**:
/// `dst/a.bin -> link/x.bin`, `dst/link -> dst/real`. Canonicalizing the
/// target's parent names only `real/x.bin`, so both spellings and every link
/// component are spared — the alias and the intermediate link are storage now,
/// and the target survives. This host refuses file symlinks without developer
/// mode, so the case may skip; the resolver is unit-covered.
#[test]
fn sync_spares_a_kept_link_through_a_dst_directory_link() {
    let t = temp_root("kept_link_via_dir_link");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.bin", b"same");
    wfile(&dst, "real/x.bin", b"same");
    common::sync_mtime(&src.join("a.bin"), &dst.join("real/x.bin"));
    skip_if_no_link!("directory", dir_link(&dst.join("real"), &dst.join("link")));
    skip_if_no_link!(
        "file",
        file_link(Path::new("link/x.bin"), &dst.join("a.bin"))
    );

    for run in 1..=2 {
        let (status, recs) = sync_bin(&src, &dst, &[]);
        assert!(status.success(), "run {run} failed: {recs:?}");
        assert!(
            is_link(&dst.join("a.bin")),
            "the kept link survives run {run}: {recs:?}"
        );
        assert!(
            is_link(&dst.join("link")),
            "the intermediate directory link survives run {run}: {recs:?}"
        );
        assert_eq!(
            rfile(&dst, "real/x.bin"),
            b"same",
            "and so does the content it names, after run {run}"
        );
        assert!(
            !paths(&recs, "delete").contains(&"link"),
            "the delete plan never unlinks the chain component: {recs:?}"
        );
    }
}

/// A kept link whose chain passes through an intermediate dst file link under
/// a dst-only directory: `dst/a.bin -> extra/b`, `dst/extra/b ->
/// outside/real.bin`. Every kept path carries its ancestors, so `remove_dir_all`
/// cannot take the chain with a dst-only directory. File symlinks skip here as
/// above.
#[test]
fn sync_spares_an_intermediate_file_links_ancestor_directory() {
    let t = temp_root("kept_link_intermediate");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    let outside = t.subdir("outside");
    wfile(&src, "a.bin", b"payload");
    wfile(&outside, "real.bin", b"payload");
    common::sync_mtime(&src.join("a.bin"), &outside.join("real.bin"));
    std::fs::create_dir_all(dst.join("extra")).unwrap();
    skip_if_no_link!(
        "file",
        file_link(&outside.join("real.bin"), &dst.join("extra/b"))
    );
    skip_if_no_link!("file", file_link(Path::new("extra/b"), &dst.join("a.bin")));

    for run in 1..=2 {
        let (status, recs) = sync_bin(&src, &dst, &[]);
        assert!(status.success(), "run {run} failed: {recs:?}");
        assert!(
            is_link(&dst.join("a.bin")),
            "the kept link survives run {run}: {recs:?}"
        );
        assert!(
            is_link(&dst.join("extra/b")),
            "the intermediate link survives run {run}: {recs:?}"
        );
        assert!(
            !paths(&recs, "rmdir").contains(&"extra"),
            "the directory above the kept link is not removed: {recs:?}"
        );
    }
}

/// Two kept links to one dst-only file, `dst/a -> c` and `dst/b -> c`, with
/// different src files at `a` and `b`. The per-link check never saw the second
/// name, so both copies wrote through the one inode and one mirrored path ended
/// wrong at exit 0. The plan refuses before any mutation, in both modes. File
/// symlinks skip here.
#[test]
fn sync_refuses_two_kept_links_that_share_one_dst_file() {
    let t = temp_root("kept_links_shared");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a", b"AAAA");
    wfile(&src, "b", b"BBBBBBBB");
    wfile(&dst, "c", b"old");
    skip_if_no_link!("file", file_link(Path::new("c"), &dst.join("a")));
    skip_if_no_link!("file", file_link(Path::new("c"), &dst.join("b")));

    for extra in [&["--dry-run"][..], &[][..]] {
        let out = sync_bin_raw(&src, &dst, extra);
        assert_eq!(
            out.status.code(),
            Some(3),
            "one inode cannot be given two contents ({extra:?}): {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("refusing") && stderr.contains("`a`") && stderr.contains("`b`"),
            "the error names both links: {stderr}"
        );
        assert_eq!(rfile(&dst, "c"), b"old", "the shared file is untouched");
        assert!(is_link(&dst.join("a")) && is_link(&dst.join("b")));
    }
}

/// Chained: `dst/a -> b -> c` with `src/a` and `src/b` both regular files. `b`
/// is an intermediate link *and* a kept link; copying `a` rewrites the file `b`
/// mirrors. The terminal storage groups both and refuses. File symlinks skip
/// here.
#[test]
fn sync_refuses_a_chained_pair_of_kept_links_sharing_storage() {
    let t = temp_root("kept_links_chained");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a", b"AAAA");
    wfile(&src, "b", b"BBBBBBBB");
    wfile(&dst, "c", b"old");
    skip_if_no_link!("file", file_link(Path::new("c"), &dst.join("b")));
    skip_if_no_link!("file", file_link(Path::new("b"), &dst.join("a")));

    let out = sync_bin_raw(&src, &dst, &[]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "copying one end would break the other: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("refusing") && stderr.contains("`a`") && stderr.contains("`b`"),
        "the error names both links: {stderr}"
    );
    assert_eq!(rfile(&dst, "c"), b"old", "the shared file is untouched");
    assert!(is_link(&dst.join("a")) && is_link(&dst.join("b")));
}
