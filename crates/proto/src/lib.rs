//! The shared Sophon chunk-manifest protobuf wire types.
//!
//! These are *wire types*, not a contract between the two programs that decode
//! them. The interface between `girpr` and `gircap` is the artifact on disk; the
//! only thing shared here is the byte layout of the Starward `Sophon.proto`
//! messages. Each program keeps its own behaviour around these types: its own
//! decode boundary, validation, digest policy and dump format.
//!
//! ## Mechanics, not program logic
//!
//! This crate declares the three messages, their prost tags and their field
//! names, and derives `prost::Message` and `serde::Serialize`. It holds no
//! method, no policy and no logic of either program. `gircap` keeps its
//! measurement, validation and normalisation helpers as extension impls in its
//! own crate; `girpr` keeps its `WantedManifest` wrapper locally. Nothing here
//! decides when a manifest is fetched, validated, dumped or assembled.
//!
//! ## Field names are byte-critical
//!
//! `chuncks` is upstream's misspelling (Starward parity), kept verbatim so
//! diffs against the reference stay obvious. Tag 6 is an opaque upstream
//! `int64` preserved for decode compatibility. Both Rust field names and the
//! `serde` key names are observable: renaming `chuncks` or `unknown_field6`
//! silently changes the bytes of a full-decode dump without changing any type
//! on the wire. Neither may be "fixed".
//!
//! See <https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/Sophon.proto#L8-L30>

/// `SophonChunkManifest`: the top-level message.
#[derive(Clone, PartialEq, ::prost::Message, serde::Serialize)]
pub struct SophonChunkManifest {
    /// Upstream's misspelling, preserved for parity and artifact stability.
    #[prost(message, repeated, tag = "1")]
    pub chuncks: ::prost::alloc::vec::Vec<SophonChunkFile>,
}

/// One file's entry: its path, its size, its whole-file MD5, and its chunk list.
#[derive(Clone, PartialEq, ::prost::Message, serde::Serialize)]
pub struct SophonChunkFile {
    #[prost(string, tag = "1")]
    pub file: ::prost::alloc::string::String,
    #[prost(message, repeated, tag = "2")]
    pub chunks: ::prost::alloc::vec::Vec<SophonChunk>,
    #[prost(bool, tag = "3")]
    pub is_folder: bool,
    #[prost(int64, tag = "4")]
    pub size: i64,
    #[prost(string, tag = "5")]
    pub md5: ::prost::alloc::string::String,
}

/// One chunk: where its bytes go in the file and how to check them.
#[derive(Clone, PartialEq, ::prost::Message, serde::Serialize)]
pub struct SophonChunk {
    #[prost(string, tag = "1")]
    pub id: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub uncompressed_md5: ::prost::alloc::string::String,
    #[prost(int64, tag = "3")]
    pub offset: i64,
    #[prost(int64, tag = "4")]
    pub compressed_size: i64,
    #[prost(int64, tag = "5")]
    pub uncompressed_size: i64,
    // NOTE(starward-parity): opaque upstream field, preserved for decode compat.
    // It is named for what is observable about it — an `int64` on the wire — and
    // the name is also the full-decode dump's JSON key, so it is load-bearing.
    #[prost(int64, tag = "6")]
    pub unknown_field6: i64,
    #[prost(string, tag = "7")]
    pub compressed_md5: ::prost::alloc::string::String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message as _;

    /// Tag 6 encodes as `0x30` (field 6, wire type 0). Pin it so a re-tag would
    /// change the wire bytes and the passthrough would silently stop matching.
    #[test]
    fn tag_six_is_the_opaque_int64_passthrough() {
        let chunk = SophonChunk {
            id: String::new(),
            uncompressed_md5: String::new(),
            offset: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            unknown_field6: 150,
            compressed_md5: String::new(),
        };
        let bytes = chunk.encode_to_vec();
        assert_eq!(bytes, vec![0x30, 0x96, 0x01]);
        let back = SophonChunk::decode(bytes.as_slice()).unwrap();
        assert_eq!(back.unknown_field6, 150);
    }
}
