//! HYP + Sophon fetch mechanics for the girpr repair pipeline.
//!
//! This crate holds the two heavy halves of the product's download path:
//!
//! - [`hyp`] — the HYP JSON client (`getGameConfigs`, `getGameBranches`,
//!   `getBuild`, `getGameDeprecatedFileConfigs`) and the DTOs it decodes;
//! - [`sophon`] — the chunk-manifest protobuf wrapper, the manifest filter,
//!   the streaming fetch/decode/verify path and the local chunk-reuse map.
//!
//! It also owns the attempt loop ([`retry`]) the fetch path drives. The crate
//! depends only on logic-free leaves, so the repair, report and CLI modules
//! that consume it can depend on it directly: an edit to those modules leaves
//! this crate, and the whole reqwest/zstd/prost tree behind it, cached.
//!
//! The wire types stay in `girpr-proto` and the field spellings (`chuncks`,
//! opaque tag 6) are unchanged. Nothing about the report plane lives here;
//! the begin report borrows scalar provenance values from its caller.

pub mod hyp;
/// Persistent manifests and the full-file hotfix downloader: no product
/// wiring, only injectable seams and offline-tested gates.
pub mod persistent;
pub mod retry;
pub mod sophon;

// The canned-reply loopback origin used by this crate's own unit tests. It is
// a deliberate local copy: the product and both capture tools keep their own
// fake servers because each encodes a different contract.
#[cfg(test)]
mod test_http;
