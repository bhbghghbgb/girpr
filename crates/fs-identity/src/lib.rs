//! The shared file-identity and link-count facts.
//!
//! girpr's single-writer lock and girsync's copy guard both need two facts
//! about a file: how many names it has, and which file it is. The platform
//! reads those facts the same way on both sides — `nlink` on Unix, the
//! information record behind `GetFileInformationByHandle` on Windows — but the
//! callers ask different questions with them. girpr's guard is fail-closed (a
//! platform that cannot answer leaves the file alone) and compares an open
//! handle with a path read *without* following a reparse point; girsync's guard
//! reports explicit I/O errors and uses a handle-backed identity that stays
//! valid after the name is taken away.
//!
//! This crate owns only the facts and their platform mechanics. It classifies
//! no symlink, decides no refusal and interprets no count: each caller keeps
//! its own error type, its own path-read policy and its own link-count guard.
//! Both platforms are reached through maintained crates, so there is no FFI
//! here and no `unsafe_code` exception.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::fs::File;
#[cfg(unix)]
use std::fs::Metadata;
use std::io;
use std::path::Path;

/// The identity of a file, backed by an open handle so it stays exact after
/// the file's name has been taken away.
///
/// On Unix and Windows this is `same-file`'s `Handle`, whose equality is the
/// platform's own pair — device and inode, or volume serial number and file
/// index. On a platform with no such primitive identities are inert and all
/// compare equal; link counts are inert there too.
#[cfg(any(unix, windows))]
pub type Identity = same_file::Handle;

/// The inert identity on a platform without file identity.
#[cfg(not(any(unix, windows)))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Identity;

/// The raw platform identity pair, comparable, for callers that control how
/// the path side of a comparison is opened.
///
/// It is the same pair [`Identity`] compares, without the held handle. A caller
/// that must read the path itself rather than its target uses this after its
/// own no-follow open or `symlink_metadata` call; the classification stays in
/// the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IdentityFacts {
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(windows)]
    volume: u64,
    #[cfg(windows)]
    index: u64,
}

/// The number of names the file at `path` has, following links.
///
/// Every failure is returned as an error rather than read as a count; a
/// missing path is `NotFound`. The caller decides what a count above one means.
#[cfg(unix)]
pub fn link_count_at(path: &Path) -> io::Result<u64> {
    use std::os::unix::fs::MetadataExt;

    Ok(std::fs::metadata(path)?.nlink())
}

/// The number of names the file at `path` has, following links.
///
/// `winapi-util` reaches `GetFileInformationByHandle` and opens directories
/// with backup semantics, so the count comes from the platform without FFI in
/// this crate.
#[cfg(windows)]
pub fn link_count_at(path: &Path) -> io::Result<u64> {
    use winapi_util::Handle;

    let handle = Handle::from_path_any(path)?;
    Ok(winapi_util::file::information(&handle)?.number_of_links())
}

/// No portable link metadata exists here, so the count is inert.
#[cfg(not(any(unix, windows)))]
pub fn link_count_at(_path: &Path) -> io::Result<u64> {
    Ok(1)
}

/// The number of names the file `file` names, or `None` when the platform
/// cannot report one.
///
/// `None` is not zero or one: it is the answer for a platform whose metadata
/// has no link count, so a caller that must not guess can stay fail-closed.
#[cfg(unix)]
pub fn link_count_of(file: &File) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;

    file.metadata().ok().map(|meta| meta.nlink())
}

#[cfg(windows)]
pub fn link_count_of(file: &File) -> Option<u64> {
    winapi_util::file::information(file)
        .ok()
        .map(|info| info.number_of_links())
}

#[cfg(not(any(unix, windows)))]
pub fn link_count_of(_file: &File) -> Option<u64> {
    None
}

/// The identity of the file at `path`, following links.
#[cfg(any(unix, windows))]
pub fn identity_at(path: &Path) -> io::Result<Identity> {
    Identity::from_path(path)
}

#[cfg(not(any(unix, windows)))]
pub fn identity_at(_path: &Path) -> io::Result<Identity> {
    Ok(Identity)
}

/// The identity of the file a handle names.
///
/// `same-file` takes ownership of the `File`, so the caller's handle is
/// duplicated and kept; the returned identity holds a handle of its own for as
/// long as it is held.
#[cfg(any(unix, windows))]
pub fn identity_of(file: &File) -> io::Result<Identity> {
    Identity::from_file(file.try_clone()?)
}

#[cfg(not(any(unix, windows)))]
pub fn identity_of(_file: &File) -> io::Result<Identity> {
    Ok(Identity)
}

/// The raw identity facts of the file a handle names, or `None` when the
/// platform cannot report them.
#[cfg(unix)]
pub fn facts_of(file: &File) -> Option<IdentityFacts> {
    use std::os::unix::fs::MetadataExt;

    let meta = file.metadata().ok()?;
    Some(IdentityFacts {
        dev: meta.dev(),
        ino: meta.ino(),
    })
}

#[cfg(windows)]
pub fn facts_of(file: &File) -> Option<IdentityFacts> {
    let info = winapi_util::file::information(file).ok()?;
    Some(IdentityFacts {
        volume: info.volume_serial_number(),
        index: info.file_index(),
    })
}

#[cfg(not(any(unix, windows)))]
pub fn facts_of(_file: &File) -> Option<IdentityFacts> {
    None
}

/// The raw identity facts of metadata already read from a path or handle, or
/// `None` when the platform cannot report them.
///
/// Unix only: it exists for the no-follow `symlink_metadata` read a caller can
/// make without opening a file. Windows reports volume serial and file index
/// only through a handle, so callers there use [`facts_of`] instead.
#[cfg(unix)]
pub fn facts_of_metadata(meta: &Metadata) -> Option<IdentityFacts> {
    use std::os::unix::fs::MetadataExt;

    Some(IdentityFacts {
        dev: meta.dev(),
        ino: meta.ino(),
    })
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    /// A private scratch directory. The leaf has no dependency on any program
    /// (including its test helper), so tests make their own unique directory.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "girpr-fs-identity-{tag}-{}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("create scratch directory");
            Self(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_plain_file_has_one_link_and_one_identity() {
        let t = Scratch::new("one");
        let p = t.join("a.txt");
        std::fs::write(&p, b"x").unwrap();
        let file = File::open(&p).unwrap();

        assert_eq!(link_count_at(&p).unwrap(), 1);
        assert_eq!(link_count_of(&file), Some(1));
        assert_eq!(identity_at(&p).unwrap(), identity_of(&file).unwrap());
        let again = File::open(&p).unwrap();
        assert_eq!(facts_of(&file).unwrap(), facts_of(&again).unwrap());
        #[cfg(unix)]
        assert_eq!(
            facts_of(&file).unwrap(),
            facts_of_metadata(&file.metadata().unwrap()).unwrap()
        );
    }

    #[test]
    fn two_names_share_the_count_and_the_facts() {
        let t = Scratch::new("two");
        let a = t.join("a.txt");
        let b = t.join("b.txt");
        std::fs::write(&a, b"shared").unwrap();
        std::fs::hard_link(&a, &b).unwrap();

        assert_eq!(link_count_at(&a).unwrap(), 2);
        assert_eq!(link_count_at(&b).unwrap(), 2);
        let fa = File::open(&a).unwrap();
        let fb = File::open(&b).unwrap();
        assert_eq!(facts_of(&fa).unwrap(), facts_of(&fb).unwrap());
        assert_eq!(identity_at(&a).unwrap(), identity_at(&b).unwrap());
    }

    #[test]
    fn separate_files_have_distinct_facts() {
        let t = Scratch::new("distinct");
        let a = t.join("a.txt");
        let b = t.join("b.txt");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();

        let fa = File::open(&a).unwrap();
        let fb = File::open(&b).unwrap();
        assert_ne!(facts_of(&fa).unwrap(), facts_of(&fb).unwrap());
        assert_ne!(identity_at(&a).unwrap(), identity_at(&b).unwrap());
    }

    #[test]
    fn a_missing_path_is_not_found() {
        let t = Scratch::new("missing");
        let err = link_count_at(&t.join("gone")).expect_err("there is no count to take");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
