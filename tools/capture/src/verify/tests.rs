use super::*;

/// Create a directory link. A host that denies it (Windows without the
/// privilege) makes the link tests skip with a clear message rather than fail
/// the suite.
#[cfg(unix)]
fn link_dir(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}
#[cfg(windows)]
fn link_dir(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}

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
/// Treating any stat failure as "not a bundle at all" would hide a share or
/// ACL denial behind "no manifest here". A path holding a NUL cannot be
/// stat'ed for a reason other than absence on either host; the walk itself
/// only reaches this arm when a share or an ACL denies the stat, which a unit
/// test cannot set up portably.
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

/// A link back up the tree closes a cycle. `outer` has no manifest so the walk
/// descends into it; without a visited set `outer/back` would send the walk
/// back to the root forever. The one real bundle is still listed, once.
#[test]
fn a_directory_cycle_terminates_and_lists_each_real_bundle_once() {
    let d = girpr_tempdir::TempDir::new("gircap-list-cycle", "cycle");
    let outer = d.join("outer");
    let nested = outer.join("nested");
    std::fs::create_dir_all(&nested).expect("makes the directories");
    write_minimal_bundle(&nested, "nested_capture");
    if !link_dir(&d, &outer.join("back")) {
        eprintln!("skipping list cycle test: directory symlinks unavailable on this host");
        return;
    }
    let index = list(&d).expect("lists");
    assert_eq!(index.bundles.len(), 1, "{index:?}");
    assert_eq!(index.bundles[0].path, "outer/nested", "{index:?}");
    assert_eq!(index.bundles[0].capture_id, "nested_capture");
    assert!(index.unreadable.is_empty(), "{index:?}");
}

/// A bundle reachable only through a link whose target is outside the root is
/// not part of the inventory: its bytes belong to another tree.
#[test]
fn a_link_to_a_bundle_outside_the_root_is_not_listed() {
    let d = girpr_tempdir::TempDir::new("gircap-list-outside-link", "outside_link");
    let outside = girpr_tempdir::TempDir::new("gircap-list-outside-target", "outside_target");
    write_minimal_bundle(&outside, "outside_capture");
    if !link_dir(&outside, &d.join("escape")) {
        eprintln!("skipping list outside-link test: directory symlinks unavailable on this host");
        return;
    }
    let index = list(&d).expect("lists");
    assert!(
        index.bundles.is_empty(),
        "a bundle behind a link out of the root is not listed: {index:?}"
    );
    assert!(index.unreadable.is_empty(), "{index:?}");
}

/// A symlinked root is legitimate: the canonical root is its target, and rows
/// stay relative to the link that was passed to `list`.
#[test]
fn a_symlinked_root_is_listed_as_its_target() {
    let real = girpr_tempdir::TempDir::new("gircap-list-linked-target", "linked_target");
    let bundle = real.join("cap");
    write_minimal_bundle(&bundle, "linked_capture");
    let alias = girpr_tempdir::TempDir::new("gircap-list-linked-alias", "linked_alias");
    let link = alias.join("root_link");
    if !link_dir(&real, &link) {
        eprintln!("skipping symlinked-root test: directory symlinks unavailable on this host");
        return;
    }
    let index = list(&link).expect("lists the symlinked root");
    assert_eq!(index.bundles.len(), 1, "{index:?}");
    assert_eq!(index.bundles[0].path, "cap", "{index:?}");
    assert_eq!(index.bundles[0].capture_id, "linked_capture");
}

/// The smallest directory `Bundle::open` accepts, written with the crate's own
/// value shapes so a reader change cannot leave a stale hand-rolled fixture.
/// There are no blobs and no captures: `list` only needs a bundle that opens.
fn write_minimal_bundle(root: &Path, capture_id: &str) {
    use crate::emit::index::Checksums;
    use crate::emit::selfcheck::{Counts, Result_, Selfcheck};
    use crate::http::ErrorResponses;
    use crate::http::probe::ErrorShapes;
    use crate::pathmap::PathMapDocument;
    use crate::rejections::RejectionsDocument;

    let write = |rel: &str, value: serde_json::Value| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().expect("has a parent")).expect("makes dirs");
        std::fs::write(&path, serde_json::to_vec(&value).expect("serializes")).expect("writes");
    };
    let v = crate::SCHEMA_VERSION;
    write(
        "MANIFEST.json",
        serde_json::json!({
            "schema_version": v,
            "tool": { "name": "gircap", "version": "test" },
            "capture": {
                "id": capture_id,
                "captured_at": "20260101T000000Z",
                "host_os": "test",
                "tool_argv": [],
                "tool_argv_raw": [],
                "complete": true,
            },
            "biz": {
                "id": "biz", "launcher_id": "", "game_id": "", "channel": "",
                "sub_channel": "", "cps": "", "hyp_base": "", "sophon_base": "",
            },
            "versions": {
                "new": { "tag": "2.0.0", "build_id": "new-build", "source": "test" },
                "old": { "tag": "1.0.0", "build_id": "old-build", "source": "test" },
                "diff_tags": [],
            },
            "field": "game",
            "audio": [],
            "gzip_threshold": 0,
            "max_decompressed_bytes": 0,
            "selection_pool": { "fields": [], "manifest_entries": {}, "note": "" },
            "totals": {
                "files_captured": 0, "chunks_blobs": 0,
                "bytes_downloaded": 0, "bytes_on_disk": 0,
            },
            "matrix": [],
            "files": [],
            "totals_by_set": {},
        }),
    );
    write(
        "chunks/index.json",
        serde_json::to_value(ChunkIndex::build(vec![])).expect("serializes"),
    );
    write(
        "verify/checksums.json",
        serde_json::to_value(Checksums {
            schema_version: v,
            algorithm: "sha256".into(),
            entries: vec![],
            file_count: 0,
            total_bytes: 0,
        })
        .expect("serializes"),
    );
    write(
        "verify/selfcheck.json",
        serde_json::to_value(Selfcheck {
            schema_version: v,
            result: Result_::Pass,
            counts: Counts {
                total: 0,
                passed: 0,
                failed: 0,
            },
            assertions: vec![],
            matrix: vec![],
        })
        .expect("serializes"),
    );
    write(
        "api/error_shapes.json",
        serde_json::to_value(ErrorShapes {
            schema_version: v,
            shapes: vec![],
        })
        .expect("serializes"),
    );
    write(
        "api/error_responses.json",
        serde_json::to_value(ErrorResponses {
            schema_version: v,
            budget_bytes: 0,
            total_retained_bytes: 0,
            responses: vec![],
        })
        .expect("serializes"),
    );
    write(
        "diagnostics/rejections.json",
        serde_json::to_value(RejectionsDocument {
            schema_version: v,
            cap_bytes: 0,
            total: 0,
            omitted: 0,
            rejections: vec![],
        })
        .expect("serializes"),
    );
    write(
        "diagnostics/path_map.json",
        serde_json::to_value(PathMapDocument {
            schema_version: v,
            entries: vec![],
        })
        .expect("serializes"),
    );
}

/// Add one patchmap to a minimal bundle, under the file name its id gives.
fn write_patchmap(root: &Path, map: &crate::assemble::PatchMap) {
    let dir = root.join("patchmaps");
    std::fs::create_dir_all(&dir).expect("patchmaps dir");
    std::fs::write(
        dir.join(format!("{}.json", map.capture_id)),
        serde_json::to_vec(map).expect("serializes"),
    )
    .expect("writes the patchmap");
}

/// A directory with a manifest that is not a bundle is not a boundary: the
/// walk descends through it, so a bundle nested beneath a half-written one is
/// still in the inventory. The child is a real, openable bundle to make the
/// result unambiguous.
#[test]
fn a_non_bundle_directory_does_not_hide_a_nested_bundle_from_list() {
    let d = girpr_tempdir::TempDir::new("gircap-list-nested", "nested_found");
    let outer = d.join("outer");
    let nested = outer.join("nested");
    std::fs::create_dir_all(&nested).expect("makes the directories");

    // A malformed manifest: the probe reports NotABundle, which is stepped over
    // and descended into, not a boundary.
    std::fs::write(outer.join("MANIFEST.json"), b"{ not json").expect("writes it");
    // The child opens on its own, so it is a bundle the inventory ought to hold.
    write_minimal_bundle(&nested, "nested_capture");
    let direct = list(&nested).expect("the nested bundle lists on its own");
    assert_eq!(direct.bundles.len(), 1, "{direct:?}");
    assert_eq!(direct.bundles[0].capture_id, "nested_capture");

    // From the root the walk descends through `outer` and finds the child.
    let index = list(&d).expect("lists");
    assert_eq!(index.bundles.len(), 1, "{index:?}");
    assert_eq!(index.bundles[0].path, "outer/nested", "{index:?}");
    assert_eq!(index.bundles[0].capture_id, "nested_capture");
    assert!(
        index.unreadable.is_empty(),
        "a non-bundle directory is not named as unreadable: {index:?}"
    );
}

/// A manifest that parses as JSON but declares no `schema_version` is
/// `NotABundle` too, so the walk descends through it exactly as it does for
/// unparseable JSON.
#[test]
fn a_manifest_without_a_schema_version_does_not_hide_a_nested_bundle() {
    let d = girpr_tempdir::TempDir::new("gircap-list-no-version", "nested_no_version");
    let outer = d.join("outer");
    let nested = outer.join("nested");
    std::fs::create_dir_all(&nested).expect("makes the directories");

    std::fs::write(outer.join("MANIFEST.json"), b"{}").expect("writes it");
    write_minimal_bundle(&nested, "nested_capture");

    let index = list(&d).expect("lists");
    assert_eq!(index.bundles.len(), 1, "{index:?}");
    assert_eq!(index.bundles[0].path, "outer/nested", "{index:?}");
    assert_eq!(index.bundles[0].capture_id, "nested_capture");
    assert!(index.unreadable.is_empty(), "{index:?}");
}

/// A directory that *is* a bundle is a real boundary: the walk lists it and
/// does not descend, so a second valid bundle nested inside it is not listed
/// again.
#[test]
fn a_valid_bundle_stops_descent_into_a_nested_bundle() {
    let d = girpr_tempdir::TempDir::new("gircap-list-boundary", "bundle_boundary");
    write_minimal_bundle(&d, "outer_capture");
    write_minimal_bundle(&d.join("nested"), "nested_capture");

    let index = list(&d).expect("lists");
    assert_eq!(
        index.bundles.len(),
        1,
        "only the outer bundle is listed: {index:?}"
    );
    assert_eq!(index.bundles[0].capture_id, "outer_capture");
    assert_eq!(index.bundles[0].path, ".", "{index:?}");
    assert!(index.unreadable.is_empty(), "{index:?}");
}

/// A manifest declaring a version this build refuses is a boundary even though
/// `Bundle::open` fails: the directory is named as unreadable and the walk does
/// not descend, so a bundle nested beneath it is not listed.
#[test]
fn a_bundle_that_fails_to_open_stops_descent_without_listing_the_nested_bundle() {
    let d = girpr_tempdir::TempDir::new("gircap-list-bad-boundary", "bad_boundary");
    let outer = d.join("outer");
    let nested = outer.join("nested");
    std::fs::create_dir_all(&nested).expect("makes the directories");

    // Version 1 is a readable version, so the probe says Version and the walk
    // treats `outer` as a bundle boundary — one this build refuses to open.
    std::fs::write(outer.join("MANIFEST.json"), br#"{"schema_version": 1}"#).expect("writes it");
    write_minimal_bundle(&nested, "nested_capture");

    let index = list(&d).expect("lists");
    assert!(
        index.bundles.is_empty(),
        "the nested bundle is behind a boundary: {index:?}"
    );
    assert_eq!(index.unreadable.len(), 1, "{index:?}");
    assert_eq!(index.unreadable[0].path, "outer", "{index:?}");
    assert!(
        index.unreadable[0].reason.contains("schema version 1"),
        "{index:?}"
    );
}

/// The manifest version is checked on its own first, but every secondary
/// document is still read back through the type that wrote it. A malformed one
/// is refused by name, which is how `list` comes to name the bundle as
/// unreadable instead of listing it.
#[test]
fn currently_a_malformed_secondary_document_is_refused_at_open() {
    let d = girpr_tempdir::TempDir::new("gircap-malformed-doc", "bad_secondary");
    write_minimal_bundle(&d, "c");
    Bundle::open(&d).expect("opens while every document is well formed");

    std::fs::write(d.join("api/error_shapes.json"), b"{ not json").expect("writes it");
    let e = Bundle::open(&d).unwrap_err();
    assert!(e.contains("error_shapes.json"), "{e}");

    let index = list(&d).expect("lists");
    assert!(index.bundles.is_empty(), "{index:?}");
    assert_eq!(index.unreadable.len(), 1, "{index:?}");
    assert!(
        index.unreadable[0].reason.contains("error_shapes.json"),
        "{index:?}"
    );
}

/// A patchmap's `expectations.result_path` is a bundle-relative path, so a
/// result that points outside the bundle is refused at the door: `verify` is
/// where an operator learns a bundle has been edited, not after a rebuild has
/// written bytes through the field.
#[test]
fn an_escaping_result_path_is_refused_at_open() {
    let d = girpr_tempdir::TempDir::new("gircap-result-path", "escape");
    write_minimal_bundle(&d, "c");
    let mut map = arithmetic_map(&[0]);
    map.expectations.result_path = Some("../escaped/result".into());
    write_patchmap(&d, &map);

    let e = Bundle::open(&d).unwrap_err();
    assert!(e.contains("a20.json"), "the patchmap is named: {e}");
    assert!(e.contains("result_path"), "the field is named: {e}");
    assert!(e.contains("escapes the bundle root"), "{e}");
}

/// Every document this reader requires carries a schema version; a bundle
/// written with all of them current still opens.
#[test]
fn a_bundle_with_all_current_document_versions_opens() {
    let d = girpr_tempdir::TempDir::new("gircap-current-docs", "current");
    write_minimal_bundle(&d, "c");
    let bundle = Bundle::open(&d).expect("opens with every document at the current version");
    assert_eq!(bundle.manifest.schema_version, crate::SCHEMA_VERSION);
}

/// Schema drift is diagnosed at the document that caused it: the file, the
/// version found and the version expected are all in the refusal. Only the
/// version is edited here — no field the current type wants is missing — so
/// the check has to read it before the full document.
#[test]
fn a_wrong_version_in_any_secondary_document_is_named_at_open() {
    for doc in [
        "chunks/index.json",
        "verify/checksums.json",
        "verify/selfcheck.json",
        "api/error_shapes.json",
        "api/error_responses.json",
        "diagnostics/rejections.json",
        "diagnostics/path_map.json",
    ] {
        let d = girpr_tempdir::TempDir::new("gircap-doc-version", "version_drift");
        write_minimal_bundle(&d, "c");
        let path = d.join(doc);
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("reads the document"))
                .expect("parses as JSON");
        value["schema_version"] = serde_json::json!(1);
        std::fs::write(&path, serde_json::to_vec(&value).expect("serializes"))
            .expect("writes the version-only edit");

        let e = Bundle::open(&d).unwrap_err();
        assert!(e.contains(doc), "{doc}: {e}");
        assert!(e.contains("schema version 1"), "{doc}: {e}");
        assert!(
            e.contains(&format!("current {}", crate::SCHEMA_VERSION)),
            "{doc}: {e}"
        );
    }
}

/// A wrong version in one patchmap fails at that file, not at the directory.
/// `Bundle::open` is the door every offline command passes through, so the
/// refusal comes before any command rebuilds a result from the bad map.
#[test]
fn a_wrong_version_in_one_patchmap_names_that_file_before_any_rebuild() {
    let d = girpr_tempdir::TempDir::new("gircap-patchmap-version", "bad_patchmap");
    write_minimal_bundle(&d, "c");
    let mut map = arithmetic_map(&[0]);
    map.schema_version = 1;
    write_patchmap(&d, &map);

    let e = Bundle::open(&d).unwrap_err();
    assert!(e.contains("a20.json"), "the patchmap file is named: {e}");
    assert!(e.contains("schema version 1"), "{e}");
    assert!(
        e.contains(&format!("current {}", crate::SCHEMA_VERSION)),
        "{e}"
    );
}

/// `result_path` carries the `expected/` tree over the game's own path, so it
/// is checked with the logical rule: escapes, a drive prefix and a backslash
/// are refused, while a normal file under `expected/` is accepted.
#[test]
fn result_path_variants_are_checked_at_the_door() {
    for (bad, want) in [
        ("../escape", "escapes the bundle root"),
        ("C:evil", "drive-qualified"),
        ("expected/..", "escapes the bundle root"),
        ("expected\\a.bin", "backslashes"),
    ] {
        let d = girpr_tempdir::TempDir::new("gircap-result-path", "bad_variant");
        write_minimal_bundle(&d, "c");
        let mut map = arithmetic_map(&[0]);
        map.expectations.result_path = Some(bad.into());
        write_patchmap(&d, &map);

        let e = Bundle::open(&d).unwrap_err();
        assert!(e.contains("result_path"), "{bad:?}: {e}");
        assert!(e.contains(want), "{bad:?}: {e}");
    }

    let d = girpr_tempdir::TempDir::new("gircap-result-path", "good_variant");
    write_minimal_bundle(&d, "c");
    let mut map = arithmetic_map(&[0]);
    map.expectations.result_path = Some("expected/a.bin".into());
    write_patchmap(&d, &map);
    let bundle = Bundle::open(&d).expect("a bundle-relative result path opens");
    assert_eq!(
        bundle.patchmaps[&map.capture_id]
            .expectations
            .result_path
            .as_deref(),
        Some("expected/a.bin")
    );
}

/// The tail of `result_path` is the game's own path, so a device component is
/// legitimate there — it is escaped on disk — and must not be refused just
/// because it names a device.
#[test]
fn a_device_component_in_a_result_path_is_accepted() {
    let d = girpr_tempdir::TempDir::new("gircap-result-path", "device");
    write_minimal_bundle(&d, "c");
    let mut map = arithmetic_map(&[0]);
    map.expectations.result_path = Some("expected/Resources/CON".into());
    write_patchmap(&d, &map);
    Bundle::open(&d).expect("a logical device component in the result path opens");
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

/// A patchmap can be arithmetically sound on its own while describing a patch
/// that the bundle's own index cannot produce. The arithmetic check cannot see
/// that; the plan/index cross-check must, and name the chunk that disagrees.
#[test]
fn a_self_consistent_plan_fails_when_the_index_names_another_patch() {
    use crate::assemble::Side;
    use crate::blobs::IndexedBlob;
    use crate::blobs::chunks::{Reference, Set};
    use crate::blobs::props::BlobProps;

    // The plan: totals and expectations are the honest sum of its one FETCH, and
    // must_not_fetch agrees with its (empty) REUSE set.
    let mut map = arithmetic_map(&[10]);
    map.new = Some(Side {
        tag: "7.1.0".into(),
        size: 1,
        md5: "00".repeat(16),
        nchunks: 1,
    });
    let plan = map.plan.as_ref().expect("a plan");
    assert_eq!(plan.len(), 1);
    let planned_id = plan[0].chunk_id.as_str();

    // The index describes a different patch for the same capture and path: one
    // blob, referenced at an offset, whose id is not the one the plan names.
    let indexed_id = "the-index-names-a-different-chunk";
    let index = ChunkIndex::build(vec![IndexedBlob {
        props: BlobProps::unmeasured(&crate::manifest::proto::SophonChunk {
            id: indexed_id.into(),
            uncompressed_md5: "22".repeat(16),
            offset: 0,
            compressed_size: 7,
            uncompressed_size: 1,
            unknown_field6: 0,
            compressed_md5: "33".repeat(16),
        }),
        referenced_by: vec![Reference {
            tag: "7.1.0".into(),
            build_id: Some("new-build".into()),
            capture_id: map.capture_id.clone(),
            path: map.path.clone(),
            role: Set::MaterializeNew,
            offset: 0,
        }],
        sets: vec![Set::MaterializeNew],
    }]);

    // The index's only reference for this capture/path names a chunk the plan
    // does not, so the index cannot produce this plan.
    let indexed: Vec<&str> = index
        .blobs
        .iter()
        .filter(|b| {
            b.referenced_by
                .iter()
                .any(|r| r.capture_id == map.capture_id && r.path == map.path)
        })
        .map(|b| b.props.chunk_id.as_str())
        .collect();
    assert_eq!(indexed, vec![indexed_id]);
    assert!(!indexed.contains(&planned_id), "the two disagree");

    // The arithmetic still agrees with itself, as it must - that is why the
    // two checks are separate.
    assert!(check_patchmap_arithmetic(&map).iter().all(|r| r.ok));
    let by_file = materialize::ChunksByFile::build(&index);
    let rows = check_plan_against_index(&map, &by_file, &index);
    assert!(
        rows.iter().any(|r| !r.ok && r.actual.contains(planned_id)),
        "the plan's chunk id is named as disagreeing with the index: {rows:?}"
    );
}

/// A two-side fixture whose plan and index describe exactly the same patch.
///
/// Old side: `a` at 0, `b` at 64, `c` at 192. New side: `d` at 0 (fetched),
/// `b` at 64 and `c` at 192 (reused from those same old offsets). The blobs are
/// measured so the sizes the plan declares are the sizes the index holds.
fn plan_index_fixture() -> (crate::assemble::PatchMap, ChunkIndex) {
    use crate::assemble::ranges::Range;
    use crate::assemble::{Decision, Expectations, PatchMap, PlanEntry, Side, Totals};
    use crate::blobs::IndexedBlob;
    use crate::blobs::chunks::{Reference, Set};
    use crate::blobs::props::BlobProps;
    use crate::manifest::proto::SophonChunk;

    let chunk = |seed: u8, len: usize, prefix: u64, offset: i64| -> (SophonChunk, Vec<u8>) {
        let body: Vec<u8> = (0..len)
            .map(|i| seed.wrapping_mul(31).wrapping_add(i as u8))
            .collect();
        let raw = zstd::encode_all(body.as_slice(), 1).expect("encode");
        let c = SophonChunk {
            id: format!("{prefix:016x}_{}", hash::md5_hex(&body)),
            uncompressed_md5: hash::md5_hex(&body),
            offset,
            compressed_size: raw.len() as i64,
            uncompressed_size: len as i64,
            unknown_field6: 0,
            compressed_md5: hash::md5_hex(&raw),
        };
        (c, raw)
    };
    let (a, ar) = chunk(1, 64, 0x1111_1111_1111_1111, 0);
    let (b, br) = chunk(2, 128, 0x2222_2222_2222_2222, 64);
    let (c, cr) = chunk(3, 32, 0x3333_3333_3333_3333, 192);
    let (d, dr) = chunk(4, 64, 0x4444_4444_4444_4444, 0);

    let reference = |tag: &str, role: Set, offset: i64| Reference {
        tag: tag.into(),
        build_id: None,
        capture_id: "c".into(),
        path: "dll".into(),
        role,
        offset,
    };
    let entry = |ch: &SophonChunk, raw: Vec<u8>, refs: Vec<Reference>, sets: Vec<Set>| {
        let mut props = BlobProps::measure(&raw, ch).expect("measure");
        props.set_stored(BlobProps::stored_path_for(&ch.id));
        IndexedBlob {
            props,
            referenced_by: refs,
            sets,
        }
    };
    let index = ChunkIndex::build(vec![
        entry(
            &a,
            ar,
            vec![reference("7.0.0", Set::MaterializeOld, 0)],
            vec![Set::MaterializeOld],
        ),
        entry(
            &b,
            br,
            vec![
                reference("7.0.0", Set::MaterializeOld, 64),
                reference("7.1.0", Set::MaterializeNew, 64),
            ],
            vec![Set::MaterializeOld, Set::MaterializeNew],
        ),
        entry(
            &c,
            cr,
            vec![
                reference("7.0.0", Set::MaterializeOld, 192),
                reference("7.1.0", Set::MaterializeNew, 192),
            ],
            vec![Set::MaterializeOld, Set::MaterializeNew],
        ),
        entry(
            &d,
            dr,
            vec![
                reference("7.1.0", Set::MaterializeNew, 0),
                reference("7.1.0", Set::PatchFetch, 0),
            ],
            vec![Set::MaterializeNew, Set::PatchFetch],
        ),
    ]);

    let fetch = PlanEntry {
        i: 0,
        decision: Decision::Fetch,
        target_range: Range::at(0, 64),
        source_range: None,
        chunk_id: d.id.clone(),
        url: Some(format!("https://cdn.test/chunks/{}{}", d.id, "?token=x")),
        compressed_size: d.compressed_size,
        uncompressed_size: d.uncompressed_size,
        uncompressed_md5: d.uncompressed_md5.clone(),
        note: None,
    };
    let reuse = |i, offset: i64, end: i64, ch: &SophonChunk| PlanEntry {
        i,
        decision: Decision::Reuse,
        target_range: Range::at(offset, ch.uncompressed_size),
        source_range: Some(crate::assemble::SourceRange {
            start: offset,
            end,
            file_tag: "7.0.0".into(),
        }),
        chunk_id: ch.id.clone(),
        url: None,
        compressed_size: ch.compressed_size,
        uncompressed_size: ch.uncompressed_size,
        uncompressed_md5: ch.uncompressed_md5.clone(),
        note: None,
    };
    let plan = vec![fetch, reuse(1, 64, 192, &b), reuse(2, 192, 224, &c)];
    let map = PatchMap {
        schema_version: crate::SCHEMA_VERSION,
        capture_id: "c".into(),
        path: "dll".into(),
        matching_field: "game".into(),
        classes: Vec::new(),
        old: Some(Side {
            tag: "7.0.0".into(),
            size: 224,
            md5: "00".repeat(16),
            nchunks: 3,
        }),
        new: Some(Side {
            tag: "7.1.0".into(),
            size: 224,
            md5: "00".repeat(16),
            nchunks: 3,
        }),
        plan: Some(plan),
        totals: Some(Totals {
            chunks: 3,
            reuse: 2,
            fetch: 1,
            moved: 0,
            download_bytes: d.compressed_size,
            materialize_old_bytes: 0,
            materialize_new_bytes: 0,
        }),
        expectations: Expectations {
            result_path: None,
            result_md5: None,
            absent_after: false,
            download_bytes: d.compressed_size,
            must_not_fetch: vec![b.id.clone(), c.id.clone()],
        },
    };
    (map, index)
}

#[test]
fn a_correct_plan_passes_the_index_cross_check() {
    let (map, index) = plan_index_fixture();
    let by_file = materialize::ChunksByFile::build(&index);
    let rows = check_plan_against_index(&map, &by_file, &index);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].ok, "{rows:?}");
}

#[test]
fn a_plan_chunk_id_that_disagrees_with_the_index_fails() {
    let (mut map, index) = plan_index_fixture();
    map.plan.as_mut().unwrap()[0].chunk_id = "a-different-chunk".into();
    let by_file = materialize::ChunksByFile::build(&index);
    let rows = check_plan_against_index(&map, &by_file, &index);
    assert!(
        rows.iter().any(|r| {
            !r.ok && r.subject.contains("[plan 0]") && r.actual.contains("a-different-chunk")
        }),
        "{rows:?}"
    );
}

#[test]
fn a_plan_target_offset_that_disagrees_with_the_index_fails() {
    use crate::assemble::ranges::Range;
    let (mut map, index) = plan_index_fixture();
    // Offset 1 is target to no new-side reference. The index's chunk at 0 is
    // also left out of the plan, so the converse must speak up as well.
    map.plan.as_mut().unwrap()[0].target_range = Range::at(1, 64);
    let by_file = materialize::ChunksByFile::build(&index);
    let rows = check_plan_against_index(&map, &by_file, &index);
    assert!(
        rows.iter().any(|r| !r.ok && r.actual.contains("offset 1")),
        "the invented offset is named: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|r| !r.ok && r.subject.contains("new-side coverage")),
        "and the omitted reference is named: {rows:?}"
    );
}

#[test]
fn a_fetch_with_a_source_range_or_a_reuse_without_one_fails() {
    use crate::assemble::SourceRange;
    use crate::assemble::ranges::Range;
    let (mut map, index) = plan_index_fixture();
    {
        let plan = map.plan.as_mut().unwrap();
        plan[0].source_range = Some(SourceRange {
            start: 0,
            end: 64,
            file_tag: "7.0.0".into(),
        });
        plan[1].source_range = None;
    }
    let by_file = materialize::ChunksByFile::build(&index);
    let rows = check_plan_against_index(&map, &by_file, &index);
    assert!(
        rows.iter().any(|r| {
            !r.ok && r.subject.contains("[plan 0] (source_range)") && r.expected.contains("FETCH")
        }),
        "the FETCH's source range is red: {rows:?}"
    );
    assert!(
        rows.iter().any(|r| {
            !r.ok && r.subject.contains("[plan 1] (source_range)") && r.actual.contains("no source")
        }),
        "the REUSE without one is red: {rows:?}"
    );
    // A wrong decision is caught even when its other fields are tidy.
    {
        let plan = map.plan.as_mut().unwrap();
        plan[0].source_range = None;
        plan[0].decision = crate::assemble::Decision::Reuse;
        plan[0].target_range = Range::at(0, 64);
    }
    let rows = check_plan_against_index(&map, &by_file, &index);
    assert!(
        rows.iter()
            .any(|r| !r.ok && r.actual.contains("patch_fetch reference at 0")),
        "{rows:?}"
    );
}

#[test]
fn a_plan_that_omits_or_invents_a_chunk_fails() {
    let (mut map, index) = plan_index_fixture();
    let by_file = materialize::ChunksByFile::build(&index);

    // Drop the last entry: the index's reference at 192 has no plan entry.
    map.plan.as_mut().unwrap().pop();
    let rows = check_plan_against_index(&map, &by_file, &index);
    assert!(
        rows.iter().any(|r| {
            !r.ok && r.subject.contains("new-side coverage") && r.expected.contains("offset 192")
        }),
        "the omitted reference is named: {rows:?}"
    );

    // Now invent one at an offset no index reference uses.
    let (mut map, index) = plan_index_fixture();
    let mut invented = map.plan.as_ref().unwrap()[0].clone();
    invented.i = 99;
    invented.target_range = crate::assemble::ranges::Range::at(999, 64);
    invented.chunk_id = "a-chunk-the-index-never-places".into();
    map.plan.as_mut().unwrap().push(invented);
    let by_file = materialize::ChunksByFile::build(&index);
    let rows = check_plan_against_index(&map, &by_file, &index);
    assert!(
        rows.iter()
            .any(|r| !r.ok && r.actual.contains("offset 999")),
        "the invented chunk is named: {rows:?}"
    );
}

#[test]
fn two_plan_entries_with_one_target_offset_fail() {
    use crate::assemble::ranges::Range;
    let (mut map, index) = plan_index_fixture();
    {
        let plan = map.plan.as_mut().unwrap();
        plan[2].target_range = Range::at(64, 32);
    }
    let by_file = materialize::ChunksByFile::build(&index);
    let rows = check_plan_against_index(&map, &by_file, &index);
    assert!(
        rows.iter().any(|r| {
            !r.ok && r.subject.contains("target_range.start") && r.actual.contains("plan entry 1")
        }),
        "the second entry is named as the duplicate: {rows:?}"
    );
}

#[test]
fn a_plan_size_that_disagrees_with_the_index_blob_fails() {
    let (mut map, index) = plan_index_fixture();
    map.plan.as_mut().unwrap()[0].compressed_size += 1;
    let by_file = materialize::ChunksByFile::build(&index);
    let rows = check_plan_against_index(&map, &by_file, &index);
    assert!(
        rows.iter()
            .any(|r| !r.ok && r.subject.contains("compressed_size")),
        "{rows:?}"
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
    let index = ChunkIndex::build(vec![crate::blobs::IndexedBlob {
        props,
        referenced_by: Vec::new(),
        sets: Vec::new(),
    }]);
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
    let index = ChunkIndex::build(vec![crate::blobs::IndexedBlob {
        props,
        referenced_by: Vec::new(),
        sets: Vec::new(),
    }]);
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
