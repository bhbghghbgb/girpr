//! [`CacheWrite`]: batched mutations, and when they become durable.
//!
//! One write transaction per handle, committed on whichever of the three
//! thresholds in [`CommitLimits`] trips first. Bulk phases use this instead of
//! [`super::CacheDb::put`], which commits per call and would fsync per file.
//!
//! ## A write has one commit policy
//!
//! A write commits under a **policy**: the thresholds it batches to and the
//! durability it commits with. The policy is resolved **once, when the write
//! starts**, and it does not change while the write runs — see
//! [`CommitPolicy`].

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
pub const COMMIT_BATCH: usize = 50;

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

impl CommitLimits {
    /// [`Default`], with the test-only environment overrides applied.
    ///
    /// A test that needs a threshold to trip should not have to produce
    /// [`COMMIT_BATCH`] rows, [`COMMIT_BYTES`] of content, or wait
    /// [`COMMIT_INTERVAL`]. `GIRSYNC_TEST_COMMIT_OPS`,
    /// `GIRSYNC_TEST_COMMIT_BYTES`, and `GIRSYNC_TEST_COMMIT_INTERVAL_MS`
    /// (milliseconds) override the three thresholds instead. They are read
    /// when a write starts — together with the durability, as one policy (see
    /// [`CommitPolicy`]) — and not on every mutation, and an unset or
    /// unparseable value leaves the default in place.
    ///
    /// Deliberately not CLI flags: they exist so a small fixture can make a
    /// commit observable, not to tune a real run.
    pub fn from_env() -> Self {
        let mut limits = Self::default();
        if let Some(ops) = env_limit("GIRSYNC_TEST_COMMIT_OPS") {
            limits.max_ops = ops;
        }
        if let Some(bytes) = env_limit("GIRSYNC_TEST_COMMIT_BYTES") {
            limits.max_bytes = bytes;
        }
        if let Some(ms) = env_limit("GIRSYNC_TEST_COMMIT_INTERVAL_MS") {
            limits.max_interval = Duration::from_millis(ms);
        }
        limits
    }

    /// [`Some`] with the environment's overrides when any `GIRSYNC_TEST_COMMIT_*`
    /// variable is set, and [`None`] when none is. This lets a resolved
    /// [`TestHooks`](crate::config::TestHooks) leave the commit policy to the
    /// environment (its `None`) while still carrying an explicit policy when a
    /// caller injects one.
    pub fn from_env_opt() -> Option<Self> {
        let any = [
            "GIRSYNC_TEST_COMMIT_OPS",
            "GIRSYNC_TEST_COMMIT_BYTES",
            "GIRSYNC_TEST_COMMIT_INTERVAL_MS",
        ]
        .iter()
        .any(|name| std::env::var(name).is_ok());
        any.then(Self::from_env)
    }
}

/// A write's commit policy: the thresholds it batches to, and the durability
/// every transaction of it commits with.
///
/// **The two are resolved together, once, when the write starts.** The
/// thresholds are the test-only `GIRSYNC_TEST_COMMIT_*` overrides and the
/// durability is the test-only `GIRSYNC_TEST_DURABILITY` switch, and one
/// function reads both, at one moment — so the two cannot disagree about
/// when they were read, and a switch cannot answer for half of a policy.
///
/// **The policy does not change under a running write.** A handle that asked
/// the environment again for each transaction it opens would commit some of
/// its batches one way and the rest another: the durability switch exists to
/// make a whole run's commits cheap, and a handle that changed its mind half
/// way through would leave the run's own cache half durable and half not. So
/// a handle keeps the policy it opened with, and every transaction it
/// commits — including the ones it opens after a threshold trips — commits
/// under it.
pub(in crate::cache) struct CommitPolicy {
    /// The thresholds a batched write commits on.
    limits: CommitLimits,
    /// The durability every transaction of the write commits with.
    durability: redb::Durability,
}

impl CommitPolicy {
    /// The policy of a write, with the test-only overrides applied.
    ///
    /// The thresholds and the durability are read here, together, once.
    pub(in crate::cache) fn from_env() -> Self {
        Self {
            limits: CommitLimits::from_env(),
            durability: super::write_durability(),
        }
    }

    /// The policy of a write whose thresholds the caller chose.
    ///
    /// The durability is still the process's: it is read here, with the
    /// thresholds, so a write's whole policy is decided in one place.
    pub(in crate::cache) fn with_limits(limits: CommitLimits) -> Self {
        Self {
            limits,
            durability: super::write_durability(),
        }
    }

    /// The durability every transaction of this write commits with.
    pub(in crate::cache) fn durability(&self) -> redb::Durability {
        self.durability
    }
}

/// One `GIRSYNC_TEST_COMMIT_*` override, or `None` when it is unset.
///
/// A set-but-unparseable value is reported and ignored, so a typo is visible in
/// the log instead of silently running the test at the production thresholds.
fn env_limit<T: std::str::FromStr>(name: &str) -> Option<T> {
    let raw = std::env::var(name).ok()?;
    match raw.trim().parse() {
        Ok(value) => Some(value),
        Err(_) => {
            tracing::warn!(var = name, value = %raw, "ignoring commit-limit override");
            None
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
    policy: CommitPolicy,
    pub(super) dirty: usize,
    pub(super) bytes_since_commit: u64,
    last_commit: Instant,
}

impl<'a> CacheWrite<'a> {
    pub(super) fn new(db: &'a Database) -> Result<Self> {
        Self::with_policy(db, CommitPolicy::from_env())
    }

    pub(super) fn with_limits(db: &'a Database, limits: CommitLimits) -> Result<Self> {
        Self::with_policy(db, CommitPolicy::with_limits(limits))
    }

    fn with_policy(db: &'a Database, policy: CommitPolicy) -> Result<Self> {
        let mut w = Self {
            db,
            txn: None,
            policy,
            dirty: 0,
            bytes_since_commit: 0,
            last_commit: Instant::now(),
        };
        let txn = w.begin_txn()?;
        w.txn = Some(txn);
        Ok(w)
    }

    /// Open a write transaction under this handle's policy.
    ///
    /// **Every transaction a handle opens goes through here**, so the policy
    /// is applied once and cannot be re-read by accident: the handle's
    /// transactions all commit the same way, whichever of them opened the
    /// write.
    fn begin_txn(&mut self) -> Result<redb::WriteTransaction> {
        let mut txn = self.db.begin_write().context("begin write")?;
        txn.set_durability(self.policy.durability)
            .context("set cache write durability")?;
        Ok(txn)
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

    /// Every key in this transaction, one at a time, decoding nothing.
    ///
    /// The streaming counterpart of [`load_keys`](Self::load_keys): the same
    /// keys, from the same transaction, and **none of them copied out**. A
    /// caller that wants to *keep* the key set wants
    /// [`load_keys`](Self::load_keys); a caller that wants to *ask a question
    /// of* it — "which of these rows are not live?" — wants this one, because
    /// the answer is usually "none of them" and a `Vec<String>` with one
    /// string per cached row is an allocation per row for that answer.
    ///
    /// The key is borrowed for the length of the call only, so a caller that
    /// wants to keep one copies it then — and copies only the ones it keeps.
    ///
    /// It sees this transaction's own pending writes, exactly as
    /// [`load_keys`](Self::load_keys) does, because both read the same
    /// transaction.
    ///
    /// **The read is not a counted one.** The counter belongs to the read
    /// handles, and this handle reads in its own write transaction.
    pub fn visit_keys(&self, f: impl FnMut(&str) -> Result<()>) -> Result<()> {
        let txn = self.txn.as_ref().context("write txn gone")?;
        let tbl = txn.open_table(ENTRIES).context("open entries table")?;
        crate::cache::db::visit_entry_keys(&tbl, f)
    }

    fn maybe_commit(&mut self) -> Result<()> {
        // The time check runs on mutation, i.e. right after the (possibly
        // minutes-long) hash that produced this put — exactly when the
        // backlog of uncommitted work is largest.
        if self.dirty >= self.policy.limits.max_ops
            || self.bytes_since_commit >= self.policy.limits.max_bytes
            || self.last_commit.elapsed() >= self.policy.limits.max_interval
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
        // The next transaction is this handle's own: it commits under the
        // policy the handle opened with, like the one before it.
        let txn = self.begin_txn()?;
        self.txn = Some(txn);
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

    /// Test-only: pretend the last commit happened `by` earlier than it did.
    ///
    /// This lets an interval-trigger test simulate a long hash between
    /// mutations deterministically, without sleeping on the wall clock.
    #[cfg(test)]
    fn backdate_last_commit(&mut self, by: Duration) {
        self.last_commit -= by;
    }

    /// Commit all pending mutations. This is the `flush()` analogue: after
    /// it returns, the mutations survive a process kill — unless the
    /// test-only `GIRSYNC_TEST_DURABILITY=none` switch lowered the write's
    /// durability. In that case they survive a clean exit (redb flushes on
    /// `Database` drop) but not a kill. Production always keeps
    /// [`Durability::Immediate`](redb::Durability::Immediate); see
    /// `write_durability`.
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

    /// **A write's commit policy is decided when the write starts.**
    ///
    /// The durability switch is read once per write and not again while that
    /// write runs: a batched write handle reads it when it opens, and the
    /// transaction it opens after a threshold trips does not read it again. A
    /// handle that asked the environment again for each of its transactions
    /// would commit some of its batches one way and the rest another.
    ///
    /// The count is the switch's own: the policy is the only thing that
    /// reads it, so the number of readings is the number of writes that
    /// resolved a policy.
    #[test]
    fn a_write_reads_the_durability_switch_once_and_keeps_it() {
        let c = CacheDb::open_temp(true).unwrap();

        let before = crate::cache::durability_reads();
        let mut w = c
            .begin_write_with(CommitLimits {
                max_ops: 1,
                ..quiet_limits()
            })
            .unwrap();
        assert_eq!(
            crate::cache::durability_reads() - before,
            1,
            "the handle reads the switch once, when it opens"
        );

        // The op-count threshold trips on this put, so the handle commits and
        // opens another transaction.
        w.put("a", &file_rec()).unwrap();
        assert_eq!(
            crate::cache::durability_reads() - before,
            1,
            "the transaction the handle opened after the threshold does not read the switch again"
        );
        w.commit().unwrap();
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

    /// The batched streaming read sees **this transaction's own pending
    /// writes**, exactly as `load_keys` does: the prune reads the key set
    /// through the batch, and a key the batch has written and not yet
    /// committed is a key the prune has to see.
    ///
    /// The batch's answer is the same set the collecting read gives, so the
    /// two cannot disagree about which rows exist.
    #[test]
    fn a_batch_streams_its_own_uncommitted_keys() {
        let c = CacheDb::open_temp(true).unwrap();
        let mut w = c.begin_write().unwrap();
        let keys = |w: &CacheWrite| -> Vec<String> {
            let mut v = Vec::new();
            w.visit_keys(|k| {
                v.push(k.to_string());
                Ok(())
            })
            .unwrap();
            v.sort();
            v
        };

        assert!(keys(&w).is_empty(), "an empty table has no keys");

        w.put("a", &file_rec()).unwrap();
        assert_eq!(keys(&w), vec!["a".to_string()], "uncommitted, and visible");

        w.put("b", &file_rec()).unwrap();
        assert_eq!(
            keys(&w),
            vec!["a".to_string(), "b".to_string()],
            "and the streaming read agrees with the collecting one"
        );

        w.remove("a").unwrap();
        assert_eq!(
            keys(&w),
            vec!["b".to_string()],
            "a removal is visible before it is committed too"
        );
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
        // Simulate a long hash of a huge file between mutations by backdating
        // the last commit past the interval, so the next put trips the trigger
        // without a wall-clock sleep.
        w.backdate_last_commit(Duration::from_secs(1));
        w.put("b", &sized_rec(1)).unwrap();
        assert_eq!(w.dirty, 0, "interval trigger committed");
        assert!(w.get("a").unwrap().is_some());
        w.commit().unwrap();
        assert_eq!(c.load_all().unwrap().len(), 2);
    }
}
