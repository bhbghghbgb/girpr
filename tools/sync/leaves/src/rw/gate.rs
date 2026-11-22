//! [`RwLimiter`] and the semaphores behind it.
//!
//! One semaphore under [`RwLimits::Shared`], two under [`RwLimits::Split`]. This is
//! where the deadlock-freedom argument in the module docs is actually enforced —
//! the ordering inside [`RwLimiter::acquire_copy`] is the invariant.

use tokio::sync::{Semaphore, SemaphorePermit};

use super::RwLimits;
use super::limits::RwSide;

/// A counting permit pool: at most `limit` holders at once, released on drop.
///
/// Backed by [`tokio::sync::Semaphore`] rather than a hand-rolled
/// `Mutex<usize> + Condvar`: `std::sync::Semaphore` is still unstable, and tokio's
/// is the maintained, fair implementation of exactly this primitive. This crate
/// has no async runtime, so every acquire uses the blocking form; a permit is
/// returned to the semaphore when its [`SemaphorePermit`] is dropped, including
/// during unwinding.
struct Gate {
    sem: Semaphore,
    limit: usize,
}

impl Gate {
    fn new(limit: usize) -> Self {
        let limit = limit.max(1);
        Self {
            sem: Semaphore::new(limit),
            limit,
        }
    }

    fn acquire(&self) -> SemaphorePermit<'_> {
        // No tokio runtime exists in this crate, so `blocking_acquire` cannot
        // panic for being in an async context. The only other error is a closed
        // semaphore, and these are never closed.
        self.sem
            .blocking_acquire()
            .expect("an rw semaphore is never closed")
    }

    fn try_acquire(&self) -> Option<SemaphorePermit<'_>> {
        // Non-blocking, so a limit can be *asserted*: the answer `false` must be
        // available without waiting for a blocking acquire to return.
        self.sem.try_acquire().ok()
    }

    fn in_flight(&self) -> usize {
        // Permits are only ever taken by `acquire`/`try_acquire` and given back by
        // `SemaphorePermit::drop`; none are added or forgotten, so the outstanding
        // count is exactly the difference between the pool size and what is free.
        self.limit - self.sem.available_permits()
    }
}

/// The counters a [`RwRuntime`](super::RwRuntime) runs on.
///
/// One semaphore under [`RwLimits::Shared`] and two under [`RwLimits::Split`]; all
/// three exist at once because the shared counter's limit is then read by nobody, and a
/// struct whose shape changes with the mode would have to be rebuilt per run for
/// no reason.
pub struct RwLimiter {
    limits: RwLimits,
    shared: Gate,
    src: Gate,
    dst: Gate,
}

impl RwLimiter {
    pub fn new(limits: RwLimits) -> Self {
        let (shared, src, dst) = match limits {
            RwLimits::Shared(n) => (n.get(), n.get(), n.get()),
            RwLimits::Split { src, dst } => (1, src.get(), dst.get()),
        };
        Self {
            limits,
            shared: Gate::new(shared),
            src: Gate::new(src),
            dst: Gate::new(dst),
        }
    }

    pub fn limits(&self) -> RwLimits {
        self.limits
    }

    /// The semaphore a single-sided operation draws on. Under [`RwLimits::Shared`]
    /// both sides draw on the one counter, which is what "shared" means.
    fn gate_for(&self, side: RwSide) -> &Gate {
        match self.limits {
            RwLimits::Shared(_) => &self.shared,
            RwLimits::Split { .. } => match side {
                RwSide::Src => &self.src,
                RwSide::Dst => &self.dst,
            },
        }
    }

    /// One permit for one side's read or write.
    pub fn acquire(&self, side: RwSide) -> RwPermit<'_> {
        let gate = self.gate_for(side);
        RwPermit {
            held: [Some(gate.acquire()), None],
        }
    }

    /// src, then dst — or the single shared permit.
    ///
    /// **The ordering here is the deadlock-freedom invariant.** A copy is the only
    /// operation that takes two permits, it always takes src first, and a thread
    /// holding dst takes nothing further — so every wait-for edge points forward
    /// along `src → dst` and the graph has no cycle. Changing the order below is
    /// not a cosmetic edit: it would be the first way this crate can hang a
    /// run rather than fail one.
    pub fn acquire_copy(&self) -> RwPermit<'_> {
        match self.limits {
            RwLimits::Shared(_) => self.acquire(RwSide::Src),
            RwLimits::Split { .. } => {
                let (src, dst) = (self.gate_for(RwSide::Src), self.gate_for(RwSide::Dst));
                // src first, then dst: see the invariant above. Holding src while
                // waiting for dst is safe precisely because a dst holder never
                // waits for another permit.
                let src_permit = src.acquire();
                let dst_permit = dst.acquire();
                RwPermit {
                    held: [Some(src_permit), Some(dst_permit)],
                }
            }
        }
    }

    /// Non-blocking, so a limit can be *asserted* — a test asking "is the limit
    /// really enforced?" must be able to ask the question the answer to which is
    /// `false` without arranging for the blocking form to return.
    pub fn try_acquire(&self, side: RwSide) -> Option<RwPermit<'_>> {
        let gate = self.gate_for(side);
        gate.try_acquire().map(|permit| RwPermit {
            held: [Some(permit), None],
        })
    }

    /// Permits held on one side right now. Diagnostics and tests only: a run has no
    /// reason to look, and a run that did would be reading a number that is only
    /// meaningful between two statements.
    pub fn in_flight(&self, side: RwSide) -> usize {
        self.gate_for(side).in_flight()
    }
}

/// Held permits, released on drop in reverse acquisition order.
///
/// The drop is what makes the gate correct under an early `?`: every caller in this
/// crate returns from inside a scope that owns the permit, so a hash that fails,
/// a copy that fails to verify and a worker that panics all give the permit back
/// on the way out. Nothing has to remember to release it. [`SemaphorePermit`]
/// performs the release, including while unwinding.
pub struct RwPermit<'a> {
    /// At most two permits. Order is acquisition order; drop walks it backwards.
    held: [Option<SemaphorePermit<'a>>; 2],
}

impl Drop for RwPermit<'_> {
    fn drop(&mut self) {
        // dst before src, mirroring `acquire_copy`. Releases never block, so this
        // is not itself a deadlock requirement — but keeping the reverse order
        // means the teardown is the exact mirror of the setup it undoes.
        for permit in self.held.iter_mut().rev() {
            drop(permit.take());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::rw::fixtures::{shared, split};

    /// The one-counter mode has to be one counter: a permit held on either side
    /// denies the other, which is the entire difference from [`split`].
    #[test]
    fn shared_mode_is_one_counter() {
        let l = shared(1);
        let _held = l.acquire(RwSide::Src);
        assert!(l.try_acquire(RwSide::Dst).is_none());
        assert!(l.try_acquire(RwSide::Src).is_none());
        assert_eq!(
            l.in_flight(RwSide::Src),
            1,
            "a refused try must not disturb the counter"
        );
    }

    /// ...and the split mode has to be two, or the feature is a rename.
    #[test]
    fn split_mode_has_two_counters() {
        let l = split(1, 1);
        let _held = l.acquire(RwSide::Src);
        assert!(l.try_acquire(RwSide::Src).is_none(), "src is full");
        assert!(l.try_acquire(RwSide::Dst).is_some(), "dst is not");
    }

    /// A copy draws on both sides, so under `Split { 1, 1 }` one in flight leaves
    /// no capacity on either side — and only because of the *src* draw. Under a
    /// one-permit copy this assertion would pass with src at one and dst at zero.
    #[test]
    fn a_copy_holds_both_sides() {
        let l = split(1, 1);
        let _copy = l.acquire_copy();
        assert_eq!(l.in_flight(RwSide::Src), 1);
        assert_eq!(l.in_flight(RwSide::Dst), 1);
        assert!(l.try_acquire(RwSide::Src).is_none());
        assert!(l.try_acquire(RwSide::Dst).is_none());
    }

    /// A copy under a shared counter costs exactly one permit, so `--rw-threads 4`
    /// still means four concurrent copies.
    #[test]
    fn a_shared_copy_costs_one_permit() {
        let l = shared(4);
        let _a = l.acquire_copy();
        let _b = l.acquire_copy();
        let _c = l.acquire_copy();
        let _d = l.acquire_copy();
        assert!(l.try_acquire(RwSide::Src).is_none(), "four is all of it");
    }

    #[test]
    fn permits_are_released_on_drop() {
        let l = shared(2);
        for _ in 0..100 {
            let a = l.acquire(RwSide::Src);
            let b = l.acquire(RwSide::Dst);
            assert_eq!(l.in_flight(RwSide::Src), 2);
            drop((a, b));
        }
        assert_eq!(l.in_flight(RwSide::Src), 0);
    }

    /// The release lives in `Drop`, so unwinding takes the permit with it. A worker
    /// that panicked holding one and did not give it back would take a permit out
    /// of the run permanently — and the next copy would block forever on it.
    #[test]
    fn permits_are_released_on_panic() {
        let l = shared(1);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = l.acquire_copy();
            panic!("worker died holding a permit");
        }));
        assert!(panicked.is_err(), "the unwind happened");
        assert_eq!(l.in_flight(RwSide::Src), 0);
        assert!(
            l.try_acquire(RwSide::Src).is_some(),
            "the run can still make progress"
        );
    }
}
