//! The cache's cross-process run marker: a lock **sidecar**, not the cache file.
//!
//! A run holds an exclusive OS lock on a small file beside its cache for as
//! long as the cache is open. That lock is what says "a run is in its run
//! phase for this folder": another run that wants the cache takes the same
//! lock non-blockingly and is refused at once when it is held, and a bound
//! that finds the lock held leaves the folder to the run that holds it.
//!
//! The lock lives on its own file rather than on `girpr-cache` itself. redb
//! 4.3 and later lock a database with byte ranges and — on Windows — take a
//! *blocking* lock on a reserved byte before retrying a byte-range lock that
//! failed. A whole-file lock on the database file, which is what the cache's
//! marker used to be, covers that byte: a run that met another holder of it
//! blocked in redb's open instead of being refused. A sidecar that only this
//! crate locks keeps the fail-fast behaviour independent of redb's internals.
//!
//! The sidecar's name carries the tool's prefix, so it is tool state: it is
//! never mirrored as content and, like the cache itself, is classified as a
//! record when named directly. It is empty — the lock is the whole of what it
//! says, and an empty file leaves nothing to go stale. The OS drops the lock
//! when the process ends, including when it is killed, so a sidecar left by a
//! dead run does not claim the cache for good.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

/// Suffix that turns a cache's file name into its lock sidecar's name.
const LOCK_SUFFIX: &str = "-lock";

/// The path of the lock sidecar for the cache at `db_path`.
pub(crate) fn path_for(db_path: &Path) -> PathBuf {
    let mut name = db_path.file_name().unwrap_or_default().to_os_string();
    name.push(LOCK_SUFFIX);
    db_path.with_file_name(name)
}

/// What a non-blocking attempt to take a cache's sidecar lock found.
pub(super) enum TryLock {
    /// The lock is ours, held for as long as the guard is alive.
    Held(File),
    /// Another run holds the lock: the cache is busy.
    Taken,
    /// This filesystem cannot lock a file, so there is no marker to take.
    Unsupported,
}

/// Try to take the sidecar lock for `db_path`, exclusively unless `shared`.
///
/// The sidecar is created if absent. `WouldBlock` means another run holds it;
/// an unsupported lock means the filesystem has no marker to speak of, in
/// which case the caller proceeds without one — the same degradation a copy
/// record has on such a filesystem.
pub(super) fn acquire(db_path: &Path, shared: bool) -> io::Result<TryLock> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path_for(db_path))?;
    match if shared {
        file.try_lock_shared()
    } else {
        file.try_lock()
    } {
        Ok(()) => Ok(TryLock::Held(file)),
        Err(TryLockError::WouldBlock) => Ok(TryLock::Taken),
        Err(TryLockError::Error(e)) if e.kind() == io::ErrorKind::Unsupported => {
            Ok(TryLock::Unsupported)
        }
        Err(TryLockError::Error(e)) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use girpr_tempdir::TempDir;

    #[test]
    fn the_sidecar_is_named_after_the_cache_in_the_tool_namespace() {
        assert_eq!(
            path_for(Path::new("root/sub/girpr-cache")),
            PathBuf::from("root/sub/girpr-cache-lock")
        );
    }

    #[test]
    fn a_second_exclusive_acquire_is_refused_while_the_first_is_held() {
        let t = TempDir::new("girsync_cache", "sidecar");
        let db = t.subdir("w").join("girpr-cache");
        let first = acquire(&db, false).unwrap();
        assert!(matches!(first, TryLock::Held(_)), "the first acquire holds");
        assert!(
            matches!(acquire(&db, false).unwrap(), TryLock::Taken),
            "a second exclusive acquire is refused"
        );
        assert!(
            matches!(acquire(&db, true).unwrap(), TryLock::Taken),
            "a shared acquire is refused by an exclusive holder"
        );
    }

    #[test]
    fn shared_acquires_coexist_and_a_later_exclusive_one_is_refused() {
        let t = TempDir::new("girsync_cache", "sidecar_shared");
        let db = t.subdir("w").join("girpr-cache");
        let first = acquire(&db, true).unwrap();
        let second = acquire(&db, true).unwrap();
        assert!(matches!(first, TryLock::Held(_)));
        assert!(matches!(second, TryLock::Held(_)));
        assert!(matches!(acquire(&db, false).unwrap(), TryLock::Taken));
    }

    #[test]
    fn the_lock_is_released_when_its_guard_is_dropped() {
        let t = TempDir::new("girsync_cache", "sidecar_drop");
        let db = t.subdir("w").join("girpr-cache");
        drop(acquire(&db, false).unwrap());
        assert!(matches!(acquire(&db, false).unwrap(), TryLock::Held(_)));
    }
}
