//! The [`Handle`] enum: the redb handle a [`CacheDb`] wraps, read/write or
//! read-only.

use redb::{Database, ReadableDatabase};
use std::sync::Arc;

/// The redb handle a [`CacheDb`] wraps.
///
/// `ReadOnlyDatabase` is a distinct type rather than a flag on `Database`, so
/// the two have to be matched on. Read-only handles *share* the file with each
/// other; only a read/write handle excludes everyone, which is what makes the
/// one-handle-per-file rule below about writers.
pub(in crate::cache) enum Handle {
    /// `Arc` because a test memory cache is shared between the commands that open
    /// it by the same path; a file cache has exactly one owner and the extra
    /// indirection costs nothing measurable.
    Writable(Arc<Database>),
    ReadOnly(redb::ReadOnlyDatabase),
}

impl Handle {
    pub(super) fn writable(&self) -> Option<&Database> {
        match self {
            Handle::Writable(d) => Some(d.as_ref()),
            Handle::ReadOnly(_) => None,
        }
    }
}

impl ReadableDatabase for Handle {
    fn begin_read(&self) -> std::result::Result<redb::ReadTransaction, redb::TransactionError> {
        match self {
            Handle::Writable(d) => d.begin_read(),
            Handle::ReadOnly(d) => d.begin_read(),
        }
    }

    fn cache_stats(&self) -> redb::CacheStats {
        match self {
            Handle::Writable(d) => d.cache_stats(),
            Handle::ReadOnly(d) => d.cache_stats(),
        }
    }
}
