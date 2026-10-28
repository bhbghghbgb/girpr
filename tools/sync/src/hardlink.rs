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
//! Unix exposes both answers as `MetadataExt::{nlink,dev,ino}`. Windows does not
//! — `number_of_links` is still behind `windows_by_handle` on rustc 1.98.1 — so
//! it calls `GetFileInformationByHandle`, which is also the API those accessors
//! are derived from there.

use std::io;
use std::path::Path;

/// What the copy guard needs to know about one path: how many names share its
/// file, and which file that is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkInfo {
    /// The number of names the file at the path has, following links.
    pub links: u64,
    /// The file those names share.
    pub id: Identity,
}

/// The identity of a file, for comparing two paths.
///
/// Only equality is meaningful; the two words are the platform's own pair — the
/// device and inode on Unix, the volume serial number and file index on Windows
/// — and are never interpreted here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Identity {
    a: u64,
    b: u64,
}

impl Identity {
    fn new(a: u64, b: u64) -> Self {
        Self { a, b }
    }
}

/// The link count and identity of the file at `path`, following links.
///
/// A missing path is `NotFound`, which the copy guard treats as "nothing to
/// truncate" and the preflight as "no name to protect". A directory is not
/// expected here: the caller has already resolved a blocking directory, and the
/// platform results below are meaningful only for files. Every error is
/// returned rather than read as a count.
#[cfg(unix)]
pub fn info(path: &Path) -> io::Result<LinkInfo> {
    use std::os::unix::fs::MetadataExt;

    let md = std::fs::metadata(path)?;
    Ok(LinkInfo {
        links: md.nlink(),
        id: Identity::new(md.dev(), md.ino()),
    })
}

/// The link count and identity of the file at `path`, following links.
///
/// `CreateFileW` asks only for `FILE_READ_ATTRIBUTES`, so a destination the
/// process may write but not read is still measurable, and the share mode lets
/// another process keep it open. The handle is closed on every path after the
/// information call, including failure.
#[cfg(windows)]
pub fn info(path: &Path) -> io::Result<LinkInfo> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
        OPEN_EXISTING,
    };

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call, the
    // security and template arguments are the documented null defaults, and the
    // returned handle is closed below on both the success and the failure path.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle is valid and `info` is a live, correctly typed out
    // parameter for the duration of the call.
    let ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
    let err = (ok == 0).then(io::Error::last_os_error);
    // SAFETY: `handle` came from `CreateFileW` and is closed exactly once.
    unsafe {
        CloseHandle(handle);
    }
    match err {
        Some(e) => Err(e),
        None => Ok(LinkInfo {
            links: info.nNumberOfLinks as u64,
            // The two halves are one 64-bit file index, paired with the volume
            // that owns it: the stable Windows spelling of (dev, ino).
            id: Identity::new(
                info.dwVolumeSerialNumber as u64,
                ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
            ),
        }),
    }
}

/// No portable link metadata exists on this platform, so the guard is inert.
#[cfg(not(any(unix, windows)))]
pub fn info(_path: &Path) -> io::Result<LinkInfo> {
    Ok(LinkInfo {
        links: 1,
        id: Identity::new(0, 0),
    })
}

/// The number of names the file at `path` has, following links.
///
/// One definition for every caller: [`info`] is the only place the platform's
/// metadata is read, so the runtime guard and the plan-time preflight cannot
/// disagree about a count.
pub fn count(path: &Path) -> io::Result<u64> {
    info(path).map(|i| i.links)
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

    #[test]
    fn a_missing_path_is_not_found() {
        let t = girpr_tempdir::TempDir::new("girsync_hardlink", "missing");
        let err = count(&t.join("gone")).expect_err("there is no count to take");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
