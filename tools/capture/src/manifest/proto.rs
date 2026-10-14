//! The manifest protobuf.
//!
//! **M3.** The spelling of the repeated field is upstream's and is kept so the
//! wire type round-trips: renaming it would make this a lossy transform rather
//! than a decode. Field 6 is named for what is observable about it — it is an
//! `int64` on the wire — and preserved rather than guessed at.
//!
//! `Serialize` sits alongside `prost::Message` so the full decode can be dumped
//! without a second struct to keep in step. The field names *are* the dump's key
//! names, which means a rename here silently renames the artifact — so the dump
//! test asserts the exact key set.

use serde::Serialize;

/// `SophonChunkManifest`: the top-level message.
#[derive(Clone, PartialEq, ::prost::Message, Serialize)]
pub struct SophonChunkManifest {
    #[prost(message, repeated, tag = "1")]
    pub chuncks: ::prost::alloc::vec::Vec<SophonChunkFile>,
}

/// One file's entry: its path, its size, its whole-file MD5, and its chunk list.
#[derive(Clone, PartialEq, ::prost::Message, Serialize)]
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

impl SophonChunkFile {
    /// The file this entry describes, or `""` for a folder entry that carries no
    /// path of its own.
    pub fn is_folder_entry(&self) -> bool {
        self.is_folder
    }

    /// `Σ chunk.uncompressed_size`, which the manifest asserts equals `size`.
    pub fn chunk_bytes(&self) -> i64 {
        self.chunks.iter().map(|c| c.uncompressed_size).sum()
    }

    /// The last byte the chunks cover, given the last chunk's offset and length.
    pub fn covered_end(&self) -> i64 {
        self.chunks
            .last()
            .map(|c| c.offset + c.uncompressed_size)
            .unwrap_or(0)
    }
}

/// One chunk: where its bytes go in the file and how to check them.
#[derive(Clone, PartialEq, ::prost::Message, Serialize)]
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

impl SophonChunk {
    /// The key two chunks are compared by to decide they are the same content:
    /// MD5 alone is not enough, because a truncated chunk can share one.
    pub fn identity(&self) -> (&str, i64) {
        (&self.uncompressed_md5, self.uncompressed_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message as _;

    fn sample() -> SophonChunkManifest {
        SophonChunkManifest {
            chuncks: vec![
                SophonChunkFile {
                    file: "GenshinImpact_Data/Plugins/HoYoChannel.dll".into(),
                    chunks: vec![
                        SophonChunk {
                            id: "ddb31a9fd85346c4_de1cfe6927dacbc58d2d55a0b45ea0f3".into(),
                            uncompressed_md5: "de1cfe6927dacbc58d2d55a0b45ea0f3".into(),
                            offset: 0,
                            compressed_size: 259_795,
                            uncompressed_size: 690_719,
                            unknown_field6: 8_640_275_792_690_937_856,
                            compressed_md5: "06ba73bb".into(),
                        },
                        SophonChunk {
                            id: "f5dc3ca1f3a0ba3d_291a7719a4a23664d37825a41fad3c5b".into(),
                            uncompressed_md5: "291a7719a4a23664d37825a41fad3c5b".into(),
                            offset: 690_719,
                            compressed_size: 638_614,
                            uncompressed_size: 1_626_019,
                            unknown_field6: 5_814_299_429_820_432_384,
                            compressed_md5: "79f1d319".into(),
                        },
                    ],
                    is_folder: false,
                    size: 2_340_264,
                    md5: "d992a789".into(),
                },
            ],
        }
    }

    #[test]
    fn the_decoder_round_trips_a_real_shaped_entry() {
        let want = sample();
        let bytes = want.encode_to_vec();
        let got = SophonChunkManifest::decode(bytes.as_slice()).expect("decodes");
        assert_eq!(got, want);
    }

    /// A zero-length file is a real case, not an edge case invented for a test:
    /// it is what an empty config or an empty voice-pack stub decodes to, and a
    /// decoder that treats it as "no entry" would silently drop it from the diff.
    #[test]
    fn a_file_entry_with_no_chunks_decodes_to_a_zero_length_entry() {
        let want = SophonChunkManifest {
            chuncks: vec![SophonChunkFile {
                file: "GenshinImpact_Data/monoBleedingEdge/Runtime/DefaultResources/empty".into(),
                chunks: vec![],
                is_folder: false,
                size: 0,
                md5: "d41d8cd98f00b204e9800998ecf8427e".into(),
            }],
        };
        let got = SophonChunkManifest::decode(want.encode_to_vec().as_slice()).expect("decodes");
        assert_eq!(got.chuncks.len(), 1);
        assert!(got.chuncks[0].chunks.is_empty());
        assert_eq!(got.chuncks[0].size, 0);
        assert_eq!(got.chuncks[0].chunk_bytes(), 0);
        assert_eq!(got.chuncks[0].covered_end(), 0, "an empty file covers nothing");
        assert_eq!(got.chuncks[0].md5, "d41d8cd98f00b204e9800998ecf8427e");
    }

    /// Spec §3.6 records zero `is_folder` entries in the live manifests, and says
    /// a violation must be *recorded* rather than fatal. So the entry has to
    /// decode and be identifiable, not be rejected.
    #[test]
    fn a_folder_entry_decodes_and_is_distinguishable() {
        let want = SophonChunkManifest {
            chuncks: vec![
                SophonChunkFile {
                    file: "GenshinImpact_Data/StreamingAssets".into(),
                    chunks: vec![],
                    is_folder: true,
                    size: 0,
                    md5: String::new(),
                },
                SophonChunkFile {
                    file: "GenshinImpact_Data/file.bin".into(),
                    chunks: vec![],
                    is_folder: false,
                    size: 1,
                    md5: "x".into(),
                },
            ],
        };
        let got = SophonChunkManifest::decode(want.encode_to_vec().as_slice()).expect("decodes");
        assert_eq!(got.chuncks.len(), 2);
        assert!(got.chuncks[0].is_folder_entry());
        assert!(!got.chuncks[1].is_folder_entry());
    }

    #[test]
    fn an_empty_manifest_decodes_to_no_entries() {
        let got = SophonChunkManifest::decode(SophonChunkManifest { chuncks: vec![] }.encode_to_vec().as_slice())
            .expect("decodes");
        assert!(got.chuncks.is_empty());
    }

    /// A truncated frame is a corrupt blob, and saying so is better than a
    /// half-decoded manifest that would assemble into a plausible wrong file.
    #[test]
    fn a_truncated_frame_is_an_error_not_a_short_entry() {
        let bytes = sample().encode_to_vec();
        assert!(SophonChunkManifest::decode(&bytes[..bytes.len() / 2]).is_err());
    }

    #[test]
    fn the_chunk_identity_is_md5_plus_size_not_md5_alone() {
        let a = SophonChunk {
            id: "a".into(),
            uncompressed_md5: "same".into(),
            offset: 0,
            compressed_size: 1,
            uncompressed_size: 100,
            unknown_field6: 0,
            compressed_md5: "x".into(),
        };
        let mut b = a.clone();
        b.uncompressed_size = 50;
        assert_ne!(a.identity(), b.identity());
        assert_eq!(a.identity(), a.clone().identity());
    }

    /// The dump's key names are these struct's field names, so a rename here is
    /// a silent change to the artifact's byte layout.
    #[test]
    fn the_serialized_keys_are_the_documented_ones() {
        let v: serde_json::Value = serde_json::to_value(sample()).unwrap();
        let f = &v["chuncks"][0];
        let mut file_keys: Vec<&String> = f.as_object().unwrap().keys().collect();
        file_keys.sort();
        assert_eq!(
            file_keys,
            vec!["chunks", "file", "is_folder", "md5", "size"]
        );
        let mut chunk_keys: Vec<&String> = f["chunks"][0].as_object().unwrap().keys().collect();
        chunk_keys.sort();
        assert_eq!(
            chunk_keys,
            vec![
                "compressed_md5",
                "compressed_size",
                "id",
                "offset",
                "uncompressed_md5",
                "uncompressed_size",
                "unknown_field6",
            ]
        );
        // Field 6 is preserved verbatim, not dropped for being unknown.
        assert_eq!(f["chunks"][0]["unknown_field6"], 8_640_275_792_690_937_856i64);
    }
}