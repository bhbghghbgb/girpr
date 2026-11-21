//! Saturating atomic counters shared by the report and the repair ledger.
//!
//! A counter that is charged from several worker threads cannot use
//! `fetch_add`: it wraps, and a wrapped byte total reads near `u64::MAX` — which
//! switches a ceiling off and corrupts the reported figures. Every site that
//! only needs "add without wrapping" goes through [`add_saturating`], which
//! folds the saturating addition into one atomic `try_update` (the current name
//! for what was `fetch_update`).

use std::sync::atomic::{AtomicU64, Ordering};

/// Add `n` to `cell` without wrapping, saturating at [`u64::MAX`].
///
/// `try_update` retries with whatever the cell holds whenever it moved, so
/// every value it commits is `saturating_add` applied to the value it replaces,
/// and no interleaving can wrap. The load, the successful and the failed
/// compare-exchange are all `Relaxed`, matching the counters this replaces: the
/// value is a tally, not a publication point.
pub fn add_saturating(cell: &AtomicU64, n: u64) {
    let _ = cell.try_update(Ordering::Relaxed, Ordering::Relaxed, |cur| {
        Some(cur.saturating_add(n))
    });
}

#[cfg(test)]
mod tests {
    use super::add_saturating;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Barrier};

    /// A charge past the top of the range saturates instead of wrapping.
    #[test]
    fn a_charge_saturates_at_the_top_of_the_range() {
        let cell = AtomicU64::new(u64::MAX - 5);
        add_saturating(&cell, 10);
        assert_eq!(cell.load(Ordering::Relaxed), u64::MAX);
        add_saturating(&cell, 1);
        assert_eq!(cell.load(Ordering::Relaxed), u64::MAX);
    }

    /// Concurrent charges near the top saturate instead of wrapping.
    ///
    /// A `fetch_add` would wrap once the cell crossed `u64::MAX`, dropping the
    /// figure to a small number that would switch a ceiling off. A barrier
    /// releases every charging thread at once, so they contend on the same
    /// cell; in whatever order they commit, `try_update` keeps applying
    /// `saturating_add` and the final reading is `u64::MAX`.
    #[test]
    fn concurrent_charges_near_the_top_saturate_without_wrapping() {
        const THREADS: usize = 8;
        let cell = Arc::new(AtomicU64::new(u64::MAX - 5));
        let start = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let cell = Arc::clone(&cell);
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    add_saturating(&cell, 10);
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("the charging thread finishes");
        }
        assert_eq!(
            cell.load(Ordering::Relaxed),
            u64::MAX,
            "charges at the top saturate, never wrap"
        );
    }
}
