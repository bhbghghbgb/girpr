//! Sophon chunk manifest handling: the protobuf shape, the manifest filter,
//! the streaming fetch, and the local chunk-reuse map.
//!
//! - [`proto`] — the `SophonChunkManifest` protobuf messages.
//! - [`filter`] — `select_manifests`: res_category ignores + audio selection.
//! - [`stream`] — fetching and streaming blobs: `fetch_manifest`,
//!   `stream_chunk_to`, the digesting sink, and the retry policy.
//! - [`map`] — `build_local_chunk_map`: the local chunk-reuse map.
//!
//! Every public name that lived in `crate::sophon` before this split still
//! resolves here; this module only moves code into focused files.

pub mod filter;
pub mod map;
pub mod proto;
pub mod stream;

pub use self::filter::select_manifests;
pub use self::map::{FileReuse, ReuseEntry, build_local_chunk_map};
pub use self::proto::{SophonChunk, SophonChunkFile, SophonChunkManifest, WantedManifest};
pub use self::stream::{
    ChunkDigest, ChunkSink, HashingWriter, fetch_manifest, join_url, retryable, stream_chunk_to,
};
