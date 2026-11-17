//! [`CacheDb`] itself: the read/write-or-read-only handle and all of its
//! methods.

use anyhow::{Context, Result, bail};
use redb::{
    Database, DatabaseError, ReadOnlyTable, ReadTransaction, ReadableDatabase, ReadableTable,
    ReadableTableMetadata, TableHandle, backends::InMemoryBackend,
};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

use super::{Handle, SharedCache, memory_registry, redb_builder};
use crate::cache::codec::{decode_meta, decode_rec, encode_meta};
use crate::cache::open::{
    is_table_missing, open_read_only, read_meta, read_meta_bytes, require_current_version,
};
use crate::cache::schema::{CACHE_VERSION, FileRec, Meta};
use crate::cache::write::{CacheWrite, CommitLimits, CommitPolicy};
use crate::cache::{ENTRIES, META_KEY, META_TBL};

/// The `meta` row, decoded, in the read/write path's words.
///
/// A corrupt row is one `--ignore-cache` rebuilds, so the reader is told so.
/// The read-only path words the same failure differently (it cannot offer the
/// remedy), and keeps its own wording.
fn decode_meta_for_write(raw: &[u8]) -> Result<Meta> {
    decode_meta(raw).context("parse cache meta (corrupt? use --ignore-cache)")
}

/// What the preflight found about the cache, read without the write lock.
///
/// Both fields matter, and for different reasons. The row says whether the
/// open has anything to write; the entries table says whether the cache is
/// *whole*. The fast path skips the write transaction that would otherwise
/// create a missing table, so a cache that is missing one is not one the
/// fast path may leave alone.
struct Preflight {
    /// The `meta` row, decoded. `None` means the row is not there — a fresh
    /// cache, not a corrupt one — and the write path creates the tables and
    /// the row.
    meta: Option<Meta>,
    /// Whether the entries table is there.
    entries_present: bool,
}

/// Read the cache's `meta` row and check that it is whole, without taking
/// the write lock.
///
/// One read transaction, so the row and the tables come from the same
/// snapshot: a cache that changed under the read cannot be half-old and
/// half-new here.
///
/// A missing table is a fresh cache, not a corrupt one — the same rule the
/// read-only path follows — so the preflight reports absence instead of
/// failing on it.
///
/// **The entries-table open here is not one of the counted reads.** It opens
/// the table to learn whether it is there and reads no row, and it runs on a
/// raw [`Database`] before any [`CacheDb`] handle exists — so there is no
/// counter here for it to move. The counted reads are the ones that go
/// through `begin_entries_read`, and that is the whole of it: a counted read
/// need not fetch a row's value — `entry_count` fetches nothing at all,
/// `load_keys` fetches the keys alone, and `visit_keys` fetches them one at a
/// time.
fn preflight(db: &Database) -> Result<Preflight> {
    let txn = db.begin_read().context("begin read (meta)")?;
    let entries_present = match txn.open_table(ENTRIES) {
        Ok(_) => true,
        Err(e) if is_table_missing(&e) => false,
        Err(e) => bail!("open entries table: {e:#}"),
    };
    let meta = match read_meta_bytes(&txn)? {
        None => None,
        Some(raw) => Some(decode_meta_for_write(&raw)?),
    };
    Ok(Preflight {
        meta,
        entries_present,
    })
}

/// Refuse a cache this build cannot interpret, in the read/write path's words.
///
/// The read-only path words the same refusal with the file's path in it (see
/// [`require_current_version`]); this is the read/write open's wording, and it
/// does not change.
fn require_current_version_for_write(version: u32) -> Result<()> {
    if version != CACHE_VERSION {
        bail!(
            "unsupported cache version {} (want {CACHE_VERSION}; use --ignore-cache to rebuild or sled2redb to convert)",
            version
        );
    }
    Ok(())
}

/// One row, decoded, from an open `entries` table.
///
/// The same decoding for a single row and for a batch of them, so a caller
/// cannot get a different answer from [`CacheDb::get`] and
/// [`CacheDb::get_many`] by accident: the batch is not a second reading of
/// the row, it is the same reading of several rows.
fn read_row(
    tbl: &ReadOnlyTable<&'static str, &'static [u8]>,
    rel: &str,
) -> Result<Option<FileRec>> {
    match tbl.get(rel).context("cache get")? {
        None => Ok(None),
        Some(g) => decode_rec(g.value()).map(Some),
    }
}

/// Every key in an open `entries` table, one at a time.
///
/// The same walk for a read handle and for a write handle, so a caller cannot
/// get a different answer from [`CacheDb::visit_keys`] and
/// [`CacheWrite::visit_keys`](crate::cache::CacheWrite::visit_keys) by
/// accident: the batch is not a second reading of the table, it is the same
/// reading of it.
///
/// **The key is borrowed, not copied.** It lives only as long as the call, so
/// a caller that wants to keep one has to copy it — which is the point: the
/// streaming read exists for the caller that does not want to keep any.
pub(in crate::cache) fn visit_entry_keys(
    tbl: &impl ReadableTable<&'static str, &'static [u8]>,
    mut f: impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    for kv in tbl.iter().context("iterate cache")? {
        let (k, _v) = kv.context("read cache entry")?;
        f(k.value())?;
    }
    Ok(())
}

/// An open redb cache file, read/write or read-only.
///
/// The cache's own lock is a sidecar this crate takes,
/// not redb's byte-range lock on the database file. Two read/write `CacheDb`
/// handles on the same cache therefore cannot be alive at once: the second
/// open is refused at once while the first holds the sidecar. Read-only
/// handles take it shared and do not conflict with each other, but are refused
/// by a writer. All callers open, use, and drop sequentially — never hold two
/// writable handles on one path.
///
/// redb still locks each writable database itself, so the two mechanisms agree
/// on who may write: the sidecar gives the fail-fast answer, and redb's own
/// lock keeps handles from different builds that do not know the sidecar from
/// writing one file at once.
///
/// **A filesystem that cannot lock a file has none of that.** Both the sidecar
/// and redb skip their locks there, so the exclusion is not there either and
/// another handle can be open on the same file. It is the filesystem the crate
/// already knows: a copy published there carries no record of its own, the
/// record being a lock as well.
pub struct CacheDb {
    pub(super) db: Handle,
    /// The cache's sidecar lock, held for as long as this handle is alive.
    ///
    /// This is the run's "in progress" marker for the folder: another run
    /// opening the same cache is refused while it is held, and a bound that
    /// finds it held leaves the folder alone. Dropping the handle drops the
    /// guard and releases the lock. `None` on a filesystem that cannot lock a
    /// file, or for an in-memory cache that never touches one.
    ///
    /// Held for its `Drop` alone — nothing reads it.
    _lock: Option<File>,
    /// The entries-table reads counted into the counter this handle holds.
    ///
    /// A test seam, and the reason the handle carries it rather than the
    /// caller: what a test needs to know is whether a read was **batched**,
    /// and the only honest answer is the number of transactions the read
    /// cost. A count is a fact about the code; a duration would be a
    /// measurement of the machine the code ran on.
    ///
    /// The count is the **entries table's** reads and nothing else: a read
    /// of another table does not come through [`Self::begin_entries_read`],
    /// so it cannot move a number a test has pinned.
    ///
    /// **Which counter this field holds.** On the shared (test) backend it
    /// is the cache's own — the counter `SharedCache::reads` names, shared
    /// with every other handle on the path. A file-backed cache has none to
    /// share, so this handle starts a fresh one; what a fresh count means
    /// for the cache is `with_handle`'s to say.
    #[cfg(test)]
    reads: Arc<AtomicU64>,
}

impl CacheDb {
    /// Wrap a handle, with the read counter a fresh handle starts at.
    ///
    /// **A fresh counter is the cache's count for a file-backed cache.**
    /// redb locks a writable database exclusively, so the read/write handle
    /// on a cache file excludes every other handle on that file — no
    /// read-only observer can be opened beside it — and the count that
    /// handle keeps is therefore the count for the cache: there is no second
    /// reader whose reads it could be missing. Read-only handles are the
    /// other case: they share the file with each other, so each of those
    /// keeps its own count and reports only its own reads.
    ///
    /// **The fresh counter is fresh per open.** It starts at zero every
    /// time, so the count is the reads this handle has made since it opened,
    /// and not the reads an earlier open made — the shared backend's counter
    /// would carry those.
    ///
    /// **The exclusion is the lock's, and the lock may be absent.** redb
    /// skips the lock entirely on a filesystem that cannot lock a file, so
    /// there another handle can be open on the same file, and the count is
    /// then this handle's own reads and nothing more.
    ///
    /// The shared (test) backend is the one case where several handles are
    /// alive on one cache at once, and it hands them one counter between
    /// them rather than one each — see `with_shared_reads`.
    fn with_handle(db: Handle) -> Self {
        Self::with_handle_and_lock(db, None)
    }

    /// Wrap a handle together with the sidecar lock guard it must keep alive.
    fn with_handle_and_lock(db: Handle, lock: Option<File>) -> Self {
        Self {
            db,
            _lock: lock,
            #[cfg(test)]
            reads: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Wrap a handle onto a cache whose reads are counted across handles.
    ///
    /// The shared (test) backend hands every open of one path the same
    /// counter, so the count is a property of the **cache** rather than of
    /// the handle that happens to be holding it — and a handle opened after
    /// a run can say what that run's handle did.
    ///
    /// A file-backed cache needs none of that: redb locks a writable
    /// database exclusively, so the read/write handle on a cache file is the
    /// only handle on it, and that handle's own fresh counter is already the
    /// cache's count — for the reads it has made since it opened, and where
    /// the filesystem can lock a file at all (see `with_handle`).
    #[cfg(test)]
    fn with_shared_reads(db: Handle, reads: Arc<AtomicU64>) -> Self {
        Self {
            db,
            _lock: None,
            reads,
        }
    }

    /// Wrap an already-open handle and the sidecar lock this run holds on it.
    ///
    /// The only place a [`Handle::Writable`] is built, so "who may create one"
    /// has one answer rather than one per caller. `lock` is `None` for a
    /// filesystem that cannot lock a file and for in-memory caches.
    pub(in crate::cache) fn open_writable_locked(db: Database, lock: Option<File>) -> Self {
        Self::with_handle_and_lock(Handle::Writable(Arc::new(db)), lock)
    }

    /// Wrap an observer handle and the shared sidecar lock this run holds.
    pub(in crate::cache) fn open_read_only_locked(
        db: redb::ReadOnlyDatabase,
        lock: Option<File>,
    ) -> Self {
        Self::with_handle_and_lock(Handle::ReadOnly(db), lock)
    }

    /// Bring the cache's `meta` row in line with the run's case mode.
    ///
    /// The row is read **before** the write lock is taken. A cache that is
    /// already correct — the common case — has nothing to write, and the
    /// write lock plus redb's page setup is the open's dominant cost on a
    /// slow disk. So the row is read first, and the write transaction is
    /// begun only when the row has to change.
    ///
    /// The read is an optimisation, not a second source of truth: the write
    /// path below re-reads the row inside its own transaction, so a row
    /// that changed in between is still reconciled there.
    pub(in crate::cache) fn init_meta(db: &Database, case_sensitive: bool) -> Result<()> {
        match preflight(db)? {
            // No row at all: a fresh cache. The write path below creates
            // both tables and writes the row.
            Preflight { meta: None, .. } => {}
            Preflight {
                meta: Some(meta),
                entries_present,
            } => {
                require_current_version_for_write(meta.version)?;
                if entries_present && meta.case_sensitive == case_sensitive {
                    // The row already says what this run needs it to say, and
                    // the cache is whole, so there is nothing to write. The
                    // file is left exactly as it was: no write transaction,
                    // no commit.
                    return Ok(());
                }
                // The case mode disagrees, or the cache is missing a table:
                // the write path below fixes both.
            }
        }

        // The policy is resolved when the write starts, like every other
        // write's. This one is a single transaction that commits once, so it
        // has no thresholds to batch to and uses only the durability.
        let policy = CommitPolicy::from_env();
        let mut txn = db.begin_write().context("begin write (meta)")?;
        txn.set_durability(policy.durability())
            .context("set cache write durability")?;
        // A cache with a `meta` row but no entries table is not whole, and
        // the write path below is what repairs it. Opening the table creates
        // it — and a table created in a transaction that is dropped is a
        // table that was never there — so the commit decision has to know
        // the table was missing.
        let entries_missing = !txn
            .list_tables()
            .context("list tables")?
            .any(|t| t.name() == ENTRIES.name());
        let mut changed = entries_missing;
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
                    changed = true;
                }
                Some(raw) => {
                    let m = decode_meta_for_write(&raw)?;
                    require_current_version_for_write(m.version)?;
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
                        changed = true;
                    }
                }
            }
        }
        // Skip the commit when nothing changed: a write-free commit buys
        // nothing, and on this HDD the flush alone costs 10-100 ms per open.
        // Behaviour for real runs is unchanged — meta still commits whenever
        // the row is inserted or rewritten, and so does a table that had to
        // be created, because a table created in a dropped transaction is a
        // table that was never there.
        if changed {
            txn.commit().context("commit meta")?;
        } else {
            drop(txn);
        }
        Ok(())
    }

    pub(in crate::cache) fn open_file(db_path: &Path) -> Result<Database> {
        if db_path.is_dir() {
            bail!(
                "cache {} is a directory (sled format): use --ignore-cache to rebuild or sled2redb to convert",
                db_path.display()
            );
        }
        match redb_builder().create(db_path) {
            Ok(db) => Ok(db),
            // A lock is not corruption, and `--ignore-cache` would *delete* the
            // file another handle holds. Name the likely cause instead: two
            // spellings of one root that the pre-open check did not catch, or
            // another girsync process on the same cache.
            Err(e @ DatabaseError::DatabaseAlreadyOpen) => {
                Err(anyhow::Error::new(e).context(format!(
                    "open redb cache {}: the cache is already open by another handle; \
                     is src or dst the same folder, or another girsync run active?",
                    db_path.display()
                )))
            }
            Err(e) => Err(anyhow::Error::new(e).context(format!(
                "open redb cache {} (corrupt? use --ignore-cache)",
                db_path.display()
            ))),
        }
    }

    /// In-memory cache for `--dry-run` fallbacks and unit tests. Never
    /// touches the filesystem.
    pub fn open_temp(case_sensitive: bool) -> Result<Self> {
        let db = redb_builder()
            .create_with_backend(InMemoryBackend::new())
            .context("open temp cache")?;
        Self::init_meta(&db, case_sensitive)?;
        Ok(Self::with_handle(Handle::Writable(Arc::new(db))))
    }

    /// The test-only in-memory cache for `db_path`, created on first use and
    /// **shared by every later open of the same path in this process**.
    ///
    /// [`open_temp`](Self::open_temp) is deliberately not shared: it serves a
    /// single dry-run fallback, and a fresh empty DB is exactly what that answer
    /// describes. A test suite switching `TestCacheBackend` to `Memory` needs the
    /// other property instead — the rows a command wrote must still be there for
    /// the next command, because that is what a file cache does. Keying the
    /// registry by the cache path gives that without touching the filesystem, so
    /// the assertions (which read rows) stay exactly what they were.
    ///
    /// Only ever reached through [`super::TestCacheBackend::Memory`] or
    /// `GIRSYNC_TEST_CACHE=memory`; production opens files.
    pub fn open_memory_cache(db_path: &Path, case_sensitive: bool) -> Result<Self> {
        let shared = {
            let mut caches = memory_registry().lock().unwrap_or_else(|e| e.into_inner());
            match caches.get(db_path) {
                Some(shared) => shared.clone(),
                None => {
                    let db = redb_builder()
                        .create_with_backend(InMemoryBackend::new())
                        .context("open shared temp cache")?;
                    Self::init_meta(&db, case_sensitive)?;
                    let shared = SharedCache {
                        db: Arc::new(db),
                        #[cfg(test)]
                        reads: Arc::new(AtomicU64::new(0)),
                    };
                    caches.insert(db_path.to_path_buf(), shared.clone());
                    shared
                }
            }
        };
        // The counter belongs to the cache, so every handle on this path
        // counts into the same one.
        #[cfg(test)]
        let handle =
            Self::with_shared_reads(Handle::Writable(shared.db.clone()), shared.reads.clone());
        #[cfg(not(test))]
        let handle = Self::with_handle(Handle::Writable(shared.db.clone()));
        Ok(handle)
    }

    /// The shared in-memory cache at `db_path`, if a run in this process opened
    /// one. `None` means the path was served from a file (or never opened), so a
    /// caller falls back to the file cache.
    ///
    /// The handle it returns shares the cache's read counter, so it can also
    /// say how many reads the cache has served — including the ones a run made
    /// before this handle existed.
    pub fn memory_cache_at(db_path: &Path) -> Option<Self> {
        let shared = {
            let caches = memory_registry().lock().unwrap_or_else(|e| e.into_inner());
            caches.get(db_path).cloned()
        };
        #[cfg(test)]
        let handle =
            shared.map(|shared| Self::with_shared_reads(Handle::Writable(shared.db), shared.reads));
        #[cfg(not(test))]
        let handle = shared.map(|shared| Self::with_handle(Handle::Writable(shared.db)));
        handle
    }

    /// Forget the shared in-memory cache at `db_path`, so the next open is cold.
    /// Returns whether one was there.
    ///
    /// This is the memory counterpart of deleting `girpr-cache`: a test that
    /// compares a cold run against a cold run has to do it, or the second run
    /// finds the first one's rows.
    pub fn forget_memory_cache(db_path: &Path) -> bool {
        memory_registry()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(db_path)
            .is_some()
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
        Ok(Self::with_handle(Handle::ReadOnly(db)))
    }

    /// True when this handle cannot write, i.e. it was opened
    /// [`CacheOpen::ReadOnly`](crate::cache::CacheOpen::ReadOnly).
    pub fn is_read_only(&self) -> bool {
        matches!(self.db, Handle::ReadOnly(_))
    }

    /// The read/write handle, or an error naming the path this cache is being
    /// observed through. Every mutating method funnels through here, so a
    /// read-only cache cannot be written by accident.
    fn writable(&self) -> Result<&Database> {
        self.db.writable().context("cache is open read-only")
    }

    /// Begin a read transaction over the entries table, and count it.
    ///
    /// Every read of the entries table made through a read handle goes through
    /// here, so the count is the number of transactions those reads cost —
    /// which is what tells a batched read from a per-key one.
    ///
    /// A read of **another** table does not come here, and so does not move
    /// the count: the number says what the entries table cost, and nothing
    /// else can change it by accident. Neither does the preflight's check
    /// that the entries table is *there*: it reads no row, and it runs
    /// before any handle exists to count it.
    ///
    /// The batched write handle reads the same table without coming here —
    /// its reads run in its own write transaction — so they are not counted.
    /// The count is what the read handles cost.
    fn begin_entries_read(&self) -> Result<ReadTransaction> {
        #[cfg(test)]
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.db.begin_read().context("begin read")
    }

    /// How many entries-table reads this handle has counted.
    ///
    /// **On the shared (test) backend the count belongs to the cache, not to
    /// the handle that reads through it**: the shared in-memory backend hands
    /// every handle on one path the same counter, so a handle opened after a
    /// run can say what that run's handle did. A file-backed cache reaches the
    /// same answer without sharing anything — redb locks a writable database
    /// exclusively, so the read/write handle on a cache file excludes every
    /// other handle on that file and its own count is the cache's count.
    /// The read-only handles on a file share the file with each other, so
    /// each of those reports only its own reads.
    ///
    /// **Two things bound that answer.** A file-backed counter is **fresh
    /// per open**: the handle starts at zero every time, so the number is the
    /// reads this handle has made since it opened and not the reads an
    /// earlier open made, which the shared counter would carry. And the
    /// exclusion is **the lock's**: redb skips the lock entirely on a
    /// filesystem that cannot lock a file, so there another handle can be
    /// open on the same file, and the count is then this handle's own reads
    /// and nothing more.
    ///
    /// A test seam, for the reason the counter exists at all: a read that
    /// costs one transaction whatever its length is a different thing from a
    /// read that costs one per key, and only a count says which one a caller
    /// got. It counts the entries table's reads, so a read of another table
    /// leaves it where it was.
    #[cfg(test)]
    pub(crate) fn entries_read_count(&self) -> u64 {
        self.reads.load(Ordering::Relaxed)
    }

    /// Read one entry. Missing tables (fresh temp DBs) read as empty.
    pub fn get(&self, rel: &str) -> Result<Option<FileRec>> {
        let txn = self.begin_entries_read()?;
        let tbl = match txn.open_table(ENTRIES) {
            Ok(t) => t,
            Err(e) if is_table_missing(&e) => return Ok(None),
            Err(e) => bail!("open entries table: {e:#}"),
        };
        read_row(&tbl, rel)
    }

    /// Read several entries in **one** read transaction.
    ///
    /// For a caller that needs the *values* of several rows. The table is
    /// opened once and every key is read from the same snapshot, so the read
    /// costs one transaction whatever the number of keys — where a loop of
    /// [`get`](Self::get) calls costs one transaction per key.
    ///
    /// The answer is one `(key, row)` per key asked for, in the order they
    /// were asked for. A key that is not there answers `None`, exactly as
    /// [`get`](Self::get) would, and a missing table — a fresh cache, not a
    /// corrupt one — answers `None` for every key rather than failing.
    ///
    /// **The keys are copied.** A caller that only needs the rows wants
    /// [`get_many_borrowed`](Self::get_many_borrowed): it answers the same
    /// thing and borrows the keys instead of copying them.
    ///
    /// **A key-only caller wants [`load_keys`](Self::load_keys) instead.**
    /// The two answer different questions: this one decodes every row it is
    /// handed, and that one decodes nothing and answers only "which rows
    /// exist?". Asking this for the key set would decode the whole table to
    /// throw the values away.
    pub fn get_many<'a>(
        &self,
        rels: impl IntoIterator<Item = &'a str>,
    ) -> Result<Vec<(String, Option<FileRec>)>> {
        Ok(self
            .get_many_borrowed(rels)?
            .into_iter()
            .map(|(key, rec)| (key.to_string(), rec))
            .collect())
    }

    /// Read several entries in **one** read transaction, borrowing the keys.
    ///
    /// The borrowing counterpart of [`get_many`](Self::get_many): the same
    /// rows, from the same snapshot, in the same one transaction — and no
    /// `String` per key. A caller that already holds the key it asked for
    /// pays no copy for the answer.
    ///
    /// The answer is one `(key, row)` per key asked for, in the order they
    /// were asked for, with the key **borrowed** from the caller rather than
    /// copied. A key that is not there answers `None`, exactly as
    /// [`get`](Self::get) would, and a missing table — a fresh cache, not a
    /// corrupt one — answers `None` for every key rather than failing.
    pub fn get_many_borrowed<'a>(
        &self,
        rels: impl IntoIterator<Item = &'a str>,
    ) -> Result<Vec<(&'a str, Option<FileRec>)>> {
        let keys: Vec<&str> = rels.into_iter().collect();
        if keys.is_empty() {
            // Nothing was asked, so there is nothing to read: no transaction
            // and no table.
            return Ok(Vec::new());
        }
        let txn = self.begin_entries_read()?;
        let tbl = match txn.open_table(ENTRIES) {
            Ok(t) => t,
            Err(e) if is_table_missing(&e) => {
                return Ok(keys.into_iter().map(|k| (k, None)).collect());
            }
            Err(e) => bail!("open entries table: {e:#}"),
        };
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            let rec = read_row(&tbl, key)?;
            out.push((key, rec));
        }
        Ok(out)
    }

    /// Every cached path, decoding nothing.
    ///
    /// [`load_all`](Self::load_all) decodes every `FileRec` on the way — a hash
    /// map rebuilt per row, on a table that can hold one row per file in a
    /// large tree. A caller that only wants "which rows exist?" pays for all of
    /// it and then throws it away, which is the shape phase A's prune is in.
    ///
    /// The answer is the same key set, and a missing table is still an empty
    /// answer rather than an error — only the decoding is skipped. Order is
    /// redb's business and is not part of what a caller may rely on; membership
    /// is. There is deliberately no way to get a key and its value from here: a
    /// caller that needs the value wants `load_all`, and one that does not
    /// should not be able to ask for half of it by accident.
    ///
    /// **A caller that does not want to keep the keys wants
    /// [`visit_keys`](Self::visit_keys) instead**: the same keys, one at a
    /// time, and none of them copied out.
    pub fn load_keys(&self) -> Result<Vec<String>> {
        let txn = self.begin_entries_read()?;
        let tbl = match txn.open_table(ENTRIES) {
            Ok(t) => t,
            Err(e) if is_table_missing(&e) => return Ok(Vec::new()),
            Err(e) => bail!("open entries table: {e:#}"),
        };
        let mut keys = Vec::new();
        for kv in tbl.iter().context("iterate cache")? {
            let (k, _v) = kv.context("read cache entry")?;
            keys.push(k.value().to_string());
        }
        Ok(keys)
    }

    /// Every cached path, one at a time, decoding nothing.
    ///
    /// The streaming counterpart of [`load_keys`](Self::load_keys): the same
    /// keys, from the same one read transaction, and **none of them copied
    /// out**. A caller that wants to *keep* the key set wants
    /// [`load_keys`](Self::load_keys); a caller that wants to *ask a question
    /// of* it — "which of these rows are not live?" — wants this one, because
    /// the answer is usually "none of them" and a `Vec<String>` with one
    /// string per cached row is an allocation per row for that answer.
    ///
    /// The key is borrowed for the length of the call only, so a caller that
    /// wants to keep one copies it then — and copies only the ones it keeps.
    ///
    /// The answer is the same key set, and a missing table is still an empty
    /// answer rather than an error — only the collecting is skipped. Order is
    /// redb's business and is not part of what a caller may rely on;
    /// membership is.
    ///
    /// **One read transaction, whatever the number of keys.** The count the
    /// handle keeps moves by one, exactly as it does for
    /// [`load_keys`](Self::load_keys).
    pub fn visit_keys(&self, f: impl FnMut(&str) -> Result<()>) -> Result<()> {
        let txn = self.begin_entries_read()?;
        let tbl = match txn.open_table(ENTRIES) {
            Ok(t) => t,
            Err(e) if is_table_missing(&e) => return Ok(()),
            Err(e) => bail!("open entries table: {e:#}"),
        };
        visit_entry_keys(&tbl, f)
    }

    /// Number of rows in the entries table, without decoding or allocating keys.
    ///
    /// This is the raw table length, not a count of anything a caller may
    /// filter down to: a caller that needs the paths themselves wants
    /// [`load_keys`](Self::load_keys). A missing table is still an empty
    /// answer rather than an error.
    pub fn entry_count(&self) -> Result<u64> {
        let txn = self.begin_entries_read()?;
        let tbl = match txn.open_table(ENTRIES) {
            Ok(t) => t,
            Err(e) if is_table_missing(&e) => return Ok(0),
            Err(e) => bail!("open entries table: {e:#}"),
        };
        tbl.len().context("count cache entries")
    }

    /// Every cached path, with its record.
    pub fn load_all(&self) -> Result<HashMap<String, FileRec>> {
        let txn = self.begin_entries_read()?;
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
    ///
    /// The thresholds are [`CommitLimits::default()`] with the test-only
    /// `GIRSYNC_TEST_COMMIT_*` overrides applied; use
    /// [`begin_write_with`](Self::begin_write_with) to choose them explicitly.
    ///
    /// **The write's commit policy is resolved here, once.** The thresholds
    /// and the durability are read together when the handle opens, and every
    /// transaction the handle commits uses them — so the handle's policy does
    /// not change under it.
    pub fn begin_write(&self) -> Result<CacheWrite<'_>> {
        CacheWrite::new(self.writable()?)
    }

    /// Start a batched write handle with custom commit thresholds (see
    /// [`CommitLimits`]). Fails on a read-only cache.
    ///
    /// The durability is still the process's: the thresholds are the
    /// caller's, and the rest of the policy is resolved with them, when the
    /// handle opens.
    pub fn begin_write_with(&self, limits: CommitLimits) -> Result<CacheWrite<'_>> {
        CacheWrite::with_limits(self.writable()?, limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::codec::encode_meta;
    use crate::cache::open::{CacheOpen, open_db, read_meta};
    use crate::cache::schema::CACHE_PREFIX;
    use crate::cache::write::CommitLimits;
    use crate::cache::{CACHE_VERSION, ENTRIES, FileRec, META_KEY, META_TBL, Meta};
    use redb::TableHandle;
    use std::path::PathBuf;

    fn file_rec() -> FileRec {
        FileRec {
            kind: "file".into(),
            size: 42,
            mtime_ns: 123456789,
            hashes: crate::hash::DigestSet::default(),
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

    /// `load_keys` exists to answer one question — which rows exist — so the
    /// only thing that matters is that it answers it *exactly* as `load_all`
    /// would. Asserted as a set rather than a list, because the order is redb's
    /// business and is not part of what a caller may rely on; what a caller may
    /// rely on is the membership.
    ///
    /// Both kinds of row are in the fixture deliberately: a dir row and a file
    /// row decode to different shapes, so a key-only path that accidentally
    /// decoded (or skipped) one of them would still pass a files-only fixture.
    #[test]
    fn load_keys_answers_exactly_what_load_all_would() {
        let c = CacheDb::open_temp(true).unwrap();
        assert_eq!(c.load_keys().unwrap(), Vec::<String>::new());

        let mut expected = std::collections::HashSet::new();
        for rel in ["a.txt", "sub/b.bin", "z.txt"] {
            c.put(rel, &file_rec()).unwrap();
            expected.insert(rel.to_string());
        }
        c.put("a_dir", &FileRec::dir()).unwrap();
        expected.insert("a_dir".to_string());

        let keys = c.load_keys().unwrap();
        let got: std::collections::HashSet<String> = keys.iter().cloned().collect();
        assert_eq!(got, expected, "every key, and nothing else");
        assert_eq!(keys.len(), expected.len(), "no key is reported twice");
        let all = c.load_all().unwrap();
        assert_eq!(
            got,
            all.keys()
                .cloned()
                .collect::<std::collections::HashSet<_>>(),
            "load_keys and load_all describe the same table"
        );

        c.remove("a.txt").unwrap();
        assert!(
            !c.load_keys().unwrap().contains(&"a.txt".to_string()),
            "a removal is visible"
        );
        assert_eq!(c.load_keys().unwrap().len(), 3);
    }

    /// The streaming read answers exactly what the collecting one answers: the
    /// same keys, and nothing else.
    ///
    /// Asserted as a set rather than a list, because the order is redb's
    /// business and is not part of what a caller may rely on; what a caller may
    /// rely on is the membership. Both kinds of row are in the fixture
    /// deliberately: a dir row and a file row are different shapes, and a
    /// key-only read that decoded (or skipped) one of them would still pass a
    /// files-only fixture.
    #[test]
    fn visit_keys_answers_exactly_what_load_keys_would() {
        let c = CacheDb::open_temp(true).unwrap();
        let mut empty = Vec::new();
        c.visit_keys(|k| {
            empty.push(k.to_string());
            Ok(())
        })
        .unwrap();
        assert!(empty.is_empty(), "an empty table has no keys");

        for rel in ["a.txt", "sub/b.bin", "z.txt"] {
            c.put(rel, &file_rec()).unwrap();
        }
        c.put("a_dir", &FileRec::dir()).unwrap();

        let mut streamed = Vec::new();
        c.visit_keys(|k| {
            streamed.push(k.to_string());
            Ok(())
        })
        .unwrap();
        let mut loaded = c.load_keys().unwrap();
        streamed.sort();
        loaded.sort();
        assert_eq!(streamed, loaded, "the same keys, from the same table");

        c.remove("a.txt").unwrap();
        let mut after = Vec::new();
        c.visit_keys(|k| {
            after.push(k.to_string());
            Ok(())
        })
        .unwrap();
        assert!(
            !after.contains(&"a.txt".to_string()),
            "a removal is visible to the streaming read too"
        );
        assert_eq!(after.len(), 3);
    }

    /// A cache with no `entries` table is a fresh cache, not a corrupt one:
    /// the streaming read answers nothing rather than failing — and it costs
    /// the one transaction every entries read costs, whatever the number of
    /// keys.
    #[test]
    fn visit_keys_reads_a_missing_table_as_no_keys() {
        let db = redb_builder()
            .create_with_backend(InMemoryBackend::new())
            .unwrap();
        let bare = CacheDb::open_writable_locked(db, None);
        let mut got = Vec::new();
        bare.visit_keys(|k| {
            got.push(k.to_string());
            Ok(())
        })
        .unwrap();
        assert!(
            got.is_empty(),
            "a missing table is an empty answer, not an error"
        );

        let c = CacheDb::open_temp(true).unwrap();
        for rel in ["a.txt", "b.txt", "c.txt"] {
            c.put(rel, &file_rec()).unwrap();
        }
        let before = c.entries_read_count();
        let mut seen = 0;
        c.visit_keys(|_| {
            seen += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, 3, "every key is handed over");
        assert_eq!(
            c.entries_read_count() - before,
            1,
            "one read transaction, whatever the number of keys"
        );
    }

    /// `entry_count` answers the same number as the key set it stands in for,
    /// and a table that does not exist is still zero rather than an error.
    #[test]
    fn entry_count_matches_the_key_set_and_reads_a_missing_table_as_zero() {
        // No entries table at all: the count is zero, exactly as `load_keys`
        // answers an empty list for the same database.
        let db = redb_builder()
            .create_with_backend(InMemoryBackend::new())
            .unwrap();
        let bare = CacheDb::open_writable_locked(db, None);
        assert_eq!(bare.entry_count().unwrap(), 0);

        let c = CacheDb::open_temp(true).unwrap();
        assert_eq!(c.entry_count().unwrap(), 0);
        c.put("a.txt", &file_rec()).unwrap();
        c.put("sub/b.bin", &file_rec()).unwrap();
        c.put("a_dir", &FileRec::dir()).unwrap();
        assert_eq!(c.entry_count().unwrap(), 3);
        assert_eq!(
            c.entry_count().unwrap(),
            c.load_keys().unwrap().len() as u64,
            "the count is the key set's size"
        );

        c.remove("a.txt").unwrap();
        assert_eq!(c.entry_count().unwrap(), 2);
    }

    /// `get_many` answers exactly what a loop of `get` answers: the same row
    /// for a key that is there, and nothing for a key that is not.
    ///
    /// Both kinds of row are in the fixture deliberately — a dir row and a
    /// file row decode to different shapes — and the absent key is there so
    /// a batch that skipped the misses would be caught.
    #[test]
    fn get_many_answers_exactly_what_a_loop_of_get_would() {
        let c = CacheDb::open_temp(true).unwrap();
        c.put("a.txt", &file_rec()).unwrap();
        c.put("a_dir", &FileRec::dir()).unwrap();

        let keys = ["a.txt", "absent.txt", "a_dir"];
        let many = c.get_many(keys).unwrap();
        let one_by_one: Vec<Option<FileRec>> = keys.iter().map(|k| c.get(k).unwrap()).collect();

        assert_eq!(many.len(), keys.len(), "one answer per key asked for");
        for (i, key) in keys.iter().enumerate() {
            assert_eq!(many[i].0, *key, "the answer names the key it was asked for");
            assert_eq!(
                many[i].1, one_by_one[i],
                "the answer is what a single read gives"
            );
        }
        assert!(many[1].1.is_none(), "a key that is not there has no row");
    }

    /// A batched read costs **one** transaction whatever its length, where a
    /// read per key costs one per key.
    ///
    /// The count is what says so. A duration would be a measurement of the
    /// machine the read ran on; the number of transactions is a fact about
    /// the code.
    #[test]
    fn get_many_reads_every_key_in_one_transaction() {
        let c = CacheDb::open_temp(true).unwrap();
        for key in ["a.txt", "b.txt", "c.txt"] {
            c.put(key, &file_rec()).unwrap();
        }
        let keys = ["a.txt", "b.txt", "c.txt", "absent.txt"];

        let before = c.entries_read_count();
        let rows = c.get_many(keys).unwrap();
        assert_eq!(rows.len(), keys.len());
        assert_eq!(
            c.entries_read_count() - before,
            1,
            "one read transaction for every key"
        );

        let before = c.entries_read_count();
        for key in keys {
            c.get(key).unwrap();
        }
        assert_eq!(
            c.entries_read_count() - before,
            keys.len() as u64,
            "a read per key is a read per key"
        );
    }

    /// The borrowing read answers exactly what the copying one answers, and
    /// costs the same single transaction.
    ///
    /// The rows are the point; the keys are borrowed rather than copied, so
    /// a caller that already holds a key pays nothing for the answer.
    #[test]
    fn get_many_borrowed_answers_what_get_many_answers() {
        let c = CacheDb::open_temp(true).unwrap();
        c.put("a.txt", &file_rec()).unwrap();
        c.put("a_dir", &FileRec::dir()).unwrap();

        let keys = ["a.txt", "absent.txt", "a_dir"];
        let borrowed = c.get_many_borrowed(keys).unwrap();
        let copied = c.get_many(keys).unwrap();
        assert_eq!(borrowed.len(), keys.len(), "one answer per key asked for");
        for (i, key) in keys.iter().enumerate() {
            assert_eq!(
                borrowed[i].0, *key,
                "the answer names the key it was asked for"
            );
            assert_eq!(
                borrowed[i].1, copied[i].1,
                "the answer is the row the copying read gives"
            );
        }

        let before = c.entries_read_count();
        c.get_many_borrowed(keys).unwrap();
        assert_eq!(
            c.entries_read_count() - before,
            1,
            "one read transaction for every key"
        );
    }

    /// A cache with no `entries` table is a fresh cache, not a corrupt one:
    /// the batched read answers "no row" for every key, exactly as a single
    /// read does.
    #[test]
    fn get_many_reads_a_missing_table_as_no_rows() {
        let db = redb_builder()
            .create_with_backend(InMemoryBackend::new())
            .unwrap();
        let bare = CacheDb::open_writable_locked(db, None);

        let many = bare.get_many(["a.txt", "b.txt"]).unwrap();
        assert_eq!(many.len(), 2);
        assert!(
            many.iter().all(|(_, rec)| rec.is_none()),
            "a missing table is an empty answer, not an error"
        );
        for key in ["a.txt", "b.txt"] {
            assert_eq!(bare.get(key).unwrap(), None, "a single read agrees");
        }
    }

    /// Nothing asked, nothing read: a batch of no keys costs no transaction
    /// and opens no table.
    #[test]
    fn get_many_reads_nothing_when_nothing_is_asked() {
        let c = CacheDb::open_temp(true).unwrap();
        let before = c.entries_read_count();
        assert!(
            c.get_many(Vec::<&str>::new()).unwrap().is_empty(),
            "no keys, no answers"
        );
        assert_eq!(
            c.entries_read_count() - before,
            0,
            "an empty batch does not open a transaction"
        );
    }

    /// The count is the **entries** table's reads and nothing else.
    ///
    /// A read of another table is not one of them, so it cannot move the
    /// number a test has pinned. The `meta` row is the other table this
    /// cache has, so a read of it is the case that says so.
    #[test]
    fn a_read_of_another_table_does_not_move_the_count() {
        let c = CacheDb::open_temp(true).unwrap();
        let before = c.entries_read_count();
        assert!(
            read_meta(&c.db).unwrap().is_some(),
            "the row is there to be read"
        );
        assert_eq!(
            c.entries_read_count() - before,
            0,
            "a meta read is not an entries read"
        );
        // An entries read, for contrast: the count moves for that one.
        c.load_keys().unwrap();
        assert_eq!(
            c.entries_read_count() - before,
            1,
            "an entries read is counted"
        );
    }

    // -- read-only opens ---------------------------------------------------
    //
    // Scoped to what this crate decides. That redb hands out a handle with no
    // write path, and that two read-only handles share the file, are the
    // library's guarantees, not ours: nothing here asserts them.

    /// A scratch directory holding one real cache file, plus the path to it.
    ///
    /// The directory comes back as the guard that owns it, so the tree is
    /// removed when the test ends rather than when it remembers to ask.
    ///
    /// Bind it to `_dir`, not `_`: the lifetime *is* the cleanup, and a bare `_`
    /// drops the guard at the end of the statement — deleting the tree the rest of
    /// the test is about to write into.
    fn seeded(tag: &str, case_sensitive: bool) -> (girpr_tempdir::TempDir, PathBuf) {
        let dir = girpr_tempdir::TempDir::new("girsync_ro", tag);
        let p = dir.join(CACHE_PREFIX);
        let c = open_db(
            &p,
            case_sensitive,
            CacheOpen::ReadWrite {
                ignore_cache: false,
                backup_first: false,
                retain: None,
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
        let (_dir, p) = seeded("nowrite", true);
        let c = open_db(&p, true, CacheOpen::ReadOnly).unwrap();
        assert!(c.is_read_only(), "reports its mode");

        // Reads still work, and see the same rows a read/write open would.
        assert_eq!(c.get("a.txt").unwrap(), Some(file_rec()));
        assert_eq!(c.load_all().unwrap().len(), 1);
        assert_eq!(
            c.load_keys().unwrap(),
            vec!["a.txt".to_string()],
            "the key-only read is a read too, so it is allowed on a read-only cache"
        );

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
                retain: None,
            },
        )
        .unwrap();
        let all = after.load_all().unwrap();
        assert_eq!(all.len(), 1);
        assert!(all.contains_key("a.txt"));
        assert!(!all.contains_key("b.txt"), "the refused put did not land");
    }

    /// A read/write open rewrites `meta` when the recorded case mode disagrees
    /// with the run's. A read-only open cannot, and the flag is informational,
    /// so it warns and hands back a usable cache.
    #[test]
    fn read_only_open_tolerates_a_case_mode_mismatch() {
        let (_dir, p) = seeded("casemix", true);
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
                retain: None,
            },
        )
        .unwrap();
        let meta = read_meta(&after.db).unwrap().unwrap();
        assert!(
            meta.case_sensitive,
            "a read-only open must not rewrite the recorded case mode"
        );
    }

    /// A read-only open refuses anything it cannot faithfully interpret: no
    /// file to read, no `meta` to check the version against, or a version this
    /// build would decode under the wrong assumptions.
    #[test]
    fn read_only_open_requires_a_readable_current_version_cache() {
        let dir = girpr_tempdir::TempDir::new("girsync_ro", "gates");
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
            let db = redb_builder().create(&bare).unwrap();
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
            let db = redb_builder().create(&future).unwrap();
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
    }

    // -- the meta fast path ------------------------------------------------
    //
    // A read/write open reads the `meta` row before it takes the write lock,
    // and skips the write entirely when the row already says what the run
    // needs it to say. What has to survive that is pinned here: the row, the
    // tables, and the wording of the refusals.

    /// The tables a cache actually has, sorted.
    ///
    /// The names, not the rows: what matters about a table here is that it
    /// is there.
    fn table_names(db: &impl ReadableDatabase) -> Vec<String> {
        let txn = db.begin_read().unwrap();
        let mut names: Vec<String> = txn
            .list_tables()
            .unwrap()
            .map(|t| t.name().to_string())
            .collect();
        names.sort();
        names
    }

    /// A second read/write open in the same case mode succeeds, and leaves the
    /// cache exactly as it was: the same row, the same tables, the same
    /// `meta`.
    ///
    /// This is the open the fast path exists for, so the row is the point:
    /// nothing about it may change, because nothing is written.
    #[test]
    fn a_second_read_write_open_keeps_the_row_and_the_tables() {
        let (_dir, p) = seeded("fastpath", true);
        let c = open_db(
            &p,
            true,
            CacheOpen::ReadWrite {
                ignore_cache: false,
                backup_first: false,
                retain: None,
            },
        )
        .unwrap();
        assert_eq!(
            c.get("a.txt").unwrap(),
            Some(file_rec()),
            "the row survives the reopen"
        );
        assert_eq!(
            table_names(&c.db),
            vec!["entries".to_string(), "meta".to_string()],
            "both tables are still there"
        );
        let meta = read_meta(&c.db).unwrap().unwrap();
        assert_eq!(meta.version, CACHE_VERSION);
        assert!(
            meta.case_sensitive,
            "the recorded case mode is left as it was"
        );
    }

    /// A read/write open in the opposite case mode rewrites the row. The
    /// read-only half of this is
    /// [`read_only_open_tolerates_a_case_mode_mismatch`]; a read/write open
    /// is the half that has to do the rewriting.
    #[test]
    fn a_read_write_open_rewrites_meta_when_the_case_mode_differs() {
        let (_dir, p) = seeded("fastpath", true);
        // Written above as case-sensitive; read it back in insensitive mode.
        let c = open_db(
            &p,
            false,
            CacheOpen::ReadWrite {
                ignore_cache: false,
                backup_first: false,
                retain: None,
            },
        )
        .unwrap();
        let meta = read_meta(&c.db).unwrap().unwrap();
        assert_eq!(meta.version, CACHE_VERSION, "the version is unchanged");
        assert!(
            !meta.case_sensitive,
            "the row was rewritten to the run's case mode"
        );
        assert_eq!(
            c.get("a.txt").unwrap(),
            Some(file_rec()),
            "rewriting the row leaves the rows alone"
        );
    }

    /// A cache with a `meta` row but no entries table is not whole, and a
    /// read/write open repairs it: the table is created, and it is there
    /// after the open.
    ///
    /// The row is the **matching** one, so the open has nothing to write in
    /// the row itself — only the missing table stands between this cache and
    /// a scan that finds anything. A table created inside a transaction that
    /// is then dropped is a table that was never there, so the open has to
    /// commit what it created.
    #[test]
    fn a_cache_with_a_row_but_no_entries_table_gets_one() {
        let dir = girpr_tempdir::TempDir::new("girsync_ro", "noentries");
        let p = dir.join(CACHE_PREFIX);
        {
            let db = redb_builder().create(&p).unwrap();
            let txn = db.begin_write().unwrap();
            {
                let mut m = txn.open_table(META_TBL).unwrap();
                m.insert(
                    META_KEY,
                    encode_meta(&Meta {
                        version: CACHE_VERSION,
                        case_sensitive: true,
                    })
                    .as_slice(),
                )
                .unwrap();
            }
            txn.commit().unwrap();
        }
        assert_eq!(
            table_names(&redb_builder().create(&p).unwrap()),
            vec!["meta".to_string()],
            "the cache starts with a row and no entries table"
        );

        // The same case mode the row records, so the row itself needs no
        // write: the missing table is the whole of the repair.
        let c = open_db(
            &p,
            true,
            CacheOpen::ReadWrite {
                ignore_cache: false,
                backup_first: false,
                retain: None,
            },
        )
        .unwrap();
        assert_eq!(
            table_names(&c.db),
            vec!["entries".to_string(), "meta".to_string()],
            "the missing table was created, and it outlived the open"
        );
        let meta = read_meta(&c.db).unwrap().expect("the row is still there");
        assert_eq!(meta.version, CACHE_VERSION);
        assert!(meta.case_sensitive, "the repair left the row as it was");
    }

    /// A fresh cache gets both tables and a `meta` row this build can read.
    ///
    /// The fast path must not change that: there is no row to match, so the
    /// open falls through to the write path that creates them.
    #[test]
    fn a_fresh_cache_gets_both_tables_and_a_current_meta_row() {
        let dir = girpr_tempdir::TempDir::new("girsync_ro", "fresh");
        let p = dir.join(CACHE_PREFIX);
        let c = open_db(
            &p,
            false,
            CacheOpen::ReadWrite {
                ignore_cache: false,
                backup_first: false,
                retain: None,
            },
        )
        .unwrap();
        assert_eq!(
            table_names(&c.db),
            vec!["entries".to_string(), "meta".to_string()],
            "a fresh cache has both tables"
        );
        let meta = read_meta(&c.db).unwrap().expect("a fresh cache gets a row");
        assert_eq!(meta.version, CACHE_VERSION);
        assert!(!meta.case_sensitive, "the row records the run's case mode");
    }

    /// A cache stamped with a version this build cannot read is refused, and
    /// the refusal says what it always said: the version, what was wanted,
    /// and both remedies.
    #[test]
    fn a_future_version_cache_is_refused_with_the_same_message() {
        let dir = girpr_tempdir::TempDir::new("girsync_ro", "future");
        let p = dir.join(CACHE_PREFIX);
        {
            let db = redb_builder().create(&p).unwrap();
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
        let err = match open_db(
            &p,
            true,
            CacheOpen::ReadWrite {
                ignore_cache: false,
                backup_first: false,
                retain: None,
            },
        ) {
            Ok(_) => panic!("a future-version cache must be refused"),
            Err(e) => e,
        };
        assert_eq!(
            err.to_string(),
            format!(
                "unsupported cache version {} (want {CACHE_VERSION}; use --ignore-cache to rebuild or sled2redb to convert)",
                CACHE_VERSION + 1
            ),
            "the refusal says what it always said, remedy included"
        );
    }

    /// The fast path's whole point: an open whose `meta` row already matches
    /// must not take the write lock.
    ///
    /// redb's write lock is observable from the outside — a second write
    /// transaction on the same database waits for the first to finish — so
    /// the lock is held here, on purpose, and the open still completes. It
    /// can only do that if it never began a write transaction.
    ///
    /// The timeout is a deadlock guard, not a performance measurement: an
    /// open that needed the lock would wait for the one this test holds, and
    /// a hung test is a worse failure than a failed one. What is asserted is
    /// that the open returns at all.
    #[test]
    fn a_matching_open_does_not_need_the_write_lock() {
        let dir = girpr_tempdir::TempDir::new("girsync_ro", "lock");
        let p = dir.join(CACHE_PREFIX);
        let db = redb_builder().create(&p).unwrap();
        {
            let txn = db.begin_write().unwrap();
            {
                let _ = txn.open_table(ENTRIES).unwrap();
            }
            {
                let mut m = txn.open_table(META_TBL).unwrap();
                m.insert(
                    META_KEY,
                    encode_meta(&Meta {
                        version: CACHE_VERSION,
                        case_sensitive: true,
                    })
                    .as_slice(),
                )
                .unwrap();
            }
            txn.commit().unwrap();
        }
        // Hold the write lock for as long as the open takes.
        let held = db.begin_write().unwrap();
        let db = Arc::new(db);
        let (tx, rx) = std::sync::mpsc::channel();
        let opened = Arc::clone(&db);
        std::thread::spawn(move || {
            let _ = tx.send(CacheDb::init_meta(&opened, true).is_ok());
        });
        let ok = rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the open waited for a write lock it did not need");
        drop(held);
        assert!(ok, "the open failed");
    }
}
