//! What counts as a link, and how to remove one without touching its target.
//!
//! The walk follows links (`walkdir`'s `follow_links(true)`), so a directory
//! link inside `dst` is reported as the directory it points at, and its target's
//! children appear under the link's relative path. That is the documented
//! comparison behaviour, but it makes the *deletion* side dangerous: joining a
//! relative path under the link and calling `remove_file` resolves the
//! intermediate components through the link, so `DELETE`/`RMDIR` would remove
//! files in the target - possibly outside `dst`. The two helpers here are the
//! only place that decides link-ness and the only place that unlinks one, so
//! the planner, the walk and the apply phases cannot disagree about either.
//!
//! ## Junctions
//!
//! On Windows a directory junction is a reparse point that
//! [`std::fs::FileType::is_symlink`] reports as `false` while
//! [`std::fs::FileType::is_dir`] reports `true`. `symlink_metadata` therefore
//! does not identify it, and a check written against `is_symlink()` alone would
//! leave the F-1 deletion path open for junctions. [`is_link`] looks at the
//! reparse-point attribute as well.
//!
//! Removal has to match: `DeleteFileW` (Rust's `remove_file`) rejects a
//! directory entry, and `RemoveDirectoryW` (`remove_dir`) is not what removes a
//! file link. [`remove`] asks the link's target which one applies, and falls
//! back to trying both for a dangling link.

use std::io;
use std::path::Path;

/// Windows' `FILE_ATTRIBUTE_REPARSE_POINT`; carried by junctions and by
/// symbolic links, but not reported by `FileType::is_symlink` for the former.
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// True when `path` itself is a link or a junction, whichever the platform
/// calls it. Never follows the final component.
pub fn is_link(path: &Path) -> bool {
    path.is_symlink() || is_reparse_dir(path)
}

/// True when `path` is a **directory** carrying a reparse point that
/// `FileType::is_symlink` does not report - in practice a Windows junction.
///
/// Separate from [`is_link`] because the walk already knows whether an entry is
/// a symlink (`walkdir::DirEntry::path_is_symlink`) and only needs the extra
/// stat for directories on Windows. Returns `false` everywhere else, so the
/// walk pays nothing for a check that cannot succeed.
pub fn is_reparse_dir(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        std::fs::symlink_metadata(path)
            .map(|md| {
                md.file_type().is_dir() && md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            })
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
///   has no target to ask, so the file removal is tried first and the directory
///   one is the fallback.
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
        let target_is_dir = std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false);
        if target_is_dir {
            return std::fs::remove_dir(path);
        }
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            // `ERROR_ACCESS_DENIED` from `DeleteFileW` on a directory entry is
            // the documented rejection; retry as a directory.
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => std::fs::remove_dir(path),
            Err(e) => Err(e),
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        std::fs::remove_file(path)
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
pub fn topmost_link_ancestor(root: &Path, rel: &str) -> Option<String> {
    let mut end = 0;
    loop {
        let (prefix, next) = match rel[end..].find('/') {
            Some(i) => (&rel[..end + i], Some(end + i + 1)),
            None => (rel, None),
        };
        if is_link(&root.join(prefix)) {
            return Some(prefix.to_string());
        }
        match next {
            Some(end_at) => end = end_at,
            None => return None,
        }
    }
}
