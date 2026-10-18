//! [`CacheWrite`]: batched mutations, and when they become durable.
//!
//! One write transaction per handle, committed on whichever of the three
//! thresholds in [`CommitLimits`] trips first. Bulk phases use this instead of
//! [`super::CacheDb::put`], which commits per call and would fsync per file.

use anyhow::{Context, Result};
use redb::{Database, ReadableTable};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::trace;

use super::ENTRIES;
use super::codec::{decode_rec, encode_rec};
use super::schema::FileRec;

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
    pub(super) dirty: usize,
    pub(super) bytes_since_commit: u64,
    last_commit: Instant,
}

impl<'a> CacheWrite<'a> {
    pub(super) fn new(db: &'a Database) -> Result<Self> {
        Self::with_limits(db, CommitLimits::default())
    }

    pub(super) fn with_limits(db: &'a Database, limits: CommitLimits) -> Result<Self> {
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

    /// Every key in this transaction, decoding nothing.
    ///
    /// The batched counterpart of [`CacheDb::load_keys`](crate::cache::CacheDb::load_keys),
    /// and it exists for the same reason: a caller asking "which rows exist?"
    /// should not pay to rebuild every `FileRec` to find out. It sees this
    /// transaction's own pending writes, exactly as `load_all` does, because
    /// both read the same transaction.
    pub fn load_keys(&self) -> Result<Vec<String>> {
        let txn = self.txn.as_ref().context("write txn gone")?;
        let tbl = txn.open_table(ENTRIES).context("open entries table")?;
        let mut keys = Vec::new();
        for kv in tbl.iter().context("iterate cache")? {
            let (k, _v) = kv.context("read cache entry")?;
            keys.push(k.value().to_string());
        }
        Ok(keys)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheDb;
    use std::collections::HashSet;

    fn file_rec() -> FileRec {
        FileRec {
            kind: "file".into(),
            size: 42,
            mtime_ns: 123456789,
            hashes: file_digests(),
        }
    }

    fn file_digests() -> crate::hash::DigestSet {
        let mut h = crate::hash::DigestSet::default();
        h.insert_named("md5", &[1u8; 16]);
        h.insert_named("sha256", &[2u8; 32]);
        h
    }

    fn sized_rec(size: u64) -> FileRec {
        FileRec {
            kind: "file".into(),
            size,
            mtime_ns: 0,
            hashes: crate::hash::DigestSet::default(),
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

    /// The batched `load_keys` has to see **this transaction's own pending
    /// writes**, not the last committed state — that is the whole difference
    /// from reading through the `CacheDb` handle, and phase A's prune relies on
    /// it reading the same set `load_all` would have read.
    ///
    /// Pinned across an internal auto-commit too, because that reopens the
    /// transaction underneath: the keys have to be the same before and after,
    /// or the prune's answer would depend on when a threshold happened to trip.
    #[test]
    fn a_batch_sees_its_own_uncommitted_keys() {
        let c = CacheDb::open_temp(true).unwrap();
        // Trip the op-count trigger at 2 so the transaction reopens mid-test.
        let limits = CommitLimits {
            max_ops: 2,
            ..quiet_limits()
        };
        let mut w = c.begin_write_with(limits).unwrap();
        let keys =
            |w: &CacheWrite| -> HashSet<String> { w.load_keys().unwrap().into_iter().collect() };

        assert_eq!(keys(&w), HashSet::new(), "an empty table has no keys");

        w.put("a", &file_rec()).unwrap();
        assert_eq!(
            keys(&w),
            ["a".to_string()].into_iter().collect(),
            "uncommitted, and visible"
        );
        assert_eq!(keys(&w), keys_of(&w), "load_keys agrees with load_all");

        w.put("b", &file_rec()).unwrap();
        assert_eq!(w.dirty, 0, "the threshold committed mid-test");
        assert_eq!(
            keys(&w),
            ["a".to_string(), "b".to_string()].into_iter().collect(),
            "the same keys after the transaction was reopened underneath"
        );

        w.remove("a").unwrap();
        assert_eq!(
            keys(&w),
            ["b".to_string()].into_iter().collect(),
            "a removal is visible before it is committed too"
        );
        let pending = keys(&w);
        w.commit().unwrap();
        assert_eq!(
            c.load_keys().unwrap().into_iter().collect::<HashSet<_>>(),
            pending,
            "and it is what the committed table says"
        );
    }

    fn keys_of(w: &CacheWrite) -> HashSet<String> {
        w.load_all().unwrap().into_keys().collect()
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
