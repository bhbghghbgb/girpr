//! Hard-link metadata, for the copy guard.
//!
//! A hard link is another name for the same inode/file record, not a link this
//! crate can see: it is not a reparse point, the walk reports it as an ordinary
//! file, and `File::create` truncates the shared content rather than replacing
//! one name. `copy_one` therefore asks for the destination's link count before
//! it truncates, and refuses a count above one.
//!
//! Unix exposes the count as `MetadataExt::nlink`. Windows does not — the
//! `number_of_links` accessor is still behind `windows_by_handle` on rustc
//! 1.98.1 — so it calls `GetFileInformationByHandle`, which is also the API
//! `nlink` is derived from there.

use std::io;
use std::path::Path;

/// The number of names the file at `path` has, following links.
///
/// A missing path is `NotFound`, which the copy guard treats as "nothing to
/// truncate". A directory is not expected here: the caller has already resolved
/// a blocking directory, and the platform results below are meaningful only for
/// files. Every error is returned rather than read as a count.
#[cfg(unix)]
pub fn count(path: &Path) -> io::Result<u64> {
    use std::os::unix::fs::MetadataExt;

    Ok(std::fs::metadata(path)?.nlink())
}

/// The number of names the file at `path` has, following links.
///
/// `CreateFileW` asks only for `FILE_READ_ATTRIBUTES`, so a destination the
/// process may write but not read is still measurable, and the share mode lets
/// another process keep it open. The handle is closed on every path after the
/// information call, including failure.
#[cfg(windows)]
pub fn count(path: &Path) -> io::Result<u64> {
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
        None => Ok(info.nNumberOfLinks as u64),
    }
}

/// No portable link count exists on this platform, so the guard is inert.
#[cfg(not(any(unix, windows)))]
pub fn count(_path: &Path) -> io::Result<u64> {
    Ok(1)
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

    #[test]
    fn a_missing_path_is_not_found() {
        let t = girpr_tempdir::TempDir::new("girsync_hardlink", "missing");
        let err = count(&t.join("gone")).expect_err("there is no count to take");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
