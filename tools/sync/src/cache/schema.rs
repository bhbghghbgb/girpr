//! What a cache row *is*: [`Meta`] and [`FileRec`].
//!
//! The types, with no encoding and no redb. [`codec`] turns them into bytes and
//! this module says what the bytes mean.

use std::collections::HashMap;

/// Cache file name. Also the prefix that marks a path as a *record*
/// (backups and `old-*` snapshots share it).
pub const CACHE_PREFIX: &str = "girpr-cache";

/// Cache schema version. Old sled directories (and any unknown version) are
/// rejected; rebuild with `--ignore-cache` or convert with `sled2redb`.
pub const CACHE_VERSION: u32 = 2;

/// The `meta` row: the schema, and the case mode the cache was written under.
#[derive(Clone, Debug)]
pub struct Meta {
    pub version: u32,
    /// Informational only: the same record works in both case modes.
    pub case_sensitive: bool,
}

/// One cached path. `hashes` maps algorithm name (`md5`, `sha256`, ...) to
/// the **raw** digest bytes (16 for md5, 32 for sha256 — never hex).
#[derive(Clone, Debug, PartialEq)]
pub struct FileRec {
    /// "file" or "dir"
    pub kind: String,
    pub size: u64,
    /// Nanoseconds since the epoch.
    pub mtime_ns: i64,
    pub hashes: HashMap<String, Vec<u8>>,
}

impl FileRec {
    /// The presence-only row a directory gets: a dir compares by existence, so
    /// it carries no stat and no digests.
    pub fn dir() -> Self {
        Self {
            kind: "dir".into(),
            size: 0,
            mtime_ns: 0,
            hashes: HashMap::new(),
        }
    }

    pub fn is_dir(&self) -> bool {
        self.kind == "dir"
    }

    pub fn is_file(&self) -> bool {
        self.kind == "file"
    }
}