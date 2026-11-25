//! Sophon manifest handling: the protobuf shapes, the manifest filters,
//! the streaming fetch, the patch-build delete union, and the local
//! chunk-reuse map.
//!
//! - [`proto`] — the `SophonChunkManifest` protobuf messages.
//! - [`filter`] — `select_manifests` / `select_patch_manifests`: res_category
//!   ignores + audio selection, one shared predicate.
//! - [`stream`] — fetching and streaming blobs: `fetch_manifest`,
//!   `stream_chunk_to`, the digesting sink, and the retry policy.
//! - [`patch`] — `fetch_patch_deletes`: the patch-build delete-list union.
//! - [`map`] — `build_local_chunk_map`: the local chunk-reuse map.
//!
//! Every public name that lived in `crate::sophon` before this split still
//! resolves here; this module only moves code into focused files.

pub mod filter;
pub mod map;
pub mod patch;
pub mod proto;
pub mod stream;

pub use self::filter::{select_manifests, select_patch_manifests};
// The facade's unit tests construct reuse entries by hand, but a dependency's
// `cfg(test)` is not on for its consumers. The `test-support` feature supplies
// that seam instead, so the shipped crate does not expose this extra alias.
#[cfg(feature = "test-support")]
pub use self::map::ReuseEntry;
pub use self::map::{FileReuse, build_local_chunk_map};
pub use self::patch::fetch_patch_deletes;
pub use self::proto::{SophonChunk, SophonChunkFile, SophonChunkManifest, WantedManifest};
pub use self::stream::{ChunkFetch, ChunkSink, fetch_manifest, stream_chunk_to};
