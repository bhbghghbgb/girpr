//! Saturating atomic counters shared by the report and the repair ledger.
//!
//! A counter that is charged from several worker threads cannot use
//! `fetch_add`: it wraps, and a wrapped byte total reads near `u64::MAX` — which
//! switches a ceiling off and corrupts the reported figures. A load followed by
//! `fetch_add` does not fix that either: another thread can push the cell past
//! the top between the two. The decision to add has to be the same atomic step
//! that performs it, so every site that only needs "add without wrapping" goes
//! through this one compare-and-swap loop.

use std::sync::atomic::{AtomicU64, Ordering};

/// Add `n` to `cell` without wrapping, saturating at [`u64::MAX`].
///
/// The loop retries with whatever the cell holds whenever it moved, so every
/// value it commits is `saturating_add` applied to the value it replaces, and
/// no interleaving can wrap. The load, the successful and the failed
/// compare-exchange are all `Relaxed`, matching the counters this replaces: the
/// value is a tally, not a publication point.
pub(crate) fn add_saturating(cell: &AtomicU64, n: u64) {
    let mut cur = cell.load(Ordering::Relaxed);
    #[cfg(test)]
    test_support::pause_before_cas(cell);
    loop {
        let next = cur.saturating_add(n);
        match cell.compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(actual) => cur = actual,
        }
    }
}

/// The pause hook that makes the wrap race deterministic in tests.
#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;
    use std::sync::atomic::AtomicU64;
    use std::sync::{Arc, Barrier};

    /// The cell a pause is installed for, so another counter charged on the
    /// same thread does not stop at it.
    struct Pause {
        cell: usize,
        barrier: Arc<Barrier>,
    }

    thread_local! {
        /// A test-only pause after the load and before the compare-exchange.
        static CAS_PAUSE: RefCell<Option<Pause>> = const { RefCell::new(None) };
    }

    /// Clears this thread's pause when dropped, including on unwind.
    pub(crate) struct PauseGuard {
        cell: usize,
    }

    impl Drop for PauseGuard {
        fn drop(&mut self) {
            CAS_PAUSE.with(|pause| {
                let mut pause = pause.borrow_mut();
                let ours = pause.as_ref().is_some_and(|p| p.cell == self.cell);
                if ours {
                    *pause = None;
                }
            });
        }
    }

    /// Pause this thread before the next compare-exchange of `cell`.
    ///
    /// This is what makes the wrap race deterministic: every racing thread
    /// loads the counter, then stops here, so all of them are guaranteed to
    /// have seen the same value before any of them tries to commit. The pause
    /// is keyed to `cell`, so a charge against a different counter on the same
    /// thread runs to completion instead of waiting on this barrier.
    pub(crate) fn pause_before_cas_of(cell: &AtomicU64, barrier: Arc<Barrier>) -> PauseGuard {
        let cell = cell as *const AtomicU64 as usize;
        CAS_PAUSE.with(|pause| *pause.borrow_mut() = Some(Pause { cell, barrier }));
        PauseGuard { cell }
    }

    /// Wait at this thread's pause barrier, when it is installed for `cell`.
    pub(crate) fn pause_before_cas(cell: &AtomicU64) {
        let cell = cell as *const AtomicU64 as usize;
        let barrier = CAS_PAUSE.with(|pause| match &*pause.borrow() {
            Some(p) if p.cell == cell => Some(p.barrier.clone()),
            _ => None,
        });
        if let Some(barrier) = barrier {
            barrier.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::add_saturating;
    use super::test_support::pause_before_cas_of;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    /// How long the test receiver waits before deciding the helper is parked
    /// at its barrier, so a broken gate fails the test instead of hanging it.
    const RECEIVE_WAIT: Duration = Duration::from_secs(10);

    /// Releases a two-party barrier when a failing test unwinds.
    ///
    /// A helper parked at the barrier on the failure path would otherwise stay
    /// there until the process exits. The guard only spawns the releasing
    /// waiter when the test is already unwinding and the helper has not yet
    /// passed the barrier; a passing test spawns nothing.
    struct ReleaseOnPanic {
        barrier: Arc<Barrier>,
        passed: Arc<AtomicBool>,
    }

    impl Drop for ReleaseOnPanic {
        fn drop(&mut self) {
            if std::thread::panicking() && !self.passed.load(Ordering::Acquire) {
                let barrier = Arc::clone(&self.barrier);
                std::thread::spawn(move || barrier.wait());
            }
        }
    }

    /// A pause installed for one cell does not hold a charge to a different
    /// cell on the same thread: the scope gate is the cell identity, not "this
    /// thread has some pause installed".
    ///
    /// The charge runs on its own thread, because the pause is thread-local
    /// and has to be installed on the charging thread to be observable at all.
    /// The receive has a deadline so a gate that consulted the wrong cell —
    /// which would park the charge at a barrier only it could reach — fails
    /// the test rather than hanging the suite.
    #[test]
    fn a_paused_cell_does_not_hold_a_charge_to_another_cell() {
        let paused = Arc::new(AtomicU64::new(0));
        let other = Arc::new(AtomicU64::new(0));

        let barrier = Arc::new(Barrier::new(2));
        let passed = Arc::new(AtomicBool::new(false));
        let _release = ReleaseOnPanic {
            barrier: Arc::clone(&barrier),
            passed: Arc::clone(&passed),
        };

        let thread_paused = Arc::clone(&paused);
        let thread_other = Arc::clone(&other);
        let thread_passed = Arc::clone(&passed);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let charger = std::thread::spawn(move || {
            // A charge that wrongly consulted this pause would wait at the
            // two-party barrier instead of reaching its compare-exchange.
            let _pause = pause_before_cas_of(&thread_paused, barrier);

            add_saturating(&thread_other, 5);
            thread_passed.store(true, Ordering::Release);
            done_tx.send(()).expect("the test receiver is alive");
        });

        done_rx
            .recv_timeout(RECEIVE_WAIT)
            .expect("a charge to another cell must not wait on the paused cell's barrier");
        charger.join().expect("the charging thread finishes");
        assert_eq!(
            other.load(Ordering::Relaxed),
            5,
            "the other cell charges while a pause is installed for a different one"
        );
    }

    /// A pause is per charge, not a permanent block: the paused charge holds
    /// at a two-party barrier until this thread's helper arrives, then
    /// commits. Clearing the pause then leaves the cell charging normally.
    ///
    /// A one-party barrier would release immediately and prove nothing. Here
    /// the charge cannot pass the barrier on its own, and a skipped pause
    /// leaves the helper waiting at it — the bounded receive reports that as a
    /// failure instead of hanging the suite.
    #[test]
    fn a_paused_cell_resumes_after_its_barrier() {
        let cell = Arc::new(AtomicU64::new(0));
        let barrier = Arc::new(Barrier::new(2));
        let passed = Arc::new(AtomicBool::new(false));
        let _release = ReleaseOnPanic {
            barrier: Arc::clone(&barrier),
            passed: Arc::clone(&passed),
        };

        let (released_tx, released_rx) = std::sync::mpsc::channel();
        let releaser = {
            let barrier = Arc::clone(&barrier);
            let passed = Arc::clone(&passed);
            std::thread::spawn(move || {
                barrier.wait();
                passed.store(true, Ordering::Release);
                released_tx.send(()).expect("the test receiver is alive");
            })
        };

        let guard = pause_before_cas_of(&cell, Arc::clone(&barrier));
        add_saturating(&cell, 7);
        assert_eq!(
            cell.load(Ordering::Relaxed),
            7,
            "the paused cell charges once its barrier releases"
        );
        released_rx
            .recv_timeout(RECEIVE_WAIT)
            .expect("the paused charge released its barrier's second party");
        releaser.join().expect("the releasing thread finishes");

        drop(guard);
        add_saturating(&cell, 1);
        assert_eq!(
            cell.load(Ordering::Relaxed),
            8,
            "clearing the pause leaves the cell charging normally"
        );
    }
}
