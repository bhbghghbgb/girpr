//! Backend setup for the redb cache: the page-cache size, the builder, the
//! shared in-memory registry, and the test-only backend switch.

use redb::{Builder, Database};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// redb's default page cache is 1 GiB per handle. Tests run one process per
/// test case (nextest), so parallel tests each reserve that much virtual
/// memory and exhaust this machine's commit limit. Keep it small so they fit.
const CACHE_SIZE: usize = 64 * 1024 * 1024; // 64 MiB

/// A [`redb::Database`] builder with [`CACHE_SIZE`] applied, so every redb
/// open in this crate sets the cache size from one place instead of repeating
/// `.set_cache_size` at each call site.
pub(in crate::cache) fn redb_builder() -> Builder {
    let mut b = Database::builder();
    b.set_cache_size(CACHE_SIZE);
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
/// command. Deliberately not exposed on the CLI; only test helpers set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TestCacheBackend {
    #[default]
    File,
    Memory,
}
