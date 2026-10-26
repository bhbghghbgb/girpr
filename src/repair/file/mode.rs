//! File-open helpers for the in-place repair path.
//!
//! `open_inplace` is `pub(super)` so `inplace.rs` (a sibling under `file`)
//! can call it via `super::mode::open_inplace`; `open_rw`/`clear_readonly`
//! stay private (used only within mode.rs).

use std::fs::File;

use anyhow::{Context, Result};

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
            clear_readonly(path)?;
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

/// Clear the read-only attribute of a file.
///
/// On Windows this is the attribute that stops a write; on Unix it is the
/// write bit, and clearing it is what makes the file writable by its owner
/// — which it has to be for the repair to happen at all.
fn clear_readonly(path: &std::path::Path) -> Result<()> {
    let mut perms = std::fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .permissions();
    if !perms.readonly() {
        return Ok(());
    }
    // The lint is about the blanket "make it writable" reflex. This is the
    // opposite: the file is *already* read-only, and the repair has to
    // write to it, so the attribute is cleared and nothing else is touched.
    #[allow(clippy::permissions_set_readonly_false)]
    {
        perms.set_readonly(false);
    }
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("clear the read-only attribute of {}", path.display()))?;
    tracing::debug!(path = %path.display(), "read-only attribute cleared");
    Ok(())
}
