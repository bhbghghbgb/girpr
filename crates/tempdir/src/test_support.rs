//! Shared, program-free scratch-directory test fixtures.
//!
//! This is **test support**, not part of the scratch-directory API the shipped
//! programs rely on. It holds the file/link/cache-path helpers that need no
//! program's types — the ones `girsync` and the repair binary used to keep as
//! byte-identical copies in their own `tests/` directories. Each tool
//! reaches them through a thin `tests/common/mod.rs` re-export, so every call
//! site (`common::helper(..)`, `use common::*`, and the two exported macros)
//! keeps working unchanged.
//!
//! The girsync-coupled half of the old module (the option/cache/planner
//! builders) now lives in `girsync-test-support`, which depends on girsync.
//! Keeping it out of this crate is the point: `girpr-tempdir` stays a
//! girsync-free leaf, so a program that does not otherwise depend on girsync no
//! longer inherits it just for a scratch directory.
//!
//! Every helper's behaviour and signature is preserved from the original
//! shared file. In particular the scratch trees are still made through this
//! crate's [`TempDir`], so the naming and cleanup policy is the one described
//! in the crate docs.

#![allow(dead_code)]
// each test binary used only a subset of these helpers
// These are test fixtures: panicking on a broken fixture is the point. Keep the
// crate-level lint gate (which exists for code a user runs) out of this module.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

// The scratch-directory helper this crate already owns, so the tree a test
// writes into is named and removed exactly the way it is everywhere else in
// the workspace.
pub use crate::TempDir;

/// The prefix every scratch directory in this suite carries: it says which
/// suite leaked a directory if one survives a crash.
pub const TEMP_PREFIX: &str = "girsync";

/// The console's records, with the colour taken out.
///
/// `tracing`'s human format colours the level and the target separately, and
/// the escape sequences land *between* them: the text is `DEBUG`, then an
/// escape, then the target. A substring that spans one is not a substring at
/// all — and an assertion that a coloured string does *not* contain one passes
/// for the wrong reason, which is worse than no assertion. The escape grammar
/// belongs to `strip-ansi-escapes`; callers only need the plain text.
pub fn strip_ansi(s: &str) -> String {
    strip_ansi_escapes::strip_str(s)
}

/// A fresh, uniquely named scratch directory carrying the caller's prefix and
/// tag, removed when the returned guard drops.
///
/// The prefix is supplied by each caller so a leftover still says which suite
/// leaked it. `cargo nextest run` gives every test its own process, and two
/// suites may run at once, so the name must be random rather than fixed; the
/// naming and cleanup policy is the one `girpr-tempdir` documents.
///
/// This is the one scratch-directory construction shared by the test suites;
/// each suite keeps a thin wrapper that pins its own prefix.
pub fn temp_dir(prefix: &str, tag: &str) -> TempDir {
    TempDir::new(prefix, tag)
}

/// Isolated temp directory per test, so parallel `cargo nextest run` workers
/// never share a cache file.
///
/// The tree is named so nothing can collide with it and removed when the
/// guard drops; see `girpr-tempdir` for why each part of the name is there.
pub fn temp_root(tag: &str) -> TempDir {
    temp_dir(TEMP_PREFIX, tag)
}

pub fn wfile(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, bytes).unwrap();
}

// ---------------------------------------------------------------------------
// Links
// ---------------------------------------------------------------------------
//
// Tests that need a link call these; a platform that refuses the creation
// (Windows without developer mode refuses `symlink_file` and `symlink_dir`) gets
// an `Err` and the test skips with the OS message. A directory link has a
// fallback that needs no privilege - a junction - because that is the shape a
// game tree on Windows is most likely to contain anyway.

/// Create a link to a file.
pub fn file_link(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "no link API",
        ))
    }
}

/// Create a link to a directory, falling back to a junction on Windows.
///
/// `cmd /c mklink /J` creates a directory reparse point without elevation, and
/// the walk and the deletion guard treat it as a link either way
/// (`girsync::link::try_is_link`). If both calls fail, the original error is
/// returned so the caller can report why it skipped.
pub fn dir_link(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        let first = match std::os::windows::fs::symlink_dir(target, link) {
            Ok(()) => return Ok(()),
            Err(e) => e,
        };
        // `mklink` is a `cmd` builtin and reads a forward slash as a switch:
        // a target like `dst/sub` would fail with "Invalid switch". std accepts
        // either separator on Windows, so hand the command native ones.
        let native = |p: &Path| PathBuf::from(p.to_string_lossy().replace('/', "\\"));
        let created = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(native(link))
            .arg(native(target))
            .output();
        match created {
            Ok(out) if out.status.success() => Ok(()),
            Ok(out) => Err(std::io::Error::new(
                first.kind(),
                format!(
                    "{first}; junction fallback failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            )),
            Err(e) => Err(std::io::Error::new(
                first.kind(),
                format!("{first}; junction fallback failed: {e}"),
            )),
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "no link API",
        ))
    }
}

/// Skip the calling test, with the reason, when the platform refused the link.
///
/// A macro rather than a helper so the `return` belongs to the test. A silent
/// skip is not acceptable: the point of the link cases is to pin behaviour, and
/// a run that quietly covered nothing would read as coverage.
#[macro_export]
macro_rules! skip_if_no_link {
    ($what:expr, $res:expr) => {
        match $res {
            Ok(()) => {}
            Err(e) => {
                eprintln!("skip: this platform refused a {} link: {e}", $what);
                return;
            }
        }
    };
}

/// True when `dir`'s filesystem tells `CaseProbe` and `caseprobe` apart.
///
/// The probe is created and removed, so the answer describes the scratch tree
/// the caller is about to use rather than the platform in the abstract: a
/// Windows host may be case-sensitive per directory, and a mounted volume may
/// differ from the system drive.
pub fn is_case_sensitive(dir: &Path) -> bool {
    let probe = dir.join("CaseProbe");
    std::fs::write(&probe, b"x").unwrap();
    let insensitive = dir.join("caseprobe").exists();
    std::fs::remove_file(&probe).unwrap();
    !insensitive
}

/// Skip the calling test when the filesystem pairs `CaseProbe` with
/// `caseprobe`.
///
/// A case-only rename is a different question on an insensitive filesystem: the
/// old spelling still resolves after the rename was planned, so the parity this
/// pins cannot be observed. Like `skip_if_no_link!`, the skip says why, because
/// a silent pass would read as coverage it never had.
#[macro_export]
macro_rules! skip_if_case_insensitive {
    ($dir:expr) => {
        if !$crate::test_support::is_case_sensitive($dir) {
            eprintln!("skip: this filesystem pairs `CaseProbe` with `caseprobe`");
            return;
        }
    };
}

/// True when `path` is itself a link or junction: the test-side mirror of
/// `girsync::link::try_is_link`. The reparse attribute alone is not enough on
/// Windows (cloud placeholders and other non-redirecting tags carry it), so the
/// tag is confirmed through std's name-surrogate test or `read_link`; this form
/// is infallible because every caller is an assertion, not a destructive guard.
pub fn is_link(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        match std::fs::symlink_metadata(path) {
            Ok(md) => {
                md.file_type().is_symlink()
                    || (md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
                        && std::fs::read_link(path).is_ok())
            }
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        path.is_symlink()
    }
}

pub fn rfile(root: &Path, rel: &str) -> Vec<u8> {
    std::fs::read(root.join(rel)).unwrap()
}

// ---------------------------------------------------------------------------
// Game-tree file helpers
// ---------------------------------------------------------------------------
//
// The product's integration suite used to keep these two in its own
// `tests/common`. They are host-relative file mechanics with no product logic:
// fixture paths are written with `/` on every host, and a missing parent is
// created so a fixture can describe a nested tree without pre-creating it.
// Both are copied verbatim so call sites behave exactly as before; each suite
// reaches them through a re-export shim in its own `tests/common`.

/// Write `bytes` to `root/rel`, treating `/` in `rel` as the host separator and
/// creating any missing parent directories.
pub fn write_game_file(root: &Path, rel: &str, bytes: &[u8]) {
    let rel_fs = rel.replace('/', std::path::MAIN_SEPARATOR_STR);
    let p = root.join(rel_fs);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(&p, bytes).expect("write game file");
}

/// Read `root/rel`, treating `/` in `rel` as the host separator.
pub fn read_game_file(root: &Path, rel: &str) -> Option<Vec<u8>> {
    let rel_fs = rel.replace('/', std::path::MAIN_SEPARATOR_STR);
    std::fs::read(root.join(rel_fs)).ok()
}

/// Actual directory entry names, excluding `girpr-cache*`.
///
/// Path existence cannot answer casing questions on a case-insensitive
/// filesystem: `data.txt` and `Data.txt` resolve to the same entry there.
pub fn entry_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with("girpr-cache"))
        .collect();
    names.sort();
    names
}

/// Pin `target`'s mtime to `source`'s, so two identical-content files compare
/// equal (compare treats size+mtime+hash as identity).
pub fn sync_mtime(source: &Path, target: &Path) {
    let mtime = std::fs::metadata(source).unwrap().modified().unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(target)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
}
