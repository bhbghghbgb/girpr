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

use super::path::{has_windows_trailing_dot_or_space, temp_sibling};

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
/// A `path` that does not end in a usable file name is refused before the parent
/// is created or a temp is named: a spelling like `dir/.` or `dir/` names the
/// directory rather than a file, and a Windows-trailing dot or space
/// (`config.`, `config `) aliases another name. There is no location a file can
/// be published to under those spellings, and the temp helper's fallback would
/// put the temp under the path's directories instead. The trailing dot/space
/// rule is [`has_windows_trailing_dot_or_space`], shared with
/// [`super::path::contained_rel`] and applied on every host.
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

/// Does `path` end in a file name rather than a directory spelling?
///
/// [`Path::file_name`] ignores a trailing `.`, so it reports `Some("dir")` for
/// `dir/.` even though that spelling names the directory, not a file. Look at
/// the raw spelling instead: an empty tail (a trailing separator, a root or an
/// empty path) or a `.` tail names no file, while a real name does. A name that
/// ends in a dot or space is a name here; the separate Windows alias rule in
/// [`publish`] refuses it.
fn ends_in_a_file_name(path: &Path) -> bool {
    if path.file_name().is_none() {
        return false;
    }
    let raw = path.as_os_str().to_string_lossy();
    let tail = raw
        .rsplit(std::path::is_separator)
        .next()
        .unwrap_or_default();
    !tail.is_empty() && tail != "."
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
    // A target that does not end in a file name cannot be replaced by one.
    // `file_name()` alone is not enough: it ignores a trailing `.`, so `dir/.`
    // reports `Some("dir")` even though the spelling names the directory. The
    // temp helper still names a fallback under the target's directories (a
    // trailing `..` survives), so the write would only fail at the rename,
    // after the parent was created and the temp was written there. Refuse up
    // front so the caller gets the reason instead of a rename error from
    // another directory.
    if !ends_in_a_file_name(path) {
        anyhow::bail!("publish target has no file name: {}", path.display());
    }
    // Windows strips a trailing dot or space when it resolves a name, so
    // `config.` and `config ` would publish over `config`. The rule is shared
    // with `contained_rel` and lexical, so it applies on every host: the
    // install is a Windows tree even when girpr runs elsewhere.
    if let Some(name) = path.file_name()
        && has_windows_trailing_dot_or_space(&name.to_string_lossy())
    {
        anyhow::bail!(
            "publish target has a Windows-trailing dot or space: {}",
            path.display()
        );
    }
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

    /// A target that names no file has nothing to publish under. The refusal
    /// comes before the parent is created or a temp is named, rather than after
    /// writing at the helper's fallback location and failing at the rename.
    #[test]
    fn a_target_with_no_file_name_is_refused_before_anything_is_written() {
        let d = dir("no_file_name");
        let path = d.join("missing").join("..");
        assert!(path.file_name().is_none(), "the fixture names no file");

        let err = write_atomic(&path, b"{}", false).unwrap_err();

        assert!(
            format!("{err:#}").contains("publish target has no file name"),
            "{err:#}"
        );
        assert!(
            !d.join("missing").exists(),
            "the refusal happens before the parent directory is created"
        );
        // The helper's fallback resolves here once `missing` exists, so the
        // unresolved `missing/../.girpr_tmp` cannot be probed: it is absent
        // whether or not a temp was written. Assert the resolved location.
        assert!(
            !d.join(".girpr_tmp").exists(),
            "no temp is written at the fallback location"
        );
    }

    /// `Path::file_name()` ignores a trailing `.`, so `dir/.` reports
    /// `Some("dir")` even though the spelling names the directory. The guard
    /// reads the spelling, so the target is refused before the parent is
    /// created or a temp is written, rather than at the rename.
    #[test]
    fn a_directory_spelling_is_refused_before_anything_is_written() {
        let d = dir("directory_spelling");
        // The parent does not exist, so creating it is observable on both
        // platforms, and the temp would land in it.
        let parent = d.join("missing");
        let target = parent.join("game");
        for path in [target.join("."), target.join("")] {
            assert_eq!(
                path.file_name(),
                Some(std::ffi::OsStr::new("game")),
                "the fixture exercises a spelling `file_name` cannot tell from a file"
            );

            let err = write_atomic(&path, b"{}", false).unwrap_err();

            assert!(
                format!("{err:#}").contains("publish target has no file name"),
                "{err:#}"
            );
            assert!(
                !parent.exists(),
                "the refusal happens before the parent directory is created"
            );
            assert!(
                !target.exists(),
                "the directory spelling is not published as a file"
            );
            assert!(
                !parent.join(".girpr_tmp").exists(),
                "no temp is written at the resolved fallback location"
            );
        }
    }

    /// Windows strips a trailing dot or space from a name, so `config.` or
    /// `config ` would alias `config`. The publisher refuses that spelling
    /// before the parent is created or a temp is written, on every host.
    #[test]
    fn a_windows_trailing_dot_or_space_is_refused_before_anything_is_written() {
        let d = dir("windows_alias");
        let parent = d.join("missing");
        for name in ["config.", "config "] {
            let path = parent.join(name);
            assert!(
                path.file_name().is_some(),
                "the fixture has a file name; only the alias rule refuses it"
            );

            let err = write_atomic(&path, b"{}", false).unwrap_err();

            assert!(
                format!("{err:#}").contains("Windows-trailing dot or space"),
                "{err:#}"
            );
            assert!(
                !parent.exists(),
                "the refusal happens before the parent directory is created"
            );
        }
    }

    /// A dot inside the name is not an alias: Windows strips only a trailing
    /// dot or space, so an ordinary name is still published.
    #[test]
    fn an_ordinary_file_name_is_still_published() {
        let d = dir("ordinary_name");
        let path = d.join("config.ini");
        write_atomic(&path, b"{}", false).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
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
