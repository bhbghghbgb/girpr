//! The shared Sophon protobuf wire types: chunk and patch manifests.
//!
//! These are *wire types*, not a contract between the two programs that decode
//! them. The interface between `girpr` and `gircap` is the artifact on disk; the
//! only thing shared here is the byte layout of the Starward `Sophon.proto`
//! messages. Each program keeps its own behaviour around these types: its own
//! decode boundary, validation, digest policy and dump format.
//!
//! ## Mechanics, not program logic
//!
//! This crate declares both message families, their prost tags and their field
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

// NOTE(starward-parity): the patch family below is Starward's second manifest
// shape, for version upgrades rather than chunk repair. Field numbers and
// names are copied verbatim from the pinned Sophon.proto; Collapse's copy
// agrees on the numbers. See
// <https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/Sophon.proto#L32-L92>

/// `SophonPatchManifest`: the top-level patch message — new-version files,
/// per-version deletions and the compression mode.
#[derive(Clone, PartialEq, ::prost::Message, serde::Serialize)]
pub struct SophonPatchManifest {
    #[prost(message, repeated, tag = "1")]
    pub patches: ::prost::alloc::vec::Vec<SophonPatchFile>,
    #[prost(message, repeated, tag = "2")]
    pub delete_tags: ::prost::alloc::vec::Vec<SophonPatchDeleteTag>,
    #[prost(int32, tag = "3")]
    pub compress_mode: i32,
}

/// One new-version file: its path, whole-file size/MD5 and its patches.
/// An empty `patches` list means the file needs no patch.
#[derive(Clone, PartialEq, ::prost::Message, serde::Serialize)]
pub struct SophonPatchFile {
    #[prost(string, tag = "1")]
    pub file: ::prost::alloc::string::String,
    #[prost(int64, tag = "2")]
    pub size: i64,
    #[prost(string, tag = "3")]
    pub md5: ::prost::alloc::string::String,
    #[prost(message, repeated, tag = "4")]
    pub patches: ::prost::alloc::vec::Vec<SophonPatchInfo>,
}

/// One patch source for a file, labelled with the version it upgrades from.
#[derive(Clone, PartialEq, ::prost::Message, serde::Serialize)]
pub struct SophonPatchInfo {
    #[prost(string, tag = "1")]
    pub tag: ::prost::alloc::string::String,
    #[prost(message, tag = "2")]
    pub patch: Option<SophonPatch>,
}

/// The patch itself: where its bytes go and what the result must hash to.
/// An absent `original_file_name` means a new file, whose patch is the file.
#[derive(Clone, PartialEq, ::prost::Message, serde::Serialize)]
pub struct SophonPatch {
    #[prost(string, tag = "1")]
    pub id: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub tag: ::prost::alloc::string::String,
    #[prost(string, tag = "3")]
    pub build_id: ::prost::alloc::string::String,
    #[prost(int64, tag = "4")]
    pub patch_file_size: i64,
    #[prost(string, tag = "5")]
    pub patch_file_md5: ::prost::alloc::string::String,
    #[prost(int64, tag = "6")]
    pub patch_offset: i64,
    #[prost(int64, tag = "7")]
    pub patch_length: i64,
    #[prost(string, tag = "8")]
    pub original_file_name: ::prost::alloc::string::String,
    #[prost(int64, tag = "9")]
    pub original_file_size: i64,
    #[prost(string, tag = "10")]
    pub original_file_md5: ::prost::alloc::string::String,
}

/// Files to delete when upgrading from a given version.
#[derive(Clone, PartialEq, ::prost::Message, serde::Serialize)]
pub struct SophonPatchDeleteTag {
    #[prost(string, tag = "1")]
    pub tag: ::prost::alloc::string::String,
    #[prost(message, tag = "2")]
    pub delete_collection: Option<SophonPatchDeleteFileCollection>,
}

/// The delete list for one version, wrapped as upstream wraps it.
#[derive(Clone, PartialEq, ::prost::Message, serde::Serialize)]
pub struct SophonPatchDeleteFileCollection {
    #[prost(message, repeated, tag = "1")]
    pub delete_files: ::prost::alloc::vec::Vec<SophonPatchDeleteFile>,
}

/// One stale file and the size/MD5 it had in the old version.
#[derive(Clone, PartialEq, ::prost::Message, serde::Serialize)]
pub struct SophonPatchDeleteFile {
    #[prost(string, tag = "1")]
    pub file: ::prost::alloc::string::String,
    #[prost(int64, tag = "2")]
    pub size: i64,
    #[prost(string, tag = "3")]
    pub md5: ::prost::alloc::string::String,
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

    /// The delete half is what a version-upgrade stale-file sweep reads. Pin
    /// its nested wire keys as literal bytes so a re-tag cannot silently change
    /// the layout, then round-trip the path through prost.
    #[test]
    fn patch_manifest_delete_tag_round_trips() {
        let manifest = SophonPatchManifest {
            patches: vec![],
            delete_tags: vec![SophonPatchDeleteTag {
                tag: "5.0.0".into(),
                delete_collection: Some(SophonPatchDeleteFileCollection {
                    delete_files: vec![SophonPatchDeleteFile {
                        file: "assets/old.dat".into(),
                        size: 42,
                        md5: "abc".into(),
                    }],
                }),
            }],
            compress_mode: 0,
        };
        let bytes = manifest.encode_to_vec();
        let mut expected = vec![
            0x12, 0x22, // delete_tags (field 2), 34 bytes
            0x0a, 0x05, b'5', b'.', b'0', b'.', b'0', // tag (field 1)
            0x12, 0x19, // delete_collection (field 2), 25 bytes
            0x0a, 0x17, // delete_files (field 1), 23 bytes
            0x0a, 0x0e, // file (field 1), 14 bytes
        ];
        expected.extend_from_slice(b"assets/old.dat");
        expected.extend_from_slice(&[
            0x10, 0x2a, // size (field 2, varint), 42
            0x1a, 0x03, b'a', b'b', b'c', // md5 (field 3)
        ]);
        assert_eq!(bytes, expected, "nested field keys are byte-critical");

        let back = SophonPatchManifest::decode(bytes.as_slice()).unwrap();
        assert_eq!(back, manifest);
        assert_eq!(
            back.delete_tags[0]
                .delete_collection
                .as_ref()
                .unwrap()
                .delete_files[0]
                .file,
            "assets/old.dat"
        );
    }
}
