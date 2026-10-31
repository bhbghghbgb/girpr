//! Small path, time, and filesystem helpers with no girsync-specific state.

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
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

/// True when one path component names cache state: it starts with `girpr-cache`,
/// covering the live DB, its backups and its `old-*` snapshots.
///
/// The comparison folds case on Windows only. There the filesystem cannot tell
/// `GIRPR-CACHE` from `girpr-cache`, so a case-only rename of the tool's own
/// cache is the same file and must be reserved the same way. On POSIX the two
/// spellings are distinct entries, and an upper-case one is ordinary content
/// that must not be swept up by the reservation. This is the cache half of the
/// platform rule [`component_eq`] applies to path identity.
fn has_cache_prefix(component: &str) -> bool {
    #[cfg(windows)]
    {
        component
            .get(..CACHE_PREFIX.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(CACHE_PREFIX))
    }
    #[cfg(not(windows))]
    {
        component.starts_with(CACHE_PREFIX)
    }
}

/// True when the final path component starts with `girpr-cache`, i.e. the path is
/// a record (a DB directory) rather than a folder root.
pub fn is_record_path(p: &Path) -> bool {
    p.file_name()
        .and_then(|s| s.to_str())
        .is_some_and(has_cache_prefix)
}

/// True for any relative path with a `girpr-cache*` component: the live DB,
/// its backups, and its `old-*` snapshots. These are never part of a mirror.
///
/// Every component, not only the first. A cache belongs to the root it sits in,
/// so a nested root carries one at `<root>/sub/girpr-cache`, and a followed
/// link exposes its target's cache as `link/girpr-cache`. Both are tool state,
/// not content; copying them would at best duplicate a database and at worst
/// fail on its live lock. The prefix is reserved at every depth, exactly as
/// [`is_record_path`] reserves it for a record's own name. The same spelling
/// rule applies to both: case is folded on Windows only, through
/// `has_cache_prefix`.
pub fn is_cache_rel(rel: &str) -> bool {
    rel.split('/').any(has_cache_prefix)
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
        let (a, b) = (a.as_os_str(), b.as_os_str());
        if a == b {
            return true;
        }
        // The usual case: two different ASCII spellings, compared without
        // allocating. Non-ASCII and non-UTF-8 names keep the Unicode fold.
        if let (Some(a), Some(b)) = (a.to_str(), b.to_str())
            && a.is_ascii()
            && b.is_ascii()
        {
            return a.eq_ignore_ascii_case(b);
        }
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

/// Group `keys` by lowercase spelling and abort when a group has more than one
/// spelling.
///
/// An exact duplicate is not a collision, so each group is sorted and
/// deduplicated before it is measured. The reported spellings are sorted for
/// the same reason: an error message must not depend on map iteration order.
/// `prefix` and `label` are joined to name the side in the message.
///
/// Shared by the disk walk and the record read: both compare paths by name, and
/// neither owns the rule.
pub(crate) fn check_mixed_case_keys<'a>(
    keys: impl IntoIterator<Item = &'a str>,
    prefix: &str,
    label: &str,
) -> Result<()> {
    let mut groups: HashMap<String, Vec<&'a str>> = HashMap::new();
    for key in keys {
        groups.entry(key.to_lowercase()).or_default().push(key);
    }
    for (_, mut group) in groups {
        group.sort_unstable();
        group.dedup();
        if group.len() > 1 {
            bail!(
                "mixed-case collision in {}{}: {}",
                prefix,
                label,
                group.join(" vs ")
            );
        }
    }
    Ok(())
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

    /// A cache belongs to the root it sits in, so the reservation is per
    /// component rather than per path: a nested root's cache and a cache seen
    /// through a followed link are just as much tool state as the top-level one.
    #[test]
    fn cache_paths_are_reserved_at_any_depth() {
        for rel in [
            "girpr-cache",
            "girpr-cache/rows",
            "girpr-cache-backup-1",
            "girpr-cache-old-2/inner",
            "sub/girpr-cache",
            "sub/girpr-cache/rows",
            "sub/deeper/girpr-cache-backup-3/x",
            "link/girpr-cache",
            "girpr-cache-notes/a.txt",
        ] {
            assert!(is_cache_rel(rel), "`{rel}` is tool state, not content");
        }
        for rel in ["a.txt", "sub/a.txt", "docs/my-girpr-cache.md", "girprcache"] {
            // The prefix is reserved as the start of a *component*, so a name
            // that merely contains it is ordinary content.
            assert!(!is_cache_rel(rel), "`{rel}` is ordinary content");
        }
    }

    /// The cache reservation is spelling-insensitive exactly where the
    /// filesystem is. On Windows a case-only rename of `girpr-cache` is the
    /// same live DB, so `GIRPR-CACHE` and its backups must be tool state too;
    /// on POSIX they are distinct entries and genuine content.
    #[test]
    fn the_cache_prefix_folds_case_on_windows_only() {
        for rel in [
            "girpr-cache",
            "girpr-cache/rows",
            "sub/girpr-cache-backup-1",
        ] {
            assert!(is_cache_rel(rel), "`{rel}` is tool state");
        }
        assert!(is_record_path(Path::new("x/girpr-cache")));
        assert!(is_record_path(Path::new("girpr-cache-backup-1")));

        for rel in [
            "GIRPR-CACHE",
            "sub/GIRPR-CACHE/rows",
            "GIRPR-CACHE-backup-1",
            "sub/Girpr-Cache-old-2",
        ] {
            #[cfg(windows)]
            assert!(
                is_cache_rel(rel),
                "`{rel}` names the same file as `girpr-cache`"
            );
            #[cfg(not(windows))]
            assert!(!is_cache_rel(rel), "`{rel}` is a distinct POSIX entry");
        }

        #[cfg(windows)]
        {
            assert!(is_record_path(Path::new("x/GIRPR-CACHE")));
            assert!(is_record_path(Path::new("GIRPR-CACHE-old-2")));
        }
        #[cfg(not(windows))]
        {
            assert!(!is_record_path(Path::new("x/GIRPR-CACHE")));
            assert!(!is_record_path(Path::new("GIRPR-CACHE-old-2")));
        }
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

    /// The fast path covers ASCII spelling differences, and names outside
    /// ASCII still fold exactly as before: the filesystem treats both as one.
    #[cfg(windows)]
    #[test]
    fn component_equality_folds_ascii_and_non_ascii_case() {
        assert!(same_path(
            Path::new("C:/Users/ACER/Readme.TXT"),
            Path::new("c:/users/acer/readme.txt")
        ));
        assert!(path_within(
            Path::new("C:/ROOT/Sub/Ärger"),
            Path::new("c:/root/sub/ärger")
        ));
        assert!(!same_path(Path::new("C:/Ärger"), Path::new("c:/Arger")));
    }
}
