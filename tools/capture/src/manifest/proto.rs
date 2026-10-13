//! The manifest protobuf.
//!
//! **M3.** The spelling of the repeated field is upstream's and is kept so the
//! wire type round-trips: renaming it would make this a lossy transform rather
//! than a decode. Field 6 is named for what is observable about it — it is an
//! `int64` on the wire — and preserved rather than guessed at.

/// `SophonChunkManifest`: the top-level message.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SophonChunkManifest {
    #[prost(message, repeated, tag = "1")]
    pub chuncks: ::prost::alloc::vec::Vec<SophonChunkFile>,
}

/// One file's entry: its path, its size, its whole-file MD5, and its chunk list.
#[derive(Clone, PartialEq, ::prost::Message)]
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
#[derive(Clone, PartialEq, ::prost::Message)]
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
    #[prost(int64, tag = "6")]
    pub unknown_field6: i64,
    #[prost(string, tag = "7")]
    pub compressed_md5: ::prost::alloc::string::String,
}