//! The content-addressed chunk store and the three retention sets.
//!
//! **M6.** Blobs are stored once by chunk id however many files or both builds
//! reference them. The three sets — `materialize_old`, `materialize_new`,
//! `patch_fetch` — are what make the bundle closed: nothing in it may depend on a
//! blob it did not retain.

pub mod chunks;
pub mod props;