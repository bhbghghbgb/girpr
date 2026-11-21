//! Counter primitives: the saturating add shared by the byte counters.
//!
//! - [`counter`] — `add_saturating`: the saturating `try_update` shared by the
//!   atomic byte counters, so a charge cannot wrap.
//!
//! The re-export keeps the shared name at the crate root. Nothing here names a
//! product data model: the crate holds only mechanics.

pub mod counter;

pub use self::counter::add_saturating;
