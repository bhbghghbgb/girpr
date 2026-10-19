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

use std::sync::atomic::Ordering;

use crate::report::Summary;

impl Summary {
    /// `--max-download-bytes`, or `None` when the flag is absent.
    pub fn max_download_bytes(&self) -> Option<u64> {
        let l = self.download_limit.load(Ordering::Relaxed);
        (l != u64::MAX).then_some(l)
    }

    /// Install the runtime ceiling. Called once, before any task starts.
    /// `None` is stored as `u64::MAX`, so the hot path is a compare-and-swap
    /// against a number rather than an `Option` check.
    pub fn set_download_limit(&mut self, limit: Option<u64>) {
        self.download_limit
            .store(limit.unwrap_or(u64::MAX), Ordering::Relaxed);
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
        let limit = self.download_limit.load(Ordering::Relaxed);
        let mut cur = self.download_bytes.load(Ordering::Relaxed);
        loop {
            let next = cur.saturating_add(n);
            if next > limit {
                return false;
            }
            match self.download_bytes.compare_exchange_weak(
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

    /// Correct a reservation once the real byte count is known.
    ///
    /// Capture invariant A6 says `len(blob) == compressed_size`, so the delta is
    /// normally zero; folding it in anyway keeps `SUMMARY download_bytes` exact
    /// *by construction* rather than by assumption, which is what `gircap`'s
    /// `download_bytes == patchmaps/<id>.json → totals.download_bytes`
    /// assertion is written against.
    pub fn adjust_download_bytes(&self, delta: i64) {
        if delta < 0 {
            self.download_bytes
                .fetch_sub(delta.unsigned_abs(), Ordering::Relaxed);
        } else if delta > 0 {
            self.download_bytes
                .fetch_add(delta as u64, Ordering::Relaxed);
        }
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
    /// **This is the only genuinely un-counted cost in the tool.**
    /// `SUMMARY download_bytes` counts the network; this counts the disk. Every
    /// reuse-copy (`out.write_all(&buf[..n])` in `try_reuse_slice`) and every
    /// downloaded-chunk write lands in `*_tmp` and appears in *neither*
    /// `download_bytes` nor the plan's estimate, so a zero-reuse run writes its
    /// entire payload to disk while reporting a network cost of almost nothing.
    pub fn reserve_write_bytes(&self, n: u64) -> bool {
        let limit = self.write_limit.load(Ordering::Relaxed);
        let mut cur = self.write_bytes.load(Ordering::Relaxed);
        loop {
            let next = cur.saturating_add(n);
            if next > limit {
                return false;
            }
            match self.write_bytes.compare_exchange_weak(
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

    #[test]
    fn an_unlimited_summary_always_accepts() {
        let s = Summary::default();
        assert_eq!(s.max_download_bytes(), None);
        assert!(s.reserve_download_bytes(1));
        assert!(s.reserve_download_bytes(u64::MAX / 2));
        assert_eq!(s.download_bytes.load(Ordering::Relaxed), 1 + u64::MAX / 2);
    }

    #[test]
    fn a_reservation_is_all_or_nothing_and_the_boundary_is_inclusive() {
        let mut s = Summary::default();
        s.set_download_limit(Some(100));
        assert_eq!(s.max_download_bytes(), Some(100));
        assert!(s.reserve_download_bytes(60));
        assert_eq!(s.download_bytes.load(Ordering::Relaxed), 60);
        // 60 + 40 == the limit: allowed.
        assert!(s.reserve_download_bytes(40));
        assert_eq!(s.download_bytes.load(Ordering::Relaxed), 100);
        // One more byte: refused, and nothing was added.
        assert!(!s.reserve_download_bytes(1));
        assert_eq!(
            s.download_bytes.load(Ordering::Relaxed),
            100,
            "no partial add"
        );
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
        let counted = s.download_bytes.load(Ordering::Relaxed);
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
        assert_eq!(s.download_bytes.load(Ordering::Relaxed), 70);
        s.adjust_download_bytes(5);
        assert_eq!(s.download_bytes.load(Ordering::Relaxed), 75);
        s.adjust_download_bytes(0);
        assert_eq!(s.download_bytes.load(Ordering::Relaxed), 75);
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
