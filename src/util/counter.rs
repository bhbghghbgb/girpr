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
