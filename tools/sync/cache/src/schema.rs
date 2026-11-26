//! What a cache row *is*: [`Meta`] and [`FileRec`].
//!
//! The types, with no encoding and no redb. [`super::wire`] turns them into
//! bytes and this module says what the bytes mean.

use serde::{Deserialize, Deserializer, Serialize};

use crate::hash::DigestSet;

/// Cache file name. Also the prefix that marks a path as a *record*
/// (backups and `old-*` snapshots share it).
pub const CACHE_PREFIX: &str = "girpr-cache";

/// Cache schema version. Old sled directories (and any unknown version) are
/// rejected; rebuild with `--ignore-cache` or convert with `sled2redb`.
///
/// Bumped when the row's schema changes — either the positional `postcard`
/// layout (`postcard` writes fields in declaration order, with no field names),
/// or the set of algorithms with a typed slot in [`crate::hash::DigestSet`].
/// Version 4 adds the eight-byte xxhash64 slot to the two the earlier versions
/// carried. The version gate is what refuses an older cache cleanly and points
/// at the rebuild instead of letting a decoder misread its rows under the old
/// vocabulary.
pub const CACHE_VERSION: u32 = 4;

/// The `meta` row: the schema, and the case mode the cache was written under.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Meta {
    pub version: u32,
    /// Informational only: the same record works in both case modes.
    pub case_sensitive: bool,
}

/// One cached path. `hashes` holds the **raw** digest bytes (16 for md5, 32 for
/// sha256, 8 for xxhash64 — never hex), under an algorithm this build may or may
/// not recognise.
///
/// `PartialEq` is what lets a wire round-trip be asserted as a whole, and what
/// the effective-merge rule is checked against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileRec {
    /// "file" or "dir". A closed registry on the wire: an unknown value is an
    /// error, the opposite of an algorithm name (see `DigestSet`'s serde impl).
    #[serde(deserialize_with = "deserialize_kind")]
    pub kind: String,
    pub size: u64,
    /// Nanoseconds since the epoch.
    pub mtime_ns: i64,
    pub hashes: DigestSet,
}

/// The `kind` field is deliberately closed: a third spelling would break an
/// older binary that cannot represent it, so refuse it loudly. This is the
/// deserialization half of that rule; serialization writes the string as it is.
fn deserialize_kind<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let kind = String::deserialize(deserializer)?;
    match kind.as_str() {
        "file" | "dir" => Ok(kind),
        other => Err(serde::de::Error::custom(format!(
            "invalid record kind {other:?}"
        ))),
    }
}

impl FileRec {
    /// The presence-only row a directory gets: a dir compares by existence, so
    /// it carries no stat and no digests.
    pub fn dir() -> Self {
        Self {
            kind: "dir".into(),
            size: 0,
            mtime_ns: 0,
            hashes: DigestSet::default(),
        }
    }

    pub fn is_dir(&self) -> bool {
        self.kind == "dir"
    }

    pub fn is_file(&self) -> bool {
        self.kind == "file"
    }
}
