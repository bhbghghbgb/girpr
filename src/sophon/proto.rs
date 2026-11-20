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
//! wrapper and test. Nothing about how a manifest is fetched or used moved out.

use crate::hyp::ChunkManifestMeta;

pub use girpr_proto::{SophonChunk, SophonChunkFile, SophonChunkManifest};

#[derive(Debug, Clone)]
pub struct WantedManifest {
    // Carried with the fetched files; no consumer reads it back.
    #[allow(dead_code)]
    pub meta: ChunkManifestMeta,
    pub files: Vec<SophonChunkFile>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn proto_roundtrip() {
        let m = SophonChunkManifest {
            chuncks: vec![SophonChunkFile {
                file: "a/b.dat".into(),
                chunks: vec![SophonChunk {
                    id: "abc".into(),
                    uncompressed_md5: "d41d8cd98f00b204e9800998ecf8427e".into(),
                    offset: 0,
                    compressed_size: 10,
                    uncompressed_size: 20,
                    unknown_field6: 0,
                    compressed_md5: "x".into(),
                }],
                is_folder: false,
                size: 20,
                md5: "d41d8cd98f00b204e9800998ecf8427e".into(),
            }],
        };
        let mut buf = Vec::new();
        prost::Message::encode(&m, &mut buf).unwrap();
        let back = SophonChunkManifest::decode(buf.as_slice()).unwrap();
        assert_eq!(back.chuncks[0].file, "a/b.dat");
    }
}
