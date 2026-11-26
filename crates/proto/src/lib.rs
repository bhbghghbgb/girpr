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
//! This crate re-exports the message types generated at build time by
//! `prost-build` from the vendored, pinned `proto/Sophon.proto`. prost owns the
//! tags, field names and `alloc` Vec/String shapes; `serde::Serialize` is
//! derived on every generated type. The crate holds no method, no policy and no
//! logic of either program. `gircap` keeps its measurement, validation and
//! normalisation helpers as extension impls in its own crate; `girpr` keeps its
//! `WantedManifest` wrapper locally. Nothing here decides when a manifest is
//! fetched, validated, dumped or assembled.
//!
//! ## Field names are byte-critical
//!
//! `chuncks` is upstream's misspelling (Starward parity), kept verbatim so
//! diffs against the reference stay obvious. Tag 6 is an opaque upstream
//! `int64` preserved for decode compatibility. Both Rust field names and the
//! `serde` key names are observable: renaming `chuncks` or `unknown_field6`
//! silently changes the bytes of a full-decode dump without changing any type
//! on the wire. Neither may be "fixed". The vendored `.proto` pins field 6 as
//! `unknown_field6` (upstream calls it `unknown`) precisely for this reason;
//! its tag and type are unchanged.
//!
//! See <https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/Sophon.proto#L8-L30>

mod generated {
    include!(concat!(env!("OUT_DIR"), "/starward.rpc.game_install.rs"));
}

pub use generated::*;

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message as _;
    use std::collections::BTreeSet;

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

    /// The two names frozen by the full-decode artifacts must survive codegen:
    /// `chuncks` (upstream's misspelling) and `unknown_field6` (opaque tag 6).
    /// The serde keys and the wire tag of `chuncks` are pinned together.
    #[test]
    fn dump_keys_chuncks_and_unknown_field6_are_frozen() {
        let manifest = SophonChunkManifest {
            chuncks: vec![SophonChunkFile {
                file: "a.dat".into(),
                chunks: vec![SophonChunk {
                    id: String::new(),
                    uncompressed_md5: String::new(),
                    offset: 0,
                    compressed_size: 0,
                    uncompressed_size: 0,
                    unknown_field6: 150,
                    compressed_md5: String::new(),
                }],
                is_folder: false,
                size: 0,
                md5: String::new(),
            }],
        };

        let bytes = manifest.encode_to_vec();
        assert_eq!(bytes[0], 0x0a, "chuncks stays field 1, wire type 2");

        let json = serde_json::to_value(&manifest).expect("serialize dump");
        let chuncks = json.get("chuncks").expect("misspelled dump key");
        assert!(json.get("chunks").is_none());
        let chunk = &chuncks[0]["chunks"][0];
        assert_eq!(chunk["unknown_field6"], serde_json::json!(150));
        assert!(
            chunk.get("unknown").is_none(),
            "upstream name must not leak"
        );
    }

    /// Every generated message contributes keys to the frozen full-decode
    /// dump. Compare *names only*, as an order-independent set: values, key
    /// order and JSON spacing are not contracts. The listing is deliberate —
    /// a message added to the vendored schema needs a row here.
    #[test]
    fn every_generated_message_has_a_frozen_serde_key_set() {
        fn key_set<T: serde::Serialize>(message: &T) -> BTreeSet<String> {
            let value = serde_json::to_value(message).expect("serialize generated message");
            value
                .as_object()
                .expect("a generated message is a JSON object")
                .keys()
                .cloned()
                .collect()
        }

        fn assert_keys<T: serde::Serialize>(message: &T, expected: &[&str]) {
            let expected: BTreeSet<String> = expected.iter().map(|key| (*key).into()).collect();
            assert_eq!(key_set(message), expected);
        }

        assert_keys(&SophonChunkManifest::default(), &["chuncks"]);
        assert_keys(
            &SophonChunkFile::default(),
            &["file", "chunks", "is_folder", "size", "md5"],
        );
        assert_keys(
            &SophonChunk::default(),
            &[
                "id",
                "uncompressed_md5",
                "offset",
                "compressed_size",
                "uncompressed_size",
                "unknown_field6",
                "compressed_md5",
            ],
        );
        assert_keys(
            &SophonPatchManifest::default(),
            &["patches", "delete_tags", "compress_mode"],
        );
        assert_keys(
            &SophonPatchFile::default(),
            &["file", "size", "md5", "patches"],
        );
        assert_keys(&SophonPatchInfo::default(), &["tag", "patch"]);
        assert_keys(
            &SophonPatch::default(),
            &[
                "id",
                "tag",
                "build_id",
                "patch_file_size",
                "patch_file_md5",
                "patch_offset",
                "patch_length",
                "original_file_name",
                "original_file_size",
                "original_file_md5",
            ],
        );
        assert_keys(
            &SophonPatchDeleteTag::default(),
            &["tag", "delete_collection"],
        );
        assert_keys(
            &SophonPatchDeleteFileCollection::default(),
            &["delete_files"],
        );
        assert_keys(&SophonPatchDeleteFile::default(), &["file", "size", "md5"]);
    }
}
