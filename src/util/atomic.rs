//! One atomic-publish path for the files girpr writes.
//!
//! A writer creates a temp sibling in the target's directory, writes the whole
//! artifact there, and renames it over the target. A crash or a full disk can
//! therefore leave the temp or the old target, never a truncated artifact.
//! [`temp_sibling`](super::path::temp_sibling) owns the temp name, so every
//! writer and any leftover sweep share one rule.
//!
//! The helper takes bytes or a caller-supplied writer; serialization stays at
//! the call site, because each artifact has its own format and error context.
//! `durable` selects the flush policy explicitly: the config writer syncs the
//! file before publishing (its version bump must survive a power loss), while
//! the scan and plan artifacts only need the rename's all-or-nothing shape.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result};

use super::path::temp_sibling;

/// Publish `bytes` to `path` through a temp sibling.
///
/// `durable` syncs the temp file before the rename; see [`write_atomic_with`].
pub fn write_atomic(path: &Path, bytes: &[u8], durable: bool) -> Result<()> {
    write_atomic_with(path, durable, |writer| {
        writer.write_all(bytes).map_err(Into::into)
    })
}

/// Publish whatever `write` produces to `path` through a temp sibling.
///
/// The parent directory is created when the path names one. The temp is written
/// and flushed beside the target, then renamed over it. Any failure before the
/// rename removes the temp and returns the original error with the temp path in
/// its context; a rename failure removes the temp too.
///
/// With `durable`, the temp file is `sync_all`ed before the rename and the
/// parent directory is synced best-effort afterwards: a directory sync failure
/// is logged, not returned, because the publish itself already succeeded.
///
/// Bytes are buffered, so a caller that serializes incrementally does not pay a
/// syscall per fragment. The caller still owns the serialization and its error
/// context; this helper never inspects the bytes.
pub(crate) fn write_atomic_with(
    path: &Path,
    durable: bool,
    write: impl FnOnce(&mut dyn Write) -> Result<()>,
) -> Result<()> {
    publish(path, durable, |p| File::create(p), write)
}

/// The extra operation publishing needs from the temp file handle.
///
/// Split out so the durable decision can be pinned with a counting wrapper
/// instead of by inspecting the filesystem.
trait DurableFile: Write {
    fn sync(&mut self) -> io::Result<()>;
}

impl DurableFile for File {
    fn sync(&mut self) -> io::Result<()> {
        self.sync_all()
    }
}

/// The publish body, with creation and writing left to the caller.
fn publish<F>(
    path: &Path,
    durable: bool,
    create: impl FnOnce(&Path) -> io::Result<F>,
    write: impl FnOnce(&mut dyn Write) -> Result<()>,
) -> Result<()>
where
    F: DurableFile,
{
    // A bare file name has no directory component; there is nothing to create.
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir {}", parent.display()))?;
    }
    let tmp = temp_sibling(path);
    let published = (|| -> Result<()> {
        let mut file =
            create(&tmp).with_context(|| format!("create temp file {}", tmp.display()))?;
        {
            let mut writer = io::BufWriter::new(&mut file);
            write(&mut writer).with_context(|| format!("write temp file {}", tmp.display()))?;
            writer
                .flush()
                .with_context(|| format!("flush temp file {}", tmp.display()))?;
        }
        if durable {
            // Publishing a value whose write-back is still only in the page
            // cache is the failure this sync exists to prevent, so its error is
            // fatal and the temp is cleaned up like any other write failure.
            file.sync()
                .with_context(|| format!("sync temp file {}", tmp.display()))?;
        }
        // Close before the rename: Windows cannot replace a file that is open.
        drop(file);
        std::fs::rename(&tmp, path).with_context(|| format!("publish {}", path.display()))?;
        Ok(())
    })();
    if let Err(e) = published {
        if let Err(cleanup) = std::fs::remove_file(&tmp)
            && cleanup.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %tmp.display(),
                error = %cleanup,
                "could not remove the temp file after a failed publish"
            );
        }
        return Err(e);
    }
    if durable {
        sync_parent(path);
    }
    Ok(())
}

/// Sync the published file's directory, best-effort.
///
/// The rename is already visible; this only makes its directory entry durable.
/// A failure is logged rather than returned, so a host that refuses to open or
/// sync a directory cannot turn a successful publish into an error.
#[cfg(unix)]
fn sync_parent(path: &Path) {
    let dir = match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(parent) => parent,
        None => Path::new("."),
    };
    match File::open(dir).and_then(|d| d.sync_all()) {
        Ok(()) => {}
        Err(e) => tracing::warn!(
            path = %dir.display(),
            error = %e,
            "could not sync the directory after publishing"
        ),
    }
}

/// Windows has no directory handle to fsync through `File`; the rename already
/// carries the entry and NTFS journals it.
#[cfg(not(unix))]
fn sync_parent(_path: &Path) {}
