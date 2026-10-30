use super::*;

fn chunk(offset: i64, usize_: i64) -> SophonChunk {
    SophonChunk {
        id: format!("id{offset}"),
        uncompressed_md5: format!("{offset:032}"),
        offset,
        compressed_size: 100,
        uncompressed_size: usize_,
        unknown_field6: 7,
        compressed_md5: "c".into(),
    }
}

fn file(path: &str, chunks: Vec<SophonChunk>, size: i64) -> SophonChunkFile {
    SophonChunkFile {
        file: path.into(),
        chunks,
        is_folder: false,
        size,
        md5: "m".into(),
    }
}

/// A manifest is data, and every number in it came off the wire as an
/// `i64`. `decode_counts` must therefore *measure* a hostile one rather than
/// abort on it — it is the function that exists to tell a reader what a
/// manifest says, and it cannot do that by dying.
#[test]
fn a_manifest_whose_arithmetic_overflows_is_measured_not_fatal() {
    // `offset = i64::MAX` then `offset + uncompressed_size` overflowed: an
    // abort in debug and test builds, so this line alone could kill
    // `cargo nextest run`, and a silent wrap in release — which would report
    // the file as tiled when nothing had been checked.
    let entries = vec![file("huge", vec![chunk(i64::MAX, 4096)], 4096)];
    let d = decode_counts(&entries);
    assert_eq!(
        d.structure.non_gapless_files, 1,
        "an unaddable offset is not a tiling"
    );
    assert_eq!(d.total_chunks, 1, "and the entry is still counted");

    // And the reverse: a huge declared size must not become 18.4 quintillion
    // bytes in the artifact. `as u64` alone turned -1 into 2^64-1.
    let negative = vec![file("neg", vec![], -1)];
    let d = decode_counts(&negative);
    assert_eq!(d.total_file_bytes, 0, "a file of -1 B contributes 0 B");

    // Same for the field the doc calls "what bounds the tool's whole-chunk
    // buffering" — a negative compressed size must not record as ~1.8e19.
    let mut c = chunk(0, 10);
    c.compressed_size = -1;
    let d = decode_counts(&[file("neg", vec![c], 10)]);
    assert_eq!(
        d.max_chunk_compressed_bytes, 0,
        "and -1 B is not the maximum"
    );

    // A positive measurement still works, which is the half of this that has
    // to keep working.
    let ok = decode_counts(&[file("ok", vec![chunk(0, 10)], 10)]);
    assert_eq!(ok.max_chunk_compressed_bytes, 100);
    assert_eq!(ok.total_file_bytes, 10);
}

/// A manifest nobody can build on is refused by name, with exit `3`.
///
/// This is the check that has to run *before* a blob is fetched, and it was
/// unreachable from a unit test until it moved out of the command
/// dispatcher: a manifest that fails A3 has no good decode to compare
/// against, no `chunks/`, and no bundle to capture against.
#[test]
fn a_manifest_that_cannot_be_built_on_is_refused_by_name() {
    let good = |decoded: Decoded, stats: Option<crate::hyp::Stats>| CapturedManifest {
        ..captured(green(), decoded, stats)
    };

    // 1. A3/A4 failed: named as the verification it is, before any
    //    reconciliation, and the message carries both booleans.
    let mut v = green();
    v.decompressed_md5_matches_checksum = false;
    let e = verify(&captured(v, Decoded::default(), None), "7.0.0", "game").unwrap_err();
    assert_eq!(e.code, crate::exit::ExitCode::AssertionFailed, "{e:?}");
    assert!(
        e.detail.starts_with("verify the 7.0.0 game manifest:"),
        "{e:?}"
    );
    assert!(
        e.detail
            .contains("decompressed md5 matches checksum: false"),
        "{e}"
    );

    // 2. A5 failed: every disagreeing stat named with both numbers.
    let d = decode_counts(&[file("a", vec![chunk(0, 150)], 150)]);
    let wrong = Some(crate::hyp::Stats {
        compressed_size: Some("100".into()),
        uncompressed_size: Some("999".into()),
        file_count: Some("1".into()),
        chunk_count: Some("1".into()),
    });
    let e = verify(&good(d, wrong), "7.0.0", "game").unwrap_err();
    assert_eq!(e.code, crate::exit::ExitCode::AssertionFailed, "{e:?}");
    assert!(
        e.detail.starts_with("reconcile the 7.0.0 game manifest:"),
        "{e}"
    );
    assert!(
        e.detail
            .contains("uncompressed_size: declared Some(999), decoded 150"),
        "{e}"
    );
    assert!(
        !e.detail.contains("chunk_count"),
        "and only the one that disagrees: {e}"
    );

    // 3. Everything green passes, and a structural violation does not stop
    //    it — spec §3.6 says the capture still happens and M8 makes it a row.
    let d = decode_counts(&[file("a", vec![chunk(0, 150)], 150)]);
    let ok = Some(crate::hyp::Stats {
        compressed_size: Some("100".into()),
        uncompressed_size: Some("150".into()),
        file_count: Some("1".into()),
        chunk_count: Some("1".into()),
    });
    assert!(verify(&good(d.clone(), ok.clone()), "7.0.0", "game").is_ok());

    // A gap between the chunks, which reconciles byte-for-byte anyway: the
    // chunks still total 140, so A5 is satisfied and only §3.6's structural
    // property is broken.
    let holed = decode_counts(&[file("a", vec![chunk(0, 50), chunk(60, 90)], 140)]);
    let holed_stats = Some(crate::hyp::Stats {
        compressed_size: Some("100".into()),
        uncompressed_size: Some("140".into()),
        file_count: Some("1".into()),
        chunk_count: Some("2".into()),
    });
    assert!(
        verify(&good(holed, holed_stats), "7.0.0", "game").is_ok(),
        "a structural violation is recorded, not fatal"
    );
}

#[test]
fn a_clean_manifest_reports_every_structure_counter_zero() {
    let entries = vec![
        file("a", vec![chunk(0, 100), chunk(100, 50)], 150),
        file("b/c", vec![], 0),
    ];
    let d = decode_counts(&entries);
    assert_eq!(
        (d.entries, d.files, d.folders, d.total_chunks),
        (2, 2, 0, 2)
    );
    assert_eq!(d.total_file_bytes, 150);
    assert_eq!(d.max_chunk_compressed_bytes, 100);
    assert!(d.structure.clean(), "{:?}", d.structure);
    assert!(d.structure.violations().is_empty());
}

/// The verification a manifest that reconciles would carry.
fn green() -> Verified {
    Verified {
        decompressed_md5_matches_checksum: true,
        checksum_matches_id_suffix: true,
        id_prefix_verified: false,
    }
}

/// A captured manifest, assembled field by field.
///
/// Deliberately not a `Default`-derived fixture: three of the six structs on
/// the way here have no honest empty state (`IdParts.prefix_semantics` and
/// `Stored.container` are `&'static str` that carry real claims), and adding
/// a `Default` to production types purely to shorten a test is the wrong
/// trade. What `verify` reads is `meta.verified`, `meta.decoded`,
/// `meta.declared` and `meta.decoded.structure`; the rest is spelled out so
/// that stays visible.
fn captured(
    verified: Verified,
    decoded: Decoded,
    stats: Option<crate::hyp::Stats>,
) -> CapturedManifest {
    CapturedManifest {
        tag: "7.0.0".into(),
        build_id: "3XnPviMfoILi".into(),
        matching_field: "game".into(),
        seq: 8,
        raw: Vec::new(),
        entries: Vec::new(),
        meta: ManifestMeta {
            schema_version: crate::SCHEMA_VERSION,
            tag: "7.0.0".into(),
            build_id: "3XnPviMfoILi".into(),
            matching_field: "game".into(),
            category_id: "10016".into(),
            category_name: "Genshin Impact".into(),
            manifest: DeclaredManifest {
                id: "manifest_abc".into(),
                checksum: "c".into(),
                compressed_size: "100".into(),
                uncompressed_size: "150".into(),
            },
            declared: Declared {
                stats,
                ..Declared::default()
            },
            id_parts: IdParts {
                prefix_16hex: "abc".into(),
                prefix_semantics: "opaque",
                suffix_32hex: "c".into(),
                suffix_equals_checksum: true,
            },
            stored: Stored {
                path: String::new(),
                bytes: 0,
                container: "zstd",
                md5_compressed: String::new(),
                sha256_compressed: String::new(),
                md5_decompressed: String::new(),
                bytes_decompressed: 0,
                encoding: Encoding::Identity,
            },
            verified,
            decoded,
            full_decode_written: true,
        },
    }
}

/// Each input breaks exactly one structural counter.
///
/// Spec §3.6: a violation must be *counted*, not fatal — the capture still
/// works and the selfcheck says which property broke.
///
/// Asserting *containment* rather than equality is deliberate, and is the honest
/// form: the six properties are not independent. Walking a chunk list in the
/// order it is stored, any out-of-order offsets are *also* non-gapless — there
/// is no way to present them out of order and still cover `0..size` in one pass.
/// A test demanding that one broken property be the only broken one would be
/// demanding the counters be wrong.
#[test]
fn each_input_breaks_the_property_it_is_named_for() {
    let cases: Vec<(&str, SophonChunkFile, &str)> = vec![
        ("empty_paths", file("", vec![], 0), "empty_paths"),
        (
            "backslash_paths",
            file("a\\b", vec![], 0),
            "backslash_paths",
        ),
        (
            "non_ascii_paths",
            file("a/\u{4e2d}\u{6587}", vec![], 0),
            "non_ascii_paths",
        ),
        (
            "unsorted_chunk_offsets",
            file("x", vec![chunk(100, 50), chunk(0, 100)], 150),
            "unsorted_chunk_offsets",
        ),
        (
            "non_gapless_files",
            file("x", vec![chunk(0, 50), chunk(100, 50)], 150),
            "non_gapless_files",
        ),
        (
            "size_mismatch_files",
            file("x", vec![chunk(0, 50), chunk(50, 50)], 200),
            "size_mismatch_files",
        ),
    ];
    for (name, entry, prop) in cases {
        let d = decode_counts(&[entry]);
        assert!(!d.structure.clean(), "{name} should not be clean");
        assert!(
            d.structure.violations().contains(&prop),
            "{name}: expected {prop} among {:?}",
            d.structure.violations()
        );
    }
}

/// The properties that *can* be isolated: ascending-but-gapped, and
/// gapless-but-wrong-length, are each a single violation.
#[test]
fn a_gap_and_a_length_disagreement_are_each_reported_alone() {
    // Offsets 0 and 60 with lengths 50 and 90 tile 0..150 in total but leave
    // a hole at 50..60, so `Σ == size` holds and only the gap is reported.
    let gap = decode_counts(&[file("x", vec![chunk(0, 50), chunk(60, 90)], 140)]);
    assert_eq!(gap.structure.violations(), vec!["non_gapless_files"]);
    assert_eq!(
        gap.structure.unsorted_chunk_offsets, 0,
        "offsets are still ascending"
    );
    assert_eq!(gap.structure.size_mismatch_files, 0);

    // Chunks covering 0..100 exactly, against a declared size of 200: the
    // coverage is fine and only the length disagrees.
    let len = decode_counts(&[file("x", vec![chunk(0, 50), chunk(50, 50)], 200)]);
    assert_eq!(len.structure.violations(), vec!["size_mismatch_files"]);
    assert_eq!(len.structure.non_gapless_files, 0);
}

/// An overlap is a gap in the wrong direction, and is counted as one.
#[test]
fn an_overlapping_chunk_is_non_gapless_too() {
    // 0..100 then 50..150: total 150 matches the declared size, offsets
    // ascend, and the two ranges overlap.
    let d = decode_counts(&[file("x", vec![chunk(0, 100), chunk(50, 50)], 150)]);
    assert_eq!(d.structure.violations(), vec!["non_gapless_files"]);
}

#[test]
fn a_duplicate_path_is_counted_per_extra_occurrence() {
    let d = decode_counts(&[
        file("a", vec![], 0),
        file("a", vec![], 0),
        file("a", vec![], 0),
    ]);
    assert_eq!(d.structure.duplicate_paths, 2);
    assert_eq!(d.structure.violations(), vec!["duplicate_paths"]);
}

/// A folder carries no bytes, so reporting it as a gap would make a single
/// folder entry look like six violations.
#[test]
fn a_folder_entry_is_not_counted_as_a_byte_level_violation() {
    let folder = SophonChunkFile {
        file: "GenshinImpact_Data/StreamingAssets".into(),
        chunks: vec![],
        is_folder: true,
        size: 0,
        md5: String::new(),
    };
    let d = decode_counts(&[folder]);
    assert_eq!((d.files, d.folders), (0, 1));
    assert!(d.structure.clean(), "{:?}", d.structure);
}

#[test]
fn the_largest_chunk_is_recorded_across_every_entry() {
    let mut big = chunk(0, 10);
    big.compressed_size = 3_689_476;
    let d = decode_counts(&[file("a", vec![big, chunk(10, 10)], 20)]);
    // The measured ceiling for 7.0.0 and 7.1.0, spec §3.6.
    assert_eq!(d.max_chunk_compressed_bytes, 3_689_476);
}

// ------------------------------------------------------------ A5

#[test]
fn a_reconciling_manifest_reports_nothing() {
    let d = decode_counts(&[file("a", vec![chunk(0, 150)], 150)]);
    let stats = crate::hyp::Stats {
        compressed_size: Some("100".into()),
        uncompressed_size: Some("150".into()),
        file_count: Some("1".into()),
        chunk_count: Some("1".into()),
    };
    assert!(reconcile(&d, Some(&stats)).is_empty());
    // No declared stats is not a violation; it is simply nothing to check.
    assert!(reconcile(&d, None).is_empty());
}

#[test]
fn each_disagreeing_stat_is_named_with_both_numbers() {
    let d = decode_counts(&[file("a", vec![chunk(0, 150)], 150)]);
    let stats = crate::hyp::Stats {
        compressed_size: Some("100".into()),
        uncompressed_size: Some("999".into()),
        file_count: Some("7".into()),
        chunk_count: Some("1".into()),
    };
    let bad = reconcile(&d, Some(&stats));
    let names: Vec<&str> = bad.iter().map(|(n, _, _)| *n).collect();
    assert_eq!(names, vec!["file_count", "uncompressed_size"]);
    assert_eq!(bad[0], ("file_count", Some(7), 1));
    assert_eq!(bad[1], ("uncompressed_size", Some(999), 150));
}

#[test]
fn an_unparsable_declared_size_is_skipped_rather_than_called_a_violation() {
    let d = decode_counts(&[file("a", vec![chunk(0, 150)], 150)]);
    let stats = crate::hyp::Stats {
        compressed_size: None,
        uncompressed_size: Some("not a number".into()),
        file_count: Some("1".into()),
        chunk_count: None,
    };
    assert!(reconcile(&d, Some(&stats)).is_empty());
}

/// A manifest entry is data from the wire, and the guard runs before `--out`
/// is created. `a/b.` and `a/b` are one file on Windows — and `a/./b` is a
/// third spelling of it — so accepting the shapes would make the last write
/// win wherever the tree is materialised. This pins that `manifest::capture`
/// consults the shared rule, not only `--files`.
///
/// **And that it collects every offending name before refusing.** Stopping at
/// the first would make a second bad path discoverable only by editing the
/// first out of a downloaded manifest.
#[test]
fn a_manifest_entry_windows_folds_onto_another_is_a_shape_error() {
    for (bad, rule) in [
        ("a/b.", "path.windows-fold"),
        ("a/b ", "path.windows-fold"),
        ("a/./b", "path.dot-component"),
    ] {
        let entries = vec![file(bad, vec![], 10)];
        let rejections = crate::rejections::Rejections::new();
        let err = validate_entry_paths("7.1.0/game", &entries, &rejections)
            .expect_err("a folded name must stop the run");
        let ApiError::Shape { subject, detail } = err else {
            panic!("{bad:?} must be a shape error");
        };
        assert_eq!(subject, "7.1.0/game");
        assert!(detail.contains("1 refused name"), "{bad:?}: {detail}");
        let rows = rejections.snapshot();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value, bad);
        assert_eq!(rows[0].rule, rule);
        assert_eq!(rows[0].source, "manifest 7.1.0/game entry 0 file");
    }
    // An ordinary dotted name is still legal.
    assert!(
        validate_entry_paths(
            "u",
            &[file("a/b.c", vec![], 10)],
            &crate::rejections::Rejections::new()
        )
        .is_ok(),
        "a dot inside a name is not a folded trailing dot"
    );
}

/// One refusal does not hide the next: every offending entry and chunk id is
/// named in the failure and recorded in the rejection document. A device
/// name is **not** one of them — it is accepted and escaped on disk.
#[test]
fn every_hostile_name_in_one_manifest_is_reported_together() {
    let mut bad_chunk = chunk(0, 1);
    bad_chunk.id = "not-a-chunk-id".into();
    let entries = vec![
        file("a/b.", vec![], 1),
        file("CON", vec![], 1),
        file("ok/file", vec![bad_chunk], 1),
    ];
    let rejections = crate::rejections::Rejections::new();
    let err = validate_entry_paths("7.1.0/game", &entries, &rejections)
        .expect_err("two bad names must stop the run");
    let ApiError::Shape { detail, .. } = err else {
        panic!("a shape error");
    };
    assert!(detail.contains("2 refused names"), "{detail}");
    let rows = rejections.snapshot();
    let rules: Vec<&str> = rows.iter().map(|r| r.rule.as_str()).collect();
    assert_eq!(
        rules,
        vec!["path.windows-fold", "chunk-id.shape"],
        "one row per offending name, with its own rule"
    );
    let values: Vec<&str> = rows.iter().map(|r| r.value.as_str()).collect();
    assert_eq!(values, vec!["a/b.", "not-a-chunk-id"]);
    assert!(
        !rows.iter().any(|r| r.value == "CON"),
        "a device name is kept, not refused; the writer escapes it"
    );
}

// ------------------------------------------------------------ A4

#[test]
fn the_id_parts_are_split_and_the_prefix_is_labelled_opaque() {
    let m = crate::hyp::ManifestRef {
        id: "manifest_671e1a92a6cf53ff_8d4dfb34d2ee2cf64aae45a9b1ecf58d".into(),
        checksum: "8d4dfb34d2ee2cf64aae45a9b1ecf58d".into(),
        compressed_size: Some("9007187".into()),
        uncompressed_size: Some("16827424".into()),
    };
    assert_eq!(m.id_prefix(), Some("manifest_671e1a92a6cf53ff"));
    assert_eq!(m.id_suffix(), Some(m.checksum.as_str()));
    let parts = IdParts {
        prefix_16hex: m.id_prefix().unwrap().to_string(),
        prefix_semantics: "opaque",
        suffix_32hex: m.id_suffix().unwrap().to_string(),
        suffix_equals_checksum: true,
    };
    // The field exists so the artifact *states* the finding rather than
    // implying the prefix went unchecked.
    assert_eq!(parts.prefix_semantics, "opaque");
    assert!(parts.suffix_equals_checksum);
    assert_eq!(parts.suffix_32hex.len(), 32);
}

#[test]
fn a_verified_report_is_green_only_when_both_checks_hold() {
    let ok = Verified {
        decompressed_md5_matches_checksum: true,
        checksum_matches_id_suffix: true,
        id_prefix_verified: false,
    };
    assert!(ok.all_green());
    for bad in [
        Verified {
            decompressed_md5_matches_checksum: false,
            ..ok.clone()
        },
        Verified {
            checksum_matches_id_suffix: false,
            ..ok.clone()
        },
    ] {
        assert!(!bad.all_green());
    }
    // The prefix is never verified and that is not a failure.
    assert!(ok.all_green() && !ok.id_prefix_verified);
}

#[test]
fn a_declared_size_is_carried_as_the_api_sent_it() {
    assert_eq!(render_size(Some(&"9007187".into())), "9007187");
    assert_eq!(render_size(Some(&crate::hyp::IntOrStr::Int(42))), "42");
    assert_eq!(render_size(None), "");
    assert_eq!(size_of(Some(&"9007187".into())), Some(9_007_187));
    assert_eq!(size_of(None), None);
}
