//! The `list` command: walking a root and inventorying every bundle under it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use super::bundle::Bundle;

/// One row of `list`: a bundle found under a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// The bundle root, relative to the `--root` that was walked.
    pub path: String,
    /// `biz.old_tag__new_tag`, the directory the pair lives in.
    pub pair: String,
    /// The capture id.
    pub capture_id: String,
    /// When the capture ran, as the manifest recorded it.
    pub captured_at: String,
    /// `old_tag -> new_tag`.
    pub tags: String,
    /// How many captures the bundle holds.
    pub patchmaps: usize,
    /// Blobs stored on disk.
    pub blobs: usize,
    /// `pass` | `fail` | `incomplete`, as recorded.
    pub selfcheck: String,
    /// Whether the capture said it finished.
    pub complete: bool,
    /// Whether the optional `dispatch/` record is present.
    pub dispatch_record: bool,
    /// Raw persistent manifests in `persistent/`, zero for an old bundle.
    pub persistent_manifests: usize,
    /// Full-file blobs in `persistent/`, zero for an old bundle.
    pub persistent_blobs: usize,
}

/// What [`list`] found under a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    /// The bundles that opened.
    pub bundles: Vec<Listed>,
    /// The directories that hold a `MANIFEST.json` this build will not read.
    ///
    /// **Not a record.** The spec says a directory that does not open is
    /// stderr-only, because a record would have to carry the failure instead
    /// of the facts. That stays true — but the operator's inventory is still
    /// incomplete, so these are counted and named rather than dropped.
    pub unreadable: Vec<Unreadable>,
}

/// A bundle under the root that this build will not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreadable {
    /// The bundle root, relative to the `--root` that was walked.
    pub path: String,
    /// Why it was not read.
    pub reason: String,
}

/// Every bundle under `root`, sorted by relative path.
///
/// Recursive rather than a fixed depth, because the layout is
/// `<biz>/<old>__<new>/<capture_id>/` today and a root that points at
/// `artifacts/hk4e_global` and one that points at `artifacts/` should both work
/// without the caller knowing which. The order is **not** deepest first: the
/// walk is depth-first, but both lists are re-sorted by relative-path string
/// before they are returned, so a mixed-depth root is ordered by name at the
/// top level first. The spec does not promise an order; this one is stable and
/// decides the summary's `first` and `last`.
///
/// **The walk stays inside the root it was given.** Before descending, each
/// child directory is resolved against the canonical root: a directory that
/// resolves outside it (through a symlink or junction) is skipped, and one that
/// resolves to a directory already visited is skipped too, so a cycle cannot
/// loop the walk. A root that is itself a link still works — its canonical form
/// is the target that bounds the walk.
///
/// **A bundle that fails to open is not an error, and it is not dropped.**
/// `list` is an index, and a half-written capture next to a good one is
/// exactly what an operator wants to see listed — so a directory that is not a
/// bundle at all is stepped over silently, and the walk descends through it so
/// a bundle nested beneath one is still found. A directory that *is* a bundle
/// this build refuses — an older schema, which is a permanent condition with a
/// known fix — is named, with the reason, in [`Index::unreadable`], so the
/// operator's inventory is complete; it is a boundary, so the walk does not
/// descend into it.
///
/// **A walk that cannot read a directory or a manifest is an error, not a
/// shorter inventory.** The `unreadable` list can name a bundle that does not
/// open, but it cannot name what it never saw: an entry error, an ACL-denied
/// directory, or a `MANIFEST.json` that cannot be read would be dropped, so
/// `list` returned a confident, incomplete answer with exit `0`. An audit
/// command fails fast instead and names the directory, entry or manifest it
/// could not read.
pub fn list(root: &Path) -> Result<Index, String> {
    // `WalkDir` reads the directories itself, so the only pre-descent check
    // production needs is the containment probe below; a real read failure is
    // reported as a `WalkDir` error and surfaced by the walk loop.
    list_with(root, |_| Ok(()), |p| std::fs::read(p))
}

/// [`list`] with a per-directory access check and the manifest reader injected,
/// so a test can make one read fail and see that the walk stops rather than
/// shortening the inventory. `WalkDir` owns the traversal; the seam is called
/// on each directory just before the walk would descend into it, which is the
/// point where a failed read used to be able to drop a subtree.
pub fn list_with(
    root: &Path,
    mut check_dir: impl FnMut(&Path) -> std::io::Result<()>,
    mut read_manifest: impl FnMut(&Path) -> std::io::Result<Vec<u8>>,
) -> Result<Index, String> {
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }
    let canonical_root =
        std::fs::canonicalize(root).map_err(|e| format!("cannot read {}: {e}", root.display()))?;
    let mut visited: HashSet<PathBuf> = HashSet::new();
    visited.insert(canonical_root.clone());
    let mut found = Vec::new();
    let mut unreadable = Vec::new();
    let mut failure: Option<String> = None;

    let entries = WalkDir::new(root)
        .follow_links(true)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            if failure.is_some() {
                return false;
            }
            // The root bounds the walk; it is never resolved as an entry.
            if e.depth() > 0 {
                // A child is entered only when its canonical path is inside the
                // canonical root and has not been seen. A link out of the root
                // would put bytes the root does not own into the inventory, and
                // a link back up the tree would loop or list a bundle twice.
                let Some(canonical) = canonical_under_root(&canonical_root, e.path()) else {
                    if e.file_type().is_dir() {
                        tracing::warn!(
                            dir = %e.path().display(),
                            "a directory under --root resolves outside the root through a link; skipped"
                        );
                    }
                    return false;
                };
                if !visited.insert(canonical) {
                    return false;
                }
            }
            // Only directories can be bundles; a file contributes nothing.
            if !e.file_type().is_dir() {
                return false;
            }
            let dir = e.path();
            let manifest = dir.join("MANIFEST.json");
            let probe = match super::bundle::probe_manifest(&manifest, &mut read_manifest) {
                Ok(probe) => probe,
                Err(err) => {
                    failure.get_or_insert(err);
                    return false;
                }
            };
            match probe {
                // No manifest, or a manifest that is not a bundle this build
                // can read: step over it silently and keep descending, so a
                // bundle nested under a half-written directory is still found.
                super::bundle::ManifestProbe::Missing
                | super::bundle::ManifestProbe::NotABundle(_) => {}
                // A manifest declaring this schema version is a real bundle
                // boundary: list it, or name it as unreadable if it does not
                // open, and do not descend into it. A bundle does not contain
                // a bundle.
                super::bundle::ManifestProbe::Version(_) => {
                    match Bundle::open(dir) {
                        Ok(b) => found.push(listed_of(root, &b)),
                        Err(e) => {
                            unreadable.push(Unreadable {
                                path: crate::emit::index::rel(root, dir),
                                reason: e.clone(),
                            });
                            tracing::warn!(
                                dir = %dir.display(),
                                error = %e,
                                "a directory under --root looks like a bundle but does not open; skipped"
                            );
                        }
                    }
                    return false;
                }
            }
            // About to descend: a directory the walk cannot read is an error,
            // not a shorter inventory.
            if let Err(err) = check_dir(dir) {
                failure.get_or_insert(format!("cannot read {}: {err}", dir.display()));
                return false;
            }
            true
        });

    for entry in entries {
        match entry {
            Ok(_) => {}
            Err(err) => {
                // A link to an ancestor is a cycle: `WalkDir` refuses to follow
                // it, and the old walk skipped it rather than failing. Every
                // other error is a directory or entry the walk could not see,
                // which must stop the walk rather than shorten the inventory.
                if err.loop_ancestor().is_some() {
                    continue;
                }
                let path = err.path().unwrap_or(root).to_path_buf();
                failure.get_or_insert(format!("cannot read {}: {err}", path.display()));
                break;
            }
        }
    }

    if let Some(err) = failure {
        return Err(err);
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    unreadable.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Index {
        bundles: found,
        unreadable,
    })
}

/// The canonical form of `path`, but only when it resolves inside
/// `canonical_root`; `None` when it resolves outside the root or cannot be
/// resolved at all (a broken link). Comparing canonical paths is what makes a
/// symlinked root legitimate while a link that escapes it is rejected.
///
/// Shared with [`super::bundle`] so the one-shot manifest read applies the same
/// rule as the walkers: a file reached through a link out of the root is not
/// owned by that root.
pub(super) fn canonical_under_root(canonical_root: &Path, path: &Path) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(path).ok()?;
    canonical.starts_with(canonical_root).then_some(canonical)
}

/// One `Listed` row from an opened bundle.
fn listed_of(root: &Path, b: &Bundle) -> Listed {
    let rel = crate::emit::index::rel(root, &b.root);
    let v = &b.manifest.versions;
    Listed {
        // The three levels below `--root` are the pair and the capture, and the
        // pair is one directory's name only - so it is read from the bundle's own
        // tags rather than assumed to be the parent directory.
        pair: format!("{}__{}", v.old.tag, v.new.tag),
        path: if rel.is_empty() { ".".to_string() } else { rel },
        capture_id: b.manifest.capture.id.clone(),
        captured_at: b.manifest.capture.captured_at.clone(),
        tags: format!("{} -> {}", v.old.tag, v.new.tag),
        patchmaps: b.patchmaps.len(),
        blobs: b
            .index
            .blobs
            .iter()
            .filter(|x| x.props.stored_path.is_some())
            .count(),
        selfcheck: serde_json::to_string(&b.recorded.result)
            .unwrap_or_else(|_| "?".into())
            .trim_matches('"')
            .to_string(),
        complete: b.manifest.capture.complete,
        dispatch_record: b.root.join("dispatch").join("record.json").is_file(),
        persistent_manifests: persistent_counts(&b.root).0,
        persistent_blobs: persistent_counts(&b.root).1,
    }
}

/// The persistent index's own counts, read through the same type the writer
/// used. An unreadable index is not a crash here — `verify` owns the verdict —
/// but reporting zero would hide a record that is sitting on disk, so the file's
/// mere presence counts as one manifest until it can be parsed.
fn persistent_counts(root: &Path) -> (usize, usize) {
    let path = root.join("persistent").join("index.json");
    let Ok(raw) = std::fs::read(path) else {
        return (0, 0);
    };
    match serde_json::from_slice::<crate::persistent::PersistentIndex>(&raw) {
        Ok(i) => (i.manifests.len(), i.blobs.len()),
        Err(_) => (1, 0),
    }
}
