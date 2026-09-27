//! Small path, time, and filesystem helpers with no girsync-specific state.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::cache::CACHE_PREFIX;

/// Seconds since `t0`, for `elapsed_s` span fields.
pub fn elapsed_s(t0: std::time::Instant) -> f64 {
    t0.elapsed().as_secs_f64()
}

/// Local-time stamp used to name backups and snapshots.
pub fn ts_now() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
}

/// First free `<parent>/<prefix>` (then `<prefix>_2`, ...). Backups are keep-all,
/// so a colliding timestamp must not clobber an existing directory.
pub fn unique_sibling(parent: &Path, prefix: String) -> PathBuf {
    let mut p = parent.join(&prefix);
    let mut n = 1;
    while p.exists() {
        n += 1;
        p = parent.join(format!("{}_{}", prefix, n));
    }
    p
}

/// Recursive plain-file copy, used only for cache backup/snapshot directories.
pub fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("create dir {}", dst.display()))?;
    for ent in std::fs::read_dir(src).with_context(|| format!("read dir {}", src.display()))? {
        let ent = ent?;
        let ft = ent.file_type()?;
        let d = dst.join(ent.file_name());
        if ft.is_dir() {
            copy_dir_all(&ent.path(), &d)?;
        } else {
            std::fs::copy(ent.path(), &d)
                .with_context(|| format!("copy {} -> {}", ent.path().display(), d.display()))?;
        }
    }
    Ok(())
}

/// True when the final path component starts with `girpr-cache`, i.e. the path is
/// a record (a DB directory) rather than a folder root.
pub fn is_record_path(p: &Path) -> bool {
    p.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.starts_with(CACHE_PREFIX))
        .unwrap_or(false)
}

/// True for any `girpr-cache*` relative path: the live DB, its backups, and its
/// `old-*` snapshots. These are never part of a mirror.
pub fn is_cache_rel(rel: &str) -> bool {
    // Covers girpr-cache, girpr-cache-backup-*, girpr-cache-old-* at root.
    rel == CACHE_PREFIX
        || rel.starts_with("girpr-cache/")
        || rel.starts_with("girpr-cache-")
        || rel
            .split('/')
            .next()
            .map(|f| f.starts_with(CACHE_PREFIX))
            .unwrap_or(false)
}

/// Modification time as nanoseconds since the epoch (0 if unavailable).
pub fn mtime_ns_of(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| (d.as_nanos().min(i64::MAX as u128)) as i64)
        .unwrap_or(0)
}
