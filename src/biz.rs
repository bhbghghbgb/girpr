//! The game/channel selector is the shared region identity.
//!
//! The three values, their ids, host key and channel tuple are defined once in
//! `girpr-model::region`. This binary keeps its resolve and request policy in
//! [`crate::hyp`] and [`crate::repair`]; only the values come from the leaf.

pub use girpr_model::region::Region as Biz;
