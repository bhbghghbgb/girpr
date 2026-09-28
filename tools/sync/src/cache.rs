//! The redb-backed hash cache: schema, open/rebuild, and backup snapshots.
//!
//! A cache is `<root>/girpr-cache`, a redb **file** (backups and `old-*`
//! snapshots are files too). Keys are `/`-separated relative paths (UTF-8,
//! casing as stored) in the `entries` table; values are [`FileRec`] encoded
//! with the binary codec below (no JSON, raw digest bytes, no hex). The
//! `meta` table holds a single `meta` key with the schema [`Meta`].
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

use anyhow::{Context, Result, bail};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition, backends::InMemoryBackend};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tracing::{info, trace};

use crate::util::{copy_dir_all, ts_now, unique_sibling};

/// Cache file name. Also the prefix that marks a path as a *record*
/// (backups and `old-*` snapshots share it).
pub const CACHE_PREFIX: &str = "girpr-cache";

/// Cache schema version. Old sled directories (and any unknown version) are
/// rejected; rebuild with `--ignore-cache` or convert with `sled2redb`.
pub const CACHE_VERSION: u32 = 2;

/// How many mutations a [`CacheWrite`] batches before committing, so
/// interrupted runs keep their progress without an fsync per file.
pub const COMMIT_BATCH: usize = 500;

/// How much file content (sum of `FileRec.size` on `put`) a [`CacheWrite`]
/// batches before committing. Bounds re-hash work lost to interruption when
/// a few huge files dominate a scan.
pub const COMMIT_BYTES: u64 = 256 * 1024 * 1024;

/// How long a [`CacheWrite`] goes between commits at most. Bounds progress
/// lost to interruption when hashing is slow (large files) and mutations
/// are infrequent.
pub const COMMIT_INTERVAL: Duration = Duration::from_secs(5);

/// Periodic-commit thresholds for [`CacheWrite`]; see the `COMMIT_*`
/// constants for the defaults.
#[derive(Clone, Copy, Debug)]
pub struct CommitLimits {
    /// Commit after this many `put`/`remove` calls.
    pub max_ops: usize,
    /// Commit after this many bytes of `put` file content.
    pub max_bytes: u64,
    /// Commit if this much wall time passed since the last commit.
    pub max_interval: Duration,
}

impl Default for CommitLimits {
    fn default() -> Self {
        Self {
            max_ops: COMMIT_BATCH,
            max_bytes: COMMIT_BYTES,
            max_interval: COMMIT_INTERVAL,
        }
    }
}

const ENTRIES: TableDefinition<&str, &[u8]> = TableDefinition::new("entries");
const META_TBL: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");
const META_KEY: &str = "meta";

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

// ---------------------------------------------------------------------------
// Binary codec
// ---------------------------------------------------------------------------
//
// Entry value layout (all integers little-endian):
//   [u8 fmt=1][u8 kind: 0=file, 1=dir][u64 size][i64 mtime_ns]
//   [u8 n_hashes][ per hash: u8 name_len, name bytes, u8 digest_len, digest ]
//
// Algorithm names are stored inline (not a fixed id registry), so a cache
// written with a new algorithm stays readable by older binaries: unknown
// names decode into the map and are simply ignored unless requested.
// Meta value: [u32 version][u8 case_sensitive].

const FMT_V1: u8 = 1;
const KIND_FILE: u8 = 0;
const KIND_DIR: u8 = 1;

fn kind_to_byte(kind: &str) -> Result<u8> {
    match kind {
        "file" => Ok(KIND_FILE),
        "dir" => Ok(KIND_DIR),
        other => bail!("invalid record kind {other:?}"),
    }
}

fn kind_from_byte(b: u8) -> Result<String> {
    match b {
        KIND_FILE => Ok("file".into()),
        KIND_DIR => Ok("dir".into()),
        other => bail!("invalid record kind byte {other}"),
    }
}

fn encode_rec(rec: &FileRec) -> Result<Vec<u8>> {
    let mut v = Vec::with_capacity(19 + rec.hashes.len() * 24);
    v.push(FMT_V1);
    v.push(kind_to_byte(&rec.kind)?);
    v.extend_from_slice(&rec.size.to_le_bytes());
    v.extend_from_slice(&rec.mtime_ns.to_le_bytes());
    if rec.hashes.len() > 255 {
        bail!("too many hashes for {}", rec.hashes.len());
    }
    // Sorted names for deterministic bytes.
    let mut names: Vec<&String> = rec.hashes.keys().collect();
    names.sort();
    v.push(names.len() as u8);
    for name in names {
        let digest = &rec.hashes[name];
        if name.len() > 255 || digest.len() > 255 {
            bail!("hash entry too large for {name:?}");
        }
        v.push(name.len() as u8);
        v.extend_from_slice(name.as_bytes());
        v.push(digest.len() as u8);
        v.extend_from_slice(digest);
    }
    Ok(v)
}

fn decode_rec(bytes: &[u8]) -> Result<FileRec> {
    let mut cur = bytes;
    let take = |cur: &mut &[u8], n: usize| -> Result<Vec<u8>> {
        if cur.len() < n {
            bail!("truncated cache entry");
        }
        let (h, t) = cur.split_at(n);
        *cur = t;
        Ok(h.to_vec())
    };
    let fmt = take(&mut cur, 1)?[0];
    if fmt != FMT_V1 {
        bail!("unsupported entry format {fmt}");
    }
    let kind = kind_from_byte(take(&mut cur, 1)?[0])?;
    let size = u64::from_le_bytes(take(&mut cur, 8)?.try_into().unwrap());
    let mtime_ns = i64::from_le_bytes(take(&mut cur, 8)?.try_into().unwrap());
    let n = take(&mut cur, 1)?[0] as usize;
    let mut hashes = HashMap::with_capacity(n);
    for _ in 0..n {
        let nl = take(&mut cur, 1)?[0] as usize;
        let name = String::from_utf8(take(&mut cur, nl)?).context("non-UTF8 algo name")?;
        let dl = take(&mut cur, 1)?[0] as usize;
        let digest = take(&mut cur, dl)?;
        hashes.insert(name, digest);
    }
    if !cur.is_empty() {
        bail!("trailing bytes in cache entry");
    }
    Ok(FileRec {
        kind,
        size,
        mtime_ns,
        hashes,
    })
}

fn encode_meta(m: &Meta) -> Vec<u8> {
    let mut v = Vec::with_capacity(5);
    v.extend_from_slice(&m.version.to_le_bytes());
    v.push(u8::from(m.case_sensitive));
    v
}

fn decode_meta(bytes: &[u8]) -> Result<Meta> {
    if bytes.len() != 5 {
        bail!("corrupt cache meta (len {})", bytes.len());
    }
    let version = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    Ok(Meta {
        version,
        case_sensitive: bytes[4] != 0,
    })
}

fn is_table_missing(e: &redb::TableError) -> bool {
    matches!(e, redb::TableError::TableDoesNotExist(_))
}

// ---------------------------------------------------------------------------
// Cache handle
// ---------------------------------------------------------------------------

/// An open redb cache file.
///
/// Note: redb takes a file lock per open database, so two `CacheDb` handles
/// on the **same** file cannot be alive at once (the second open fails with
/// "Database already open"). All callers open, use, and drop sequentially —
/// never hold two handles on one path.
pub struct CacheDb {
    db: Database,
}

impl CacheDb {
    fn init_meta(db: &Database, case_sensitive: bool) -> Result<()> {
        let txn = db.begin_write().context("begin write (meta)")?;
        {
            // Touch the entries table so a fresh file has both tables.
            let _ = txn.open_table(ENTRIES).context("open entries table")?;
            let mut meta = txn.open_table(META_TBL).context("open meta table")?;
            // Read the current meta into an owned buffer first: the access
            // guard borrows the table, which would block the write below.
            let cur: Option<Vec<u8>> = meta
                .get(META_KEY)
                .context("read meta")?
                .map(|g| g.value().to_vec());
            match cur {
                None => {
                    meta.insert(
                        META_KEY,
                        encode_meta(&Meta {
                            version: CACHE_VERSION,
                            case_sensitive,
                        })
                        .as_slice(),
                    )
                    .context("write meta")?;
                }
                Some(raw) => {
                    let m = decode_meta(&raw)
                        .context("parse cache meta (corrupt? use --ignore-cache)")?;
                    if m.version != CACHE_VERSION {
                        bail!(
                            "unsupported cache version {} (want {CACHE_VERSION}; use --ignore-cache to rebuild or sled2redb to convert)",
                            m.version
                        );
                    }
                    // NOTE: case_sensitive is informational only (see open_db).
                    if m.case_sensitive != case_sensitive {
                        meta.insert(
                            META_KEY,
                            encode_meta(&Meta {
                                version: CACHE_VERSION,
                                case_sensitive,
                            })
                            .as_slice(),
                        )
                        .context("write meta")?;
                    }
                }
            }
        }
        txn.commit().context("commit meta")?;
        Ok(())
    }

    fn open_file(db_path: &Path) -> Result<Database> {
        if db_path.is_dir() {
            bail!(
                "cache {} is a directory (sled format): use --ignore-cache to rebuild or sled2redb to convert",
                db_path.display()
            );
        }
        Database::create(db_path).with_context(|| {
            format!(
                "open redb cache {} (corrupt? use --ignore-cache)",
                db_path.display()
            )
        })
    }

    /// In-memory cache for `--dry-run` fallbacks and unit tests. Never
    /// touches the filesystem.
    pub fn open_temp(case_sensitive: bool) -> Result<Self> {
        let db = Database::builder()
            .create_with_backend(InMemoryBackend::new())
            .context("open temp cache")?;
        Self::init_meta(&db, case_sensitive)?;
        Ok(Self { db })
    }

    /// Open an existing record file read-only-ish: no writes, no meta
    /// creation — but the version is still validated.
    pub fn open_record(db_path: &Path) -> Result<Self> {
        if !db_path.exists() {
            bail!("record {} not found", db_path.display());
        }
        if db_path.is_dir() {
            bail!(
                "record {} is a directory (sled format): convert it with sled2redb first",
                db_path.display()
            );
        }
        let db = Database::open(db_path)
            .with_context(|| format!("open record {} (corrupt?)", db_path.display()))?;
        let txn = db.begin_read().context("begin read (record meta)")?;
        let meta = txn.open_table(META_TBL).map_err(|e| {
            if is_table_missing(&e) {
                anyhow::anyhow!("record {} has no meta table (corrupt?)", db_path.display())
            } else {
                anyhow::anyhow!("open record meta {}: {e:#}", db_path.display())
            }
        })?;
        let g = meta
            .get(META_KEY)
            .context("read record meta")?
            .context(format!(
                "record {} has no meta (corrupt?)",
                db_path.display()
            ))?;
        let m = decode_meta(g.value()).context("parse record meta (corrupt?)")?;
        if m.version != CACHE_VERSION {
            bail!(
                "unsupported record version {} in {} (want {CACHE_VERSION})",
                m.version,
                db_path.display()
            );
        }
        Ok(Self { db })
    }

    /// Read one entry. Missing tables (fresh temp DBs) read as empty.
    pub fn get(&self, rel: &str) -> Result<Option<FileRec>> {
        let txn = self.db.begin_read().context("begin read")?;
        let tbl = match txn.open_table(ENTRIES) {
            Ok(t) => t,
            Err(e) if is_table_missing(&e) => return Ok(None),
            Err(e) => bail!("open entries table: {e:#}"),
        };
        match tbl.get(rel).context("cache get")? {
            None => Ok(None),
            Some(g) => decode_rec(g.value()).map(Some),
        }
    }

    /// Every cached path.
    pub fn load_all(&self) -> Result<HashMap<String, FileRec>> {
        let txn = self.db.begin_read().context("begin read")?;
        let tbl = match txn.open_table(ENTRIES) {
            Ok(t) => t,
            Err(e) if is_table_missing(&e) => return Ok(HashMap::new()),
            Err(e) => bail!("open entries table: {e:#}"),
        };
        let mut map = HashMap::new();
        for kv in tbl.iter().context("iterate cache")? {
            let (k, v) = kv.context("read cache entry")?;
            map.insert(k.value().to_string(), decode_rec(v.value())?);
        }
        Ok(map)
    }

    /// Single upsert committed immediately. Hot loops should use
    /// [`CacheWrite`] batching instead.
    pub fn put(&self, rel: &str, rec: &FileRec) -> Result<()> {
        let mut w = self.begin_write()?;
        w.put(rel, rec)?;
        w.commit()
    }

    /// Single remove committed immediately.
    pub fn remove(&self, rel: &str) -> Result<()> {
        let mut w = self.begin_write()?;
        w.remove(rel)?;
        w.commit()
    }

    /// Start a batched write handle. Must be committed (explicitly or via
    /// periodic auto-commit) for mutations to become durable.
    pub fn begin_write(&self) -> Result<CacheWrite<'_>> {
        CacheWrite::new(&self.db)
    }

    /// Start a batched write handle with custom commit thresholds (see
    /// [`CommitLimits`]).
    pub fn begin_write_with(&self, limits: CommitLimits) -> Result<CacheWrite<'_>> {
        CacheWrite::with_limits(&self.db, limits)
    }
}

/// Batched writes over one cache: mutations accumulate in a single write
/// transaction and are committed periodically — every `max_ops` operations,
/// every `max_bytes` of `put` file content, or every `max_interval` of wall
/// time, whichever comes first — plus once more at
/// [`commit`](CacheWrite::commit). Reads see the handle's own pending
/// writes, mirroring the old immediate-visibility semantics.
pub struct CacheWrite<'a> {
    db: &'a Database,
    txn: Option<redb::WriteTransaction>,
    limits: CommitLimits,
    dirty: usize,
    bytes_since_commit: u64,
    last_commit: Instant,
}

impl<'a> CacheWrite<'a> {
    fn new(db: &'a Database) -> Result<Self> {
        Self::with_limits(db, CommitLimits::default())
    }

    fn with_limits(db: &'a Database, limits: CommitLimits) -> Result<Self> {
        let txn = db.begin_write().context("begin write")?;
        Ok(Self {
            db,
            txn: Some(txn),
            limits,
            dirty: 0,
            bytes_since_commit: 0,
            last_commit: Instant::now(),
        })
    }

    pub fn get(&self, rel: &str) -> Result<Option<FileRec>> {
        let txn = self.txn.as_ref().context("write txn gone")?;
        let tbl = txn.open_table(ENTRIES).context("open entries table")?;
        match tbl.get(rel).context("cache get")? {
            None => Ok(None),
            Some(g) => decode_rec(g.value()).map(Some),
        }
    }

    pub fn put(&mut self, rel: &str, rec: &FileRec) -> Result<()> {
        let bytes = encode_rec(rec)?;
        {
            let txn = self.txn.as_mut().context("write txn gone")?;
            let mut tbl = txn.open_table(ENTRIES).context("open entries table")?;
            tbl.insert(rel, bytes.as_slice()).context("cache put")?;
        }
        self.dirty += 1;
        // Dirs carry no content; only file payload counts toward the byte
        // trigger (remove() has no size to account, so it counts ops only).
        self.bytes_since_commit = self.bytes_since_commit.saturating_add(rec.size);
        self.maybe_commit()
    }

    pub fn remove(&mut self, rel: &str) -> Result<()> {
        {
            let txn = self.txn.as_mut().context("write txn gone")?;
            let mut tbl = txn.open_table(ENTRIES).context("open entries table")?;
            tbl.remove(rel).context("cache remove")?;
        }
        self.dirty += 1;
        self.maybe_commit()
    }

    pub fn load_all(&self) -> Result<HashMap<String, FileRec>> {
        let txn = self.txn.as_ref().context("write txn gone")?;
        let tbl = txn.open_table(ENTRIES).context("open entries table")?;
        let mut map = HashMap::new();
        for kv in tbl.iter().context("iterate cache")? {
            let (k, v) = kv.context("read cache entry")?;
            map.insert(k.value().to_string(), decode_rec(v.value())?);
        }
        Ok(map)
    }

    fn maybe_commit(&mut self) -> Result<()> {
        // The time check runs on mutation, i.e. right after the (possibly
        // minutes-long) hash that produced this put — exactly when the
        // backlog of uncommitted work is largest.
        if self.dirty >= self.limits.max_ops
            || self.bytes_since_commit >= self.limits.max_bytes
            || self.last_commit.elapsed() >= self.limits.max_interval
        {
            self.flush_txn()?;
        }
        Ok(())
    }

    fn flush_txn(&mut self) -> Result<()> {
        let ops = self.dirty;
        let bytes = self.bytes_since_commit;
        let elapsed = self.last_commit.elapsed();
        let txn = self.txn.take().context("write txn gone")?;
        txn.commit().context("commit cache batch")?;
        self.txn = Some(self.db.begin_write().context("begin write")?);
        self.dirty = 0;
        self.bytes_since_commit = 0;
        self.last_commit = Instant::now();
        trace!(
            ops,
            bytes,
            elapsed_s = elapsed.as_secs_f64(),
            "cache batch committed"
        );
        Ok(())
    }

    /// Commit all pending mutations. This is the `flush()` analogue: after
    /// it returns, the mutations survive a process kill.
    pub fn commit(mut self) -> Result<()> {
        let txn = self.txn.take().context("write txn gone")?;
        txn.commit().context("commit cache")?;
        Ok(())
    }
}

/// Remove a cache path regardless of whether it is a redb file or a legacy
/// sled directory.
pub fn remove_cache_path(db_path: &Path) -> Result<()> {
    if db_path.is_dir() {
        std::fs::remove_dir_all(db_path)
            .with_context(|| format!("remove {}", db_path.display()))?;
    } else if db_path.is_file() {
        std::fs::remove_file(db_path).with_context(|| format!("remove {}", db_path.display()))?;
    }
    Ok(())
}

/// Copy an existing cache to `<parent>/girpr-cache-backup-<ts>`.
/// Works for redb files and legacy sled directories alike.
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
    if db_path.is_dir() {
        copy_dir_all(db_path, &dest)?;
    } else {
        std::fs::copy(db_path, &dest)
            .with_context(|| format!("copy {} -> {}", db_path.display(), dest.display()))?;
    }
    info!(src = %db_path.display(), dst = %dest.display(), "backup done");
    Ok(Some(dest))
}

/// Copy an existing cache to `<parent>/girpr-cache-old-<ts>` to record the
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
    if db_path.is_dir() {
        copy_dir_all(db_path, &dest)?;
    } else {
        std::fs::copy(db_path, &dest)
            .with_context(|| format!("copy {} -> {}", db_path.display(), dest.display()))?;
    }
    info!(src = %db_path.display(), dst = %dest.display(), "snapshot-old done");
    Ok(Some(dest))
}

/// Open (creating if needed) the cache at `db_path`.
///
/// `ignore_cache` backs up (when `backup_first`) and deletes the cache to
/// force a rebuild. A corrupt cache is a hard error; the message points at
/// `--ignore-cache`.
#[tracing::instrument(skip_all, fields(db = %db_path.display(), case_sensitive, ignore_cache, backup_first))]
pub fn open_db(
    db_path: &Path,
    case_sensitive: bool,
    ignore_cache: bool,
    backup_first: bool,
) -> Result<CacheDb> {
    if ignore_cache && db_path.exists() {
        if backup_first {
            backup_db(db_path)?;
        }
        remove_cache_path(db_path)?;
        info!(path = %db_path.display(), "ignore-cache removed");
    } else if backup_first && db_path.exists() {
        backup_db(db_path)?;
    }
    let db = CacheDb::open_file(db_path)?;
    CacheDb::init_meta(&db, case_sensitive)?;
    // NOTE: case_sensitive is informational only. The cache stays usable
    // across modes: in insensitive mode a disk/cached casing difference is
    // fixed to the on-disk name (disk always governs), so the same record
    // remains valid for a later sensitive run. Only genuine conflicts
    // (two live/record paths differing only by case in insensitive mode)
    // abort, detected by the callers — never here.
    Ok(CacheDb { db })
}

/// Every cached path.
pub fn load_all_records(cache: &CacheDb) -> Result<HashMap<String, FileRec>> {
    cache.load_all()
}

/// Upsert one cached path (immediately committed; bulk callers prefer
/// [`CacheWrite`]).
pub fn put_rec(cache: &CacheDb, rel: &str, rec: &FileRec) -> Result<()> {
    cache.put(rel, rec)
}

/// Fetch one cached path.
pub fn get_rec(cache: &CacheDb, rel: &str) -> Result<Option<FileRec>> {
    cache.get(rel)
}

/// Remove one cached path (immediately committed).
pub fn remove_rec(cache: &CacheDb, rel: &str) -> Result<()> {
    cache.remove(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_rec() -> FileRec {
        FileRec {
            kind: "file".into(),
            size: 42,
            mtime_ns: 123456789,
            hashes: [
                ("md5".to_string(), vec![1u8; 16]),
                ("sha256".to_string(), vec![2u8; 32]),
            ]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn codec_roundtrips_file_with_hashes() {
        let rec = file_rec();
        let back = decode_rec(&encode_rec(&rec).unwrap()).unwrap();
        assert_eq!(rec, back);
    }

    #[test]
    fn codec_roundtrips_dir_without_hashes() {
        let rec = FileRec::dir();
        let back = decode_rec(&encode_rec(&rec).unwrap()).unwrap();
        assert_eq!(rec, back);
    }

    #[test]
    fn codec_preserves_unknown_algorithms() {
        // Forward compat: a future `--hash blake3` row must survive a
        // roundtrip through this binary, which never drops opaque entries.
        let mut rec = FileRec {
            kind: "file".into(),
            size: 7,
            mtime_ns: 9,
            hashes: [("blake3".to_string(), vec![7u8; 32])]
                .into_iter()
                .collect(),
        };
        rec.hashes.insert("md5".to_string(), vec![3u8; 16]);
        let back = decode_rec(&encode_rec(&rec).unwrap()).unwrap();
        assert_eq!(rec, back);
    }

    #[test]
    fn codec_rejects_truncated_and_bad_version() {
        let good = encode_rec(&file_rec()).unwrap();
        assert!(decode_rec(&good[..good.len() - 2]).is_err());
        let mut bad = good.clone();
        bad[0] = 99;
        assert!(decode_rec(&bad).is_err());
        assert!(decode_rec(&[1u8]).is_err());
    }

    #[test]
    fn meta_codec_roundtrips() {
        for cs in [true, false] {
            let m = Meta {
                version: CACHE_VERSION,
                case_sensitive: cs,
            };
            let back = decode_meta(&encode_meta(&m)).unwrap();
            assert_eq!(back.version, CACHE_VERSION);
            assert_eq!(back.case_sensitive, cs);
        }
    }

    #[test]
    fn temp_db_put_get_remove_iter() {
        let c = CacheDb::open_temp(true).unwrap();
        assert!(c.get("a").unwrap().is_none());
        assert!(c.load_all().unwrap().is_empty());
        c.put("a", &file_rec()).unwrap();
        c.put("b", &FileRec::dir()).unwrap();
        assert_eq!(c.get("a").unwrap().unwrap(), file_rec());
        assert_eq!(c.load_all().unwrap().len(), 2);
        c.remove("a").unwrap();
        assert!(c.get("a").unwrap().is_none());
        // Meta flag update across modes keeps rows.
        let c2 = CacheDb::open_temp(false).unwrap();
        c2.put("x", &file_rec()).unwrap();
        assert_eq!(c2.load_all().unwrap().len(), 1);
    }

    #[test]
    fn write_batch_sees_own_writes_and_commits() {
        let c = CacheDb::open_temp(true).unwrap();
        {
            let mut w = c.begin_write().unwrap();
            w.put("a", &file_rec()).unwrap();
            assert_eq!(w.get("a").unwrap().unwrap(), file_rec());
            w.remove("a").unwrap();
            assert!(w.get("a").unwrap().is_none());
            w.put("b", &file_rec()).unwrap();
            w.commit().unwrap();
        }
        assert!(c.get("a").unwrap().is_none());
        assert_eq!(c.get("b").unwrap().unwrap(), file_rec());
    }

    fn sized_rec(size: u64) -> FileRec {
        FileRec {
            kind: "file".into(),
            size,
            mtime_ns: 0,
            hashes: HashMap::new(),
        }
    }

    fn quiet_limits() -> CommitLimits {
        CommitLimits {
            max_ops: usize::MAX,
            max_bytes: u64::MAX,
            max_interval: Duration::from_secs(3600),
        }
    }

    #[test]
    fn batch_commits_on_op_count() {
        let c = CacheDb::open_temp(true).unwrap();
        let limits = CommitLimits {
            max_ops: 2,
            ..quiet_limits()
        };
        let mut w = c.begin_write_with(limits).unwrap();
        w.put("a", &sized_rec(1)).unwrap();
        assert_eq!(w.dirty, 1);
        w.put("b", &sized_rec(1)).unwrap();
        assert_eq!(w.dirty, 0, "op-count trigger committed");
        // Rows stay visible across the internal reopen.
        assert!(w.get("a").unwrap().is_some());
        w.commit().unwrap();
        assert_eq!(c.get("b").unwrap().unwrap().size, 1);
    }

    #[test]
    fn batch_commits_on_byte_count() {
        let c = CacheDb::open_temp(true).unwrap();
        let limits = CommitLimits {
            max_bytes: 10,
            ..quiet_limits()
        };
        let mut w = c.begin_write_with(limits).unwrap();
        w.put("a", &sized_rec(6)).unwrap();
        assert_eq!((w.dirty, w.bytes_since_commit), (1, 6));
        w.put("b", &sized_rec(5)).unwrap();
        assert_eq!(
            (w.dirty, w.bytes_since_commit),
            (0, 0),
            "byte-count trigger committed"
        );
        assert!(w.get("a").unwrap().is_some());
        w.commit().unwrap();
        assert_eq!(c.load_all().unwrap().len(), 2);
    }

    #[test]
    fn batch_commits_on_interval() {
        let c = CacheDb::open_temp(true).unwrap();
        let limits = CommitLimits {
            max_interval: Duration::from_millis(50),
            ..quiet_limits()
        };
        let mut w = c.begin_write_with(limits).unwrap();
        w.put("a", &sized_rec(1)).unwrap();
        assert_eq!(w.dirty, 1);
        // Simulate a long hash of a huge file between mutations.
        std::thread::sleep(Duration::from_millis(150));
        w.put("b", &sized_rec(1)).unwrap();
        assert_eq!(w.dirty, 0, "interval trigger committed");
        assert!(w.get("a").unwrap().is_some());
        w.commit().unwrap();
        assert_eq!(c.load_all().unwrap().len(), 2);
    }
}
