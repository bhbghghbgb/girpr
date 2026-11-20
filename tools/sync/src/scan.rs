//! Live filesystem scan: one entry per file and dir, with cache dirs pruned.

use anyhow::{Context, Result, bail};
use std::path::Path;
use tracing::{debug, info, trace};

use crate::util::{check_mixed_case_keys, elapsed_s, is_cache_rel, mtime_ns_of, slash_rel};

/// The production heartbeat period: how many entries the walk pushes between
/// two `scan progress: walk` events.
///
/// The heartbeat is the walk's own progress report: it says how many entries
/// the walk has pushed so far, every so often, so a long walk is visibly
/// alive. It is the `scan progress: walk` event. The period is a number
/// rather than a rule: a tree of exactly this many entries produces exactly
/// one heartbeat, and a tree of fewer produces none.
///
/// This is the **production** period. `heartbeat_entries` layers a test-only
/// env hook over it so a small fixture can trip the heartbeat; with the hook
/// unset, the walk uses this constant and behaves exactly as before.
pub const HEARTBEAT_ENTRIES: usize = 2000;

/// The heartbeat period for this process.
///
/// The test-only `GIRSYNC_TEST_HEARTBEAT_ENTRIES` hook lowers the period inside
/// a spawned child so a fixture of a handful of files can trip the heartbeat
/// and be counted, instead of the ~2,000 the production period needs. An unset,
/// non-numeric, or zero value leaves [`HEARTBEAT_ENTRIES`] in place, so the hook
/// is default-off and production is unchanged. Deliberately not `cfg(test)`:
/// the slow suites spawn the real binary and set the variable on the child.
pub(crate) fn heartbeat_entries() -> usize {
    heartbeat_entries_from(
        std::env::var("GIRSYNC_TEST_HEARTBEAT_ENTRIES")
            .ok()
            .as_deref(),
    )
}

/// The pure part of [`heartbeat_entries`], so the parse/filter contract can be
/// unit-tested without relying on the ambient environment being unset (nextest
/// runs tests as parallel threads in one process). An unset, non-numeric or
/// zero value falls back to the production period; only a positive integer
/// overrides it. This is exactly the chain [`heartbeat_entries`] used before
/// the extraction, so production behaviour is unchanged.
fn heartbeat_entries_from(raw: Option<&str>) -> usize {
    raw.and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(HEARTBEAT_ENTRIES)
}

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
    ///
    /// On POSIX a backslash is a literal filename character, not a separator,
    /// and stays in the name exactly as the disk spells it.
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

/// Walk `root` up to `max_depth`, following symlinks, with the resolved
/// heartbeat period.
///
/// Resolves `heartbeat_entries` once per walk and delegates to
/// [`walk_live_with`]. With the test hook unset that is [`HEARTBEAT_ENTRIES`],
/// so every existing caller behaves exactly as it did before the seam; the
/// child-process suites that set `GIRSYNC_TEST_HEARTBEAT_ENTRIES` keep that
/// fallback.
pub fn walk_live(root: &Path, max_depth: usize) -> Result<Vec<LiveEnt>> {
    walk_live_with(root, max_depth, heartbeat_entries())
}

/// Walk `root` up to `max_depth`, following symlinks, with an explicit
/// heartbeat period.
///
/// `girpr-cache*` subtrees are pruned. Any walk error (dangling link, loop,
/// IO failure) aborts the whole run rather than yielding a partial listing.
///
/// The period is a parameter rather than a read of the environment so a caller
/// that already holds a resolved period — a [`crate::config::ScanMode`], see
/// [`crate::config::ScanMode::with_heartbeat_entries`] — can trip the heartbeat
/// with a small fixture. [`walk_live`] is the environment-driven default path.
/// A period of `0` is treated as `1`, so the modulo below cannot divide by zero.
///
/// The walk reports itself in three events, and each of them names it: `scan
/// start`, `scan progress: walk` (the heartbeat) and `scan done`. A reader
/// therefore tells them from the phases' own events by the name alone, and
/// does not have to know which fields an event carries to say where it came
/// from.
///
/// The "scan done" event carries what the walk saw — the entries it pushed and
/// the files and directories among them — and those counts are the walk's own,
/// not the effective map's. They are also returned to the caller as the walk's
/// length, which is what an in-process assertion reads; see the counters' note
/// at the subtraction below.
#[tracing::instrument(skip_all, fields(root = %root.display(), max_depth, heartbeat_entries))]
pub fn walk_live_with(
    root: &Path,
    max_depth: usize,
    heartbeat_entries: usize,
) -> Result<Vec<LiveEnt>> {
    debug!(root = %root.display(), max_depth, "scan start");
    let t0 = std::time::Instant::now();
    let mut out = Vec::new();
    // Resolved once per walk, so a walk cannot change its heartbeat period
    // halfway through. The caller passes either the environment-resolved
    // period (`walk_live`) or one injected through `ScanMode`.
    let heartbeat_period = heartbeat_entries.max(1);
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
                    .with_context(|| format!("non-UTF8 path {}", ent.path().display()))?;
                let rel = slash_rel(rel);
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
                // period's worth of entries. The period is resolved once per
                // walk (see `walk_live_with`); the production default is
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
                if out.len() % heartbeat_period == 0 {
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

    /// The production heartbeat period is the default: with the test hook unset
    /// the walk uses 2,000 entries, so a small fixture does not silently turn the
    /// heartbeat into a per-entry event. The nextest harness sets no
    /// `GIRSYNC_TEST_HEARTBEAT_ENTRIES` (see `.config/nextest-test-env`), so each
    /// test process sees the default; pin it here against a hook leaking into the
    /// shared environment or against a change to the constant itself.
    #[test]
    fn the_heartbeat_period_defaults_to_the_production_2000() {
        assert_eq!(HEARTBEAT_ENTRIES, 2000);
        assert_eq!(
            heartbeat_entries(),
            2000,
            "GIRSYNC_TEST_HEARTBEAT_ENTRIES must be unset for this test"
        );
    }

    /// The hook's parse/filter contract, in-process: unset, non-numeric and
    /// zero (and negative) all leave the production period in place; only a
    /// positive integer overrides it. Testing the pure helper means these
    /// branches do not depend on the ambient env being unset (which would race
    /// with the other tests sharing this process).
    #[test]
    fn heartbeat_entries_from_defaults_except_for_a_positive_override() {
        for unset_or_bad in [None, Some("abc"), Some(""), Some("0"), Some("-1")] {
            assert_eq!(
                heartbeat_entries_from(unset_or_bad),
                2000,
                "{unset_or_bad:?} must keep the production period"
            );
        }
        assert_eq!(heartbeat_entries_from(Some("1")), 1);
        assert_eq!(heartbeat_entries_from(Some("7")), 7);
        assert_eq!(heartbeat_entries_from(Some("2000")), 2000);
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

    /// On POSIX a backslash is a filename character. A leading one must not turn
    /// the relative path into an absolute one (which would replace the root when
    /// joined onto it), and an interior one must not invent a directory that does
    /// not exist on disk. The walk reports both names exactly as the disk spells
    /// them.
    #[cfg(not(windows))]
    #[test]
    fn a_backslash_in_a_posix_name_stays_a_literal_character() {
        let root = girpr_tempdir::TempDir::new("girsync_scan", "backslash");
        for name in ["\\lead.txt", "a\\b.txt"] {
            std::fs::write(root.join(name), b"x").unwrap();
        }

        let mut rels: Vec<String> = walk_live(&root, 4)
            .unwrap()
            .into_iter()
            .map(|e| e.rel)
            .collect();
        rels.sort();

        assert_eq!(rels, vec!["\\lead.txt", "a\\b.txt"]);
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
