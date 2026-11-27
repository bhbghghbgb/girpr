//! Saturating atomic counters shared by the report and the repair ledger.
//!
//! A counter that is charged from several worker threads cannot use
//! `fetch_add`: it wraps, and a wrapped byte total reads near `u64::MAX` — which
//! switches a ceiling off and corrupts the reported figures. Every site that
//! only needs "add without wrapping" goes through [`add_saturating`], which
//! folds the saturating addition into one atomic `try_update` (the current name
//! for what was `fetch_update`).
//!
//! Sites that also need a ceiling or a refund use [`try_charge`] and
//! [`try_refund`]: the same saturating arithmetic in one compare-exchange loop,
//! returning a structured outcome so the caller — not this leaf — decides what
//! a refusal or a saturated subtraction means.

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

/// What a ceiling-bound charge did to the cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct ChargeOutcome {
    /// The value left in the cell: the new total when the charge committed,
    /// or the unchanged total when it was refused.
    pub next: u64,
    /// `false` only when the proposed charge would exceed the ceiling; in that
    /// case nothing was added.
    pub committed: bool,
}

/// Add `n` to `cell` with saturating arithmetic, committing only when the
/// resulting figure stays within `limit`.
///
/// The ceiling test and the write are one compare-exchange loop: the decision
/// uses the same value that is swapped in, so two chargers that both see room
/// cannot both commit past the ceiling. When the result would exceed `limit`,
/// nothing is added and [`ChargeOutcome::committed`] is `false`. Passing
/// `u64::MAX` as `limit` charges with no ceiling — a saturating add can never
/// exceed it.
///
/// All orderings are `Relaxed`, matching [`add_saturating`]: the cell is a
/// tally, not a publication point.
pub fn try_charge(cell: &AtomicU64, n: u64, limit: u64) -> ChargeOutcome {
    let mut cur = cell.load(Ordering::Relaxed);
    loop {
        let next = cur.saturating_add(n);
        if next > limit {
            return ChargeOutcome {
                next: cur,
                committed: false,
            };
        }
        match cell.compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => {
                return ChargeOutcome {
                    next,
                    committed: true,
                };
            }
            Err(actual) => cur = actual,
        }
    }
}

/// What a refunding subtraction did to the cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct RefundOutcome {
    /// The value the cell held when the subtraction committed.
    pub previous: u64,
    /// The value left in the cell after subtracting `n`, saturated at zero.
    pub next: u64,
    /// `true` when `previous < n`: the subtraction saturated and less than `n`
    /// could be removed. Whether that is noteworthy — and its wording — is the
    /// caller's decision.
    pub saturated: bool,
}

/// Subtract `n` from `cell` with saturating arithmetic and report the result.
///
/// Like [`try_charge`], this is one compare-exchange loop, so a concurrent
/// charge cannot be lost between the read and the write. The result saturates
/// at zero rather than wrapping to a figure near `u64::MAX`, which would switch
/// a ceiling off for every later charge.
pub fn try_refund(cell: &AtomicU64, n: u64) -> RefundOutcome {
    let mut cur = cell.load(Ordering::Relaxed);
    loop {
        let next = cur.saturating_sub(n);
        match cell.compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => {
                return RefundOutcome {
                    previous: cur,
                    next,
                    saturated: cur < n,
                };
            }
            Err(actual) => cur = actual,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ChargeOutcome, RefundOutcome, add_saturating, try_charge, try_refund};
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

    /// A charge is all or nothing: at the ceiling the boundary is inclusive,
    /// and one byte past it adds nothing at all.
    #[test]
    fn try_charge_is_all_or_nothing_at_the_ceiling() {
        let cell = AtomicU64::new(60);
        assert_eq!(
            try_charge(&cell, 40, 100),
            ChargeOutcome {
                next: 100,
                committed: true
            }
        );
        assert_eq!(cell.load(Ordering::Relaxed), 100);
        assert_eq!(
            try_charge(&cell, 1, 100),
            ChargeOutcome {
                next: 100,
                committed: false
            }
        );
        assert_eq!(
            cell.load(Ordering::Relaxed),
            100,
            "a refused charge adds nothing"
        );
        // A zero ceiling still takes a zero charge, but not one byte.
        let cell = AtomicU64::new(0);
        assert!(try_charge(&cell, 0, 0).committed);
        assert!(!try_charge(&cell, 1, 0).committed);
    }

    /// `u64::MAX` means "no ceiling": the charge always commits and saturates
    /// instead of wrapping past the top of the range.
    #[test]
    fn try_charge_without_a_ceiling_saturates_and_never_refuses() {
        let cell = AtomicU64::new(u64::MAX - 5);
        let outcome = try_charge(&cell, 10, u64::MAX);
        assert_eq!(
            outcome,
            ChargeOutcome {
                next: u64::MAX,
                committed: true
            }
        );
        assert_eq!(cell.load(Ordering::Relaxed), u64::MAX);
    }

    /// Racing charges cannot both commit the same "free" room: every accepted
    /// charge is counted once and the ceiling still holds afterwards.
    #[test]
    fn concurrent_charges_never_overshoot_the_ceiling() {
        const THREADS: usize = 8;
        let cell = Arc::new(AtomicU64::new(0));
        let start = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let cell = Arc::clone(&cell);
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    let mut taken = 0u64;
                    start.wait();
                    for _ in 0..100 {
                        if try_charge(&cell, 100, 1000).committed {
                            taken += 1;
                        }
                    }
                    taken
                })
            })
            .collect();
        let taken: u64 = handles
            .into_iter()
            .map(|h| h.join().expect("the charging thread finishes"))
            .sum();
        assert_eq!(cell.load(Ordering::Relaxed), taken * 100);
        assert!(cell.load(Ordering::Relaxed) <= 1000);
    }

    /// A refund that exceeds the cell saturates at zero and reports both the
    /// value it replaced and the fact that it saturated, so the caller can warn
    /// in its own words.
    #[test]
    fn try_refund_saturates_at_zero_and_reports_the_attempt() {
        let cell = AtomicU64::new(100);
        assert_eq!(
            try_refund(&cell, 30),
            RefundOutcome {
                previous: 100,
                next: 70,
                saturated: false
            }
        );
        assert_eq!(
            try_refund(&cell, 1000),
            RefundOutcome {
                previous: 70,
                next: 0,
                saturated: true
            }
        );
        assert_eq!(cell.load(Ordering::Relaxed), 0);
    }

    /// Under contention, refunds remove exactly the bytes that were there: no
    /// subtraction is lost, and the surplus requests are the ones that report
    /// saturation.
    #[test]
    fn concurrent_refunds_remove_exactly_what_was_available() {
        const THREADS: u64 = 8;
        const PER_THREAD: u64 = 25;
        let cell = Arc::new(AtomicU64::new(120));
        let start = Arc::new(Barrier::new(THREADS as usize));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let cell = Arc::clone(&cell);
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    let mut removed = 0u64;
                    let mut saturated = 0u64;
                    for _ in 0..PER_THREAD {
                        let outcome = try_refund(&cell, 1);
                        removed += outcome.previous - outcome.next;
                        saturated += u64::from(outcome.saturated);
                    }
                    (removed, saturated)
                })
            })
            .collect();
        let (removed, saturated): (u64, u64) = handles
            .into_iter()
            .map(|h| h.join().expect("the refunding thread finishes"))
            .fold((0, 0), |(r, s), (rr, ss)| (r + rr, s + ss));
        assert_eq!(removed, 120, "every available byte was removed once");
        assert_eq!(saturated, THREADS * PER_THREAD - 120);
        assert_eq!(cell.load(Ordering::Relaxed), 0);
    }
}
