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
//! deadlock. [`RwRuntime::new`] sizes the worker pool at the largest number of
//! permits a run can hold at once, so in the ordinary case no worker blocks on the
//! gate either — but that is a throughput property, not the correctness argument.
//! The ordering above is.
//!
//! ## The submodules
//!
//! | module | what it owns |
//! | --- | --- |
//! | [`limits`] | [`RwLimits`] and [`RwSide`]: **the shape** the flags resolve to |
//! | [`gate`] | [`RwLimiter`], [`RwPermit`] and the counting gate: **the counters** |
//! | this file | [`RwRuntime`]: the limits, the counters and the worker pool, as one run's resource |

mod gate;
mod limits;

#[cfg(test)]
mod fixtures;

pub use gate::{RwLimiter, RwPermit};
pub use limits::{RwLimits, RwSide};

use anyhow::{Context, Result};
use tracing::warn;

use limits::LOUD_THREAD_COUNT;

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