//! What counts as a link, and how to remove one without touching its target.
//!
//! The walk follows links (`walkdir`'s `follow_links(true)`), so a directory
//! link inside `dst` is reported as the directory it points at, and its target's
//! children appear under the link's relative path. That is the documented
//! comparison behaviour, but it makes the *deletion* side dangerous: joining a
//! relative path under the link and calling `remove_file` resolves the
//! intermediate components through the link, so `DELETE`/`RMDIR` would remove
//! files in the target - possibly outside `dst`. The helpers here are the single
//! definition of link-ness; the walk, the planner and every destructive apply
//! path ask them rather than deciding for themselves.
//!
//! ## Links and junctions
//!
//! On Windows both a symbolic link and a directory junction are reparse points
//! (`FILE_ATTRIBUTE_REPARSE_POINT`). The two `FileType` bits are not a reliable
//! test across toolchains: on rustc 1.98.1 a junction reports
//! `is_symlink = true` and `is_dir = false` from `symlink_metadata`, while older
//! reports had `is_symlink = false`. [`is_link`] therefore asks the reparse
//! attribute, which is the property Windows itself uses, and falls back to
//! `is_symlink` on other platforms.
//!
//! Removal has to match: `DeleteFileW` (Rust's `remove_file`) rejects a
//! directory entry, and `RemoveDirectoryW` (`remove_dir`) is not what removes a
//! file link. [`remove`] asks the link's target which one applies, and falls
//! back to trying both for a dangling link.

use std::io;
use std::path::Path;

/// Windows' `FILE_ATTRIBUTE_REPARSE_POINT`; carried by junctions and by
/// symbolic links.
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// True when `path` itself is a link or a junction, whichever the platform
/// calls it. Never follows the final component.
///
/// This is the infallible convenience form. Destructive guards use
/// [`try_is_link`] instead, because a metadata error treated as "not a link" is
/// the unsafe direction: the guard would then reach through the path it could
/// not read.
pub fn is_link(path: &Path) -> bool {
    try_is_link(path).unwrap_or(false)
}

/// [`is_link`] with metadata errors reported.
///
/// `NotFound` is `Ok(false)`: there is no entry, so there is nothing to
/// traverse. Every other error is returned, so a caller that is about to
/// remove or write through a path can fail instead of silently assuming the
/// path is not a link.
pub fn try_is_link(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(md) => Ok(is_link_metadata(&md)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Whether `symlink_metadata`'s result describes a link.
fn is_link_metadata(md: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        md.file_type().is_symlink()
    }
}

/// True when `path` carries a Windows reparse point. Never follows, and never
/// errors: the walk already has a live entry, so this is the extra junction
/// check for a directory whose `path_is_symlink` was false on some toolchain.
///
/// Returns `false` everywhere else, so the walk pays nothing for a check that
/// cannot succeed.
pub fn is_reparse_point(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        std::fs::symlink_metadata(path)
            .map(|md| md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Remove the link at `path`, leaving its target alone.
///
/// - Unix: `unlink(2)` removes either kind of link, so one call covers both.
/// - Windows: a file link needs `remove_file` and a directory link or junction
///   needs `remove_dir`; the target's own type decides which. A dangling link
///   has no target to ask, so both are tried and the second error is reported
///   with the first rather than lost.
///
/// A metadata error other than `NotFound` is returned rather than being read as
/// "not a directory" and followed by a guess.
///
/// **Do not replace this with `remove_dir_all`.** Its documented
/// "does not follow symbolic links" behaviour is not something this crate
/// verifies, and the whole point of this module is to stop removals from
/// reaching a target.
pub fn remove(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::remove_file(path)
    }
    #[cfg(windows)]
    {
        match std::fs::metadata(path) {
            Ok(md) if md.is_dir() => std::fs::remove_dir(path),
            Ok(_) => remove_file_or_dir(path),
            // A dangling link has no target to ask. Try the file removal first;
            // if the entry is a directory link, retry as a directory and keep
            // both errors if neither works.
            Err(e) if e.kind() == io::ErrorKind::NotFound => match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(file_err) => match std::fs::remove_dir(path) {
                    Ok(()) => Ok(()),
                    Err(dir_err) => Err(io::Error::other(format!(
                        "remove_file: {file_err}; remove_dir: {dir_err}"
                    ))),
                },
            },
            Err(e) => Err(e),
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        std::fs::remove_file(path)
    }
}

/// Windows: `DeleteFileW` rejects a directory entry with `ERROR_ACCESS_DENIED`;
/// retry it as a directory rather than swapping one failure for another.
#[cfg(windows)]
fn remove_file_or_dir(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => std::fs::remove_dir(path),
        Err(e) => Err(e),
    }
}

/// The shortest ancestor of `rel` under `root` that is a link, or `None`.
///
/// The entry itself counts: a link that is itself extra is the path to remove.
/// This is the runtime counterpart of the planner's map-based lookup, and it is
/// the belt-and-braces half of the F-1 guard: a plan is computed before the run
/// changes anything, so the apply phases re-check before reaching through a
/// path component.
///
/// One `symlink_metadata` per component, so it costs depth stats per removal and
/// only on paths the plan already decided to remove.
pub fn topmost_link_ancestor(root: &Path, rel: &str) -> io::Result<Option<String>> {
    let mut end = 0;
    loop {
        let (prefix, next) = match rel[end..].find('/') {
            Some(i) => (&rel[..end + i], Some(end + i + 1)),
            None => (rel, None),
        };
        if try_is_link(&root.join(prefix))? {
            return Ok(Some(prefix.to_string()));
        }
        match next {
            Some(next) => end = next,
            None => return Ok(None),
        }
    }
}

/// The shortest **strict** ancestor of `rel` that is a link, or `None`.
///
/// Unlike [`topmost_link_ancestor`], the final component is not considered:
/// a link *at* the path is something a copy or a type-conflict fix is allowed
/// to replace, while a link *above* it must never be traversed. A relative path
/// with no `/` has no ancestors and needs no filesystem access at all.
pub fn link_ancestor_above(root: &Path, rel: &str) -> io::Result<Option<String>> {
    match rel.rfind('/') {
        Some(last) => topmost_link_ancestor(root, &rel[..last]),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A missing component is not a link: there is nothing to traverse, so the
    /// guard can let the operation proceed and fail (or succeed) on its own.
    #[test]
    fn a_missing_path_is_not_a_link() {
        let root = girpr_tempdir::TempDir::new("girsync_link", "missing");
        assert!(!try_is_link(&root.join("gone")).unwrap());
        assert_eq!(topmost_link_ancestor(&root, "gone/child").unwrap(), None);
    }

    /// The strict helper ignores the final component, which is the difference
    /// between "replace this link" (allowed) and "traverse this link" (refused).
    /// No slash means no ancestor, so the check must not touch the filesystem.
    #[test]
    fn a_top_level_path_has_no_strict_ancestor() {
        let root = girpr_tempdir::TempDir::new("girsync_link", "top");
        assert_eq!(link_ancestor_above(&root, "a.txt").unwrap(), None);
    }

    /// A regular directory in the chain is not a boundary; only the link at the
    /// end of the ancestor prefix is. This pins that the helper walks prefixes
    /// rather than reporting any existing ancestor.
    #[test]
    fn a_regular_ancestor_is_not_a_link() {
        let root = girpr_tempdir::TempDir::new("girsync_link", "regular");
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        assert_eq!(link_ancestor_above(&root, "a/b/c.txt").unwrap(), None);
        assert_eq!(topmost_link_ancestor(&root, "a/b/c.txt").unwrap(), None);
    }
}
