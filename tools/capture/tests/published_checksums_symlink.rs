//! End-to-end: a file link that sorts before its real target must not make the
//! published `checksums.json` drop the real path.
//!
//! The walk that digests the artifact tree deduplicates only directories, so a
//! symlink to a file cannot filter out the real file when it sorts first. This
//! test plants exactly that shape in a real capture's tree, regenerates the
//! published document through the same writer close-out uses, and asserts that
//! both names are listed with the real file's bytes. Nothing here touches the
//! shared bundle or the recorded-JSON golden fixture: the capture and the
//! regenerated checksums live in a scratch tree, and both entries are compared
//! by path so their order is deterministic regardless of directory iteration.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use common::{Mock, World, dispatch_with};
use gircap::emit::index::{Checksums, FileEntry};
use girpr_tempdir::TempDir;

/// The one bundle under `out`, found by its manifest (the same shape a capture
/// writes regardless of how the output root nests it).
fn find_bundle(out: &Path) -> PathBuf {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, found);
                } else {
                    found.push(path);
                }
            }
        }
    }
    let mut files = Vec::new();
    walk(out, &mut files);
    files
        .into_iter()
        .find(|p| p.file_name().is_some_and(|n| n == "MANIFEST.json"))
        .map(|p| p.parent().expect("bundle has a parent").to_path_buf())
        .expect("a bundle under the output root")
}

#[cfg(unix)]
fn link_file(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn link_file(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[test]
fn a_file_link_sorting_first_is_listed_alongside_its_real_target() {
    let scratch = TempDir::new("gircap-symlink-checksums", "fixture");
    let out = scratch.join("artifacts");
    let mock = Mock::in_memory(World::all_shapes());
    let args: Vec<String> = vec![
        "capture".into(),
        "--out".into(),
        out.to_string_lossy().into_owned(),
        "--hyp-base".into(),
        mock.hyp_base(),
        "--sophon-base".into(),
        mock.sophon_base(),
    ];
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let dispatch = dispatch_with(&borrowed, Some(mock.transport()));
    assert_eq!(dispatch.code, 0, "capture failed: {:?}", dispatch.detail);
    let bundle = find_bundle(&out);

    // A real file and a link to it whose name sorts before the real one. The
    // link is created only in this test fixture, never by capture code.
    let game = bundle.join("game");
    let real = game.join("z_real.bin");
    let link = game.join("a_link.bin");
    let bytes: Vec<u8> = (0..64_u8).collect();
    std::fs::write(&real, &bytes).expect("write the real file");
    if !link_file(&real, &link) {
        eprintln!("skipping: file symlinks unavailable on this host");
        return;
    }

    // Regenerate the published document over the modified tree through the very
    // writer close-out uses, so the assertion is about the bytes a reader sees.
    let (_, written) = gircap::emit::write_checksums(&bundle).expect("write published checksums");
    let raw =
        std::fs::read(bundle.join("verify/checksums.json")).expect("read published checksums");
    let published: Checksums = serde_json::from_slice(&raw).expect("parse published checksums");
    assert_eq!(published, written, "the file and the writer's return agree");

    let entries: BTreeMap<&str, &FileEntry> = published
        .entries
        .iter()
        .map(|e| (e.path.as_str(), e))
        .collect();
    let expected_sha = gircap::hash::sha256_hex(&bytes);
    let expected_md5 = gircap::hash::md5_hex(&bytes);
    for path in ["game/a_link.bin", "game/z_real.bin"] {
        let entry = entries
            .get(path)
            .unwrap_or_else(|| panic!("published checksums omits {path}"));
        assert_eq!(entry.bytes, bytes.len() as u64, "{path}: byte count");
        assert_eq!(entry.sha256, expected_sha, "{path}: real file's sha256");
        assert_eq!(entry.md5, expected_md5, "{path}: real file's md5");
    }
    // Both names are present and sorted, so the result does not depend on
    // directory iteration order (assumption 16: deterministic artifact bytes).
    let paths: Vec<&str> = published.entries.iter().map(|e| e.path.as_str()).collect();
    let sorted = {
        let mut p = paths.clone();
        p.sort_unstable();
        p
    };
    assert_eq!(paths, sorted, "published entries are not in path order");
}
