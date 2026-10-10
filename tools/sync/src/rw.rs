//! Side-independent read/write concurrency limits.
//!
//! One gate, applied everywhere, with a per-side variant. The gate governs **file
//! content reads and writes**, which is why the flag family is called *rw* and not
//! *io*:
//!
//! - **gated** — phase C's `hash_file`, and everything `copy_one` does between
//!   opening src and returning its record (the stream copy plus both verify
//!   re-hashes);
//! - **not gated** — `stat`. `walk_live`'s `metadata()`, `copy_one`'s own `stat`
//!   calls, `create_dir_all` / `remove_file` / `remove_dir_all`. A `stat` is orders
//!   of magnitude cheaper than a read, and a tree of many small files would spend
//!   its whole budget on calls that are not the thing the budget is about.
//!
//! It also does not cover the cache file's own reads and writes (redb commits), or
//! `backup_db` / `snapshot_old`. Those are `std::fs::copy` over a file that lives
//! inside src or dst, so they do touch the same physical disk — but they run
//! **once, serially, before any parallel phase exists**, so gating them is a no-op.
//! The claim "the limiter covers file reads and writes" means the ones the limiter
//! can order, which is every one of them that overlaps with another.
//!
//! ## Why the name, given `girpr` has an `--io-threads`
//!
//! The sibling binary's `--io-threads` means "this many files concurrently, with
//! the chunks inside one file strictly sequential" — concurrency *within* one file
//! stream, bounded per file. Nothing here chunks a stream. Different binary,
//! different semantics, no shared code; `rw` is precisely what keeps the two from
//! being conflated in a log line or a commit message.
//!
//! ## What a permit costs
//!
//! A copy is **one operation**. Under [`RwLimits::Shared`] that is one permit.
//! Under [`RwLimits::Split`] it is **one src permit and one dst permit**, held for
//! the whole of `copy_one`, because a copy is simultaneously reading src and
//! writing dst and is not finished until it has verified.
//!
//! That asymmetry is the whole feature, and it is why `--rw-dual-drive` is *not*
//! the same request as `--rw-threads 1`:
//!
//! - `--rw-threads 1` allows at most one rw operation in the entire run;
//! - `--rw-dual-drive` allows one src-side **and** one dst-side operation, so the
//!   two sides' hash phases genuinely overlap. A copy still runs one at a time,
//!   because it needs both permits.
//!
//! ## Lock ordering — the deadlock-freedom argument
//!
//! A copy acquires **src, then dst, always** ([`RwLimiter::acquire_copy`]). A hash
//! acquires exactly one gate and nothing else. So the only operation that ever
//! holds a gate while waiting for another is a copy waiting on dst while holding
//! src, and a thread holding dst never waits for anything at all. The wait-for
//! graph is therefore a DAG ordered `src → dst`: no cycle exists, so no cycle can
//! deadlock. `RwRuntime::new` sizes the worker pool at the largest number of
//! permits a run can hold at once, so in the ordinary case no worker blocks on the
//! gate either — but that is a throughput property, not the correctness argument.
//! The ordering above is.

use anyhow::{Context, Result};
use std::num::NonZeroUsize;
use std::sync::{Condvar, Mutex};
use tracing::warn;

/// A thread count past this is almost certainly a typo, and a typo that quietly
/// spawns 128 OS threads is worth a word in the log rather than in a bug report.
const LOUD_THREAD_COUNT: usize = 64;

/// How many pending hashes a resolve window holds.
///
/// A window rather than one `collect()` over the whole pending set, because three
/// properties depend on the bound:
///
/// - **durability.** [`crate::cache::COMMIT_INTERVAL`] bounds how much hashing work
///   an interruption can lose, and that only holds if a digest becomes durable
///   shortly after it is computed. Per-window keeps that true; one big `collect()`
///   would put the whole hash phase in memory and none of it on disk.
/// - **memory.** One `HashMap<String, Vec<u8>>` per pending file is real overhead on
///   a large tree — roughly 25–50 MB of map structure at 200k files, which the
///   sequential form never allocated.
/// - **the progress heartbeat**, which reports a `done` count and stays meaningful
///   with a window in flight and `pending.len() - window` still to go.
const MIN_WINDOW: usize = 32;
const WINDOW_PER_THREAD: usize = 8;

/// Which side of a two-sided run an operation reads or writes.
///
/// Under [`RwLimits::Shared`] the side is not consulted — there is one counter and
/// it does not care — but the distinction still has to be *expressible*, because a
/// resolve is handed one side and the other phases share the same limiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RwSide {
    Src,
    Dst,
}

/// The limiter's shape, as a resolved run option.
///
/// An enum rather than two loose `usize`s so that "a per-side limit with only one
/// side given" is unrepresentable: [`Split`](Self::Split) requires both or
/// neither, and the unmentioned side is a `1` the type system applied rather than a
/// default somebody has to remember to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RwLimits {
    /// One counter spanning both sides: as if src and dst share a disk. This is the
    /// default, because they usually do.
    Shared(NonZeroUsize),
    /// One counter per side. Two counters mean the two trees are treated as
    /// independent, which is the truth when they are on separate physical drives.
    Split {
        src: NonZeroUsize,
        dst: NonZeroUsize,
    },
}

impl Default for RwLimits {
    fn default() -> Self {
        RwLimits::Shared(NonZeroUsize::MIN)
    }
}

impl RwLimits {
    /// One shared permit — the unflagged run.
    pub fn one() -> NonZeroUsize {
        NonZeroUsize::MIN
    }

    /// `"shared"` or `"split"`, for the run span.
    pub fn mode(&self) -> &'static str {
        match self {
            RwLimits::Shared(_) => "shared",
            RwLimits::Split { .. } => "split",
        }
    }

    /// The src side's budget. Under [`Shared`](Self::Shared) this is the one
    /// counter, reported on both sides because both draw on it.
    pub fn src_threads(&self) -> usize {
        match self {
            RwLimits::Shared(n) => n.get(),
            RwLimits::Split { src, .. } => src.get(),
        }
    }

    /// The dst side's budget. See [`src_threads`](Self::src_threads).
    pub fn dst_threads(&self) -> usize {
        match self {
            RwLimits::Shared(n) => n.get(),
            RwLimits::Split { dst, .. } => dst.get(),
        }
    }

    /// The largest number of permits a run can hold at any one moment.
    ///
    /// | limits | pool size | why |
    /// | --- | --- | --- |
    /// | `Shared(n)` | `n` | every operation takes exactly one of `n` |
    /// | `Split { s, d }` | `s + d` | each copy holds one of each, so `c` copies |
    ///   account for `2c` permits and the remaining `s + d - 2c` workers can each
    ///   hold one more |
    pub fn pool_size(&self) -> usize {
        let n = match self {
            RwLimits::Shared(n) => n.get(),
            RwLimits::Split { src, dst } => src.get().saturating_add(dst.get()),
        };
        n.max(1)
    }

    /// How many pending hashes one resolve window holds for these limits.
    pub fn window_size(&self) -> usize {
        self.pool_size()
            .saturating_mul(WINDOW_PER_THREAD)
            .max(MIN_WINDOW)
    }

    /// One line naming the shape, for `--log-level debug` and the run span.
    pub fn describe(&self) -> String {
        match self {
            RwLimits::Shared(n) => format!("shared({})", n),
            RwLimits::Split { src, dst } => format!("split(src={}, dst={})", src, dst),
        }
    }
}

/// A counting gate: at most `limit` holders at once, released on drop.
///
/// `Mutex<usize> + Condvar` rather than `std::sync::Semaphore`, which is still
/// unstable. The critical section holds no user code and no other lock, so it is a
/// leaf in the same sense the copy phase's emit mutex is.
struct Gate {
    in_flight: Mutex<usize>,
    freed: Condvar,
    limit: usize,
}

impl Gate {
    fn new(limit: usize) -> Self {
        Self {
            in_flight: Mutex::new(0),
            freed: Condvar::new(),
            limit: limit.max(1),
        }
    }

    fn acquire(&self) {
        let mut n = self.lock();
        while *n >= self.limit {
            n = self.freed.wait(n).expect("rw gate mutex is never poisoned");
        }
        *n += 1;
    }

    fn try_acquire(&self) -> bool {
        let mut n = self.lock();
        if *n >= self.limit {
            return false;
        }
        *n += 1;
        true
    }

    fn release(&self) {
        {
            let mut n = self.lock();
            *n = n.saturating_sub(1);
        }
        // `notify_all`, not `notify_one`: a woken thread may be a copy that
        // already holds src and now needs dst, so the single wakeup it was given
        // would have to be spent waiting again. Waking everyone costs one pass
        // over the waiters and removes the whole failure mode.
        self.freed.notify_all();
    }

    fn in_flight(&self) -> usize {
        *self.lock()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, usize> {
        self.in_flight
            .lock()
            .expect("rw gate mutex is never poisoned")
    }
}

/// The counters a [`RwRuntime`] runs on.
///
/// One gate under [`RwLimits::Shared`] and two under [`RwLimits::Split`]; all three
/// exist at once because the shared counter's limit is then read by nobody, and a
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

    /// The gate a single-sided operation draws on. Under [`RwLimits::Shared`] both
    /// sides draw on the one counter, which is what "shared" means.
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
        gate.acquire();
        RwPermit {
            held: [Some(gate), None],
        }
    }

    /// src, then dst — or the single shared permit.
    ///
    /// **The ordering here is the deadlock-freedom invariant.** A copy is the only
    /// operation that takes two permits, it always takes src first, and a thread
    /// holding dst takes nothing further — so every wait-for edge points forward
    /// along `src → dst` and the graph has no cycle. Changing the order below
    /// would not be a refactor; it would be the first way this crate can hang a
    /// run rather than fail one.
    pub fn acquire_copy(&self) -> RwPermit<'_> {
        match self.limits {
            RwLimits::Shared(_) => self.acquire(RwSide::Src),
            RwLimits::Split { .. } => {
                let (src, dst) = (self.gate_for(RwSide::Src), self.gate_for(RwSide::Dst));
                src.acquire();
                dst.acquire();
                RwPermit {
                    held: [Some(src), Some(dst)],
                }
            }
        }
    }

    /// Non-blocking, so a limit can be *asserted* — a test asking "is the limit
    /// really enforced?" must be able to ask the question the answer to which is
    /// `false` without arranging for the blocking form to return.
    pub fn try_acquire(&self, side: RwSide) -> Option<RwPermit<'_>> {
        let gate = self.gate_for(side);
        // `then_some` would build the permit *before* knowing whether it was
        // earned, and dropping that unearned value would release a permit nobody
        // took — so the branch is spelled out rather than closed.
        if gate.try_acquire() {
            Some(RwPermit {
                held: [Some(gate), None],
            })
        } else {
            None
        }
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
/// on the way out. Nothing has to remember to release it.
pub struct RwPermit<'a> {
    /// At most two gates. Order is acquisition order; drop walks it backwards.
    held: [Option<&'a Gate>; 2],
}

impl Drop for RwPermit<'_> {
    fn drop(&mut self) {
        for gate in self.held.iter().rev().flatten() {
            gate.release();
        }
    }
}

/// One run's limits, its counters, and the worker pool that draws on them.
///
/// Built once per command and passed by reference, because a limiter whose counters
/// were split across phases would be two limiters wearing one flag's name.
pub struct RwRuntime {
    limits: RwLimits,
    limiter: RwLimiter,
    pool: rayon::ThreadPool,
}

impl RwRuntime {
    pub fn new(limits: RwLimits) -> Result<Self> {
        if limits.pool_size() > LOUD_THREAD_COUNT {
            warn!(
                threads = limits.pool_size(),
                limits = %limits.describe(),
                "a large rw thread count: this spawns that many OS threads, so check \
                 the flags are what you meant"
            );
        }
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(limits.pool_size())
            .thread_name(|i| format!("girsync-rw-{i}"))
            .build()
            .context("build rw thread pool")?;
        Ok(Self {
            limits,
            limiter: RwLimiter::new(limits),
            pool,
        })
    }

    /// [`RwLimits::Shared(1)`]: strictly one rw operation at a time, anywhere.
    ///
    /// Not a user-facing setting — the CLI's answer for an unflagged run is a
    /// resolved [`RwLimits`] like any other. It exists for callers that have no run
    /// options to hand yet.
    pub fn serial() -> Result<Self> {
        Self::new(RwLimits::default())
    }

    pub fn limits(&self) -> RwLimits {
        self.limits
    }

    pub fn limiter(&self) -> &RwLimiter {
        &self.limiter
    }

    /// The pool every parallel phase installs into. One pool for the whole run, so
    /// two phases cannot each size their own and quietly double the thread count.
    pub fn pool(&self) -> &rayon::ThreadPool {
        &self.pool
    }

    /// One permit for one side's read or write.
    pub fn acquire(&self, side: RwSide) -> RwPermit<'_> {
        self.limiter.acquire(side)
    }

    /// Both permits a copy needs, src first. See [`RwLimiter::acquire_copy`].
    pub fn acquire_copy(&self) -> RwPermit<'_> {
        self.limiter.acquire_copy()
    }

    /// How many pending hashes a resolve window holds under these limits.
    pub fn window_size(&self) -> usize {
        self.limits.window_size()
    }

    pub fn describe(&self) -> String {
        self.limits.describe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(s: usize, d: usize) -> RwLimiter {
        RwLimiter::new(RwLimits::Split {
            src: NonZeroUsize::new(s).unwrap(),
            dst: NonZeroUsize::new(d).unwrap(),
        })
    }

    fn shared(n: usize) -> RwLimiter {
        RwLimiter::new(RwLimits::Shared(NonZeroUsize::new(n).unwrap()))
    }

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

    /// `0` is unrepresentable in both shapes. The CLI layer still has to reject it,
    /// because a flag value arrives as a `usize` and this is where that becomes a
    /// `NonZeroUsize` — see `TryFrom<RwArgs> for RwLimits`.
    #[test]
    fn zero_is_unrepresentable() {
        assert!(NonZeroUsize::new(0).is_none());
        assert!(NonZeroUsize::new(1).is_some());
    }

    #[test]
    fn pool_size_matches_the_limits() {
        assert_eq!(RwLimits::default().pool_size(), 1);
        assert_eq!(shared(4).limits().pool_size(), 4);
        assert_eq!(split(3, 5).limits().pool_size(), 8);
        // Reported per side even under one counter, because the span should show
        // the budget rather than the mode's internals.
        assert_eq!(shared(4).limits().src_threads(), 4);
        assert_eq!(shared(4).limits().dst_threads(), 4);
        assert_eq!(split(3, 5).limits().src_threads(), 3);
        assert_eq!(split(3, 5).limits().dst_threads(), 5);
    }

    #[test]
    fn the_window_is_bounded_whatever_the_limits() {
        assert_eq!(RwLimits::default().window_size(), MIN_WINDOW);
        assert_eq!(shared(64).limits().window_size(), 64 * WINDOW_PER_THREAD);
    }

    #[test]
    fn describe_names_the_shape() {
        assert_eq!(RwLimits::default().describe(), "shared(1)");
        assert_eq!(shared(4).limits().describe(), "shared(4)");
        assert_eq!(split(2, 1).limits().describe(), "split(src=2, dst=1)");
    }
}
