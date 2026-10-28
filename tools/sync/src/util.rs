//! Small path, time, and filesystem helpers with no girsync-specific state.

use anyhow::{Context, Result};
use std::path::{Component, Path, PathBuf};

use crate::cache::CACHE_PREFIX;

/// Seconds since `t0`, for `elapsed_s` span fields.
pub fn elapsed_s(t0: std::time::Instant) -> f64 {
    t0.elapsed().as_secs_f64()
}

/// UTC stamp in the shared ISO format, used to name backups and snapshots.
pub fn ts_now() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%S%.6fZ").to_string()
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

/// Recursive plain-file copy, used only for legacy sled-directory cache
/// backups/snapshots. Live redb caches are single files copied with
/// `std::fs::copy` (see [`crate::cache::backup_db`]).
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

/// The canonical form of an existing path, or the path itself when it cannot
/// be resolved (a missing root still has to reach the caller's own error).
///
/// Used for *identity* checks: two spellings of one target must compare equal
/// before either side is opened, and a canonical root joined with a cache name
/// is a path that need not exist yet. Both sync roots must already be
/// directories by the time this is called, so the final component exists; only
/// the cache name joined afterwards may be absent.
pub fn canonical_or_raw(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Whether two paths name the same location.
///
/// Component-wise, so `C:\A` does not match `C:\AB`; case-insensitive on
/// Windows, where the filesystem is. Everywhere else this is exact.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let mut ai = a.components();
    let mut bi = b.components();
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if component_eq(x, y) => {}
            _ => return false,
        }
    }
}

/// Whether `child` is `parent` or lies beneath it.
///
/// Component-wise for the same reason as [`same_path`]: a textual prefix would
/// treat `C:\AB` as inside `C:\A`. Case-insensitive on Windows.
pub fn path_within(child: &Path, parent: &Path) -> bool {
    let mut child = child.components();
    for parent in parent.components() {
        match child.next() {
            Some(c) if component_eq(c, parent) => {}
            _ => return false,
        }
    }
    true
}

fn component_eq(a: Component<'_>, b: Component<'_>) -> bool {
    #[cfg(windows)]
    {
        a.as_os_str().to_string_lossy().to_lowercase()
            == b.as_os_str().to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

/// Modification time as nanoseconds since the epoch (0 if unavailable).
pub fn mtime_ns_of(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| (d.as_nanos().min(i64::MAX as u128)) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Containment is component-wise, not a string prefix: `C:\AB` is not
    /// inside `C:\A`, and the root itself counts as within itself.
    #[test]
    fn containment_is_component_wise() {
        let a = Path::new("C:/A");
        assert!(path_within(Path::new("C:/A"), a), "a path contains itself");
        assert!(path_within(Path::new("C:/A/b"), a));
        assert!(!path_within(Path::new("C:/AB"), a));
        assert!(!path_within(Path::new("C:/"), a));
    }

    /// On Windows the same tree can be spelled with different case, so an
    /// identity check that compared bytes would miss the collision.
    #[cfg(windows)]
    #[test]
    fn path_identity_ignores_case_on_windows() {
        assert!(same_path(
            Path::new("C:/Users/ACER"),
            Path::new("c:/users/acer")
        ));
        assert!(path_within(Path::new("C:/ROOT/Sub"), Path::new("c:/root")));
        assert!(!path_within(Path::new("C:/ROOTER"), Path::new("c:/root")));
    }
}
