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

pub use db::{CacheDb, TestCacheBackend};
pub use open::{
    CacheOpen, backup_db, get_rec, load_all_records, open_db, put_rec, remove_cache_path,
    remove_rec, snapshot_old,
};
pub use schema::{CACHE_PREFIX, CACHE_VERSION, FileRec, Meta};
pub use write::{COMMIT_BATCH, COMMIT_BYTES, COMMIT_INTERVAL, CacheWrite, CommitLimits};
