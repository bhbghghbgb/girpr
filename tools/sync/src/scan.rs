//! Live filesystem scan: one entry per file and dir, with cache dirs pruned.

use anyhow::{Context, Result, bail};
use std::path::Path;
use tracing::{debug, info, trace};

use crate::util::{check_mixed_case_keys, elapsed_s, is_cache_rel, mtime_ns_of};

/// How many entries the walk pushes between two heartbeats.
///
/// The heartbeat is the walk's own progress report: it says how many entries
/// the walk has pushed so far, every so often, so a long walk is visibly
/// alive. It is the `scan progress: walk` event. The period is a number
/// rather than a rule, and it is named here so that the walk and the case
/// that pins it agree in one place: a tree of exactly this many entries
/// produces exactly one heartbeat, and a tree of fewer produces none.
pub const HEARTBEAT_ENTRIES: usize = 2000;

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
    /// True when this entry is itself a link (a junction counts, though
    /// `FileType::is_symlink` does not report one).
    ///
    /// The cmd point is deletion rather than comparison: the walk follows links,
    /// so an entry *under* this one resolves through it, and removing that
    /// descendant would remove a file in the target. The planner needs to know
    /// which paths are links to stop that; see [`crate::link`].
    pub is_link: bool,
}

/// Walk `root` up to `max_depth`, following symlinks.
///
/// `girpr-cache*` subtrees are pruned. Any walk error (dangling link, loop,
/// IO failure) aborts the whole run rather than yielding a partial listing.
///
/// The walk reports itself in three events, and each of them names it: `scan
/// start`, `scan progress: walk` (the heartbeat) and `scan done`. A reader
/// therefore tells them from the phases' own events by the name alone, and
/// does not have to know which fields an event carries to say where it came
/// from.
///
/// The "scan done" event carries what the walk saw — the entries it pushed and
/// the files and directories among them — and those counts are the walk's own,
/// not the effective map's. They are read from the log file; see the counters'
/// note at the subtraction below.
#[tracing::instrument(skip_all, fields(root = %root.display(), max_depth))]
pub fn walk_live(root: &Path, max_depth: usize) -> Result<Vec<LiveEnt>> {
    debug!(root = %root.display(), max_depth, "scan start");
    let t0 = std::time::Instant::now();
    let mut out = Vec::new();
    // Counted as the entries are pushed, so the walk does not walk its own
    // output a second time to find out how many files it saw. A dir is the
    // other half of the count, and the two add up to the entries.
    let mut files = 0usize;
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
                // `path_is_symlink` is true for a followed symlink even though
                // `file_type()` is the target's type (walkdir remembers the
                // original entry). Some toolchains report a Windows junction as
                // a symlink and some do not, so a directory gets the full
                // link test as a second check; a file link is already covered by
                // `path_is_symlink`, and gating on `is_dir` keeps a stat per
                // regular file out of a cold scan. The test itself excludes
                // non-link reparse points (cloud placeholders and the like),
                // which are ordinary entries.
                let is_link = if ent.path_is_symlink() {
                    true
                } else if ft.is_dir() {
                    crate::link::try_is_link(ent.path())
                        .with_context(|| format!("check link {}", ent.path().display()))?
                } else {
                    false
                };
                // Progress: trace every entry (file log), debug heartbeat every
                // HEARTBEAT_ENTRIES.
                trace!(path = %ent.path().display(), rel = %rel, "scan entry");
                if ft.is_dir() {
                    out.push(LiveEnt {
                        rel,
                        is_dir: true,
                        size: 0,
                        mtime_ns: 0,
                        is_link,
                    });
                } else if ft.is_file() {
                    let meta = std::fs::metadata(ent.path())
                        .with_context(|| format!("stat {}", ent.path().display()))?;
                    files += 1;
                    out.push(LiveEnt {
                        rel,
                        is_dir: false,
                        size: meta.len(),
                        mtime_ns: mtime_ns_of(&meta),
                        is_link,
                    });
                } else {
                    bail!("unsupported file type {}", ent.path().display());
                }
                if out.len() % HEARTBEAT_ENTRIES == 0 {
                    // The heartbeat names the walk, because the phases report
                    // their progress under their own names — and two progress
                    // events that share one name are told apart by the fields
                    // they carry, which is a thing a reader has to know.
                    debug!(root = %root.display(), entries = out.len(), elapsed_s = t0.elapsed().as_secs_f64(), "scan progress: walk");
                }
            }
        }
    }
    // The counters the "scan done" event carries are the **walk's own**: every
    // entry this walk saw, before anything else looked at them. The filter is
    // applied to the walk's output afterwards, in phase A, so a path the run
    // hides is counted here and is not in the effective map — the two numbers
    // answer different questions, and a run with an `--exclude` is where they
    // part company.
    //
    // `dirs` is the remainder rather than a second counter: a directory is
    // pushed into `out` without being counted as a file, so the entries that
    // are not files are the directories. One subtraction, and the two cannot
    // drift apart.
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
    check_mixed_case_keys(live.iter().map(|e| e.rel.as_str()), "", label)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for f in [
            "a.txt",
            "keep/b.txt",
            "girpr-cache/rows",
            // A nested root keeps its own cache; a followed link can expose
            // one too. Both are tool state, not mirror content.
            "nest/girpr-cache/rows",
            "nest/keep.txt",
            "link/girpr-cache",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        }
        for d in [
            "girpr-cache-backup-1",
            "girpr-cache-old-2",
            "nest/girpr-cache-backup-3",
        ] {
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

        assert_eq!(
            rels,
            vec![
                "a.txt",
                "keep",
                "keep/b.txt",
                "link",
                "nest",
                "nest/keep.txt"
            ]
        );
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
}
