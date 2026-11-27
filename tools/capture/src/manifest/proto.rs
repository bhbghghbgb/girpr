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
//! test asserts the exact key set. The wire types themselves now live in the
//! logic-free `girpr-proto` leaf; both programs decode that one byte layout.
//! Everything that is *capture* behaviour — measurement, validation and digest
//! normalisation — stays here as extension traits, so no program logic moved
//! into the leaf.

use girpr_chunk_identity::{ChunkIdentity, normalize_hex};
use girpr_chunk_layout::{RangeProblem, check_range};
pub use girpr_proto::{SophonChunk, SophonChunkFile, SophonChunkManifest};

/// Capture's measurements and validation of one decoded file entry.
///
/// An extension trait, not an inherent impl: the wire type is shared and the
/// leaf must stay logic-free. Methods here describe how *this tool* reads a
/// manifest, not how the message is encoded.
pub trait SophonChunkFileExt {
    /// The file this entry describes, or `""` for a folder entry that carries no
    /// path of its own.
    fn is_folder_entry(&self) -> bool;

    /// `Σ chunk.uncompressed_size`, which the manifest asserts equals `size`.
    ///
    /// Saturating: this is a measurement of a document, and a hostile size must
    /// not abort the tool that is only counting it.
    fn chunk_bytes(&self) -> i64;

    /// The last byte the chunks cover, given the last chunk's offset and length.
    fn covered_end(&self) -> i64;

    /// Refuse a declaration whose offsets and sizes cannot be added.
    ///
    /// **The manifest is a document from the network.** `offset`, `size` and
    /// `compressed_size` are `int64`s on the wire with no domain check, so a
    /// negative cost or an offset past `i64::MAX` reaches the arithmetic that
    /// places chunks unless it is refused here. The capture path calls this once
    /// per entry at the decode boundary; `assemble` calls it again so a
    /// hand-built `FilePair` cannot bypass it.
    fn validate_arithmetic(&self) -> Result<(), String>;

    /// Lower-case this entry's declared digests — the whole-file one and every
    /// chunk's. See [`SophonChunkExt::normalise`] for why.
    fn normalise(&mut self);
}

impl SophonChunkFileExt for SophonChunkFile {
    fn is_folder_entry(&self) -> bool {
        self.is_folder
    }

    fn chunk_bytes(&self) -> i64 {
        self.chunks
            .iter()
            .fold(0_i64, |total, c| total.saturating_add(c.uncompressed_size))
    }

    fn covered_end(&self) -> i64 {
        self.chunks
            .last()
            .map(|c| c.offset.saturating_add(c.uncompressed_size))
            .unwrap_or(0)
    }

    fn validate_arithmetic(&self) -> Result<(), String> {
        for (i, c) in self.chunks.iter().enumerate() {
            if c.offset < 0 {
                return Err(format!(
                    "chunk {i} ({}): offset {} is negative",
                    c.id, c.offset
                ));
            }
            if c.compressed_size < 0 {
                return Err(format!(
                    "chunk {i} ({}): compressed_size {} is negative",
                    c.id, c.compressed_size
                ));
            }
            // The leaf decides which rule broke; this tool publishes the
            // wording, including the chunk index and id. Negative offsets and
            // compressed sizes keep their own order above: they are checked
            // before any of the leaf's range rules.
            match check_range(c.offset, c.uncompressed_size) {
                Ok(_) => {}
                Err(RangeProblem::NegativeLen(len)) => {
                    return Err(format!(
                        "chunk {i} ({}): uncompressed_size {} is negative",
                        c.id, len
                    ));
                }
                Err(RangeProblem::Overflow { offset, len }) => {
                    return Err(format!(
                        "chunk {i} ({}): offset {} + uncompressed_size {} overflows i64",
                        c.id, offset, len
                    ));
                }
                Err(RangeProblem::NegativeOffset(offset)) => {
                    return Err(format!(
                        "chunk {i} ({}): offset {} is negative",
                        c.id, offset
                    ));
                }
                Err(RangeProblem::PastEnd { .. }) => unreachable!("no bound is supplied here"),
            }
        }
        Ok(())
    }

    fn normalise(&mut self) {
        normalize_hex(&mut self.md5);
        for c in &mut self.chunks {
            c.normalise();
        }
    }
}

/// Capture's identity and digest policy for one decoded chunk.
pub trait SophonChunkExt {
    /// The key two chunks are compared by to decide they are the same content:
    /// MD5 alone is not enough, because a truncated chunk can share one. The
    /// borrowed view is the shared leaf's; capture keeps only the policy of
    /// normalising it at the decode boundary below.
    fn identity(&self) -> ChunkIdentity<'_>;

    /// Lower-case the declared digests.
    ///
    /// **A digest is an identity, not a spelling.** `girpr` compares chunk
    /// identities case-insensitively (`repair::reuse::sources_for`,
    /// `repair::plan::Plan`), so a pair of manifests that spell one chunk's
    /// md5 `AB…` and `ab…` describes the same content to the tool. Comparing
    /// them exactly made the two tools disagree by construction: `gircap` calls
    /// such a chunk fetched where `girpr` calls it reused, and the bundle would
    /// publish the wrong numbers.
    ///
    /// The fix is here rather than at each comparison: normalising once, where
    /// the manifest enters the crate, makes the identity keys, the declared
    /// whole-file md5s and the digest checks agree with the tool by
    /// construction instead of by every site remembering to.
    ///
    /// **No allocation in the normal case.** The live manifests spell their
    /// digests in lower case already, and `to_ascii_lowercase` would allocate
    /// for every one of the ~454 000 chunks regardless.
    fn normalise(&mut self);
}

impl SophonChunkExt for SophonChunk {
    fn identity(&self) -> ChunkIdentity<'_> {
        ChunkIdentity::new(&self.uncompressed_md5, self.uncompressed_size)
    }

    fn normalise(&mut self) {
        normalize_hex(&mut self.uncompressed_md5);
        normalize_hex(&mut self.compressed_md5);
    }
}

/// Capture's one-entry-point normalisation for a whole decoded manifest.
pub trait SophonChunkManifestExt {
    /// Lower-case every declared digest in the manifest.
    ///
    /// The one place a manifest enters the crate, and therefore the one place
    /// the normalisation has to happen for every later comparison to be
    /// right. See [`SophonChunkExt::normalise`].
    fn normalise(&mut self);
}

impl SophonChunkManifestExt for SophonChunkManifest {
    fn normalise(&mut self) {
        for e in &mut self.chuncks {
            e.normalise();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message as _;

    fn sample() -> SophonChunkManifest {
        SophonChunkManifest {
            chuncks: vec![SophonChunkFile {
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
            }],
        }
    }

    /// The digests are normalised once, where the manifest enters the crate,
    /// so that every later comparison agrees with `girpr`'s case-insensitive
    /// one by construction. An already-lower-case manifest is left untouched.
    #[test]
    fn normalising_lowercases_digests_and_leaves_lower_case_untouched() {
        let mut m = sample();
        m.chuncks[0].md5 = "D992A789".into();
        m.chuncks[0].chunks[0].uncompressed_md5 = "DE1CFE6927DACBC58D2D55A0B45EA0F3".into();
        m.chuncks[0].chunks[0].compressed_md5 = "06BA73BB".into();
        m.normalise();
        assert_eq!(m.chuncks[0].md5, "d992a789");
        assert_eq!(
            m.chuncks[0].chunks[0].uncompressed_md5,
            "de1cfe6927dacbc58d2d55a0b45ea0f3"
        );
        assert_eq!(m.chuncks[0].chunks[0].compressed_md5, "06ba73bb");

        // The live manifests spell their digests in lower case already; a
        // normalise of one must leave it byte-for-byte equal.
        let want = sample();
        let mut got = want.clone();
        got.normalise();
        assert_eq!(got, want);
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
        assert_eq!(
            got.chuncks[0].covered_end(),
            0,
            "an empty file covers nothing"
        );
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
        let got = SophonChunkManifest::decode(
            SophonChunkManifest { chuncks: vec![] }
                .encode_to_vec()
                .as_slice(),
        )
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
        assert_eq!(
            f["chunks"][0]["unknown_field6"],
            8_640_275_792_690_937_856i64
        );
    }

    /// **K1.** The decode boundary refuses a negative cost or offset with the
    /// chunk index, id and value, rather than letting it into the plan.
    #[test]
    fn a_negative_manifest_number_is_refused_with_its_chunk_and_value() {
        let mut m = sample();
        m.chuncks[0].chunks[0].compressed_size = -1;
        let entry = &m.chuncks[0];
        let e = entry.validate_arithmetic().unwrap_err();
        assert!(
            e.contains("chunk 0") && e.contains(&entry.chunks[0].id),
            "{e}"
        );
        assert!(e.contains("compressed_size -1"), "{e}");

        m.chuncks[0].chunks[0].compressed_size = 1;
        m.chuncks[0].chunks[1].offset = -7;
        let e = m.chuncks[0].validate_arithmetic().unwrap_err();
        assert!(e.contains("offset -7") && e.contains("chunk 1"), "{e}");
    }

    /// **K1.** An offset that cannot be added to its length is refused by name,
    /// and the totals that only measure the document saturate rather than abort.
    #[test]
    fn an_unaddable_manifest_offset_is_refused_and_totals_saturate() {
        let mut m = sample();
        m.chuncks[0].chunks[0].offset = i64::MAX;
        let e = m.chuncks[0].validate_arithmetic().unwrap_err();
        assert!(
            e.contains(&i64::MAX.to_string()) && e.contains("overflows i64"),
            "{e}"
        );

        let mut huge = m.chuncks[0].clone();
        huge.chunks[0].uncompressed_size = i64::MAX;
        huge.chunks[1].uncompressed_size = i64::MAX;
        assert_eq!(huge.chunk_bytes(), i64::MAX, "Σ saturates");
        huge.chunks[1].offset = i64::MAX;
        assert_eq!(huge.covered_end(), i64::MAX, "end saturates");
    }
}
