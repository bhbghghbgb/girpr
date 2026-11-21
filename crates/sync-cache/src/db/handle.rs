//! The [`Handle`] enum: the redb handle a [`super::CacheDb`] wraps,
//! read/write or read-only.

use redb::{Database, ReadTransaction, ReadableDatabase, TransactionError};
use std::sync::Arc;

/// The slice of redb's `ReadableDatabase` this crate needs.
///
/// redb 5 sealed `ReadableDatabase`, so a type outside the crate can no longer
/// implement it. This local trait keeps the same read-only surface over the
/// three handles the crate reads through: a writable [`Database`], a
/// [`redb::ReadOnlyDatabase`], and the [`Handle`] enum that erases the
/// difference. The two redb types delegate to their sealed-trait methods;
/// `Handle` dispatches to whichever variant it holds.
pub(crate) trait ReadableCache {
    fn begin_read(&self) -> Result<ReadTransaction, TransactionError>;
}

impl ReadableCache for Database {
    fn begin_read(&self) -> Result<ReadTransaction, TransactionError> {
        ReadableDatabase::begin_read(self)
    }
}

impl ReadableCache for redb::ReadOnlyDatabase {
    fn begin_read(&self) -> Result<ReadTransaction, TransactionError> {
        ReadableDatabase::begin_read(self)
    }
}

/// The redb handle a [`super::CacheDb`] wraps.
///
/// `ReadOnlyDatabase` is a distinct type rather than a flag on `Database`, so
/// the two have to be matched on. Read-only handles *share* the file with each
/// other; only a read/write handle excludes everyone, which is what makes the
/// one-handle-per-file rule below about writers.
pub(crate) enum Handle {
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

impl ReadableCache for Handle {
    fn begin_read(&self) -> Result<ReadTransaction, TransactionError> {
        match self {
            Handle::Writable(d) => ReadableCache::begin_read(d.as_ref()),
            Handle::ReadOnly(d) => ReadableCache::begin_read(d),
        }
    }
}
