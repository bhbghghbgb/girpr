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
use redb::{
    Database, ReadOnlyDatabase, ReadableDatabase, ReadableTable, TableDefinition,
    backends::InMemoryBackend,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tracing::{info, trace, warn};

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

/// How a cache file should be opened.
///
/// A read/write open is a *participant* in the run: it may create the file,
/// back it up, and rewrite `meta`. A read-only open is an *observer*: it
/// requires the file to already be a valid current-version cache and never
/// opens it for writing, so the file comes out byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheOpen {
    /// Create if missing, and optionally back up and/or rebuild first.
    ///
    /// `ignore_cache` backs up (when `backup_first`) and deletes the file so it
    /// is rebuilt from scratch. `backup_first` copies it aside to
    /// `girpr-cache-backup-<ts>` before opening.
    ReadWrite {
        ignore_cache: bool,
        backup_first: bool,
    },
    /// Open the existing file for reading only: no create-if-missing, no
    /// backup, no `meta` rewrite.
    ///
    /// A read/write open silently rewrites `meta` when the recorded
    /// `case_sensitive` flag disagrees with the mode the run is in. That flag is
    /// documented as informational, so a read-only open cannot act on the
    /// disagreement — it warns and moves on rather than refusing a cache that
    /// is perfectly usable.
    ReadOnly,
}

/// The redb handle a [`CacheDb`] wraps.
///
/// `ReadOnlyDatabase` is a distinct type rather than a flag on `Database`, so
/// the two have to be matched on. Read-only handles *share* the file with each
/// other; only a read/write handle excludes everyone, which is what makes the
/// one-handle-per-file rule below about writers.
enum Handle {
    Writable(Database),
    ReadOnly(ReadOnlyDatabase),
}

impl Handle {
    fn writable(&self) -> Option<&Database> {
        match self {
            Handle::Writable(d) => Some(d),
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

/// An open redb cache file, read/write or read-only.
///
/// Note: redb takes a file lock per *writable* database, so two read/write
/// `CacheDb` handles on the **same** file cannot be alive at once (the second
/// open fails with "Database already open"). All callers open, use, and drop
/// sequentially — never hold two writable handles on one path. Read-only
/// handles do not conflict with each other.
pub struct CacheDb {
    db: Handle,
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
        Ok(Self {
            db: Handle::Writable(db),
        })
    }

    /// Open an existing record file read-only: no writes, no meta creation,
    /// no case-mode rewrite — but the version is still validated.
    ///
    /// A record is an input, not a participant, so it has no case mode of its
    /// own and no `meta` reconciliation to do: the only thing worth checking is
    /// that the file is a cache this build can read.
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
        let db = open_read_only(db_path).with_context(|| {
            format!(
                "open record {} (corrupt? use --ignore-cache to rebuild or sled2redb to convert)",
                db_path.display()
            )
        })?;
        let meta = read_meta(&db)
            .with_context(|| format!("read record meta {}", db_path.display()))?
            .ok_or_else(|| {
                anyhow::anyhow!("record {} has no meta (corrupt?)", db_path.display())
            })?;
        require_current_version(meta.version, db_path, "record")?;
        Ok(Self {
            db: Handle::ReadOnly(db),
        })
    }

    /// True when this handle cannot write, i.e. it was opened
    /// [`CacheOpen::ReadOnly`].
    pub fn is_read_only(&self) -> bool {
        matches!(self.db, Handle::ReadOnly(_))
    }

    /// The read/write handle, or an error naming the path this cache is being
    /// observed through. Every mutating method funnels through here, so a
    /// read-only cache cannot be written by accident.
    fn writable(&self) -> Result<&Database> {
        self.db.writable().context("cache is open read-only")
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
    /// periodic auto-commit) for mutations to become durable. Fails on a
    /// read-only cache.
    pub fn begin_write(&self) -> Result<CacheWrite<'_>> {
        CacheWrite::new(self.writable()?)
    }

    /// Start a batched write handle with custom commit thresholds (see
    /// [`CommitLimits`]). Fails on a read-only cache.
    pub fn begin_write_with(&self, limits: CommitLimits) -> Result<CacheWrite<'_>> {
        CacheWrite::with_limits(self.writable()?, limits)
    }
}

/// Batched writes over one cache: mutations accumulate in a single write
/// transaction and are committed periodically — every `max_ops` operations,
/// every `max_bytes` of `put` file content, or every `max_interval` of wall
/// time, whichever comes first — plus once more at
/// [`commit`](CacheWrite::commit). Reads see the handle's own pending writes, so a
/// write is visible to the writer before it is visible to anyone else.
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

/// Open an existing cache file for reading, without ever opening it for
/// writing.
///
/// `ReadOnlyDatabase` is the reason a read-only run can promise the file is
/// untouched: the backend has no write path at all, so there is nothing to
/// accidentally commit through, and open-time recovery cannot rewrite pages.
fn open_read_only(db_path: &Path) -> Result<ReadOnlyDatabase> {
    ReadOnlyDatabase::open(db_path).with_context(|| {
        format!(
            "open redb cache {} (corrupt? use --ignore-cache)",
            db_path.display()
        )
    })
}

/// The `meta` row, or `None` when the table or the key is absent.
///
/// Absence is reported rather than invented: a read-only open cannot create
/// the row a read/write open would, so the caller has to decide what a
/// meta-less cache means.
fn read_meta(db: &impl ReadableDatabase) -> Result<Option<Meta>> {
    let txn = db.begin_read().context("begin read (meta)")?;
    let tbl = match txn.open_table(META_TBL) {
        Ok(t) => t,
        Err(e) if is_table_missing(&e) => return Ok(None),
        Err(e) => bail!("open meta table: {e:#}"),
    };
    let g = tbl.get(META_KEY).context("read meta")?;
    match g {
        None => Ok(None),
        Some(g) => Ok(Some(
            decode_meta(g.value()).context("parse cache meta (corrupt?)")?,
        )),
    }
}

/// Refuse a cache this build cannot interpret, whatever the open mode: a wrong
/// version means every row would be decoded under the wrong assumptions.
fn require_current_version(version: u32, db_path: &Path, label: &str) -> Result<()> {
    if version != CACHE_VERSION {
        bail!(
            "unsupported {label} version {} in {} (want {CACHE_VERSION}; use --ignore-cache to rebuild or sled2redb to convert)",
            version,
            db_path.display()
        );
    }
    Ok(())
}

/// Open (creating if needed) the cache at `db_path`.
///
/// `ignore_cache` backs up (when `backup_first`) and deletes the cache to
/// force a rebuild. A corrupt cache is a hard error; the message points at
/// `--ignore-cache`.
///
/// [`CacheOpen::ReadOnly`] inverts every side effect: the file must exist, is
/// never opened for writing, and is left byte-identical. It still validates the
/// schema version, because a cache this build cannot decode is useless
/// whatever the mode. A `case_sensitive` flag disagreeing with the run's mode
/// only warns — a read/write open would silently rewrite `meta` to match, and
/// that flag is informational (see [`Meta::case_sensitive`]).
#[tracing::instrument(skip_all, fields(db = %db_path.display(), mode = ?mode))]
pub fn open_db(db_path: &Path, case_sensitive: bool, mode: CacheOpen) -> Result<CacheDb> {
    match mode {
        CacheOpen::ReadWrite {
            ignore_cache,
            backup_first,
        } => {
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
            Ok(CacheDb {
                db: Handle::Writable(db),
            })
        }
        CacheOpen::ReadOnly => {
            if !db_path.exists() {
                bail!("cache {} not found", db_path.display());
            }
            let db = open_read_only(db_path)?;
            let Some(meta) = read_meta(&db)? else {
                bail!(
                    "cache {} has no meta (corrupt? use --ignore-cache)",
                    db_path.display()
                );
            };
            require_current_version(meta.version, db_path, "cache")?;
            if meta.case_sensitive != case_sensitive {
                // A read/write open would quietly rewrite meta to match. It is
                // documented as informational, so the disagreement is not a
                // reason to refuse a cache that reads fine.
                warn!(
                    path = %db_path.display(),
                    recorded = meta.case_sensitive,
                    requested = case_sensitive,
                    "cache case mode differs; read-only open leaves meta as recorded"
                );
            }
            Ok(CacheDb {
                db: Handle::ReadOnly(db),
            })
        }
    }
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

    // -- read-only opens ---------------------------------------------------
    //
    // Scoped to what this crate decides. That redb hands out a handle with no
    // write path, and that two read-only handles share the file, are the
    // library's guarantees, not ours: nothing here asserts them.

    /// A scratch directory holding one real cache file, plus its path.
    fn seeded(tag: &str, case_sensitive: bool) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("girsync_ro_{}_{}", std::process::id(), tag));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(CACHE_PREFIX);
        let c = open_db(
            &p,
            case_sensitive,
            CacheOpen::ReadWrite {
                ignore_cache: false,
                backup_first: false,
            },
        )
        .unwrap();
        c.put("a.txt", &file_rec()).unwrap();
        drop(c);
        (dir, p)
    }

    /// Every mutating entry point must refuse a read-only cache, so an observer
    /// run cannot corrupt the file it is observing by accident.
    #[test]
    fn read_only_cache_refuses_every_write() {
        let (dir, p) = seeded("nowrite", true);
        let c = open_db(&p, true, CacheOpen::ReadOnly).unwrap();
        assert!(c.is_read_only(), "reports its mode");

        // Reads still work, and see the same rows a read/write open would.
        assert_eq!(c.get("a.txt").unwrap(), Some(file_rec()));
        assert_eq!(c.load_all().unwrap().len(), 1);

        let refusals: Vec<Result<()>> = vec![
            c.put("b.txt", &file_rec()),
            c.remove("a.txt"),
            c.begin_write().map(|_| ()),
            c.begin_write_with(CommitLimits::default()).map(|_| ()),
        ];
        for r in refusals {
            let err = r.expect_err("a read-only cache must refuse writes");
            assert!(
                format!("{:#}", err).contains("read-only"),
                "error must name the cause: {:#}",
                err
            );
        }
        drop(c);

        // The rows are exactly as they were.
        let after = open_db(
            &p,
            true,
            CacheOpen::ReadWrite {
                ignore_cache: false,
                backup_first: false,
            },
        )
        .unwrap();
        let all = after.load_all().unwrap();
        assert_eq!(all.len(), 1);
        assert!(all.contains_key("a.txt"));
        assert!(!all.contains_key("b.txt"), "the refused put did not land");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A read/write open rewrites `meta` when the recorded case mode disagrees
    /// with the run's. A read-only open cannot, and the flag is informational,
    /// so it warns and hands back a usable cache.
    #[test]
    fn read_only_open_tolerates_a_case_mode_mismatch() {
        let (dir, p) = seeded("casemix", true);
        // Written above as case-sensitive; read it back in insensitive mode.
        let c = open_db(&p, false, CacheOpen::ReadOnly).unwrap();
        assert_eq!(c.get("a.txt").unwrap(), Some(file_rec()));
        drop(c);

        // meta is left as recorded, not rewritten to match the caller.
        let after = open_db(
            &p,
            true,
            CacheOpen::ReadWrite {
                ignore_cache: false,
                backup_first: false,
            },
        )
        .unwrap();
        let meta = read_meta(&after.db).unwrap().unwrap();
        assert!(
            meta.case_sensitive,
            "a read-only open must not rewrite the recorded case mode"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A read-only open refuses anything it cannot faithfully interpret: no
    /// file to read, no `meta` to check the version against, or a version this
    /// build would decode under the wrong assumptions.
    #[test]
    fn read_only_open_requires_a_readable_current_version_cache() {
        let dir = std::env::temp_dir().join(format!("girsync_ro_{}_gates", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let ro = |p: &Path| open_db(p, true, CacheOpen::ReadOnly);

        // No file: there is nothing to observe, and creating one would be a write.
        assert!(ro(&dir.join("absent")).is_err(), "missing file is refused");

        // A file that is not a cache at all.
        let junk = dir.join("junk");
        std::fs::write(&junk, b"not a redb database").unwrap();
        assert!(ro(&junk).is_err(), "garbage is refused");

        // A redb file with no `meta` row: nothing to validate the schema against.
        let bare = dir.join("bare");
        {
            let db = Database::create(&bare).unwrap();
            let txn = db.begin_write().unwrap();
            {
                let _ = txn.open_table(ENTRIES).unwrap();
            }
            txn.commit().unwrap();
        }
        assert!(ro(&bare).is_err(), "a meta-less cache is refused");

        // A well-formed cache stamped with a version this build cannot read.
        let future = dir.join("future");
        {
            let db = Database::create(&future).unwrap();
            let txn = db.begin_write().unwrap();
            {
                let mut m = txn.open_table(META_TBL).unwrap();
                m.insert(
                    META_KEY,
                    encode_meta(&Meta {
                        version: CACHE_VERSION + 1,
                        case_sensitive: true,
                    })
                    .as_slice(),
                )
                .unwrap();
            }
            txn.commit().unwrap();
        }
        assert!(ro(&future).is_err(), "a future version is refused");

        std::fs::remove_dir_all(&dir).ok();
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
