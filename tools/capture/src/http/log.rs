use super::record::RequestRecord;
use chrono;
use serde_json;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The accumulated request log, shared by every fetch in a run.
///
/// Cheap to clone; all clones are the same log.
#[derive(Debug, Clone, Default)]
pub struct RequestLog(pub(crate) Arc<Mutex<LogState>>);

impl RequestLog {
    /// Take the lock, **recovering from poisoning**.
    ///
    /// This was `.lock().expect("request log is not poisoned")` at eight sites,
    /// and poisoning is genuinely reachable rather than theoretical: the lock
    /// sits inside the concurrent chunk fetcher, so one task panicking while it
    /// holds this lock poisons it for every other task — and a poisoned lock
    /// makes every subsequent `.expect` panic too, so one panic anywhere turns
    /// into a run that cannot even record its own failure.
    ///
    /// The crate already *tolerates* panicking fetch tasks (`blobs::Store::one`
    /// records "a chunk fetch task panicked" and carries on), so the two
    /// policies contradicted each other: a task was allowed to panic, and the
    /// log it poisoned was then allowed to take the whole run down.
    ///
    /// `PoisonError::into_inner` hands back the guard anyway, and a half-updated
    /// log is still a perfectly good log for provenance — which is the only job
    /// this has. `Record` and `reserve` are the only things that touch
    /// `LogState`, and both are single field writes.
    fn lock(&self) -> std::sync::MutexGuard<'_, LogState> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Debug, Default)]
pub(crate) struct LogState {
    next_seq: u64,
    /// In **completion** order, which under `--jobs > 1` is not issue order.
    records: Vec<RequestRecord>,
    /// `seq -> index into records`.
    ///
    /// The index is what makes [`RequestLog::set_stored_as`] O(log n) instead of
    /// O(n). A linear walk under the lock, once per chunk, over a vector that
    /// reaches 113 610 entries is a ~23 MB memory walk *while holding the one
    /// process-wide lock every other request needs*, which is enough to stop
    /// `--jobs 8` scaling at all.
    ///
    /// A `BTreeMap` rather than a `Vec<Option<usize>>`: `reserve()` hands out
    /// dense `1..N` ids, so a vector would do — but `push` is told the `seq` and
    /// `reserve` is a separate call, and a map stays correct if that ever
    /// changes, whereas a vector would silently misplace a record.
    by_seq: BTreeMap<u64, usize>,
}

impl RequestLog {
    /// An empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserve the next sequence number, at **issue** time.
    ///
    /// Reserved before the request goes out rather than assigned when the record
    /// lands, so `seq` is the order the requests were made in. A capture with
    /// `--jobs 8` would otherwise number its chunk fetches by which download
    /// happened to finish first, which is a function of the network.
    pub fn reserve(&self) -> u64 {
        let mut s = self.lock();
        s.next_seq += 1;
        s.next_seq
    }

    /// Append a record. Its `seq` is already assigned.
    pub fn push(&self, record: RequestRecord) {
        let mut s = self.lock();
        let at = s.records.len();
        let seq = record.seq;
        s.records.push(record);
        s.by_seq.insert(seq, at);
    }

    /// Every record, in `seq` order.
    pub fn records(&self) -> Vec<RequestRecord> {
        let mut v = self.lock().records.clone();
        // Sorted here, not appended in order. `reserve()` assigns `seq` at issue
        // time, so a `--jobs 8` capture reserves 1..N and then *lands* them in
        // whatever order the network finished; appending would leave the file's
        // rows in completion order while its `seq` column claimed otherwise,
        // and a reader who trusted the row order would reconstruct the wrong
        // provenance. Sorted on read rather than on push because `push` is
        // called once per chunk - a 113 610-chunk capture would spend O(n²)
        // keeping a vector ordered - and `records()` is read a handful of times.
        //
        // **Deliberately not read through `by_seq`.** That index would make the
        // ordering structural and drop the sort, but a `seq` pushed twice would
        // then silently *lose* a row from the file this tool exists to produce,
        // and an audit log that quietly drops entries is worse than one that
        // spends n log n to put them in order. `reserve()` is the only allocator
        // and `attempt()` calls it once per record, so the duplicate cannot
        // happen today; the sort is what keeps that from being load-bearing.
        //
        // `reserve()` makes `seq` unique, so this is a total order.
        v.sort_by_key(|r| r.seq);
        v
    }

    /// Every record matching `f`, in `seq` order.
    ///
    /// The filter runs under the one lock and only the matches are cloned and
    /// sorted, so a caller looking for a handful of rows in a log that holds
    /// every request pays for the handful rather than for the log. The order is
    /// [`RequestLog::records`]'s, for the same reason: `seq` is assigned at
    /// reserve time, so completion order is not the order a reader expects.
    pub fn records_matching(&self, f: impl Fn(&RequestRecord) -> bool) -> Vec<RequestRecord> {
        let mut v: Vec<RequestRecord> = {
            let s = self.lock();
            s.records.iter().filter(|r| f(r)).cloned().collect()
        };
        v.sort_by_key(|r| r.seq);
        v
    }

    /// Total bytes received across the run.
    pub fn bytes_in(&self) -> u64 {
        self.lock().records.iter().map(|r| r.bytes_in).sum()
    }

    /// How many requests were made.
    pub fn len(&self) -> usize {
        self.lock().records.len()
    }

    /// Whether nothing was requested.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Record where a body ended up, after the writer stored it.
    ///
    /// By `seq` rather than by URL, because a chunk id can legitimately be
    /// requested twice and the two requests are not the same request.
    ///
    /// **O(log n), holding the lock only long enough to look up an index** — see
    /// `LogState::by_seq`.
    pub fn set_stored_as(&self, seq: u64, path: impl Into<String>) {
        let mut s = self.lock();
        if let Some(&i) = s.by_seq.get(&seq) {
            s.records[i].stored_as = Some(path.into());
        }
    }

    /// Write the log as newline-delimited JSON to `out`, one object per line.
    ///
    /// **Nothing here needs a clone.** A record is serialised, written, and
    /// forgotten, so the lock is taken once to build the *ordering* — a
    /// `Vec<usize>` of indices — and then once per record, for the few microseconds
    /// it takes to serialise one. At 113 610 records that is a 0.9 MB index
    /// rather than 113 610 deep copies of a ~200 B record, and it is taken on the
    /// blocking pool rather than a tokio worker.
    ///
    /// The ordering is by `seq`, for the reason [`RequestLog::records`] gives:
    /// `seq` is assigned at reserve time, so a `--jobs 8` capture lands rows in
    /// completion order while the `seq` column claims otherwise.
    pub fn write_ndjson_to<W: std::io::Write>(&self, out: &mut W) -> std::io::Result<usize> {
        // One lock for the whole ordering. Sorting under it keeps `records`
        // borrowed for the comparison instead of re-locking 113 610 times.
        let mut order: Vec<usize> = {
            let s = self.lock();
            let mut order: Vec<usize> = (0..s.records.len()).collect();
            order.sort_by_key(|&i| s.records[i].seq);
            order
        };
        let n = order.len();

        for i in order.drain(..) {
            // The lock is held for one record's serialisation and **not** for the
            // write, so a slow disk cannot stall a fetch that is still logging.
            let line = {
                let s = self.lock();
                let r = &s.records[i];
                match serde_json::to_string(r) {
                    Ok(line) => line,
                    // A record that cannot serialize would otherwise vanish
                    // silently, and a log with a hole in it is worse than one that
                    // says it has one. `RequestRecord` is all primitives, so this
                    // is unreachable; the line says so rather than being dropped.
                    Err(e) => {
                        format!(
                            "{{\"seq\":{},\"error\":\"unserializable record: {e}\"}}",
                            r.seq
                        )
                    }
                }
            };
            out.write_all(line.as_bytes())?;
            out.write_all(b"\n")?;
        }
        Ok(n)
    }

    /// The log as newline-delimited JSON, one object per line.
    ///
    /// **For tests and small logs.** The capture path uses
    /// [`RequestLog::write_ndjson_to`], which does not build the whole document in
    /// memory; this exists because a handful of assertions want to compare a string.
    pub fn to_ndjson(&self) -> String {
        let mut out = Vec::new();
        self.write_ndjson_to(&mut out)
            .expect("writing to a Vec cannot fail");
        String::from_utf8(out).expect("serde_json emits UTF-8")
    }

    /// Write the log to `path`, creating parent directories.
    ///
    /// **Buffered, and one record at a time**, so a 113 610-row log never exists
    /// whole in memory.
    ///
    /// The `flush` is explicit rather than left to `BufWriter`'s `Drop`, and that is
    /// the whole reason it is here: a full disk surfaces at flush, and `Drop` throws
    /// that error away. Swallowed, the run would report having written a request log
    /// and leave a truncated file where `verify` would later read it.
    pub fn write_ndjson(&self, path: &Path) -> std::io::Result<usize> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let f = std::fs::File::create(path)?;
        let mut w = std::io::BufWriter::with_capacity(256 * 1024, f);
        let n = self.write_ndjson_to(&mut w)?;
        std::io::Write::flush(&mut w)?;
        Ok(n)
    }
}

/// RFC 3339 in UTC, millisecond precision.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
