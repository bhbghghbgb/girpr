use super::*;

#[test]
fn opening_a_path_that_is_not_a_directory_says_what_the_flag_takes() {
    let e = Bundle::open(Path::new("E:/definitely/not/here")).unwrap_err();
    assert!(e.contains("not a directory"), "{e}");
    assert!(e.contains("--artifact"), "{e}");
    assert!(e.contains("MANIFEST.json"), "{e}");
}

#[test]
fn opening_a_directory_with_no_manifest_names_the_missing_document() {
    let d = girpr_tempdir::TempDir::new("gircap-m9-empty", "no_manifest");
    let e = Bundle::open(&d).unwrap_err();
    assert!(e.contains("MANIFEST.json"), "{e}");
}

/// **A bundle written by an older schema is refused, and the refusal says
/// what to do.**
///
/// The schema bump exists precisely so that a version-1 bundle can be
/// refused: its patchmaps carry a `repair` block and an
/// `expectations.summary` block, and its class ids named a repair tool's
/// internal decisions — a reader that understood the current shape would
/// ignore all three and report the bundle as consistent when it is not.
///
/// The manifest here is a version-1 manifest with **nothing else in it**,
/// which is the point: the version is read on its own, so the refusal
/// names the version rather than a missing field the version caused.
#[test]
fn a_v1_bundle_is_refused_with_the_command_that_makes_a_current_one() {
    let d = girpr_tempdir::TempDir::new("gircap-v1", "v1_bundle");
    std::fs::write(d.join("MANIFEST.json"), br#"{"schema_version": 1}"#)
        .expect("writes the manifest");
    let e = Bundle::open(&d).unwrap_err();
    assert!(e.contains("schema version 1"), "{e}");
    assert!(e.contains("is not the current"), "{e}");
    assert!(
        e.contains("cargo run -p gircap -- capture --out"),
        "the refusal must say how to make a current one: {e}"
    );
}

/// **And `list` does not let it vanish.**
///
/// `list` is an index, and an index that quietly omits a bundle is wrong.
/// A directory that is not a bundle at all is stepped over — a half-written
/// capture is a normal transient — but one that is a bundle this build
/// refuses is named, with the reason.
#[test]
fn a_v1_bundle_is_named_by_list_rather_than_dropped() {
    let d = girpr_tempdir::TempDir::new("gircap-v1-list", "v1_list");
    let old = d.join("old");
    std::fs::create_dir_all(&old).expect("makes the directory");
    std::fs::write(old.join("MANIFEST.json"), br#"{"schema_version": 1}"#)
        .expect("writes the manifest");

    let index = list(&d).expect("lists");
    assert!(
        index.bundles.is_empty(),
        "a bundle that does not open is not a row: {index:?}"
    );
    assert_eq!(index.unreadable.len(), 1, "{index:?}");
    assert_eq!(index.unreadable[0].path, "old");
    assert!(
        index.unreadable[0].reason.contains("schema version 1"),
        "and the reason is the version: {index:?}"
    );
}

/// A directory that is not a bundle at all is stepped over silently: it is
/// a half-written capture, not something the operator has to act on.
#[test]
fn a_directory_that_is_not_a_bundle_is_not_named_by_list() {
    let d = girpr_tempdir::TempDir::new("gircap-not-a-bundle", "not_a_bundle");
    let broken = d.join("broken");
    std::fs::create_dir_all(&broken).expect("makes the directory");
    std::fs::write(broken.join("MANIFEST.json"), b"{ not json").expect("writes it");

    let index = list(&d).expect("lists");
    assert!(index.bundles.is_empty(), "{index:?}");
    assert!(
        index.unreadable.is_empty(),
        "a manifest that is not JSON is not a bundle this build refuses: {index:?}"
    );
}

/// A directory the walk cannot read is **named**, not skipped.
///
/// `list` is the operator's inventory; dropping a subtree makes a confident
/// answer that is short. The reader is real here — a file passed where a
/// directory is expected is the cheapest read failure a test can make.
#[test]
fn a_walk_that_cannot_read_a_directory_names_it() {
    let d = girpr_tempdir::TempDir::new("gircap-list-unreadable", "unreadable");
    let file = d.join("not_a_directory");
    std::fs::write(&file, b"x").expect("writes a file");

    let e = child_dirs(&file, |p| {
        let rd = std::fs::read_dir(p)?;
        Ok(rd.map(|e| e.map(|e| e.path())).collect())
    })
    .expect_err("a file cannot be read as a directory");
    assert!(e.contains("cannot read"), "{e}");
    assert!(e.contains("not_a_directory"), "the path is named: {e}");
}

/// And the walk propagates it: `list` fails rather than returning the
/// directories it managed to see before the unreadable one.
#[test]
fn a_list_walk_that_cannot_read_a_directory_fails_instead_of_answering() {
    let d = girpr_tempdir::TempDir::new("gircap-list-fails", "walk_fails");
    let e = list_with(
        &d,
        |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "share offline",
            ))
        },
        |p| std::fs::read(p),
    )
    .expect_err("the walk did not finish");
    assert!(e.contains("cannot read"), "{e}");
    assert!(
        e.contains(&d.display().to_string()),
        "the root is named: {e}"
    );
}

/// A manifest the walk cannot **read** is a bundle it could not see, not a
/// directory that is not a bundle.
///
/// A backup tool, an indexer or an editor can hold `MANIFEST.json` with an
/// exclusive share. The file is still there and still a file, so the old
/// `is_file()` probe passed and the failed read in `schema_version_of` was
/// flattened to "not a bundle": the bundle vanished from the inventory and
/// `list` exited `0`. The reader is injected because a share lock is
/// platform-specific; the failure it simulates is the same `fs::read` arm
/// the real lock takes.
#[test]
fn a_list_walk_that_cannot_read_a_manifest_fails_instead_of_dropping_the_bundle() {
    let d = girpr_tempdir::TempDir::new("gircap-list-manifest", "manifest_read");
    let bundle = d.join("hk4e_global").join("7.0.0__7.1.0").join("cap");
    std::fs::create_dir_all(&bundle).expect("makes the bundle directory");
    std::fs::write(bundle.join("MANIFEST.json"), br#"{"schema_version": 2}"#)
        .expect("writes a real manifest");

    let e = list_with(
        &d,
        |p| {
            let rd = std::fs::read_dir(p)?;
            Ok(rd.map(|e| e.map(|e| e.path())).collect())
        },
        |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "held by another process",
            ))
        },
    )
    .expect_err("a manifest that cannot be read is not a shorter inventory");
    assert!(e.contains("cannot read"), "{e}");
    assert!(e.contains("MANIFEST.json"), "the manifest is named: {e}");
    assert!(
        e.contains("held by another process"),
        "the cause is kept: {e}"
    );
}

/// A stat failure that is not `NotFound` is a manifest the walk could not
/// see, not an absent one.
///
/// The old `is_file()` probe turned any stat failure into `false` and the
/// directory was treated as "not a bundle at all" before any read happened.
/// A path holding a NUL cannot be stat'ed for a reason other than absence
/// on either host; the walk itself only reaches this arm when a share or an
/// ACL denies the stat, which a unit test cannot set up portably.
#[test]
fn a_manifest_that_cannot_be_stat_ed_fails_the_probe() {
    let bad = Path::new("bad\0name").join("MANIFEST.json");
    let e = probe_manifest(&bad, |_| {
        panic!("the read is not reached when the stat fails")
    })
    .expect_err("a stat failure is not an absent manifest");
    assert!(e.contains("cannot read"), "{e}");
    assert!(e.contains("MANIFEST.json"), "the manifest is named: {e}");
}

/// One failed entry is the whole walk's failure: a child that vanished
/// between `read_dir` and the stat may be a bundle, so it cannot be dropped.
#[test]
fn a_walk_that_loses_one_entry_names_the_directory() {
    let d = girpr_tempdir::TempDir::new("gircap-list-entry", "entry_error");
    let denied = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
    let e = child_dirs(&d, move |_| Ok(vec![Err(denied)]))
        .expect_err("an entry error is not a shorter listing");
    assert!(e.contains("cannot read an entry"), "{e}");
    assert!(e.contains("denied"), "the cause is kept: {e}");
}

/// An entry whose metadata cannot be read is an error too; `is_dir`'s
/// `false` on a failed stat is the same silent omission one level down.
#[test]
fn a_walk_that_cannot_stat_one_entry_names_it() {
    let d = girpr_tempdir::TempDir::new("gircap-list-stat", "stat_error");
    let vanished = d.join("vanished");
    let e =
        child_dirs(&d, move |_| Ok(vec![Ok(vanished)])).expect_err("a missing entry is not a file");
    assert!(e.contains("cannot read"), "{e}");
    assert!(e.contains("vanished"), "the path is named: {e}");
}

/// `REDERIVED` and the spec's own list must not overlap, or "re-derived"
/// would be claiming an assertion this command never looked at.
#[test]
fn the_rederived_ids_are_ones_this_command_actually_checks() {
    for id in REDERIVED {
        assert!(
            crate::emit::selfcheck::wording(id) != "an assertion this build does not describe",
            "A{id} has no wording, so re-deriving it would be meaningless"
        );
    }
    assert!(REDERIVED.contains(&"A7") && REDERIVED.contains(&"A8"));
    // The two sets must not overlap. `check_agreement` compares REDERIVED
    // rows against what the capture recorded; a row that is never recorded
    // has nothing to agree with, and listing it in both would have the
    // summary claim a cross-check that cannot happen.
    for id in DERIVED_UNRECORDED {
        assert!(
            !REDERIVED.contains(&id),
            "A{id} is in both sets, so it would be checked against a row no capture writes"
        );
    }
    // And they are disjoint from the ids a capture does record, which is
    // what keeps `check_agreement` honest.
    for id in DERIVED_UNRECORDED {
        assert!(
            !crate::emit::selfcheck::WORDING
                .iter()
                .any(|(w, _)| *w == id),
            "A{id} is claimed as never recorded, but a capture records it"
        );
    }
    assert_eq!(
        derived_ids().len(),
        REDERIVED.len() + DERIVED_UNRECORDED.len()
    );
}

/// A20 re-derives a patchmap's arithmetic from the patchmap, so it needs no
/// API body - which is the whole reason it can live here.
#[test]
fn a20_is_derived_from_bundle_documents_alone() {
    assert!(DERIVED_UNRECORDED.contains(&A20_ID));
    assert_eq!(A20_ID, "A20");
    assert!(!A20.is_empty());
}

/// A patchmap whose plan is exactly these FETCH costs, with totals and
/// expectations carrying an honest (or deliberately wrong) sum.
fn arithmetic_map(costs: &[i64]) -> crate::assemble::PatchMap {
    use crate::assemble::ranges::Range;
    use crate::assemble::{Decision, Expectations, PatchMap, PlanEntry, Totals};
    let plan: Vec<PlanEntry> = costs
        .iter()
        .enumerate()
        .map(|(i, &cost)| PlanEntry {
            i,
            decision: Decision::Fetch,
            target_range: Range::at(i as i64, 1),
            source_range: None,
            chunk_id: format!("chunk-{i}"),
            url: None,
            compressed_size: cost,
            uncompressed_size: 1,
            uncompressed_md5: "00".repeat(16),
            note: None,
        })
        .collect();
    let download_bytes = costs.iter().fold(0_i64, |t, c| t.saturating_add(*c));
    PatchMap {
        schema_version: crate::SCHEMA_VERSION,
        capture_id: "a20".into(),
        path: "dll".into(),
        matching_field: "game".into(),
        classes: Vec::new(),
        old: None,
        new: None,
        totals: Some(Totals {
            chunks: plan.len(),
            reuse: 0,
            fetch: plan.len(),
            moved: 0,
            download_bytes,
            materialize_old_bytes: 0,
            materialize_new_bytes: 0,
        }),
        plan: Some(plan),
        expectations: Expectations {
            result_path: None,
            result_md5: None,
            absent_after: false,
            download_bytes,
            must_not_fetch: Vec::new(),
        },
    }
}

/// **K1.** A negative `compressed_size` is not a small download: A20 must
/// fail with the chunk and value rather than let a negative expectation
/// agree with a negative sum.
#[test]
fn a_negative_fetch_cost_fails_a20_with_the_chunk_and_value() {
    let rows = check_patchmap_arithmetic(&arithmetic_map(&[-1]));
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(!rows[0].ok);
    assert!(rows[0].actual.contains("chunk-0"), "{}", rows[0].actual);
    assert!(rows[0].actual.contains("-1"), "{}", rows[0].actual);
}

/// **K1.** Two `i64::MAX` costs do not fit; A20 names the term and the
/// running total instead of aborting in debug or wrapping in release.
#[test]
fn an_overflowing_fetch_cost_fails_a20_instead_of_aborting() {
    let rows = check_patchmap_arithmetic(&arithmetic_map(&[i64::MAX, i64::MAX]));
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(!rows[0].ok);
    assert!(
        rows[0].actual.contains("chunk-1") && rows[0].actual.contains("overflows i64"),
        "{}",
        rows[0].actual
    );
}

/// The set is deliberately smaller than the seventeen.
#[test]
fn assertions_needing_the_api_are_not_claimed_as_re_derived() {
    for id in [
        "A1", "A2", "A3", "A4", "A5", "A9", "A10", "A12", "A14", "A15",
    ] {
        assert!(!REDERIVED.contains(&id), "A{id} is listed as re-derived");
    }
}

#[test]
fn a_blob_indexed_but_not_stored_is_red_rather_than_skipped() {
    let index = ChunkIndex::build(vec![]);
    assert!(
        check_blobs(
            Path::new("."),
            &index,
            crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES
        )
        .is_empty()
    );
    let props = crate::blobs::props::BlobProps::unmeasured(&crate::manifest::proto::SophonChunk {
        id: "aa00_".to_string() + &"0".repeat(32),
        uncompressed_md5: "0".repeat(32),
        offset: 0,
        compressed_size: 10,
        uncompressed_size: 100,
        unknown_field6: 0,
        compressed_md5: "1".repeat(32),
    });
    assert!(props.stored_path.is_none());
    let index = ChunkIndex::build(vec![(
        crate::blobs::IndexedBlob {
            props,
            referenced_by: Vec::new(),
            sets: Vec::new(),
        },
        Vec::new(),
    )]);
    let rows = check_blobs(
        Path::new("."),
        &index,
        crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
    );
    assert_eq!(rows.len(), 1);
    assert!(
        !rows[0].ok,
        "a listed-but-unstored blob cannot be materialized"
    );
    assert!(
        rows[0].actual.contains("nothing was stored"),
        "{}",
        rows[0].actual
    );
}

#[test]
fn a_blob_whose_bytes_were_altered_turns_its_row_red() {
    let d = girpr_tempdir::TempDir::new("gircap-m9-blob", "altered_bytes");
    std::fs::create_dir_all(d.join("chunks/ab")).unwrap();
    let body = vec![7_u8; 64];
    let raw = zstd::encode_all(body.as_slice(), 1).unwrap();
    std::fs::write(d.join("chunks/ab/cd"), &raw).unwrap();
    let chunk = crate::manifest::proto::SophonChunk {
        id: format!("ab{:0>29}", "cd"),
        uncompressed_md5: hash::md5_hex(&body),
        offset: 0,
        compressed_size: raw.len() as i64,
        uncompressed_size: 64,
        unknown_field6: 0,
        compressed_md5: hash::md5_hex(&raw),
    };
    let mut props = crate::blobs::props::BlobProps::measure(&raw, &chunk).unwrap();
    props.set_stored("chunks/ab/cd");
    let index = ChunkIndex::build(vec![(
        crate::blobs::IndexedBlob {
            props,
            referenced_by: Vec::new(),
            sets: Vec::new(),
        },
        Vec::new(),
    )]);
    assert!(
        check_blobs(&d, &index, crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES)[0].ok,
        "the good bundle's row is green"
    );

    // Flip one byte inside the frame: it still decodes, to different bytes.
    let mut bad = raw.clone();
    let last = bad.len() - 1;
    bad[last] ^= 0xFF;
    std::fs::write(d.join("chunks/ab/cd"), &bad).unwrap();
    let row = &check_blobs(&d, &index, crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES)[0];
    assert!(!row.ok, "one altered byte turns the row red");
    assert_eq!(row.subject, "chunks/ab/cd", "and names the blob");
    assert!(row.id == "A6", "and the assertion: {}", row.id);
}

#[test]
fn a_status_the_reader_cannot_agree_with_is_not_counted_as_ok() {
    assert!(crate::emit::selfcheck::MatrixRow::ok_status("captured"));
    assert!(crate::emit::selfcheck::MatrixRow::ok_status("covered_by"));
    assert!(crate::emit::selfcheck::MatrixRow::ok_status("over_budget"));
    assert!(crate::emit::selfcheck::MatrixRow::ok_status("unavailable"));
    assert!(!crate::emit::selfcheck::MatrixRow::ok_status(
        "something else"
    ));
}
