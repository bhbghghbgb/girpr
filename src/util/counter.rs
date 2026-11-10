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
    use std::sync::{Arc, Barrier, Condvar, Mutex};
    use std::time::{Duration, Instant};

    /// A two-party rendezvous that a panic guard can open early.
    ///
    /// A [`Barrier`] releases every party at once but cannot be opened before
    /// both arrive. A guard that tries to release a possibly-parked helper by
    /// adding a party therefore parks itself when the rendezvous has already
    /// completed: the barrier has moved to its next generation. This gate keeps
    /// the normal path's meaning — both parties must arrive — and gives the
    /// failure path a [`Gate::release`] that opens the gate for a parked
    /// party without ever adding a waiter.
    pub(crate) struct Gate {
        state: Mutex<GateState>,
        opened: Condvar,
        arrivals: Condvar,
    }

    #[derive(Default)]
    struct GateState {
        arrived: usize,
        open: bool,
        releases: usize,
    }

    impl Gate {
        pub(crate) fn new() -> Self {
            Self {
                state: Mutex::new(GateState::default()),
                opened: Condvar::new(),
                arrivals: Condvar::new(),
            }
        }

        /// Wait until both parties have arrived or the gate is opened.
        pub(crate) fn arrive_and_wait(&self) {
            let mut state = self.state.lock().unwrap();
            state.arrived += 1;
            self.arrivals.notify_all();
            if state.arrived == 2 {
                state.open = true;
                self.opened.notify_all();
            }
            while !state.open {
                state = self.opened.wait(state).unwrap();
            }
        }

        /// Open the gate for a waiting party and for any party that arrives
        /// later.
        ///
        /// Repeating this after a normal release leaves the gate's state
        /// unchanged: the gate stays open and no party is added. The call
        /// still counts in [`Gate::release_count`], so the tally records
        /// every release and not only the ones that opened the gate.
        pub(crate) fn release(&self) {
            let mut state = self.state.lock().unwrap();
            state.releases += 1;
            state.open = true;
            self.opened.notify_all();
        }

        /// How many parties have called [`Gate::arrive_and_wait`].
        pub(crate) fn arrival_count(&self) -> usize {
            self.state.lock().unwrap().arrived
        }

        /// How many times [`Gate::release`] was called, including the calls
        /// that left the gate's state unchanged.
        pub(crate) fn release_count(&self) -> usize {
            self.state.lock().unwrap().releases
        }

        /// Whether the gate is open.
        pub(crate) fn is_open(&self) -> bool {
            self.state.lock().unwrap().open
        }

        /// Wait until at least `n` parties have arrived, or `timeout` elapses.
        ///
        /// Returns whether the parties arrived. This is how a test observes a
        /// party parked at the gate without sleeping or polling the count:
        /// the arrival notifies the wait directly.
        pub(crate) fn wait_for_arrivals(&self, n: usize, timeout: Duration) -> bool {
            let deadline = Instant::now() + timeout;
            let mut state = self.state.lock().unwrap();
            while state.arrived < n {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return false;
                }
                let (next, _) = self.arrivals.wait_timeout(state, remaining).unwrap();
                state = next;
            }
            true
        }
    }

    /// The wait a pause performs: a plain barrier for the wrap race, or a gate
    /// the owning test can open from its panic guard.
    #[derive(Clone)]
    enum Wait {
        Barrier(Arc<Barrier>),
        Gate(Arc<Gate>),
    }

    impl Wait {
        fn wait(&self) {
            match self {
                Self::Barrier(barrier) => {
                    barrier.wait();
                }
                Self::Gate(gate) => gate.arrive_and_wait(),
            }
        }
    }

    /// The cell a pause is installed for, so another counter charged on the
    /// same thread does not stop at it.
    struct Pause {
        cell: usize,
        wait: Wait,
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
        CAS_PAUSE.with(|pause| {
            *pause.borrow_mut() = Some(Pause {
                cell,
                wait: Wait::Barrier(barrier),
            })
        });
        PauseGuard { cell }
    }

    /// Pause this thread at a gate the owning test can open from a panic guard.
    pub(crate) fn pause_before_cas_of_gate(cell: &AtomicU64, gate: Arc<Gate>) -> PauseGuard {
        let cell = cell as *const AtomicU64 as usize;
        CAS_PAUSE.with(|pause| {
            *pause.borrow_mut() = Some(Pause {
                cell,
                wait: Wait::Gate(gate),
            })
        });
        PauseGuard { cell }
    }

    /// Wait at this thread's pause, when it is installed for `cell`.
    pub(crate) fn pause_before_cas(cell: &AtomicU64) {
        let cell = cell as *const AtomicU64 as usize;
        let wait = CAS_PAUSE.with(|pause| match &*pause.borrow() {
            Some(p) if p.cell == cell => Some(p.wait.clone()),
            _ => None,
        });
        if let Some(wait) = wait {
            wait.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::add_saturating;
    use super::test_support::{Gate, pause_before_cas_of_gate};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    /// How long the test receiver waits before deciding the helper is parked
    /// at its gate, so a broken gate fails the test instead of hanging it.
    const RECEIVE_WAIT: Duration = Duration::from_secs(10);

    /// Opens the two-party gate when a failing test unwinds.
    ///
    /// A helper parked at the gate on the failure path would otherwise stay
    /// there until the process exits. Opening an already-open gate leaves its
    /// state unchanged, so a guard that drops after the normal release — but
    /// before the helper records `passed` — cannot leave a waiter parked on a
    /// fresh generation.
    struct ReleaseOnPanic {
        gate: Arc<Gate>,
        passed: Arc<AtomicBool>,
    }

    impl Drop for ReleaseOnPanic {
        fn drop(&mut self) {
            if std::thread::panicking() && !self.passed.load(Ordering::Acquire) {
                self.gate.release();
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
    /// which would park the charge at a gate only it could reach — fails
    /// the test rather than hanging the suite.
    #[test]
    fn a_paused_cell_does_not_hold_a_charge_to_another_cell() {
        let paused = Arc::new(AtomicU64::new(0));
        let other = Arc::new(AtomicU64::new(0));

        let gate = Arc::new(Gate::new());
        let passed = Arc::new(AtomicBool::new(false));
        let _release = ReleaseOnPanic {
            gate: Arc::clone(&gate),
            passed: Arc::clone(&passed),
        };

        let thread_paused = Arc::clone(&paused);
        let thread_other = Arc::clone(&other);
        let thread_passed = Arc::clone(&passed);
        let thread_gate = Arc::clone(&gate);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let charger = std::thread::spawn(move || {
            // A charge that wrongly consulted this pause would wait at the
            // two-party gate instead of reaching its compare-exchange.
            let _pause = pause_before_cas_of_gate(&thread_paused, thread_gate);

            add_saturating(&thread_other, 5);
            thread_passed.store(true, Ordering::Release);
            done_tx.send(()).expect("the test receiver is alive");
        });

        done_rx
            .recv_timeout(RECEIVE_WAIT)
            .expect("a charge to another cell must not wait on the paused cell's gate");
        charger.join().expect("the charging thread finishes");
        assert_eq!(
            other.load(Ordering::Relaxed),
            5,
            "the other cell charges while a pause is installed for a different one"
        );
    }

    /// A pause is per charge, not a permanent block: the paused charge holds
    /// at a two-party gate until this thread's helper arrives, then commits.
    /// Clearing the pause then leaves the cell charging normally.
    ///
    /// A one-party gate would release immediately and prove nothing. Here the
    /// charge cannot pass the gate on its own, and a skipped pause leaves the
    /// helper waiting at it — the bounded receive reports that as a failure
    /// instead of hanging the suite.
    #[test]
    fn a_paused_cell_resumes_after_its_gate() {
        let cell = Arc::new(AtomicU64::new(0));
        let gate = Arc::new(Gate::new());
        let passed = Arc::new(AtomicBool::new(false));
        let _release = ReleaseOnPanic {
            gate: Arc::clone(&gate),
            passed: Arc::clone(&passed),
        };

        let (released_tx, released_rx) = std::sync::mpsc::channel();
        let releaser = {
            let gate = Arc::clone(&gate);
            let passed = Arc::clone(&passed);
            std::thread::spawn(move || {
                gate.arrive_and_wait();
                passed.store(true, Ordering::Release);
                released_tx.send(()).expect("the test receiver is alive");
            })
        };

        let guard = pause_before_cas_of_gate(&cell, Arc::clone(&gate));
        add_saturating(&cell, 7);
        assert_eq!(
            cell.load(Ordering::Relaxed),
            7,
            "the paused cell charges once its gate releases"
        );
        released_rx
            .recv_timeout(RECEIVE_WAIT)
            .expect("the paused charge released its gate's second party");
        releaser.join().expect("the releasing thread finishes");

        drop(guard);
        add_saturating(&cell, 1);
        assert_eq!(
            cell.load(Ordering::Relaxed),
            8,
            "clearing the pause leaves the cell charging normally"
        );
    }

    /// Dropping the guard after the gate has released the helper but before
    /// the helper records `passed` must not add a waiter: the gate is already
    /// open, and opening it again leaves its state unchanged. The helper then
    /// finishes normally, so the window between its wake-up and its store is
    /// covered by a test rather than left to interleaving.
    #[test]
    fn a_panic_guard_after_the_release_leaves_no_waiter() {
        let gate = Arc::new(Gate::new());
        let passed = Arc::new(AtomicBool::new(false));
        let held = Arc::new((Mutex::new(false), Condvar::new()));
        let (done_tx, done_rx) = std::sync::mpsc::channel();

        let helper = {
            let gate = Arc::clone(&gate);
            let passed = Arc::clone(&passed);
            let held = Arc::clone(&held);
            std::thread::spawn(move || {
                gate.arrive_and_wait();
                // The gate has released this helper. Hold it before it records
                // `passed`, which is exactly the window the guard races.
                let (lock, cv) = &*held;
                let mut released = lock.lock().unwrap();
                *released = true;
                cv.notify_all();
                while *released {
                    released = cv.wait(released).unwrap();
                }
                drop(released);

                passed.store(true, Ordering::Release);
                done_tx.send(()).expect("the test receiver is alive");
            })
        };

        // The second party: this releases the helper and completes the normal
        // rendezvous path.
        gate.arrive_and_wait();

        let (lock, cv) = &*held;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = cv.wait(released).unwrap();
        }
        drop(released);

        // Drop the guard while the helper is released but `passed` is still
        // false. Opening the already-open gate cannot leave a parked waiter.
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _release = ReleaseOnPanic {
                gate: Arc::clone(&gate),
                passed: Arc::clone(&passed),
            };
            panic!("simulated failure after the gate released the helper");
        }));
        assert!(panicked.is_err(), "the simulated failure unwinds");
        assert_eq!(
            gate.release_count(),
            1,
            "the guard opens the already-open gate once"
        );

        // Let the helper record `passed` and finish.
        let (lock, cv) = &*held;
        let mut released = lock.lock().unwrap();
        *released = false;
        cv.notify_all();
        drop(released);

        done_rx
            .recv_timeout(RECEIVE_WAIT)
            .expect("the helper finishes after the guard drop");
        helper.join().expect("the helper thread finishes");
        assert_eq!(
            gate.arrival_count(),
            2,
            "the guard adds no party to an already-released rendezvous"
        );
        assert!(
            passed.load(Ordering::Acquire),
            "the helper recorded passed after the guard drop"
        );
    }

    /// The panic-release path must also open a gate that still has a party
    /// parked at it, not only a gate that is already open.
    ///
    /// The party parks at the gate with the gate closed, the guard releases
    /// it while unwinding, and the party must come out of the gate. The test
    /// waits for the arrival on the gate's own condition variable, so it
    /// neither sleeps nor polls, and the receive has a deadline, so a guard
    /// that failed to release the party fails the test instead of hanging it.
    #[test]
    fn a_panic_guard_releases_a_party_parked_at_the_gate() {
        let gate = Arc::new(Gate::new());
        let passed = Arc::new(AtomicBool::new(false));
        let (done_tx, done_rx) = std::sync::mpsc::channel();

        let party = {
            let gate = Arc::clone(&gate);
            let passed = Arc::clone(&passed);
            std::thread::spawn(move || {
                gate.arrive_and_wait();
                passed.store(true, Ordering::Release);
                done_tx.send(()).expect("the test receiver is alive");
            })
        };

        assert!(
            gate.wait_for_arrivals(1, RECEIVE_WAIT),
            "the party parks at the gate"
        );
        assert_eq!(
            gate.arrival_count(),
            1,
            "the parked party is the gate's first arrival"
        );
        assert!(
            !gate.is_open(),
            "the gate is closed while a party is parked at it"
        );
        assert_eq!(
            gate.release_count(),
            0,
            "the gate has not been released before the guard fires"
        );

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _release = ReleaseOnPanic {
                gate: Arc::clone(&gate),
                passed: Arc::clone(&passed),
            };
            panic!("simulated failure with a party parked at the gate");
        }));
        assert!(panicked.is_err(), "the simulated failure unwinds");

        done_rx
            .recv_timeout(RECEIVE_WAIT)
            .expect("the guard releases the party parked at the gate");
        party.join().expect("the released party finishes");

        assert_eq!(gate.release_count(), 1, "the guard releases the gate once");
        assert_eq!(
            gate.arrival_count(),
            1,
            "the release adds no party to the gate"
        );
        assert!(gate.is_open(), "the gate stays open after the release");
        assert!(
            passed.load(Ordering::Acquire),
            "the released party recorded passed"
        );
    }
}
