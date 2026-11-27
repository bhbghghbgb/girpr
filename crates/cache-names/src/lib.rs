//! The one `girpr-cache*` name reservation, as a std-only leaf.
//!
//! `girsync` reserves the whole `girpr-cache*` prefix for its own state — the
//! live database, its backups and its `old-*` snapshots — and never mirrors it
//! as ordinary content. Its path predicates and the fixtures' name filter ask
//! the same question; `girpr-tempdir` cannot depend on `girsync-cache`, so this
//! crate is the tiny home for the prefix and the two predicates shared by both,
//! and the reservation is stated in one place.
//!
//! The spelling rule is deliberate and is the one production uses: the comparison
//! folds ASCII case **on Windows only**, where the filesystem cannot tell
//! `GIRPR-CACHE` from `girpr-cache`, and is an exact match elsewhere, where the
//! upper-case spelling is ordinary content. The old fixture used an
//! always-case-sensitive `starts_with`, so on Windows it disagreed with the
//! product; using these predicates here reconciles the two.

/// Cache file name, and the prefix every cache record (backups and `old-*`
/// snapshots) shares.
pub const CACHE_PREFIX: &str = "girpr-cache";

/// True when one name begins with the cache prefix, i.e. it is tool state
/// rather than content.
///
/// ASCII case is folded on Windows only, matching the product's reservation:
/// there a case-only rename of the tool's own cache names the same file, so
/// `GIRPR-CACHE-backup-1` must be reserved too. On POSIX the two spellings are
/// distinct entries and an upper-case one is ordinary content.
pub fn has_cache_prefix(name: &str) -> bool {
    #[cfg(windows)]
    {
        name.get(..CACHE_PREFIX.len())
            .is_some_and(|head| ascii_case_eq(head, CACHE_PREFIX))
    }
    #[cfg(not(windows))]
    {
        name.starts_with(CACHE_PREFIX)
    }
}

/// True when `name` is the cache's own name rather than one of its copies.
///
/// The exact-name counterpart of [`has_cache_prefix`]: a backup or `old-*`
/// snapshot shares the prefix but is not the folder's own cache. The spelling
/// rule is the same — ASCII case folded on Windows only.
pub fn is_cache_name(name: &str) -> bool {
    #[cfg(windows)]
    {
        ascii_case_eq(name, CACHE_PREFIX)
    }
    #[cfg(not(windows))]
    {
        name == CACHE_PREFIX
    }
}

#[cfg(windows)]
const fn ascii_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' { b + 32 } else { b }
}

// The fold is spelled out rather than borrowed from `std`, mirroring the
// product rule: an ASCII upper-case letter is 32 above its lower-case
// counterpart, and every other byte is itself.
#[cfg(windows)]
#[allow(clippy::manual_is_ascii_check)]
const fn ascii_case_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if ascii_lower(a[i]) != ascii_lower(b[i]) {
            return false;
        }
        i += 1;
    }
    true
}
