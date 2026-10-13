//! The four HoYoPlay endpoints and Sophon's `getBuild`.
//!
//! **M2.** Bodies are captured byte-exact rather than re-serialized, so a reader
//! can `diff` a captured file against a fresh request. `retcode != 0` anywhere
//! is exit `2` with the URL named — never a silent skip.

pub mod types;