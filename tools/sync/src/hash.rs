//! Hash algorithm selection and streaming file digests.
//!
//! The vocabulary and the streaming sink now live with the cache crate: the
//! cache's wire format pins `DigestSet`'s serde impl, so keeping both halves in
//! one crate is what lets the cache stay an acyclic leaf below this lib. This
//! facade re-exports every public item, so `crate::hash::X` paths are unchanged
//! for the planner, the apply phase and the CLI.

pub use girsync_cache::hash::*;
