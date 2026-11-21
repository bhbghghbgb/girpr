//! Limiters for this module's unit tests.
//!
//! [`limits`]'s cases and [`gate`]'s both state the *shape* they are about, and
//! both spell it as `shared(4)` or `split(3, 5)`. Naming it here keeps the
//! distinction between the shape and the limiter that realises it — the shape's
//! arithmetic is what `limits` is testing, and reaching the limiter for it would
//! test a different thing.
//!
//! Compiled only under `cfg(test)`.

use std::num::NonZeroUsize;

use super::RwLimiter;
use super::RwLimits;

/// One counter spanning both sides.
pub(super) fn shared(n: usize) -> RwLimiter {
    RwLimiter::new(RwLimits::Shared(NonZeroUsize::new(n).unwrap()))
}

/// One counter per side.
pub(super) fn split(s: usize, d: usize) -> RwLimiter {
    RwLimiter::new(RwLimits::Split {
        src: NonZeroUsize::new(s).unwrap(),
        dst: NonZeroUsize::new(d).unwrap(),
    })
}
