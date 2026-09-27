//! The sled-backed hash cache: schema, open/rebuild, and backup snapshots.
//!
//! A cache is `<root>/girpr-cache`, a sled DB directory. Keys are `/`-separated
//! relative paths (UTF-8, casing as stored); values are [`FileRec`]. A separate
//! `\0meta` key holds the schema [`Meta`].

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, trace};

use crate::util::{copy_dir_all, ts_now, unique_sibling};

/// Cache directory name. Also the prefix that marks a path as a *record*
/// (backups and `old-*` snapshots share it).
pub const CACHE_PREFIX: &str = "girpr-cache";

/// Reserved key holding the [`Meta`] blob; skipped when iterating records.
pub const META_KEY: &str = "\0meta";

/// Cache schema version. Bumping this invalidates existing caches.
const META_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Meta {
    pub version: u32,
    /// Informational only: the same record works in both case modes.
    pub case_sensitive: bool,
}

/// One cached path.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FileRec {
    /// "file" or "dir"
    pub kind: String,
    pub size: u64,
    /// Nanoseconds since the epoch.
    pub mtime_ns: i64,
    pub hashes: HashMap<String, String>,
}

/// Copy an existing cache dir to `<parent>/girpr-cache-backup-<ts>`.
/// Returns the backup path, or `None` when there was nothing to copy.
#[tracing::instrument(skip_all, fields(db = %db_path.display()))]
pub fn backup_db(db_path: &Path) -> Result<Option<PathBuf>> {
    if !db_path.exists() {
        trace!(path = %db_path.display(), "backup skip: db missing");
        return Ok(None);
    }
    let parent = db_path.parent().unwrap_or_else(|| Path::new("."));
    let dest = unique_sibling(parent, format!("girpr-cache-backup-{}", ts_now()));
    info!(src = %db_path.display(), dst = %dest.display(), "backup start");
    copy_dir_all(db_path, &dest)?;
    info!(src = %db_path.display(), dst = %dest.display(), "backup done");
    Ok(Some(dest))
}

/// Copy an existing cache dir to `<parent>/girpr-cache-old-<ts>` to record the
/// pre-sync dst state. Returns the snapshot path, or `None` when absent.
#[tracing::instrument(skip_all, fields(db = %db_path.display()))]
pub fn snapshot_old(db_path: &Path) -> Result<Option<PathBuf>> {
    if !db_path.exists() {
        trace!(path = %db_path.display(), "snapshot-old skip: db missing");
        return Ok(None);
    }
    let parent = db_path.parent().unwrap_or_else(|| Path::new("."));
    let dest = unique_sibling(parent, format!("girpr-cache-old-{}", ts_now()));
    info!(src = %db_path.display(), dst = %dest.display(), "snapshot-old start");
    copy_dir_all(db_path, &dest)?;
    info!(src = %db_path.display(), dst = %dest.display(), "snapshot-old done");
    Ok(Some(dest))
}

/// Open (creating if needed) the cache at `db_path`.
///
/// `ignore_cache` backs up (when `backup_first`) and deletes the DB to force a
/// rebuild. A corrupt DB is a hard error; the message points at `--ignore-cache`.
#[tracing::instrument(skip_all, fields(db = %db_path.display(), case_sensitive, ignore_cache, backup_first))]
pub fn open_db(
    db_path: &Path,
    case_sensitive: bool,
    ignore_cache: bool,
    backup_first: bool,
) -> Result<sled::Db> {
    if ignore_cache && db_path.exists() {
        if backup_first {
            backup_db(db_path)?;
        }
        std::fs::remove_dir_all(db_path)
            .with_context(|| format!("remove {}", db_path.display()))?;
        info!(path = %db_path.display(), "ignore-cache removed");
    } else if backup_first && db_path.exists() {
        backup_db(db_path)?;
    }
    let db = sled::open(db_path).with_context(|| {
        format!(
            "open sled db {} (corrupt? use --ignore-cache)",
            db_path.display()
        )
    })?;
    match db.get(META_KEY)? {
        None => {
            db.insert(
                META_KEY,
                serde_json::to_vec(&Meta {
                    version: META_VERSION,
                    case_sensitive,
                })?,
            )?;
            db.flush()?;
        }
        Some(v) => {
            let m: Meta = serde_json::from_slice(&v)
                .context("parse cache meta (corrupt? use --ignore-cache)")?;
            if m.version != META_VERSION {
                bail!(
                    "unsupported cache version {} (use --ignore-cache to rebuild)",
                    m.version
                );
            }
            // NOTE: case_sensitive is informational only. The cache stays usable
            // across modes: in insensitive mode a disk/cached casing difference is
            // fixed to the on-disk name (disk always governs), so the same record
            // remains valid for a later sensitive run. Only genuine conflicts
            // (two live/record paths differing only by case in insensitive mode)
            // abort, detected by the callers — never here.
            if m.case_sensitive != case_sensitive {
                db.insert(
                    META_KEY,
                    serde_json::to_vec(&Meta {
                        version: META_VERSION,
                        case_sensitive,
                    })?,
                )?;
            }
        }
    }
    Ok(db)
}

/// Every cached path, excluding the meta key.
pub fn load_all_records(db: &sled::Db) -> Result<HashMap<String, FileRec>> {
    let mut map = HashMap::new();
    for kv in db.iter() {
        let (k, v) = kv?;
        if k.as_ref() == META_KEY.as_bytes() {
            continue;
        }
        let rel = String::from_utf8(k.to_vec()).context("non-UTF8 key in cache")?;
        let rec: FileRec = serde_json::from_slice(&v).context("parse cache entry (corrupt?)")?;
        map.insert(rel, rec);
    }
    Ok(map)
}

/// Upsert one cached path.
pub fn put_rec(db: &sled::Db, rel: &str, rec: &FileRec) -> Result<()> {
    db.insert(rel.as_bytes(), serde_json::to_vec(rec)?)?;
    Ok(())
}
