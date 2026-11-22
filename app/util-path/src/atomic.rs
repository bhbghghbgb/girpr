//! One atomic-publish path for the files girpr writes.
//!
//! A writer creates a temp file in the target's directory through
//! [`tempfile::NamedTempFile`], writes the whole artifact there, and publishes
//! it with `persist`, which atomically renames the temp over the target
//! (`MoveFileEx` + `MOVEFILE_REPLACE_EXISTING` on Windows). A crash or a full
//! disk can therefore leave the temp or the old target, never a truncated
//! artifact. The crate owns the temp name and its cleanup; writers never name
//! or sweep a temp themselves.
//!
//! The helper takes bytes or a caller-supplied writer; serialization stays at
//! the call site, because each artifact has its own format and error context.
//! `durable` selects the flush policy explicitly: the config writer syncs the
//! file before publishing (its version bump must survive a power loss), while
//! the scan and plan artifacts only need the rename's all-or-nothing shape.
//! `persist` does not create the parent and does not sync, so both the
//! `create_dir_all` and the sync remain here.

use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result};
use tempfile::NamedTempFile;

use super::path::any_component_has_windows_trailing_dot_or_space;

/// Publish `bytes` to `path` through a temp file in the target's directory.
///
/// `durable` syncs the temp file before the publish; see [`write_atomic_with`].
pub fn write_atomic(path: &Path, bytes: &[u8], durable: bool) -> Result<()> {
    write_atomic_with(path, durable, |writer| {
        writer.write_all(bytes).map_err(Into::into)
    })
}

/// Publish whatever `write` produces to `path` through a temp file in the
/// target's directory.
///
/// The parent directory is created when the path names one. The temp is written
/// and flushed beside the target, then `persist`ed over it. Any failure before
/// the publish drops the temp, which removes it, and returns the original error
/// with the target path in its context; a publish failure does the same.
///
/// A `path` that does not end in a usable file name is refused before the parent
/// is created or a temp is named: a spelling like `dir/.` or `dir/` names the
/// directory rather than a file. A Windows-trailing dot or space in **any**
/// component (`config.`, `config `, `dir./config.ini`) aliases another name:
/// Windows strips the character when it resolves each component, so a target
/// whose directory was spelled `dir.` publishes under the aliased `dir` while
/// the caller asked for something else. There is no location a file can be
/// published to under those spellings. The rule is
/// [`any_component_has_windows_trailing_dot_or_space`], built on the predicate
/// shared with [`super::path::contained_rel`] and applied on every host.
///
/// With `durable`, the temp file is `sync_all`ed before the publish and the
/// parent directory is synced best-effort afterwards: a directory sync failure
/// is logged, not returned, because the publish itself already succeeded.
///
/// Bytes are buffered, so a caller that serializes incrementally does not pay a
/// syscall per fragment. The caller still owns the serialization and its error
/// context; this helper never inspects the bytes.
pub fn write_atomic_with(
    path: &Path,
    durable: bool,
    write: impl FnOnce(&mut dyn Write) -> Result<()>,
) -> Result<()> {
    // A target that does not end in a file name cannot be replaced by one.
    // `file_name()` alone is not enough: it ignores a trailing `.`, so `dir/.`
    // reports `Some("dir")` even though the spelling names the directory.
    // Refuse up front, before the parent is created or a temp is written.
    if !ends_in_a_file_name(path) {
        anyhow::bail!("publish target has no file name: {}", path.display());
    }
    // Windows strips a trailing dot or space when it resolves a name, so
    // `config.` and `config ` would publish over `config`. The same holds for
    // every directory component. The rule is shared with `contained_rel` and
    // lexical, so it applies on every host. Prefixes, roots and `..` parents
    // are not checked: none of them names a file.
    if any_component_has_windows_trailing_dot_or_space(path) {
        anyhow::bail!(
            "publish target has a Windows-trailing dot or space: {}",
            path.display()
        );
    }
    // `persist` does not create the parent. A bare name has no directory
    // component, so the temp is created in the current directory instead.
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir {}", parent.display()))?;
    }
    let mut tmp = match parent {
        Some(parent) => NamedTempFile::new_in(parent),
        None => NamedTempFile::new_in("."),
    }
    .with_context(|| format!("create temp file for {}", path.display()))?;

    {
        let mut writer = io::BufWriter::new(&mut tmp);
        write(&mut writer).with_context(|| format!("write temp file for {}", path.display()))?;
        writer
            .flush()
            .with_context(|| format!("flush temp file for {}", path.display()))?;
    }
    if durable {
        // Publishing a value whose write-back is still only in the page cache
        // is the failure this sync exists to prevent, so its error is fatal.
        tmp.as_file()
            .sync_all()
            .with_context(|| format!("sync temp file for {}", path.display()))?;
    }
    // Dropping the `PersistError` drops the temp with it, so a failed publish
    // removes the temp just like a failed write does.
    tmp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("publish {}", path.display()))?;
    if durable {
        sync_parent(path);
    }
    Ok(())
}

/// Does `path` end in a file name rather than a directory spelling?
///
/// [`Path::file_name`] ignores a trailing `.`, so it reports `Some("dir")` for
/// `dir/.` even though that spelling names the directory, not a file. Look at
/// the raw spelling instead: an empty tail (a trailing separator, a root or an
/// empty path) or a `.` tail names no file, while a real name does. A name that
/// ends in a dot or space is a name here; the separate Windows alias rule in
/// [`write_atomic_with`] refuses it.
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
    match std::fs::File::open(dir).and_then(|d| d.sync_all()) {
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

    fn dir(name: &str) -> TempDir {
        TempDir::new("girpr_test_atomic", name)
    }

    /// The names left in a directory, so a test can assert that no temp file
    /// outlives a publish without knowing the temp name the crate picked.
    fn entries(path: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(path)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// A successful publish creates its parent, writes the bytes, and leaves
    /// nothing but the artifact in the parent directory.
    #[test]
    fn publishes_bytes_creates_the_parent_and_leaves_no_temp() {
        let d = dir("publish");
        let path = d.join("nested").join("plan.json");
        write_atomic(&path, b"{\"ok\":true}\n", false).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"ok\":true}\n");
        assert_eq!(
            entries(&d.join("nested")),
            ["plan.json"],
            "the temp is renamed away, not left beside the target"
        );
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
        assert_eq!(
            entries(&d),
            ["config.ini"],
            "a failed write must not leave the temp behind"
        );
    }

    /// The four "refused before anything is written" cases, folded into one
    /// process so nextest spawns one test instead of four. Every original
    /// assertion and message is kept; a labelled subcase names the refusal that
    /// failed.
    #[test]
    fn refusals_happen_before_anything_is_written() {
        let subcases: &[(&str, &dyn Fn())] = &[
            (
                "a_target_with_no_file_name_is_refused_before_anything_is_written",
                &|| {
                    // A target that names no file has nothing to publish under.
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
                    assert!(entries(&d).is_empty(), "no temp is written anywhere");
                },
            ),
            (
                "a_directory_spelling_is_refused_before_anything_is_written",
                &|| {
                    // `Path::file_name()` ignores a trailing `.`, so `dir/.`
                    // reports `Some("dir")` even though the spelling names the
                    // directory. The guard reads the spelling instead.
                    let d = dir("directory_spelling");
                    // The parent does not exist, so creating it is observable
                    // on both platforms, and the temp would land in it.
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
                        assert!(entries(&d).is_empty(), "no temp is written anywhere");
                    }
                },
            ),
            (
                "a_windows_trailing_dot_or_space_is_refused_before_anything_is_written",
                &|| {
                    // Windows strips a trailing dot or space, so `config.` or
                    // `config ` would alias `config`; every host refuses them.
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
                },
            ),
            (
                "a_nested_windows_alias_component_is_refused_before_anything_is_written",
                &|| {
                    // The alias rule also covers an **intermediate** directory
                    // spelled with a trailing dot or space. The refusal comes
                    // before the parent is created, so no alias is created.
                    let d = dir("windows_alias_nested");
                    for dir_name in ["dir.", "dir "] {
                        // Constructed lexically: such a directory is not
                        // created on any host, and the test never needs one.
                        let parent = d.join(dir_name).join("missing");
                        let path = parent.join("config.ini");
                        assert_eq!(
                            path.file_name(),
                            Some(std::ffi::OsStr::new("config.ini")),
                            "only an intermediate component is an alias"
                        );

                        let err = write_atomic(&path, b"{}", false).unwrap_err();

                        assert!(
                            format!("{err:#}").contains("Windows-trailing dot or space"),
                            "{err:#}"
                        );
                        assert!(
                            !parent.exists() && !d.join("dir").exists(),
                            "no directory is created (or aliased) for {dir_name:?}"
                        );
                    }
                },
            ),
        ];
        for (label, run) in subcases {
            println!("subcase: {label}");
            run();
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

    /// A failing publish (a directory sits where the file should be) removes
    /// the temp and returns the error.
    #[test]
    fn a_publish_failure_removes_the_temp_and_returns_the_error() {
        let d = dir("publish_failure");
        let path = d.join("plan.json");
        std::fs::create_dir(&path).unwrap();
        let err = write_atomic(&path, b"{}", false).unwrap_err();
        assert!(format!("{err:#}").contains("publish"), "{err:#}");
        assert!(path.is_dir(), "the directory target is untouched");
        assert_eq!(
            entries(&d),
            ["plan.json"],
            "a failed publish must not leave the temp behind"
        );
    }

    /// The durable decision stays an explicit caller choice: either way the
    /// same whole bytes land at the target through the crate-owned temp.
    #[test]
    fn both_durable_settings_publish_the_whole_file() {
        for durable in [false, true] {
            let d = dir("durable");
            let path = d.join("config.ini");
            write_atomic(&path, b"[General]\n", durable).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"[General]\n");
            assert_eq!(entries(&d), ["config.ini"], "durable={durable}");
        }
    }
}
