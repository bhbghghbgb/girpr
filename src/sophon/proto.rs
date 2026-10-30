//! Sophon chunk manifest protobuf (Starward `Sophon.proto` parity).
//! NOTE(starward-parity): the repeated field is misspelled `chuncks` upstream;
//! kept verbatim so diffs against the reference stay obvious. Do not "fix" the
//! spelling.
//! See <https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/Sophon.proto#L8-L30>
//! message SophonChunkManifest { repeated SophonChunkFile chuncks = 1; }
use crate::hyp::ChunkManifestMeta;

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SophonChunkManifest {
    #[prost(message, repeated, tag = "1")]
    pub chuncks: Vec<SophonChunkFile>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SophonChunkFile {
    #[prost(string, tag = "1")]
    pub file: String,
    #[prost(message, repeated, tag = "2")]
    pub chunks: Vec<SophonChunk>,
    #[prost(bool, tag = "3")]
    pub is_folder: bool,
    #[prost(int64, tag = "4")]
    pub size: i64,
    #[prost(string, tag = "5")]
    pub md5: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SophonChunk {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(string, tag = "2")]
    pub uncompressed_md5: String,
    #[prost(int64, tag = "3")]
    pub offset: i64,
    #[prost(int64, tag = "4")]
    pub compressed_size: i64,
    #[prost(int64, tag = "5")]
    pub uncompressed_size: i64,
    // NOTE(starward-parity): opaque upstream field, preserved for decode compat.
    // See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/Sophon.proto#L22-L30
    #[prost(int64, tag = "6")]
    pub unknown: i64,
    #[prost(string, tag = "7")]
    pub compressed_md5: String,
}

#[derive(Debug, Clone)]
pub struct WantedManifest {
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
                    unknown: 0,
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
