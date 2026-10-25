//! [`CacheDb`]: the handle, read/write or read-only.
//!
//! The read/write-or-read-only distinction is a **type**, not a flag — see
//! [`Handle`] — so "can this handle write?" has a real answer at every call site
//! rather than a boolean somebody has to keep in step.

mod backend;
mod cachedb;
mod handle;

pub use backend::TestCacheBackend;
pub(super) use backend::{memory_registry, redb_builder};
pub use cachedb::CacheDb;
pub(super) use handle::Handle;
