//! The `list` command: walking a root and inventorying every bundle under it.

use std::path::{Path, PathBuf};

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
/// **A bundle that fails to open is not an error, and it is not dropped.**
/// `list` is an index, and a half-written capture next to a good one is
/// exactly what an operator wants to see listed — so a directory that is not a
/// bundle at all is stepped over silently. A directory that *is* a bundle this
/// build refuses — an older schema, which is a permanent condition with a
/// known fix — is named, with the reason, in [`Index::unreadable`], so the
/// operator's inventory is complete.
pub fn list(root: &Path) -> Result<Index, String> {
    list_with(
        root,
        |d| {
            let rd = std::fs::read_dir(d)?;
            Ok(rd.map(|e| e.map(|e| e.path())).collect())
        },
        |p| std::fs::read(p),
    )
}

/// [`list`] with the directory and manifest readers injected, so a test can
/// make one read fail and see that the walk stops rather than shortening the
/// inventory.
pub fn list_with(
    root: &Path,
    mut read: impl FnMut(&Path) -> std::io::Result<Vec<std::io::Result<PathBuf>>>,
    mut read_manifest: impl FnMut(&Path) -> std::io::Result<Vec<u8>>,
) -> Result<Index, String> {
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }
    let mut found = Vec::new();
    let mut unreadable = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let manifest = dir.join("MANIFEST.json");
        let probe = super::bundle::probe_manifest(&manifest, &mut read_manifest)?;
        if !matches!(probe, super::bundle::ManifestProbe::Missing) {
            match Bundle::open(&dir) {
                Ok(b) => found.push(listed_of(root, &b)),
                Err(e) => {
                    // A directory that is not a bundle at all is stepped over;
                    // one that is a bundle this build refuses is named.
                    if matches!(probe, super::bundle::ManifestProbe::Version(_)) {
                        unreadable.push(Unreadable {
                            path: crate::emit::index::rel(root, &dir),
                            reason: e.clone(),
                        });
                    }
                    tracing::warn!(
                        dir = %dir.display(),
                        error = %e,
                        "a directory under --root looks like a bundle but does not open; skipped"
                    );
                }
            }
            // A bundle does not contain another bundle.
            continue;
        }
        // Sorted so the walk is the same on every filesystem; reversed because
        // the stack pops from the back.
        let subs = child_dirs(&dir, |d| read(d))?;
        stack.extend(subs.into_iter().rev());
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    unreadable.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Index {
        bundles: found,
        unreadable,
    })
}

/// The child directories of `dir`, sorted, or the first reason the walk could
/// not see all of them.
///
/// The reader is injected so the entry-error arm can be exercised without
/// platform-specific permissions: a test hands over a synthetic `Err` for one
/// entry, which is otherwise only reachable when a share disconnects between
/// `read_dir` and the stat. `metadata` follows symlinks, as the `is_dir` it
/// replaces did, and an entry that cannot be stat'ed is an error rather than a
/// file: losing it would be the same silent omission this walk is fixing.
pub fn child_dirs(
    dir: &Path,
    read: impl FnOnce(&Path) -> std::io::Result<Vec<std::io::Result<PathBuf>>>,
) -> Result<Vec<PathBuf>, String> {
    let entries = read(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut subs = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| format!("cannot read an entry of {}: {e}", dir.display()))?;
        let is_dir = std::fs::metadata(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?
            .is_dir();
        if is_dir {
            subs.push(path);
        }
    }
    subs.sort();
    Ok(subs)
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
    }
}
