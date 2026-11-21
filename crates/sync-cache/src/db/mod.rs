//! [`CacheDb`]: the handle, read/write or read-only.
//!
//! The read/write-or-read-only distinction is a **type**, not a flag — see
//! [`Handle`] — so "can this handle write?" has a real answer at every call site
//! rather than a boolean somebody has to keep in step.

mod backend;
mod cachedb;
mod handle;

pub use backend::TestCacheBackend;
pub(super) use backend::{SharedCache, memory_registry, redb_builder};
pub use cachedb::CacheDb;
// The one walk over an open table's keys, shared by the read handle and the
// write handle: the batch is not a second reading of the table, it is the same
// reading of it.
pub(super) use cachedb::visit_entry_keys;
pub(super) use handle::{Handle, ReadableCache};
