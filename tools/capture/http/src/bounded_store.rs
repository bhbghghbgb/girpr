//! The mutex-backed mechanics shared by gircap's run-wide bounded stores.
//!
//! What is common is small: rows behind one lock, poison recovery, a count of
//! refusals, a byte counter for budgets, and cloning the retained rows into the
//! order a document publishes. What differs is policy — which key admits or
//! replaces a row, when a row is dropped, how a byte charge is settled — and
//! each store injects that at its own call site. Nothing here chooses a cap,
//! a key or an order, and nothing here builds a document or writes a byte.

use std::sync::{Mutex, MutexGuard};

/// The state every store keeps behind its lock.
///
/// A store uses the fields its policy needs: `omitted` counts refusals a row
/// cap did not retain, `used` tracks bytes promised to retained rows or live
/// reservations. Both stay in the shared state so the lock covers the rows and
/// their accounting together, exactly as it did before the mechanics moved.
#[derive(Debug)]
pub struct State<R> {
    /// The rows the store currently retains, in that store's own order.
    pub rows: Vec<R>,
    /// Refusals not retained: rows past a cap, or a key already retained.
    pub omitted: u64,
    /// Bytes charged against a retention budget.
    pub used: u64,
}

impl<R> Default for State<R> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            omitted: 0,
            used: 0,
        }
    }
}

/// Rows shared by every worker, read under one lock.
#[derive(Debug)]
pub struct BoundedStore<R> {
    state: Mutex<State<R>>,
}

impl<R> Default for BoundedStore<R> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
        }
    }
}

impl<R> BoundedStore<R> {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Lock the state, recovering a poisoned mutex.
    ///
    /// A panic in one worker must not make every later read of the evidence
    /// fail: the state it left behind is still the best account of the run.
    /// This is the same recovery every store used before the mechanics moved.
    pub fn lock(&self) -> MutexGuard<'_, State<R>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Clone the retained rows into the order the document publishes.
    ///
    /// The ordering is injected: the byte-budget store sorts by request
    /// sequence, the other two keep the key order their inserts maintain.
    /// Cloning and sorting happen off the document path and never change the
    /// stored rows or their accounting.
    pub fn snapshot(&self, order: impl FnOnce(&mut [R])) -> Vec<R>
    where
        R: Clone,
    {
        let mut rows = self.lock().rows.clone();
        order(&mut rows);
        rows
    }
}

/// Charge up to `max` bytes against `budget`, returning what was granted.
///
/// The charge is made while the state is locked, so concurrent readers divide
/// a budget instead of each seeing all of it. `used` is the store's own counter
/// and this is the byte-budget policy the error-body store injects.
pub fn charge(budget: u64, used: &mut u64, max: u64) -> u64 {
    let room = budget.saturating_sub(*used);
    let granted = room.min(max);
    *used = used.saturating_add(granted);
    granted
}

/// Settle a grant that ultimately retained `retained` bytes.
///
/// The unused part of the grant is refunded and what was kept stays charged.
/// Saturation keeps the accounting defined at `u64::MAX`.
pub fn settle(used: &mut u64, granted: u64, retained: u64) {
    *used = used.saturating_sub(granted).saturating_add(retained);
}
