//! Live filesystem scan: one entry per file and dir, with cache dirs pruned.

use anyhow::{bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{debug, info, trace};

use crate::util::{elapsed_s, is_cache_rel, mtime_ns_of};

/// One entry as found on disk, before any cache consultation.
#[derive(Clone, Debug)]
pub struct LiveEnt {
    /// `/`-separated path relative to the walk root, original casing preserved.
    pub rel: String,
    pub abs: PathBuf,
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
        .into_iter()
        .filter_entry(|e| {
            // Prune cache dirs from traversal.
            let rel = e
                .path()
                .strip_prefix(root)
                .ok()
                .and_then(|p| p.to_str())
                .map(|s| s.replace('\\', "/"))
                .unwrap_or_default();
            if rel.is_empty() {
                return true;
            }
            !is_cache_rel(&rel)
        });
    loop {
        match it.next() {
            None => break,
            Some(Err(e)) => bail!("walk {}: {}", root.display(), e), // dangling link / loop / IO -> abort
            Some(Ok(ent)) => {
                if ent.depth() == 0 {
                    continue;
                }
                let rel = ent
                    .path()
                    .strip_prefix(root)
                    .ok()
                    .and_then(|p| p.to_str())
                    .with_context(|| format!("non-UTF8 path {}", ent.path().display()))?
                    .replace('\\', "/");
                if rel.is_empty() || is_cache_rel(&rel) {
                    continue;
                }
                let ft = ent.file_type();
                // Progress: trace every entry (file log), debug heartbeat every 2000.
                trace!(path = %ent.path().display(), rel = %rel, "scan entry");
                if ft.is_dir() {
                    out.push(LiveEnt {
                        rel,
                        abs: ent.path().to_path_buf(),
                        is_dir: true,
                        size: 0,
                        mtime_ns: 0,
                    });
                } else if ft.is_file() {
                    let meta = std::fs::metadata(ent.path())
                        .with_context(|| format!("stat {}", ent.path().display()))?;
                    out.push(LiveEnt {
                        rel,
                        abs: ent.path().to_path_buf(),
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
