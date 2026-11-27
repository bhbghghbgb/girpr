//! Counter primitives: the saturating atomic mechanics shared by the byte
//! counters.
//!
//! - [`counter`] — `add_saturating` for the plain "add without wrapping" path,
//!   plus `try_charge` and `try_refund` for ceiling-bound charges and refunds.
//!   Every one is a single atomic operation (or compare-exchange loop) that
//!   cannot wrap a tally past `u64::MAX`.
//!
//! The re-exports keep the shared names at the crate root. Nothing here names a
//! product data model or decides what a refusal means: the crate holds only
//! mechanics, and returns structured outcomes for the callers' own policies.

pub mod counter;

pub use self::counter::{ChargeOutcome, RefundOutcome, add_saturating, try_charge, try_refund};
