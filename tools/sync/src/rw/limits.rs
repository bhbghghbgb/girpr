//! [`RwLimits`]: the *shape* the `--rw-threads*` flags resolve to.
//!
//! Pure configuration — arithmetic over the flags, with no counters and no
//! locking. The counting is [`super::gate`]'s.

use std::num::NonZeroUsize;

/// A thread count past this is almost certainly a typo, and a typo that quietly
/// spawns 128 OS threads is worth a word in the log rather than in a bug report.
pub(super) const LOUD_THREAD_COUNT: usize = 64;

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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::rw::fixtures::{shared, split};

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
