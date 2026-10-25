//! Hash algorithm selection and streaming file digests.
//!
//! Four concerns, one module tree:
//!
//! | module | owns |
//! | --- | --- |
//! | `algo` | [`HashAlg`] + [`UnknownHash`]: the command-line vocabulary |
//! | `set` | [`AlgoSet`] + `parse_hash_list` / `parse_any_of` |
//! | `digestset` | [`DigestSet`]: row storage |
//! | `stream` | [`CHUNK`], [`Digests`], [`hash_file`]: streaming digests |
//!
//! All `pub` items are re-exported below, so `crate::hash::X` paths are unchanged.

mod algo;
mod digestset;
mod set;
mod stream;
#[cfg(test)]
mod tests;

pub use algo::{HashAlg, UnknownHash};
pub use digestset::DigestSet;
pub use set::{AlgoSet, parse_any_of, parse_hash_list};
pub use stream::{CHUNK, Digests, hash_file};
