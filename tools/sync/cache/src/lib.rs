//! The shared redb-backed hash cache that girsync reads and writes with:
//! schema, open/rebuild, and backup snapshots.
//!
//! This is its own crate so the cache, its byte codec and its native dependency
//! tree are an acyclic leaf below girsync. An edit to girsync's planner,
//! commands or other subsystems rebuilds only that lib; this crate and `redb`,
//! `postcard` and `tempfile` stay cached. The hash vocabulary lives here too —
//! `FileRec` embeds a [`hash::DigestSet`] and its serde impl *is* the wire
//! format — so the cache never has to depend back on girsync's lib.
//!
//! A cache is `<root>/girpr-cache`, a redb **file** (backups and `old-*`
//! snapshots are files too). Keys are `/`-separated relative paths (UTF-8,
//! casing as stored) in the `entries` table; values are [`FileRec`] encoded
//! with the binary codec in `wire`. The `meta` table holds a single `meta` key
//! with the schema [`Meta`].
//!
//! ## Durability / resumability contract
//!
//! redb persists on transaction commit, so `commit()` is the analogue of the
//! old `flush()`. Bulk phases (scan, mkdir, copy-insert, prune) batch writes
//! in a [`CacheWrite`] handle that commits periodically — every
//! [`COMMIT_BATCH`] operations, every [`COMMIT_BYTES`] of file content, or
//! every [`COMMIT_INTERVAL`], whichever comes first — so an interrupted run
//! keeps most of its progress. The byte and time triggers matter for large
//! game files, where a single hash can take longer than hundreds of small
//! puts. Crash-safety points commit unconditionally *before* the
//! corresponding filesystem change (notably the apply pre-drop), so a killed
//! run re-copies rather than trusting a half-written file.
//!
//! Production commits are durable (`Durability::Immediate`). Tests may lower
//! that to `Durability::None` through `GIRSYNC_TEST_DURABILITY=none`, which
//! removes the per-commit fsync but still flushes on a clean `Database` drop;
//! see `write_durability`. The durability is one half of a write's **commit
//! policy** — the other half is the batch thresholds — and the two are read
//! together, once, when the write starts (see `write::CommitPolicy`).
//!
//! ## The submodules
//!
//! | module | what it owns |
//! | --- | --- |
//! | `schema` | [`Meta`] and [`FileRec`]: what a row *is* |
//! | `wire` | the binary encoding of both, and its tests |
//! | `db` | [`CacheDb`]: the read/write-or-read-only handle; `db/` now holds the
//!   `backend`, `cachedb` and `handle` leaves |
//! | `write` | [`CacheWrite`]: batched mutations and when they commit |
//! | `open` | [`open_db`], the backups and snapshots, and the free functions over a handle |
//! | `backup` | the partial-write/link/claim mechanics behind the backups and snapshots |
//! | `lock` | the sidecar lock that marks a run in progress for a folder |
//! | `hash` | the digest vocabulary (algorithm names and row storage) the wire format pins |
//! | `helpers` | the timestamp, copy and file-identity helpers the subsystems share |

mod backup;
mod db;
pub mod hash;
mod helpers;
mod lock;
mod open;
mod schema;
mod wire;
mod write;

use redb::TableDefinition;
use std::path::Path;

/// Keys are `/`-separated relative paths; values are the [`wire`]'d
/// [`FileRec`]. Both tables are touched on create so a fresh file has both.
const ENTRIES: TableDefinition<&str, &[u8]> = TableDefinition::new("entries");
const META_TBL: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");
const META_KEY: &str = "meta";

/// The durability every cache write commits with.
///
/// Production always commits with [`redb::Durability::Immediate`]: once
/// `commit()` returns, the rows survive a crash. Tests may set
/// `GIRSYNC_TEST_DURABILITY=none` on the process (the test runner does it in
/// `.config/nextest-test-env`; a developer does not) to skip the per-commit
/// fsync. It is the sibling of the `GIRSYNC_TEST_CACHE` switch in
/// the resolved options, and like it the default is off:
/// an unset variable — or any value other than exactly `none` — keeps durable
/// commits, so a real run is unaffected unless someone deliberately opts in.
///
/// `None` is safe for tests because redb still flushes on a clean `Database`
/// drop: the drop runs an allocator-state commit with the default
/// `Durability::Immediate` and then flushes the shutdown header. A command
/// that reopens the file — the next `recs_of`, a backup, a snapshot, a lock
/// check — therefore sees every row. Only a process killed *between* a commit
/// and the drop loses rows, and no test does that.
///
/// **The value is read when a write starts, and not again while that write
/// runs.** The durability is one half of a write's commit policy — the other
/// half is the batch thresholds — and the two are read together, by one
/// function, at one moment (see [`write::CommitPolicy`]). A batched write
/// handle therefore keeps the durability it opened with: every transaction it
/// commits, including the ones it opens after a threshold trips, commits the
/// same way. A test that sets the variable in-process before it opens a cache
/// gets what it asked for. Any value other than exactly `none` (including a
/// typo) means `Immediate`.
///
/// redb refuses `Durability::None` in its multiprocess modes. This crate opens
/// every database in the default `ExclusiveWriter` mode, so the gate is valid here;
/// if that ever changes, the `set_durability` call sites start returning
/// `NonDurableCommitUnsupported` under the test switch rather than silently
/// weakening the commit.
pub(crate) fn write_durability() -> redb::Durability {
    #[cfg(test)]
    DURABILITY_READS.with(|c| c.set(c.get() + 1));
    if std::env::var("GIRSYNC_TEST_DURABILITY").as_deref() == Ok("none") {
        redb::Durability::None
    } else {
        redb::Durability::Immediate
    }
}

// How many times the durability switch has been read on this thread.
//
// A test seam: the switch is read when a write starts, and the only way a
// test can see that a write reads it *once* is to count the readings.
#[cfg(test)]
std::thread_local! {
    static DURABILITY_READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many times the durability switch has been read on this thread.
#[cfg(test)]
pub(crate) fn durability_reads() -> u64 {
    DURABILITY_READS.with(|c| c.get())
}

pub use db::{CacheDb, TestCacheBackend, begin_row_capture, take_row_capture};
// Test-only seams: `open_memory_cache` is the shared (path-keyed) variant of
// `CacheDb::open_temp`, and `memory_cache_at` lets a test read what a run left
// there. Both are reached only through `TestCacheBackend::Memory` or
// `GIRSYNC_TEST_CACHE=memory`.
pub use open::{
    CacheOpen, backup_db, get_rec, load_all_records, open_db, put_rec, remove_cache_path,
    remove_rec, snapshot_old,
};
pub use schema::{CACHE_PREFIX, CACHE_VERSION, FileRec, Meta};
pub use write::{COMMIT_BATCH, COMMIT_BYTES, COMMIT_INTERVAL, CacheWrite, CommitLimits};

/// Remove the lock sidecar of the cache at `db_path`, if it is there.
///
/// A caller that moves or deletes a cache file must take its sidecar with it:
/// the sidecar is named after the path, so a new cache at that path creates a
/// fresh one, and a sidecar left from the old one would be an orphan in the
/// user's tree. This is only for a cache path that is private to the caller —
/// a caller-private temporary — never for a live cache another run may lock.
pub fn remove_lock_sidecar(db_path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(lock::path_for(db_path)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
