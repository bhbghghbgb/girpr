//! [`CacheDb`]: the handle, read/write or read-only.
//!
//! The read/write-or-read-only distinction is a **type**, not a flag — see
//! [`Handle`] — so "can this handle write?" has a real answer at every call site
//! rather than a boolean somebody has to keep in step.

use anyhow::{Context, Result, bail};
use redb::{Database, ReadableDatabase, ReadableTable, backends::InMemoryBackend};
use std::collections::HashMap;
use std::path::Path;

use super::codec::encode_meta;
use super::codec::{decode_meta, decode_rec};
use super::open::{is_table_missing, open_read_only, read_meta, require_current_version};
use super::schema::{CACHE_VERSION, FileRec, Meta};
use super::write::{CacheWrite, CommitLimits};
use super::{ENTRIES, META_KEY, META_TBL};

/// The redb handle a [`CacheDb`] wraps.
///
/// `ReadOnlyDatabase` is a distinct type rather than a flag on `Database`, so
/// the two have to be matched on. Read-only handles *share* the file with each
/// other; only a read/write handle excludes everyone, which is what makes the
/// one-handle-per-file rule below about writers.
pub(super) enum Handle {
    Writable(Database),
    ReadOnly(redb::ReadOnlyDatabase),
}

impl Handle {
    pub(super) fn writable(&self) -> Option<&Database> {
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
    pub(super) db: Handle,
}

impl CacheDb {
    /// Wrap an already-open handle as this crate's participant side. The only
    /// place a [`Handle::Writable`] is built, so "who may create one" has one
    /// answer rather than one per caller.
    pub(super) fn open_writable(db: Database) -> Self {
        Self {
            db: Handle::Writable(db),
        }
    }

    /// Wrap an already-open handle as this crate's observer side.
    pub(super) fn open_read_only(db: redb::ReadOnlyDatabase) -> Self {
        Self {
            db: Handle::ReadOnly(db),
        }
    }

    pub(super) fn init_meta(db: &Database, case_sensitive: bool) -> Result<()> {
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

    pub(super) fn open_file(db_path: &Path) -> Result<Database> {
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
    /// [`CacheOpen::ReadOnly`](super::CacheOpen::ReadOnly).
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
    /// caller that needs the value wants `load_all`, and one that does not should
    /// not be able to ask for half of it by accident.
    pub fn load_keys(&self) -> Result<Vec<String>> {
        let txn = self.db.begin_read().context("begin read")?;
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

    /// Every cached path, with its record.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::open::{CacheOpen, open_db};
    use crate::cache::schema::CACHE_PREFIX;
    use std::path::PathBuf;

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
    fn seeded(tag: &str, case_sensitive: bool) -> (crate::testtemp::TempDir, PathBuf) {
        let dir = crate::testtemp::TempDir::new("girsync_ro", tag);
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
        let dir = crate::testtemp::TempDir::new("girsync_ro", "gates");
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
    }
}
