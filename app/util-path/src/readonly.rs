//! Clear the read-only bit on exactly one path.
//!
//! The stat → readonly check → clear → set_permissions sequence is shared by
//! the fetch publisher and the two repair writers. This leaf owns only that
//! mutation and reports what it did as [`ReadOnlyOutcome`]. Logging, fallbacks
//! and failure fatality stay at the call sites: a publisher that can use a
//! temp sibling warns and continues, while an in-place writer propagates.

use std::cell::Cell;
use std::path::Path;

thread_local! {
    /// Injected permission-write failure. Production never sets this: a test
    /// installs it for the life of a [`SetPermissionsFailureGuard`]. Thread
    /// locality keeps parallel tests from seeing one another's override.
    static SET_PERMISSIONS_FAILURE: Cell<bool> = const { Cell::new(false) };
}

/// A scoped override that makes [`clear_readonly`] report [`ReadOnlyOutcome::SetFailed`].
///
/// The real `std::fs::set_permissions` call is skipped only while this guard is
/// alive; dropping it restores the previous state, so guards may nest and a
/// panicking test cannot leak the override into another one.
#[doc(hidden)]
#[must_use = "the injected failure is removed when the guard is dropped"]
pub struct SetPermissionsFailureGuard {
    previous: bool,
}

impl Drop for SetPermissionsFailureGuard {
    fn drop(&mut self) {
        SET_PERMISSIONS_FAILURE.with(|slot| slot.set(self.previous));
    }
}

/// Force the permission write of [`clear_readonly`] to fail on this thread.
///
/// A test seam for the `SetFailed` outcome: no host ACL behaviour is consulted
/// and no permission is actually changed. Production never installs one.
#[doc(hidden)]
pub fn inject_set_permissions_failure() -> SetPermissionsFailureGuard {
    let previous = SET_PERMISSIONS_FAILURE.with(|slot| slot.replace(true));
    SetPermissionsFailureGuard { previous }
}

/// The one permission write the leaf performs. An injected failure short-circuits
/// it with a typed I/O error; otherwise the real host call is made unchanged.
fn apply_set_permissions(path: &Path, permissions: std::fs::Permissions) -> std::io::Result<()> {
    if SET_PERMISSIONS_FAILURE.with(Cell::get) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected set_permissions failure",
        ));
    }
    std::fs::set_permissions(path, permissions)
}

/// What clearing the read-only bit on one path did.
#[derive(Debug)]
pub enum ReadOnlyOutcome {
    /// `metadata` failed, so no permission change was attempted.
    StatFailed(std::io::Error),
    /// The file was not read-only; nothing was changed.
    NotReadOnly,
    /// The read-only bit was cleared and the new permissions were applied.
    Cleared,
    /// The file was read-only, but applying the cleared permissions failed.
    SetFailed(std::io::Error),
}

/// Clear the read-only bit on `path` if it has one, and report the outcome.
///
/// Only the file named by `path` is touched: there is no tree-wide walk and no
/// change to any other file's permissions. The new permissions are the ones
/// read from this file with just the read-only bit cleared, so no mode bit
/// beyond that attribute is altered.
pub fn clear_readonly(path: &Path) -> ReadOnlyOutcome {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) => return ReadOnlyOutcome::StatFailed(e),
    };
    let mut permissions = meta.permissions();
    if !permissions.readonly() {
        return ReadOnlyOutcome::NotReadOnly;
    }
    // The lint guards against a blanket "make it writable" reflex. This is the
    // opposite: the file is already read-only and its one attribute is being
    // cleared so the caller can write to it; nothing else is modified.
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    match apply_set_permissions(path, permissions) {
        Ok(()) => ReadOnlyOutcome::Cleared,
        Err(e) => ReadOnlyOutcome::SetFailed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use girpr_tempdir::TempDir;

    fn dir(name: &str) -> TempDir {
        TempDir::new("girpr_test_readonly", name)
    }

    fn make_file(dir: &TempDir, name: &str, read_only: bool) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"game bytes").unwrap();
        if read_only {
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(&path, permissions).unwrap();
        }
        path
    }

    /// A read-only target is cleared on that one path, and the outcome says so.
    #[test]
    fn clears_the_read_only_bit_on_one_file() {
        let d = dir("clears");
        let path = make_file(&d, "target.blk", true);
        assert!(std::fs::metadata(&path).unwrap().permissions().readonly());

        let outcome = clear_readonly(&path);

        assert!(matches!(outcome, ReadOnlyOutcome::Cleared), "{outcome:?}");
        assert!(!std::fs::metadata(&path).unwrap().permissions().readonly());
        assert_eq!(std::fs::read(&path).unwrap(), b"game bytes");
    }

    /// A writable file is left alone and reported as needing no change.
    #[test]
    fn a_writable_file_is_not_changed() {
        let d = dir("writable");
        let path = make_file(&d, "target.blk", false);
        let before = std::fs::metadata(&path).unwrap().permissions();

        let outcome = clear_readonly(&path);

        assert!(
            matches!(outcome, ReadOnlyOutcome::NotReadOnly),
            "{outcome:?}"
        );
        let after = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(before.readonly(), after.readonly());
        #[cfg(unix)]
        assert_eq!(before.mode(), after.mode());
        assert_eq!(std::fs::read(&path).unwrap(), b"game bytes");
    }

    /// A missing path is reported as a stat failure, not as a cleared file;
    /// only the caller decides whether that failure matters.
    #[test]
    fn a_missing_path_is_a_stat_failure() {
        let d = dir("missing");
        let path = d.join("absent.blk");

        let outcome = clear_readonly(&path);

        match outcome {
            ReadOnlyOutcome::StatFailed(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("expected a stat failure, got {other:?}"),
        }
        assert!(!path.exists(), "the leaf never creates a path");
    }

    /// The injected failure drives the `SetFailed` outcome without depending on
    /// a host ACL, leaves the file untouched, and is undone when the guard drops.
    #[test]
    fn an_injected_set_failure_is_reported_without_mutating_the_file() {
        let d = dir("injected_set_failure");
        let path = make_file(&d, "target.blk", true);

        let guard = inject_set_permissions_failure();
        match clear_readonly(&path) {
            ReadOnlyOutcome::SetFailed(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
            }
            other => panic!("expected an injected set failure, got {other:?}"),
        }
        assert!(
            std::fs::metadata(&path).unwrap().permissions().readonly(),
            "an injected failure changes no permission"
        );
        drop(guard);

        assert!(
            matches!(clear_readonly(&path), ReadOnlyOutcome::Cleared),
            "the guard restores production behaviour on drop"
        );
    }
}
