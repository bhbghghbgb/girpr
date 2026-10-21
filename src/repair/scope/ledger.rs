//! The runtime ledger: the live counters the mid-flight hard stops read.
//!
//! A pre-flight refusal asks *"is this plan too big?"* — answerable before any
//! work starts. A runtime hard stop asks *"has this run already done too much?"*
//! — answerable only once a counter crosses the line, and for anything the plan
//! could not predict.
//!
//! [`crate::report::Summary`] owns the counters; this module owns the *rules*
//! for reserving against a ceiling, so the arithmetic is one tested place
//! rather than two `fetch_add` sites in `repair::file`.
//!
//! The counter itself is the [`Meter`]: one type, two numbers, and the rule
//! that tells them apart.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::report::Summary;

/// A counter with two numbers.
///
/// - **live** is charged *before* the resource is consumed. It is monotone — a
///   byte that crossed the wire or landed in the install is spent, whether it
///   turns out to be good or not — it is what a runtime hard stop reads, and
///   it is what a budget is about.
/// - **settled** is charged *after* the resource validated, and it is what the
///   `SUMMARY` reports: the bytes the repair was *allowed* to spend.
///
/// The two are equal in the happy path and differ exactly when something
/// failed. `download_bytes` has always been this pair — with the settle done
/// when the file is promoted — and `write_bytes` becomes it here.
///
/// The rule that makes it work:
///
/// > A byte is **live** from the moment it is committed, and **settled** once
/// > it has been validated. The difference between the two numbers is the
/// > waste: the bytes that were spent and did not become part of the install.
///
/// A ceiling bounds the **live** figure, because the bytes are gone either
/// way. The **settled** figure is what a *verified* ceiling bounds: the bytes
/// the repair was allowed to spend, which is a function of the install rather
/// than of the transport.
#[derive(Debug)]
pub struct Meter {
    /// The bytes the run has spent.
    live: AtomicU64,
    /// The bytes that passed their gate.
    settled: AtomicU64,
    /// The ceiling on the live figure. `u64::MAX` = none, so the hot path is
    /// a compare-and-swap against a number rather than an `Option` check.
    live_limit: AtomicU64,
    /// The ceiling on the settled figure. `u64::MAX` = none.
    settled_limit: AtomicU64,
}

impl Default for Meter {
    /// A meter with no ceiling — the ceiling sentinel is `u64::MAX`, and a
    /// derived `Default` would start every run at a ceiling of **zero**.
    fn default() -> Self {
        Self::new()
    }
}

impl Meter {
    /// A meter with nothing counted and no ceiling asked for.
    pub fn new() -> Self {
        Self {
            live: AtomicU64::new(0),
            settled: AtomicU64::new(0),
            live_limit: AtomicU64::new(u64::MAX),
            settled_limit: AtomicU64::new(u64::MAX),
        }
    }

    /// A plain-value copy, for handing a shared meter back to the caller.
    pub fn snapshot(&self) -> Self {
        Self {
            live: AtomicU64::new(self.live.load(Ordering::Relaxed)),
            settled: AtomicU64::new(self.settled.load(Ordering::Relaxed)),
            live_limit: AtomicU64::new(self.live_limit.load(Ordering::Relaxed)),
            settled_limit: AtomicU64::new(self.settled_limit.load(Ordering::Relaxed)),
        }
    }

    /// The live ceiling, or `None` when no ceiling was asked for.
    pub fn live_limit(&self) -> Option<u64> {
        let l = self.live_limit.load(Ordering::Relaxed);
        (l != u64::MAX).then_some(l)
    }

    /// Install the live ceiling. `None` is stored as `u64::MAX`, so the hot
    /// path is a compare-and-swap against a number rather than an `Option`
    /// check.
    pub fn set_live_limit(&mut self, limit: Option<u64>) {
        self.live_limit
            .store(limit.unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    /// The settled ("verified") ceiling, or `None`.
    pub fn settled_limit(&self) -> Option<u64> {
        let l = self.settled_limit.load(Ordering::Relaxed);
        (l != u64::MAX).then_some(l)
    }

    /// Install the settled ceiling. `None` is stored as `u64::MAX`.
    pub fn set_settled_limit(&mut self, limit: Option<u64>) {
        self.settled_limit
            .store(limit.unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    /// Charge `n` bytes against the live ceiling — all or nothing.
    ///
    /// Reserving *before* the resource is consumed is what makes the stop
    /// exact rather than approximate: with N files in flight, a check-then-add
    /// would overshoot by up to N-1 chunks, and a ceiling is a ceiling, not a
    /// guideline. Returns `false` — having added nothing — when the ceiling
    /// blocks the charge, which is the caller's signal to stop the run.
    pub fn charge_live(&self, n: u64) -> bool {
        let limit = self.live_limit.load(Ordering::Relaxed);
        let mut cur = self.live.load(Ordering::Relaxed);
        loop {
            let next = cur.saturating_add(n);
            if next > limit {
                return false;
            }
            match self
                .live
                .compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return true,
                Err(actual) => cur = actual,
            }
        }
    }

    /// Give `n` charged bytes back: the resource turned out to deliver nothing
    /// at all, so nothing was spent.
    ///
    /// This is the **only** way the live figure goes down. A resource that was
    /// consumed and then failed is *not* refunded: those bytes were spent, and
    /// the difference between the two numbers is exactly what says so.
    pub fn refund_live(&self, n: u64) {
        self.live.fetch_sub(n, Ordering::Relaxed);
    }

    /// Correct a charge once the real byte count is known.
    ///
    /// The reservation is the manifest's `compressed_size`; the caller's real
    /// measurement is folded back in here, so the live figure is exact *by
    /// construction* rather than by assumption, which is what `gircap`'s
    /// `download_bytes == patchmaps/<id>.json → totals.download_bytes`
    /// assertion is written against.
    pub fn adjust_live(&self, delta: i64) {
        if delta < 0 {
            self.refund_live(delta.unsigned_abs());
        } else if delta > 0 {
            self.live.fetch_add(delta as u64, Ordering::Relaxed);
        }
    }

    /// Charge `n` bytes that passed their gate.
    ///
    /// The bytes are counted whether the settled ceiling is crossed or not —
    /// the charge happened, and a figure that stopped counting would be a lie.
    /// The return value says whether the settled figure is still within its
    /// ceiling: `false` means the ceiling bound, and the caller trips the
    /// stop.
    pub fn settle(&self, n: u64) -> bool {
        self.settled.fetch_add(n, Ordering::Relaxed);
        let limit = self.settled_limit.load(Ordering::Relaxed);
        self.settled.load(Ordering::Relaxed) <= limit
    }

    /// The bytes the run has spent.
    pub fn live(&self) -> u64 {
        self.live.load(Ordering::Relaxed)
    }

    /// The bytes that passed their gate.
    pub fn settled(&self) -> u64 {
        self.settled.load(Ordering::Relaxed)
    }
}

impl Summary {
    /// `--max-download-bytes`, or `None` when the flag is absent.
    pub fn max_download_bytes(&self) -> Option<u64> {
        self.download.live_limit()
    }

    /// Install the runtime ceiling. Called once, before any task starts.
    /// `None` is stored as `u64::MAX`, so the hot path is a compare-and-swap
    /// against a number rather than an `Option` check.
    pub fn set_download_limit(&mut self, limit: Option<u64>) {
        self.download.set_live_limit(limit);
    }

    /// Reserve `n` bytes against the download ceiling, adding them to the live
    /// counter — all or nothing.
    ///
    /// Reserving *before* the fetch is what makes the stop exact rather than
    /// approximate: with N files in flight, a check-then-add would overshoot by
    /// up to N-1 chunks, and `--max-download-bytes` is a ceiling, not a
    /// guideline. Returns `false` — having added nothing — when the ceiling
    /// blocks the reservation, which is the caller's signal to stop the run.
    ///
    /// The reservation is the manifest's `compressed_size`; the caller's real
    /// measurement (`blob.len()`) is folded back in by
    /// [`Self::adjust_download_bytes`].
    pub fn reserve_download_bytes(&self, n: u64) -> bool {
        self.download.charge_live(n)
    }

    /// Correct a reservation once the real byte count is known.
    ///
    /// Capture invariant A6 says `len(blob) == compressed_size`, so the delta is
    /// normally zero; folding it in anyway keeps the live figure exact *by
    /// construction* rather than by assumption, which is what `gircap`'s
    /// `download_bytes == patchmaps/<id>.json → totals.download_bytes`
    /// assertion is written against.
    pub fn adjust_download_bytes(&self, delta: i64) {
        self.download.adjust_live(delta);
    }

    /// Settle the bytes of a file that was actually repaired.
    ///
    /// This is the reported `download_bytes`: the bytes of the files that were
    /// **promoted**, not the bytes that crossed the wire. An attempt that
    /// failed and was retried settles nothing — its bytes are spent (the live
    /// figure has them) and they are not part of the install.
    ///
    /// The return value says whether the settled figure is still within the
    /// *verified* ceiling; `false` means it bound, and the caller trips the
    /// stop.
    pub fn settle_download_bytes(&self, n: u64) -> bool {
        self.download.settle(n)
    }

    /// `true` once a runtime hard stop has fired. Checked at every file-task
    /// entry, so the remaining files drop out instead of queueing behind the
    /// ceiling.
    pub fn download_stopped(&self) -> bool {
        self.download_stopped.load(Ordering::Relaxed)
    }

    /// Trip the runtime hard stop. Idempotent: the first caller to see the line
    /// wins and the rest simply observe the flag.
    pub fn stop_downloads(&self) {
        self.download_stopped.store(true, Ordering::Relaxed);
    }

    /// `--max-write-bytes`, or `None`.
    pub fn max_write_bytes(&self) -> Option<u64> {
        let l = self.write_limit.load(Ordering::Relaxed);
        (l != u64::MAX).then_some(l)
    }

    /// Install the write ceiling.
    pub fn set_write_limit(&mut self, limit: Option<u64>) {
        self.write_limit
            .store(limit.unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    /// Reserve `n` bytes of disk writes against the ceiling — see
    /// [`Self::reserve_download_bytes`] for why this is a reservation and not a
    /// check-then-add.
    ///
    /// **This is the live counter** ([`Summary::write_bytes_spent`]): it counts
    /// every byte that lands in the install, gate or no gate. The ceiling
    /// exists to bound the disk, and a chunk that fails its digest has already
    /// been written — the bytes are spent whether they are good or not.
    ///
    /// The *reported* figure is [`Summary::write_bytes`], which counts only
    /// the bytes whose source passed its gate; see
    /// [`Self::note_write_bytes`].
    ///
    /// **This is the only genuinely un-counted cost in the tool.**
    /// `SUMMARY download_bytes` counts the network; this counts the disk. Every
    /// reuse-copy (`out.write_all(&buf[..n])` in `try_reuse_slice`) and every
    /// downloaded-chunk write lands in `*_tmp` and appears in *neither*
    /// `download_bytes` nor the plan's estimate, so a zero-reuse run writes its
    /// entire payload to disk while reporting a network cost of almost nothing.
    pub fn reserve_write_bytes(&self, n: u64) -> bool {
        let limit = self.write_limit.load(Ordering::Relaxed);
        let mut cur = self.write_bytes_spent.load(Ordering::Relaxed);
        loop {
            let next = cur.saturating_add(n);
            if next > limit {
                return false;
            }
            match self.write_bytes_spent.compare_exchange_weak(
                cur,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(actual) => cur = actual,
            }
        }
    }

    /// Record `n` bytes that were written into the install **from a source
    /// that passed its gate** — a chunk's MD5, a reuse slice's MD5, a staged
    /// chunk's MD5, the scan's source verdict, the structural zero-fill.
    ///
    /// This is the figure the `write_bytes=` key reports, and the point of
    /// having it separate from [`Self::reserve_write_bytes`] is that it is a
    /// function of the *input state*: the same install against a clean origin
    /// and against one that serves a corrupt chunk reports the same number,
    /// because the corrupt chunk never passed a gate and contributes nothing.
    ///
    /// It is charged at the write sites and never taken back, so it is
    /// cumulative across attempts — the same as the counter it replaces.
    pub fn note_write_bytes(&self, n: u64) {
        self.write_bytes.fetch_add(n, Ordering::Relaxed);
    }

    /// Trip the runtime **write** hard stop. Separate from
    /// [`Self::stop_downloads`] so `SUMMARY stopped=` can say which ceiling
    /// bound — that distinction is the entire point of the key.
    pub fn stop_writes(&self) {
        self.write_stopped.store(true, Ordering::Relaxed);
    }

    /// `true` once the write ceiling has fired.
    pub fn write_stopped(&self) -> bool {
        self.write_stopped.load(Ordering::Relaxed)
    }

    /// `--max-ram-bytes`, or `None`.
    pub fn max_ram_bytes(&self) -> Option<u64> {
        let l = self.ram_limit.load(Ordering::Relaxed);
        (l != u64::MAX).then_some(l)
    }

    /// Install the RAM ceiling.
    pub fn set_ram_limit(&mut self, limit: Option<u64>) {
        self.ram_limit
            .store(limit.unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    /// Reserve `n` bytes of staging RAM — all or nothing.
    ///
    /// The same reservation shape as [`Self::reserve_write_bytes`]: a
    /// compare-and-swap against a `u64::MAX` sentinel, so the hot path is a
    /// number and not an `Option` check. A refused reservation adds nothing,
    /// and the caller degrades to the other write order rather than failing
    /// the file.
    ///
    /// The peak is tracked separately from the live total: the live total
    /// goes back down when a file is done, and the interesting number is how
    /// much was staged at once.
    pub fn reserve_ram_bytes(&self, n: u64) -> bool {
        let limit = self.ram_limit.load(Ordering::Relaxed);
        let mut cur = self.ram_reserved.load(Ordering::Relaxed);
        loop {
            let next = cur.saturating_add(n);
            if next > limit {
                return false;
            }
            match self.ram_reserved.compare_exchange_weak(
                cur,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.ram_bytes.fetch_max(next, Ordering::Relaxed);
                    return true;
                }
                Err(actual) => cur = actual,
            }
        }
    }

    /// Give a reservation back: the file that held it is done.
    pub fn release_ram_bytes(&self, n: u64) {
        self.ram_reserved.fetch_sub(n, Ordering::Relaxed);
    }

    /// The peak staged payload of the run.
    pub fn ram_peak_bytes(&self) -> u64 {
        self.ram_bytes.load(Ordering::Relaxed)
    }

    /// `--fail-fast` armed?
    pub fn fail_fast(&self) -> bool {
        self.fail_fast.load(Ordering::Relaxed)
    }

    /// Arm (or disarm) `--fail-fast`. Called once, before any task starts.
    pub fn set_fail_fast(&mut self, on: bool) {
        self.fail_fast.store(on, Ordering::Relaxed);
    }

    /// `true` once `--fail-fast` has seen a file exhaust its retries: no further
    /// files are started and in-flight ones stop retrying.
    pub fn aborted(&self) -> bool {
        self.aborted.load(Ordering::Relaxed)
    }

    /// Trip the `--fail-fast` abort.
    pub fn abort(&self) {
        self.aborted.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// The meter's two numbers, and the rule that tells them apart.
    ///
    /// The live figure is the **spend**: the bytes the run caused, whatever
    /// became of them. The settled figure is the **accomplishment**: the bytes
    /// that passed their gate. The difference between the two is the waste.
    #[test]
    fn a_meter_counts_the_spend_and_the_settled_bytes_apart() {
        let m = Meter::new();
        assert_eq!(m.live(), 0);
        assert_eq!(m.settled(), 0);
        // 100 bytes cross the wire and 60 of them pass their gate.
        assert!(m.charge_live(100));
        m.settle(60);
        assert_eq!(m.live(), 100, "the spend is what a ceiling bounds");
        assert_eq!(m.settled(), 60, "the settled figure is what is reported");
        // A resource that delivered **nothing** is the only thing that gives a
        // charge back.
        assert!(m.charge_live(10));
        m.refund_live(10);
        assert_eq!(m.live(), 100);
        // A resource that was consumed and then failed is *not* refunded: those
        // bytes were spent, and the difference between the two numbers is
        // exactly what says so.
        assert!(m.charge_live(10));
        assert_eq!(m.live(), 110);
        assert_eq!(m.settled(), 60);
    }

    /// The two ceilings bound different numbers: the live one is checked when
    /// the bytes are spent, the settled one when they are settled.
    #[test]
    fn the_two_ceilings_bound_different_numbers() {
        let mut m = Meter::new();
        m.set_live_limit(Some(100));
        m.set_settled_limit(Some(50));
        assert_eq!(m.live_limit(), Some(100));
        assert_eq!(m.settled_limit(), Some(50));
        assert!(m.charge_live(100));
        assert!(
            !m.charge_live(1),
            "the live ceiling refuses, and adds nothing"
        );
        assert_eq!(m.live(), 100);
        // The settled ceiling is checked on the settle, not on the charge: the
        // bytes were spent either way, and a figure that stopped counting
        // would be a lie.
        assert!(!m.settle(60), "the settled ceiling bound");
        assert_eq!(m.settled(), 60);
        // The boundary passes: "> N" trips, "== N" does not.
        let mut m = Meter::new();
        m.set_settled_limit(Some(50));
        assert!(m.settle(50), "the boundary passes");
        assert!(!m.settle(1), "one more byte trips it");
    }

    #[test]
    fn an_unlimited_summary_always_accepts() {
        let s = Summary::default();
        assert_eq!(s.max_download_bytes(), None);
        assert!(s.reserve_download_bytes(1));
        assert!(s.reserve_download_bytes(u64::MAX / 2));
        assert_eq!(s.download.live(), 1 + u64::MAX / 2);
    }

    #[test]
    fn a_reservation_is_all_or_nothing_and_the_boundary_is_inclusive() {
        let mut s = Summary::default();
        s.set_download_limit(Some(100));
        assert_eq!(s.max_download_bytes(), Some(100));
        assert!(s.reserve_download_bytes(60));
        assert_eq!(s.download.live(), 60);
        // 60 + 40 == the limit: allowed.
        assert!(s.reserve_download_bytes(40));
        assert_eq!(s.download.live(), 100);
        // One more byte: refused, and nothing was added.
        assert!(!s.reserve_download_bytes(1));
        assert_eq!(s.download.live(), 100, "no partial add");
        // A zero-byte reservation must still not slip past a zero ceiling.
        let mut s0 = Summary::default();
        s0.set_download_limit(Some(0));
        assert!(s0.reserve_download_bytes(0), "0 <= 0");
        assert!(!s0.reserve_download_bytes(1));
    }

    #[test]
    fn concurrent_reservations_never_exceed_the_ceiling() {
        let mut s = Summary::default();
        s.set_download_limit(Some(1000));
        let s = Arc::new(s);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let s = s.clone();
                std::thread::spawn(move || {
                    let mut taken = 0u64;
                    for _ in 0..100 {
                        if s.reserve_download_bytes(100) {
                            taken += 1;
                        }
                    }
                    taken
                })
            })
            .collect();
        let total: u64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
        let counted = s.download.live();
        assert_eq!(
            total * 100,
            counted,
            "every accepted reservation counted once"
        );
        assert!(
            counted <= 1000,
            "eight racing reservations overshot: {counted}"
        );
    }

    #[test]
    fn the_stop_flag_survives_a_snapshot() {
        // `repair::run` reads `download_stopped` *after* `repair_all` has
        // snapshotted the Summary, so the copy must carry it.
        let s = Summary::default();
        assert!(!s.download_stopped());
        s.stop_downloads();
        assert!(s.download_stopped());
        assert!(s.snapshot().download_stopped());
    }

    #[test]
    fn a_delta_folds_back_into_a_reservation() {
        let s = Summary::default();
        s.reserve_download_bytes(100);
        s.adjust_download_bytes(-30);
        assert_eq!(s.download.live(), 70);
        s.adjust_download_bytes(5);
        assert_eq!(s.download.live(), 75);
        s.adjust_download_bytes(0);
        assert_eq!(s.download.live(), 75);
    }

    /// The RAM reservation has the same shape as the write reservation: all
    /// or nothing, and the boundary is inclusive. A file that cannot get its
    /// reservation degrades to the other write order — it never fails — so
    /// the refusal has to be exact.
    #[test]
    fn a_ram_reservation_is_all_or_nothing_and_the_boundary_is_inclusive() {
        let mut s = Summary::default();
        s.set_ram_limit(Some(100));
        assert_eq!(s.max_ram_bytes(), Some(100));
        assert!(s.reserve_ram_bytes(60));
        assert_eq!(s.ram_reserved.load(Ordering::Relaxed), 60);
        // 60 + 40 == the limit: allowed.
        assert!(s.reserve_ram_bytes(40));
        assert_eq!(s.ram_reserved.load(Ordering::Relaxed), 100);
        // One more byte: refused, and nothing was added.
        assert!(!s.reserve_ram_bytes(1));
        assert_eq!(
            s.ram_reserved.load(Ordering::Relaxed),
            100,
            "no partial add"
        );
        // The peak remembers what was staged at once, even after it is given
        // back.
        assert_eq!(s.ram_peak_bytes(), 100);
        s.release_ram_bytes(100);
        assert_eq!(s.ram_reserved.load(Ordering::Relaxed), 0);
        assert_eq!(s.ram_peak_bytes(), 100, "the peak is not a live total");
    }

    #[test]
    fn an_unlimited_summary_accepts_any_ram_reservation() {
        let s = Summary::default();
        assert_eq!(s.max_ram_bytes(), None);
        assert!(s.reserve_ram_bytes(1 << 30));
        assert!(s.reserve_ram_bytes(u64::MAX / 2));
    }

    #[test]
    fn concurrent_ram_reservations_never_exceed_the_budget() {
        let mut s = Summary::default();
        s.set_ram_limit(Some(1000));
        let s = Arc::new(s);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let s = s.clone();
                std::thread::spawn(move || {
                    let mut taken = 0u64;
                    for _ in 0..100 {
                        if s.reserve_ram_bytes(100) {
                            taken += 1;
                        }
                    }
                    taken
                })
            })
            .collect();
        let total: u64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(total * 100, s.ram_reserved.load(Ordering::Relaxed));
        assert!(
            s.ram_reserved.load(Ordering::Relaxed) <= 1000,
            "eight racing reservations overshot"
        );
    }
}
