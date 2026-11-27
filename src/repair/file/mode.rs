//! File-open helpers for the in-place repair path.
//!
//! `open_inplace` is `pub(super)` so `inplace.rs` (a sibling under `file`)
//! can call it via `super::mode::open_inplace`; `open_rw` stays private (used
//! only within mode.rs). The read-only clearing itself comes from the shared
//! fs leaf; this module keeps the in-place path's propagate policy and wording.

use std::fs::File;

use anyhow::{Context, Result};
use girpr_util_path::readonly::{ReadOnlyOutcome, clear_readonly};

use super::FileCtx;

/// Open the target for the in-place repair.
///
/// `read+write`, created when it is not there. `None` means "the file is not
/// there and the run may not create it" — a dry run, which changes nothing.
///
/// A file that cannot be opened for writing is **not** a failure: the temp
/// path does not need write access to the target, so the caller falls back
/// to it. The usual reason a game file cannot be opened for writing is the
/// read-only attribute, and that is cleared first — Starward does the same
/// before it writes (GameInstallService.cs:232-236).
pub(super) fn open_inplace(ctx: &FileCtx, path: &std::path::Path) -> Result<Option<File>> {
    if ctx.dry_run {
        // A dry run changes nothing: no file is created and no attribute is
        // cleared. Reading is all it needs.
        if !path.exists() {
            return Ok(None);
        }
        let f = File::open(path).with_context(|| format!("open {}", path.display()))?;
        return Ok(Some(f));
    }
    match open_rw(path) {
        Ok(f) => Ok(Some(f)),
        Err(first) => {
            tracing::debug!(
                path = %path.display(),
                error = %first,
                "in-place open failed -> clearing the read-only attribute"
            );
            match clear_readonly(path) {
                ReadOnlyOutcome::Cleared => {
                    tracing::debug!(path = %path.display(), "read-only attribute cleared");
                }
                ReadOnlyOutcome::NotReadOnly => {}
                // Unlike the publisher and the prerequisite cleanup, the
                // in-place path cannot proceed without this one attribute:
                // both a failed stat and a failed set are fatal here.
                ReadOnlyOutcome::StatFailed(e) => {
                    return Err(e).with_context(|| format!("stat {}", path.display()));
                }
                ReadOnlyOutcome::SetFailed(e) => {
                    return Err(e).with_context(|| {
                        format!("clear the read-only attribute of {}", path.display())
                    });
                }
            }
            let f =
                open_rw(path).with_context(|| format!("open for writing {}", path.display()))?;
            Ok(Some(f))
        }
    }
}

/// Open a file for the in-place repair: read + write, created when absent,
/// never truncated.
pub(super) fn open_rw(path: &std::path::Path) -> std::io::Result<File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use girpr_repair_plan::{LocalChunkMap, RepairPlan};
    use girpr_repair_policy::RepairMode;

    use super::*;
    use crate::report::Summary;
    use crate::retry::BackoffPolicy;

    fn ctx() -> FileCtx {
        FileCtx {
            game_dir: std::path::PathBuf::new(),
            root: None,
            http: reqwest::Client::new(),
            plan: Arc::new(RepairPlan::from_files("", Vec::new())),
            local_map: Arc::new(LocalChunkMap::new()),
            scan: None,
            summary: Arc::new(Summary::new()),
            dry_run: false,
            repair_mode: RepairMode::Inplace,
            backoff: BackoffPolicy::none(),
        }
    }

    /// A forced permission-write failure is fatal on the in-place path, unlike
    /// the two warn-and-continue callers. The error wording stays local here.
    #[test]
    fn a_failed_read_only_clear_is_fatal_for_an_in_place_open() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_mode", "set_failed");
        let path = dir.join("game.dat");
        std::fs::write(&path, b"game bytes").unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();

        let guard = girpr_util_path::readonly::inject_set_permissions_failure();
        let err = open_inplace(&ctx(), &path).expect_err("a failed clear is fatal in place");
        drop(guard);

        let chain = format!("{err:#}");
        assert!(
            chain.contains("clear the read-only attribute of")
                && chain.contains("injected set_permissions failure"),
            "the propagate wording is unchanged: {chain}"
        );
    }
}
