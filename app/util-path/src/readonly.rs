//! Clear the read-only bit on exactly one path.
//!
//! The stat → readonly check → clear → set_permissions sequence is shared by
//! the fetch publisher and the two repair writers. This leaf owns only that
//! mutation and reports what it did as [`ReadOnlyOutcome`]. Logging, fallbacks
//! and failure fatality stay at the call sites: a publisher that can use a
//! temp sibling warns and continues, while an in-place writer propagates.

use std::path::Path;

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
    match std::fs::set_permissions(path, permissions) {
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

        assert!(matches!(outcome, ReadOnlyOutcome::NotReadOnly), "{outcome:?}");
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
}
