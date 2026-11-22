//! Hard-link metadata, for the copy guard.
//!
//! A hard link is another name for the same inode/file record, not a link this
//! crate can see: it is not a reparse point, the walk reports it as an ordinary
//! file, and `File::create` truncates the shared content rather than replacing
//! one name. `copy_one` therefore asks for the destination's link count before
//! it truncates, and refuses a count above one. The plan-time counterpart
//! (`plan::ensure_hard_linked_targets_safe`) also needs to know *which* inode,
//! so it can tell a refused destination from one whose other name the run
//! removes first.
//!
//! The count and identity come from the shared fs-identity leaf: platform
//! `nlink`/handle information and a handle-backed `same-file` identity. This
//! module keeps girsync's own questions — the count of a path, whether it is a
//! regular file, and the error-returning shape the guards rely on — so the
//! runtime copy guard and the plan-time preflight still read one definition.

use std::fs::File;
use std::io;
use std::path::Path;

/// The identity of a file, for comparing two paths, or a path with a handle.
///
/// The leaf owns the platform pair and the held handle. Every consumer here
/// names this one type, so a cache name claim and a plan-time identity cannot
/// become two different notions of "the same file".
pub use girpr_fs_identity::Identity as FileId;

/// What the copy guard needs to know about one path: how many names share its
/// file, and which file that is.
#[derive(Debug)]
pub struct LinkInfo {
    /// The number of names the file at the path has, following links.
    ///
    /// Only meaningful when [`regular_file`](Self::regular_file) is true. On
    /// Unix a directory's `nlink` is at least 2 (`2 + its subdirectories`) and
    /// counts subdirectory names, not names of one shared file, so a caller
    /// asking whether a copy would truncate another name must not read it.
    pub links: u64,
    /// The file those names share.
    ///
    /// The leaf's `same-file` identity owns the file it names, so the identity
    /// cannot be freed and reused while it is held. Equality is the platform's
    /// own pair — device and inode on Unix, volume serial number and file index
    /// on Windows — and is never interpreted here.
    pub id: FileId,
    /// Whether the path names a regular file, following links.
    ///
    /// A copy truncates only a regular file. A directory at the destination is
    /// removed by the copy phase first, and its link count says nothing about
    /// a shared inode.
    pub regular_file: bool,
}

/// The link count and identity of the file at `path`, following links.
///
/// A missing path is `NotFound`, which the copy guard treats as "nothing to
/// truncate" and the preflight as "no name to protect". Every error is returned
/// rather than read as a count.
///
/// The result covers a directory too, but callers that are about to truncate
/// must test [`LinkInfo::regular_file`] first: a directory's `nlink` counts
/// subdirectories, not additional names for one file.
pub fn info(path: &Path) -> io::Result<LinkInfo> {
    Ok(LinkInfo {
        links: girpr_fs_identity::link_count_at(path)?,
        regular_file: std::fs::metadata(path)?.is_file(),
        id: girpr_fs_identity::identity_at(path)?,
    })
}

/// The number of names the file at `path` has, following links.
///
/// One definition for every caller: [`info`] and this function both ask the
/// leaf for the same count, so the runtime guard and the plan-time preflight
/// cannot disagree about what a name's count means. A missing path is
/// `NotFound`; any other error is returned rather than read as a count.
pub fn count(path: &Path) -> io::Result<u64> {
    girpr_fs_identity::link_count_at(path)
}

/// The identity of the file a handle names.
///
/// The same identity [`info`] reads from a path, read from the handle instead:
/// a handle names the file it was opened on even after the name has gone, so
/// this is what lets a caller ask "is the thing at this path still the file
/// this handle was opened on?". The leaf duplicates the `File` for the
/// identity, leaving the caller's own handle usable.
pub fn identity_of(file: &File) -> io::Result<FileId> {
    girpr_fs_identity::identity_of(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_file_has_one_link() {
        let t = girpr_tempdir::TempDir::new("girsync_hardlink", "one");
        let p = t.join("a.txt");
        std::fs::write(&p, b"x").unwrap();
        assert_eq!(count(&p).unwrap(), 1);
        assert!(info(&p).unwrap().regular_file);
    }

    /// A directory is not a truncated file, whatever its `nlink` says. On Unix
    /// a directory's count is `2 + subdirectories`, so reading it as a hard
    /// link would refuse every file-over-directory type conflict; on Windows it
    /// is 1, which is why the plan-time guard must ask this flag rather than
    /// the count alone.
    #[test]
    fn a_directory_is_not_a_regular_file() {
        let t = girpr_tempdir::TempDir::new("girsync_hardlink", "dir");
        let d = t.subdir("d");
        std::fs::create_dir_all(d.join("child")).unwrap();
        let info = info(&d).unwrap();
        assert!(!info.regular_file, "a directory is not a copy target");
    }

    /// The case `copy_one` refuses. `std::fs::hard_link` needs no privilege on
    /// this host, and the second name must be visible from either one.
    #[test]
    fn a_second_name_raises_the_count() {
        let t = girpr_tempdir::TempDir::new("girsync_hardlink", "two");
        let a = t.join("a.txt");
        let b = t.join("b.txt");
        std::fs::write(&a, b"shared").unwrap();
        std::fs::hard_link(&a, &b).unwrap();

        assert_eq!(count(&a).unwrap(), 2);
        assert_eq!(count(&b).unwrap(), 2);
    }

    /// The preflight's whole decision rests on "these two paths are one file":
    /// without a usable identity it cannot credit a name the run deletes first,
    /// and would refuse every hard-linked destination (including the ones a
    /// default run resolves).
    #[cfg(any(unix, windows))]
    #[test]
    fn a_second_name_shares_the_firsts_identity() {
        let t = girpr_tempdir::TempDir::new("girsync_hardlink", "same_id");
        let a = t.join("a.txt");
        let b = t.join("b.txt");
        std::fs::write(&a, b"shared").unwrap();
        std::fs::hard_link(&a, &b).unwrap();

        let (ai, bi) = (info(&a).unwrap(), info(&b).unwrap());
        assert_eq!(ai.links, 2);
        assert_eq!(bi.links, 2);
        assert_eq!(ai.id, bi.id, "two names, one file");
    }

    /// The other half: an identity that made every file equal would let the
    /// preflight credit an unrelated delete and allow the truncation.
    #[cfg(any(unix, windows))]
    #[test]
    fn separate_files_have_different_identities() {
        let t = girpr_tempdir::TempDir::new("girsync_hardlink", "diff_id");
        let a = t.join("a.txt");
        let b = t.join("b.txt");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();

        assert_ne!(info(&a).unwrap().id, info(&b).unwrap().id);
    }

    /// `identity_of` reads the same identity from an open handle as `info` does
    /// from the name, and does not consume the caller's handle.
    #[cfg(any(unix, windows))]
    #[test]
    fn a_handle_names_the_same_identity_as_the_path() {
        let t = girpr_tempdir::TempDir::new("girsync_hardlink", "handle_id");
        let a = t.join("a.txt");
        std::fs::write(&a, b"x").unwrap();

        let file = std::fs::File::open(&a).unwrap();
        let from_handle = identity_of(&file).unwrap();
        // The caller's handle is still usable after the identity was read.
        assert!(file.metadata().is_ok());
        assert_eq!(from_handle, info(&a).unwrap().id);
    }

    #[test]
    fn a_missing_path_is_not_found() {
        let t = girpr_tempdir::TempDir::new("girsync_hardlink", "missing");
        let err = count(&t.join("gone")).expect_err("there is no count to take");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
