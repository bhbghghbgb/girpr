//! Backend setup for the redb cache: the page-cache size, the builder, the
//! shared in-memory registry, and the test-only backend switch.

use redb::{Builder, Database};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use tracing::warn;

/// The default redb page-cache size, in MiB.
///
/// redb's default page cache is 1 GiB per handle. Tests run one process per
/// test case (nextest), so parallel tests each reserve that much virtual
/// memory and exhaust this machine's commit limit. Keep it small so they fit.
pub(in crate::cache) const DEFAULT_CACHE_SIZE_MIB: usize = 64;

/// The environment variable that names the page-cache size, in MiB.
const CACHE_SIZE_ENV: &str = "GIRSYNC_CACHE_SIZE_MIB";

/// The page-cache size, in MiB, that every redb handle in this process gets.
///
/// Resolved **once per process**: the variable is read the first time a
/// handle is opened and never again, so every handle in a run gets the same
/// budget and the answer cannot change under a running process.
///
/// A value that names no usable size falls back to
/// [`DEFAULT_CACHE_SIZE_MIB`] and says so, once, through the tracing
/// subscriber if one is listening. An unset variable is not a mistake and is
/// not reported.
pub(in crate::cache) fn cache_size_mib() -> usize {
    static SIZE: OnceLock<usize> = OnceLock::new();
    *SIZE.get_or_init(|| {
        let raw = std::env::var(CACHE_SIZE_ENV).ok();
        match parse_cache_size_mib(raw.as_deref()) {
            Some(mib) => mib,
            None => {
                if let Some(text) = raw.as_deref() {
                    warn!(
                        value = %text,
                        default_mib = DEFAULT_CACHE_SIZE_MIB,
                        "cache size names no usable MiB count; using the default"
                    );
                }
                DEFAULT_CACHE_SIZE_MIB
            }
        }
    })
}

/// The page-cache size, in MiB, that a raw value names, if it names one.
///
/// `None` when the value names no usable size: it is absent, it is blank, it
/// is not a whole number of MiB, or it is zero. Zero is refused because a
/// cache with no room is not a cache; the caller answers
/// [`DEFAULT_CACHE_SIZE_MIB`] for a `None`, and says so.
///
/// A count too large for the platform to address is **saturated** to the
/// largest one it can, rather than refused: the user asked for a lot of
/// memory, and the machine has a lot.
///
/// Pure: it takes the value and answers, so it can be tested without touching
/// the process's environment.
fn parse_cache_size_mib(raw: Option<&str>) -> Option<usize> {
    let text = raw?.trim();
    let mib = text.parse::<u128>().ok()?;
    if mib == 0 {
        return None;
    }
    // The builder multiplies the MiB count into bytes, so the count is capped
    // at the largest one whose bytes still fit in a `usize`: the
    // multiplication that follows then cannot overflow on any platform.
    let max_mib = usize::MAX / (1024 * 1024);
    let mib = usize::try_from(mib).unwrap_or(max_mib);
    Some(mib.min(max_mib))
}

/// A [`redb::Database`] builder with the page-cache size applied, so every redb
/// open in this crate sets the cache size from one place instead of repeating
/// `.set_cache_size` at each call site.
pub(in crate::cache) fn redb_builder() -> Builder {
    let mut b = Database::builder();
    b.set_cache_size(cache_size_mib() * 1024 * 1024);
    b
}

/// The path-keyed in-memory caches tests share across calls.
///
/// One process is one test under `cargo nextest`, so this grows to at most the
/// caches a single test opened and is released with the process.
pub(in crate::cache) fn memory_registry() -> &'static Mutex<HashMap<PathBuf, Arc<Database>>> {
    static CACHES: OnceLock<Mutex<HashMap<PathBuf, Arc<Database>>>> = OnceLock::new();
    CACHES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Test-only switch: which backend a folder cache is opened against.
///
/// `File` (the default, and the only one production ever uses) opens the
/// real `<root>/girpr-cache` redb file. `Memory` serves
/// [`super::CacheDb::open_temp`] instead, so semantic tests that assert sync
/// results — not the cache file itself — pay no 1 MB create + fsync per
/// command. Deliberately not exposed on the CLI; the `GIRSYNC_TEST_CACHE`
/// variable is read once, at command start, by
/// [`crate::config::CommonOpts::try_from`], and only there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TestCacheBackend {
    #[default]
    File,
    Memory,
}
