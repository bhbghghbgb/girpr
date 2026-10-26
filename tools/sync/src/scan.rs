//! Live filesystem scan: one entry per file and dir, with cache dirs pruned.

use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tracing::{debug, info, trace};

use crate::util::{elapsed_s, is_cache_rel, mtime_ns_of};

/// One entry as found on disk, before any cache consultation.
///
/// Paths are relative to the walk root, and a caller that needs an absolute path
/// re-derives it as `root.join(&ent.rel)`. Relative is the deliberate choice: a walk
/// of a 200 k-file tree keeps every one of these alive at once, so a `PathBuf`
/// retained per entry would be several MB of strings for a caller that mostly wants
/// the relative form anyway.
#[derive(Clone, Debug)]
pub struct LiveEnt {
    /// `/`-separated path relative to the walk root, original casing preserved.
    pub rel: String,
    pub is_dir: bool,
    /// 0 for dirs: dirs are compared by presence only.
    pub size: u64,
    pub mtime_ns: i64,
}

/// Walk `root` up to `max_depth`, following symlinks.
///
/// `girpr-cache*` subtrees are pruned. Any walk error (dangling link, loop,
/// IO failure) aborts the whole run rather than yielding a partial listing.
#[tracing::instrument(skip_all, fields(root = %root.display(), max_depth))]
pub fn walk_live(root: &Path, max_depth: usize) -> Result<Vec<LiveEnt>> {
    debug!(root = %root.display(), max_depth, "scan start");
    let t0 = std::time::Instant::now();
    let mut out = Vec::new();
    let mut it = walkdir::WalkDir::new(root)
        .follow_links(true)
        .max_depth(max_depth)
        .into_iter();
    loop {
        match it.next() {
            None => break,
            Some(Err(e)) => bail!("walk {}: {}", root.display(), e), // dangling link / loop / IO -> abort
            Some(Ok(ent)) => {
                if ent.depth() == 0 {
                    continue;
                }
                // Computed once per entry and used twice: the prune decision below and
                // the emitted `LiveEnt`. A `filter_entry` predicate cannot hand its
                // argument back, so deriving the rel inside one would mean deriving it
                // again for every entry that survived — one `String` allocation each,
                // for a path the loop already has.
                let rel = ent
                    .path()
                    .strip_prefix(root)
                    .ok()
                    .and_then(|p| p.to_str())
                    .with_context(|| format!("non-UTF8 path {}", ent.path().display()))?
                    .replace('\\', "/");
                if rel.is_empty() {
                    continue;
                }
                // Prune cache dirs from traversal. `skip_current_dir` rather than a
                // `filter_entry` predicate, so the rel above is the only copy of this
                // path the walk makes. walkdir has already yielded the directory
                // itself, so the subtree is pruned from here on; a cache *file* is
                // simply dropped.
                if is_cache_rel(&rel) {
                    if ent.file_type().is_dir() {
                        it.skip_current_dir();
                    }
                    continue;
                }
                let ft = ent.file_type();
                // Progress: trace every entry (file log), debug heartbeat every 2000.
                trace!(path = %ent.path().display(), rel = %rel, "scan entry");
                // `follow_links(true)` above makes walkdir descend into symlinked
                // directories, but `file_type()` reports the link itself, never the
                // target — a symlink is neither `is_dir` nor `is_file` here. Look
                // through it so a symlink is handled the way the docs promise:
                // followed, materialized as a plain file or dir, and a dangling link
                // aborts the walk. A link target outside the root is followed too.
                if ft.is_symlink() {
                    let meta = std::fs::metadata(ent.path())
                        .with_context(|| format!("stat symlink {}", ent.path().display()))?;
                    if meta.is_dir() {
                        out.push(LiveEnt {
                            rel,
                            is_dir: true,
                            size: 0,
                            mtime_ns: 0,
                        });
                    } else if meta.is_file() {
                        out.push(LiveEnt {
                            rel,
                            is_dir: false,
                            size: meta.len(),
                            mtime_ns: mtime_ns_of(&meta),
                        });
                    } else {
                        bail!("unsupported file type {}", ent.path().display());
                    }
                } else if ft.is_dir() {
                    out.push(LiveEnt {
                        rel,
                        is_dir: true,
                        size: 0,
                        mtime_ns: 0,
                    });
                } else if ft.is_file() {
                    let meta = std::fs::metadata(ent.path())
                        .with_context(|| format!("stat {}", ent.path().display()))?;
                    out.push(LiveEnt {
                        rel,
                        is_dir: false,
                        size: meta.len(),
                        mtime_ns: mtime_ns_of(&meta),
                    });
                } else {
                    bail!("unsupported file type {}", ent.path().display());
                }
                if out.len() % 2000 == 0 {
                    debug!(root = %root.display(), entries = out.len(), elapsed_s = t0.elapsed().as_secs_f64(), "scan progress");
                }
            }
        }
    }
    let files = out.iter().filter(|e| !e.is_dir).count();
    let dirs = out.len() - files;
    info!(
        root = %root.display(),
        entries = out.len(),
        files,
        dirs,
        elapsed_s = elapsed_s(t0),
        "scan done"
    );
    Ok(out)
}

/// Abort if any two live rels differ only by case (only possible to detect, or
/// only a problem, when `!case_sensitive`).
pub fn check_mixed_case(live: &[LiveEnt], label: &str, case_sensitive: bool) -> Result<()> {
    if case_sensitive {
        return Ok(());
    }
    let mut map: HashMap<String, Vec<&str>> = HashMap::new();
    for e in live {
        map.entry(e.rel.to_lowercase()).or_default().push(&e.rel);
    }
    for (_, v) in map {
        let uniq: HashSet<&&str> = v.iter().collect();
        if uniq.len() > 1 {
            let mut sorted: Vec<String> = uniq.into_iter().map(|s| s.to_string()).collect();
            sorted.sort();
            bail!("mixed-case collision in {}: {}", label, sorted.join(" vs "));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a symlink. On Windows links carry a type (file vs dir), so the
    /// two platforms spell the creation differently.
    #[cfg(unix)]
    fn make_symlink(original: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(original, link)
    }
    #[cfg(windows)]
    fn make_symlink(original: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(original, link)
    }
    #[cfg(unix)]
    fn make_symlink_dir(original: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(original, link)
    }
    #[cfg(windows)]
    fn make_symlink_dir(original: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(original, link)
    }

    /// Pruning is asserted as the *set* of surviving rels, which is the property
    /// every caller depends on, rather than as the order the walk happened to
    /// produce. It covers both halves of the decision that a `girpr-cache*` entry
    /// is not part of a mirror: the entry itself is dropped, and nothing beneath a
    /// cache *directory* is visited — the one that a walk which merely filtered the
    /// output would get wrong while still looking correct.
    #[test]
    fn cache_paths_are_pruned_along_with_their_contents() {
        let root = girpr_tempdir::TempDir::new("girsync_scan", "prune");
        // The live DB is a file; the backups and `old-*` snapshots are directories.
        // All three are named `girpr-cache*` and none of them is mirror content.
        for f in ["a.txt", "keep/b.txt", "girpr-cache/rows"] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        }
        for d in ["girpr-cache-backup-1", "girpr-cache-old-2"] {
            let p = root.join(d).join("inner.bin");
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }

        let mut rels: Vec<String> = walk_live(&root, 16)
            .unwrap()
            .into_iter()
            .map(|e| e.rel)
            .collect();
        rels.sort();

        assert_eq!(rels, vec!["a.txt", "keep", "keep/b.txt"]);
    }

    /// `max_depth` is the walk's own bound, and the cache prune must not consume
    /// it: a cache dir beyond the cap is never reached in the first place, and a
    /// real dir exactly at the cap is still emitted.
    #[test]
    fn the_depth_cap_bounds_the_walk_independently_of_the_cache_prune() {
        let root = girpr_tempdir::TempDir::new("girsync_scan", "depth");
        for f in ["top.txt", "one/mid.txt", "one/two/deep.txt"] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        }

        let mut rels: Vec<String> = walk_live(&root, 2)
            .unwrap()
            .into_iter()
            .map(|e| e.rel)
            .collect();
        rels.sort();

        assert_eq!(rels, vec!["one", "one/mid.txt", "one/two", "top.txt"]);
    }

    /// Symlink creation requires elevation on some hosts, so a test that needs
    /// one skips here rather than failing. The assertions run only where links
    /// actually exist — which is also where the symlink arms of `walk_live`
    /// can be exercised at all.
    fn symlink_root(tag: &str) -> Option<girpr_tempdir::TempDir> {
        let root = girpr_tempdir::TempDir::new("girsync_scan", tag);
        match make_symlink(std::path::Path::new("probe"), &root.join("probe-link")) {
            Ok(()) => Some(root),
            Err(_) => {
                eprintln!(
                    "skipping symlink test {tag}: symlinks cannot be created in this environment"
                );
                None
            }
        }
    }

    /// A symlink to a file is materialized as a plain file: present in the walk
    /// with the target's size and mtime, so `sync` can copy it like any other
    /// file.
    #[test]
    fn a_file_symlink_is_walked_as_a_file() {
        let Some(root) = symlink_root("symlink_file") else {
            return;
        };
        let target = root.join("target.bin");
        std::fs::write(&target, b"symlinked content").unwrap();
        make_symlink(&target, &root.join("link.bin")).unwrap();

        let got: std::collections::HashMap<String, LiveEnt> = walk_live(&root, 16)
            .unwrap()
            .into_iter()
            .map(|e| (e.rel.clone(), e))
            .collect();
        let link = got.get("link.bin").expect("the symlink is walked: {got:?}");
        assert!(!link.is_dir, "a file symlink is a file entry: {link:?}");
        assert_eq!(link.size, b"symlinked content".len() as u64);
        let target_meta = std::fs::metadata(&target).unwrap();
        assert_eq!(
            link.mtime_ns,
            mtime_ns_of(&target_meta),
            "the target's mtime"
        );
    }

    /// A symlink to a directory is materialized as a plain dir, and walkdir
    /// (`follow_links(true)`) descends through it, so the whole subtree shows up
    /// under the link's rel — including a link target outside the root.
    #[test]
    fn a_dir_symlink_is_walked_as_a_dir_with_its_subtree() {
        let Some(root) = symlink_root("symlink_dir") else {
            return;
        };
        std::fs::create_dir_all(root.join("real/sub")).unwrap();
        std::fs::write(root.join("real/sub/file.txt"), b"nested").unwrap();
        make_symlink_dir(&root.join("real"), &root.join("link")).unwrap();

        let got: std::collections::HashMap<String, LiveEnt> = walk_live(&root, 16)
            .unwrap()
            .into_iter()
            .map(|e| (e.rel.clone(), e))
            .collect();
        assert!(
            got.contains_key("link"),
            "the link itself is a dir entry: {got:?}"
        );
        assert!(
            got.contains_key("link/sub"),
            "descended directories follow the link"
        );
        assert!(
            got.contains_key("link/sub/file.txt"),
            "and nested files are walked through the link"
        );
        let link = got.get("link").expect("link entry");
        assert!(link.is_dir, "a dir symlink is a dir entry: {link:?}");
        assert_eq!(link.size, 0, "dirs carry no size");
    }

    /// A dangling symlink cannot be stat'd and aborts the walk, as the docs
    /// promise ("Dangling links abort"). walkdir's own error arm would catch a
    /// loop; this one is caught at stat time.
    #[test]
    fn a_dangling_symlink_aborts_the_walk() {
        let Some(root) = symlink_root("symlink_dangling") else {
            return;
        };
        make_symlink(&root.join("does-not-exist"), &root.join("broken")).unwrap();

        let err = walk_live(&root, 16).unwrap_err();
        assert!(
            err.to_string().contains("broken"),
            "the error names the dangling link: {err}"
        );
    }
}
