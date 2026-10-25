//! The redb-backed hash cache: schema, open/rebuild, and backup snapshots.
//!
//! A cache is `<root>/girpr-cache`, a redb **file** (backups and `old-*`
//! snapshots are files too). Keys are `/`-separated relative paths (UTF-8,
//! casing as stored) in the `entries` table; values are [`FileRec`] encoded
//! with the binary codec in `codec`. The `meta` table holds a single `meta` key
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
//! see [`write_durability`].
//!
//! ## The submodules
//!
//! | module | what it owns |
//! | --- | --- |
//! | `schema` | [`Meta`] and [`FileRec`]: what a row *is* |
//! | `codec` | the binary encoding of both, and its tests |
//! | `db` | [`CacheDb`]: the read/write-or-read-only handle |
//! | `write` | [`CacheWrite`]: batched mutations and when they commit |
//! | `open` | [`open_db`], the backups and snapshots, and the free functions over a handle |

mod codec;
mod db;
mod open;
mod schema;
mod write;

use redb::TableDefinition;

/// Keys are `/`-separated relative paths; values are the [`codec`]'d
/// [`FileRec`]. Both tables are touched on create so a fresh file has both.
const ENTRIES: TableDefinition<&str, &[u8]> = TableDefinition::new("entries");
const META_TBL: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");
const META_KEY: &str = "meta";

/// The durability every cache write transaction is given.
///
/// Production always commits with [`redb::Durability::Immediate`]: once
/// `commit()` returns, the rows survive a crash. Tests may set
/// `GIRSYNC_TEST_DURABILITY=none` on the process (the test runner does it in
/// `.config/nextest.toml`; a developer does not) to skip the per-commit fsync.
/// It is the sibling of the `GIRSYNC_TEST_CACHE` switch in
/// [`effective::side`](crate::effective::side), and like it the default is off:
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
/// The value is read once per transaction creation rather than cached, so a
/// test that sets the variable in-process before opening a cache gets what it
/// asked for. Any value other than exactly `none` (including a typo) means
/// `Immediate`.
///
/// redb refuses `Durability::None` in its multiprocess modes. This crate opens
/// every database in the default single-writer mode, so the gate is valid here;
/// if that ever changes, the `set_durability` call sites start returning
/// `NonDurableCommitUnsupported` under the test switch rather than silently
/// weakening the commit.
pub(crate) fn write_durability() -> redb::Durability {
    if std::env::var("GIRSYNC_TEST_DURABILITY").as_deref() == Ok("none") {
        redb::Durability::None
    } else {
        redb::Durability::Immediate
    }
}

pub use db::{CacheDb, TestCacheBackend};
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
