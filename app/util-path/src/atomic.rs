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

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tempfile::NamedTempFile;

use super::path::any_component_has_windows_trailing_dot_or_space;

/// The prefix `tempfile` uses for its default temp name.
const DEFAULT_TEMP_PREFIX: &str = ".tmp";

/// A unique temporary sibling claimed beside a target and held across a write.
///
/// This is the claim/cleanup half of a crash-safe publish. `tempfile` owns the
/// name: its exclusive create retries a collision, and dropping the guard
/// removes the file. Publishing consumes the guard, so a failed publish removes
/// the temp too; [`ClaimedTemp::keep`] disarms it for callers that own a
/// different publish protocol and must clean up themselves.
///
/// The prefix, suffix, publish mechanism and error context all stay at the call
/// site: this type only claims the sibling and owns its cleanup.
pub struct ClaimedTemp(NamedTempFile);

impl ClaimedTemp {
    /// Claim a unique temp sibling in `parent` with the caller's `prefix` and
    /// `suffix`. The raw I/O error is returned so the caller adds its own
    /// context to the claim.
    pub fn claim(parent: &Path, prefix: &str, suffix: &str) -> io::Result<Self> {
        let mut builder = tempfile::Builder::new();
        builder.prefix(prefix).suffix(suffix);
        builder.tempfile_in(parent).map(Self)
    }

    /// The claimed path.
    pub fn path(&self) -> &Path {
        self.0.path()
    }

    /// Borrow the claimed file.
    pub fn as_file(&self) -> &File {
        self.0.as_file()
    }

    /// Mutably borrow the claimed file, so a caller can write into it.
    pub fn as_file_mut(&mut self) -> &mut File {
        self.0.as_file_mut()
    }

    /// Disarm the cleanup guard and hand the claimed path to the caller.
    ///
    /// The file handle is closed first so the path can be renamed portably.
    /// From here on the caller owns removing the path on failure.
    pub fn keep(self) -> io::Result<PathBuf> {
        let (file, path) = self.0.into_parts();
        drop(file);
        path.keep().map_err(|e| e.error)
    }

    /// Publish over `dst`, replacing anything already there.
    pub fn persist(self, dst: &Path) -> io::Result<()> {
        self.0.persist(dst).map(|_| ()).map_err(|e| e.error)
    }

    /// Publish to `dst` only if nothing is there yet.
    pub fn persist_noclobber(self, dst: &Path) -> io::Result<()> {
        self.0
            .persist_noclobber(dst)
            .map(|_| ())
            .map_err(|e| e.error)
    }
}

/// Claim a unique temporary **directory** sibling in `parent` with `prefix`.
///
/// The parallel of [`ClaimedTemp::claim`] for a caller whose claim starts as a
/// directory. The guard is disarmed because the caller needs a bare path to
/// rename; removing that path on failure is the caller's business.
pub fn claim_temp_dir(parent: &Path, prefix: &str) -> io::Result<PathBuf> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix);
    Ok(builder.tempdir_in(parent)?.keep())
}

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
    let dir = parent.unwrap_or_else(|| Path::new("."));
    let mut tmp = ClaimedTemp::claim(dir, DEFAULT_TEMP_PREFIX, "")
        .with_context(|| format!("create temp file for {}", path.display()))?;

    {
        let mut writer = io::BufWriter::new(tmp.as_file_mut());
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
    // A failed publish consumes the claim and removes the temp, just like a
    // failed write does when the guard is dropped.
    tmp.persist(path)
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
        girpr_tempdir::test_support::entry_names_with(path, |_| true)
    }

    /// A claim uses the caller's prefix and suffix, is unique, and removes its
    /// file when the guard is dropped without publishing.
    #[test]
    fn a_claim_uses_the_prefix_and_suffix_and_cleans_up_on_drop() {
        let d = dir("claim_shape");
        let a = ClaimedTemp::claim(&d, "girpr-test-", ".part").unwrap();
        let b = ClaimedTemp::claim(&d, "girpr-test-", ".part").unwrap();
        assert_ne!(a.path(), b.path(), "two claims never share a name");
        for claim in [&a, &b] {
            let name = claim.path().file_name().unwrap().to_string_lossy();
            assert!(name.starts_with("girpr-test-"), "prefix: {name}");
            assert!(name.ends_with(".part"), "suffix: {name}");
        }
        let claimed: Vec<_> = [&a, &b].iter().map(|c| c.path().to_path_buf()).collect();
        drop(a);
        drop(b);
        assert!(
            claimed.iter().all(|p| !p.exists()),
            "dropping a claim removes it"
        );
        assert!(entries(&d).is_empty(), "no residue survives the claim");
    }

    /// Disarming a claim leaves the path for a caller that owns its own publish
    /// protocol and cleanup; nothing is removed behind its back.
    #[test]
    fn keep_leaves_the_path_for_the_caller_to_clean() {
        let d = dir("claim_keep");
        let path = ClaimedTemp::claim(&d, "held-", ".tmp")
            .unwrap()
            .keep()
            .unwrap();
        assert!(path.is_file(), "the claimed file stays at its name");
        assert_eq!(entries(&d).len(), 1);
        std::fs::remove_file(&path).unwrap();
    }

    /// Both publish choices move the claimed file onto the target and leave no
    /// temp behind. A no-clobber publish that fails also removes its temp.
    #[test]
    fn publish_choices_publish_and_clean_up() {
        for noclobber in [false, true] {
            let d = dir("claim_publish");
            let target = d.join("artifact.bin");
            let mut claim = ClaimedTemp::claim(&d, "out-", ".tmp").unwrap();
            claim.as_file_mut().write_all(b"payload").unwrap();
            if noclobber {
                claim.persist_noclobber(&target).unwrap();
            } else {
                claim.persist(&target).unwrap();
            }
            assert_eq!(std::fs::read(&target).unwrap(), b"payload");
            assert_eq!(entries(&d), ["artifact.bin"], "noclobber={noclobber}");
        }

        // A target that already exists makes the no-clobber publish fail; the
        // claim must still be removed.
        let d = dir("claim_publish_fail");
        let target = d.join("taken.bin");
        std::fs::write(&target, b"old").unwrap();
        let claim = ClaimedTemp::claim(&d, "out-", ".tmp").unwrap();
        assert!(claim.persist_noclobber(&target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert_eq!(
            entries(&d),
            ["taken.bin"],
            "the failed publish left no temp"
        );
    }

    /// The directory claim mirrors the file claim: unique, caller-prefixed,
    /// and disarmed into a bare path for the caller to rename and clean up.
    #[test]
    fn a_directory_claim_is_unique_and_prefixed() {
        let d = dir("claim_dir");
        let a = claim_temp_dir(&d, "dir-").unwrap();
        let b = claim_temp_dir(&d, "dir-").unwrap();
        assert_ne!(a, b);
        for path in [&a, &b] {
            assert!(path.is_dir(), "the claim is a directory");
            assert!(
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("dir-"),
                "prefix"
            );
            std::fs::remove_dir(path).unwrap();
        }
        assert!(entries(&d).is_empty());
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
