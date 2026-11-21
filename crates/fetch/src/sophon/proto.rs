//! Sophon chunk manifest protobuf (Starward `Sophon.proto` parity).
//! NOTE(starward-parity): the repeated field is misspelled `chuncks` upstream;
//! kept verbatim so diffs against the reference stay obvious. Do not "fix" the
//! spelling.
//! See <https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/Sophon.proto#L8-L30>
//! message SophonChunkManifest { repeated SophonChunkFile chuncks = 1; }
//!
//! The three wire types now live in `girpr-proto` so this binary and `gircap`
//! decode one byte layout. The leaf owns only the protobuf tags and field names
//! (`chuncks`, the opaque tag 6); this module keeps the repair binary's own
//! wrapper. Nothing about how a manifest is fetched or used moved out.

use crate::hyp::ChunkManifestMeta;

pub use girpr_proto::{SophonChunk, SophonChunkFile, SophonChunkManifest};

#[derive(Debug, Clone)]
pub struct WantedManifest {
    // Carried with the fetched files; no consumer reads it back.
    #[allow(dead_code)]
    pub meta: ChunkManifestMeta,
    pub files: Vec<SophonChunkFile>,
}
