//! One atomic-publish path for the files girpr writes.
//!
//! A writer creates a temp sibling in the target's directory, writes the whole
//! artifact there, and renames it over the target. A crash or a full disk can
//! therefore leave the temp or the old target, never a truncated artifact.
//! `temp_sibling` owns the temp name, so every writer and any leftover sweep
//! share one rule.
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

#[cfg(test)]
mod tests {
    use super::*;
    use girpr_tempdir::TempDir;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn dir(name: &str) -> TempDir {
        TempDir::new("girpr_test_atomic", name)
    }

    /// A successful publish creates its parent, writes the bytes, and renames
    /// the temp away instead of leaving it beside the artifact.
    #[test]
    fn publishes_bytes_creates_the_parent_and_leaves_no_temp() {
        let d = dir("publish");
        let path = d.join("nested").join("plan.json");
        write_atomic(&path, b"{\"ok\":true}\n", false).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"ok\":true}\n");
        assert!(
            !temp_sibling(&path).exists(),
            "the temp is renamed away, not left beside the target"
        );
    }

    /// The temp name comes from the whole file name, so a multi-dot target
    /// gets `name.ext.girpr_tmp` rather than having an extension replaced.
    #[test]
    fn a_multi_dot_target_names_its_temp_after_the_whole_file() {
        let d = dir("multi_dot");
        let path = d.join("plan.v1.json");
        // A directory where the temp would go makes creation fail and names the
        // temp in the error, without a second naming rule in the test.
        let tmp = d.join("plan.v1.json.girpr_tmp");
        std::fs::create_dir(&tmp).unwrap();
        let err = write_atomic(&path, b"{}", false).unwrap_err();
        assert!(
            format!("{err:#}").contains("plan.v1.json.girpr_tmp"),
            "{err:#}"
        );
        assert!(!path.exists(), "no artifact is published on a failed write");
    }

    /// A failure while the bytes are being written removes the temp and leaves
    /// the previous artifact exactly as it was.
    #[test]
    fn a_write_failure_keeps_the_target_and_removes_the_temp() {
        let d = dir("write_failure");
        let path = d.join("config.ini");
        let previous = b"[General]\ngame_version=1.0.0\n";
        std::fs::write(&path, previous).unwrap();
        let err = write_atomic_with(&path, true, |writer| {
            writer.write_all(b"[General]\npartial")?;
            anyhow::bail!("injected write failure");
        })
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("injected write failure"),
            "{err:#}"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            previous,
            "the target keeps its old bytes"
        );
        assert!(
            !temp_sibling(&path).exists(),
            "a failed write must not leave the temp behind"
        );
    }

    /// A failing rename (a directory sits where the file should be) removes the
    /// temp and returns the rename error.
    #[test]
    fn a_rename_failure_removes_the_temp_and_returns_the_error() {
        let d = dir("rename_failure");
        let path = d.join("plan.json");
        std::fs::create_dir(&path).unwrap();
        let err = write_atomic(&path, b"{}", false).unwrap_err();
        assert!(format!("{err:#}").contains("publish"), "{err:#}");
        assert!(path.is_dir(), "the directory target is untouched");
        assert!(
            !temp_sibling(&path).exists(),
            "a failed rename must not leave the temp behind"
        );
    }

    /// The durable decision is explicit and observable: `true` syncs the temp
    /// before publishing, `false` never does.
    #[test]
    fn durable_publishing_syncs_the_temp_and_non_durable_does_not() {
        for durable in [false, true] {
            let d = dir("durable");
            let path = d.join("config.ini");
            let syncs = Arc::new(AtomicUsize::new(0));
            let counter = Arc::clone(&syncs);
            publish(
                &path,
                durable,
                |tmp| {
                    File::create(tmp).map(|file| CountingFile {
                        file,
                        syncs: Arc::clone(&counter),
                    })
                },
                |writer| writer.write_all(b"[General]\n").map_err(Into::into),
            )
            .unwrap();
            assert_eq!(
                syncs.load(Ordering::SeqCst),
                if durable { 1 } else { 0 },
                "durable={durable}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"[General]\n");
        }
    }

    /// A real file handle that counts the syncs publishing asks for.
    struct CountingFile {
        file: File,
        syncs: Arc<AtomicUsize>,
    }

    impl Write for CountingFile {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.file.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.file.flush()
        }
    }

    impl DurableFile for CountingFile {
        fn sync(&mut self) -> io::Result<()> {
            self.syncs.fetch_add(1, Ordering::SeqCst);
            self.file.sync_all()
        }
    }
}
