//! End to end and entirely offline: build a bundle on disk, then `verify`
//! it, `materialize` it, and `list` it.
//!
//! No test here touches the network. The fixture is written with the crate's own
//! writers and then *read back through the readers*, which is the only way a
//! round-trip test can catch a writer and a reader that disagree about the
//! schema - and a disagreement there is invisible to a test that only ever
//! checks the writer.
//!
//! The bundle is the measured `HoYoChannel.dll` shape: one file, three chunks,
//! the middle one byte-identical across the pair and the first and last replaced.
//! Small enough to reason about, and the only shape in which "reused a chunk from
//! a different offset" is already true.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gircap::assemble::{PatchMap, Side};
use gircap::blobs::chunks::{Reference, Set};
use gircap::blobs::props::BlobProps;
use gircap::blobs::{ChunkIndex, IndexedBlob};
use gircap::emit::index::{Checksums, Manifest, ManifestMatrixRow};
use gircap::emit::selfcheck::{Assertion, Selfcheck};
use gircap::hash;
use gircap::manifest::proto::SophonChunk;
use gircap::verify::materialize;

const DLL: &str = "GenshinImpact_Data/Plugins/HoYoChannel.dll";
const OLD_TAG: &str = "7.0.0";
const NEW_TAG: &str = "7.1.0";
const CAPTURE: &str = "partial_same_offset";

/// A chunk and the bytes it holds.
fn chunk(seed: u8, len: usize, prefix: u64) -> (SophonChunk, Vec<u8>, Vec<u8>) {
    let body: Vec<u8> = (0..len)
        .map(|i| seed.wrapping_mul(31).wrapping_add(i as u8))
        .collect();
    let raw = zstd::encode_all(body.as_slice(), 1).expect("encode");
    let c = SophonChunk {
        id: format!("{prefix:016x}_{}", hash::md5_hex(&body)),
        uncompressed_md5: hash::md5_hex(&body),
        offset: 0,
        compressed_size: raw.len() as i64,
        uncompressed_size: len as i64,
        unknown_field6: 0,
        compressed_md5: hash::md5_hex(&raw),
    };
    (c, body, raw)
}

/// One chunk's role in the bundle.
struct Blob {
    chunk: SophonChunk,
    /// The bytes as served, written to `chunks/blobs/`.
    raw: Vec<u8>,
    refs: Vec<Reference>,
    sets: Vec<Set>,
}

fn reference(tag: &str, path: &str, role: Set, offset: i64) -> Reference {
    Reference {
        tag: tag.into(),
        build_id: Some(
            match tag {
                "7.0.0" => "3XnPviMfoILi",
                _ => "sMXGW2ll3Fuu",
            }
            .into(),
        ),
        capture_id: CAPTURE.into(),
        path: path.into(),
        role,
        offset,
    }
}

/// A complete bundle on disk, with everything removed when it goes out of scope.
///
/// `Drop` rather than an explicit teardown, because the interesting tests are
/// the ones that fail partway through and a fixture that only cleans up on the
/// happy path leaks into the next test.
struct Bundle {
    root: PathBuf,
}

impl Bundle {
    fn build(name: &str) -> Bundle {
        let root = std::env::temp_dir().join(format!(
            "gircap-offline-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");

        // The three chunks of one file, at the offsets the index will record.
        let (a, ab, ar) = chunk(0xa1, 64, 0x7d3f_0504_4383_7e58);
        let (b, bb, br) = chunk(0xb2, 128, 0xf5dc_3ca1_f3a0_ba3d);
        let (c, cb, cr) = chunk(0xc3, 32, 0xa431_4f3c_1ba1_d7d4);
        let blobs = vec![
            // Offset 0 is different content in each build: same length, new id.
            Blob {
                chunk: a.clone(),
                raw: ar,
                refs: vec![reference(OLD_TAG, DLL, Set::MaterializeOld, 0)],
                sets: vec![Set::MaterializeOld],
            },
            // Offset 690719 - here 64 - is byte-identical in both.
            Blob {
                chunk: b.clone(),
                raw: br,
                refs: vec![
                    reference(OLD_TAG, DLL, Set::MaterializeOld, 64),
                    reference(NEW_TAG, DLL, Set::MaterializeNew, 64),
                    reference(NEW_TAG, DLL, Set::PatchFetch, 64),
                ],
                sets: vec![Set::MaterializeOld, Set::MaterializeNew, Set::PatchFetch],
            },
            Blob {
                chunk: c.clone(),
                raw: cr,
                refs: vec![
                    reference(OLD_TAG, DLL, Set::MaterializeOld, 192),
                    reference(NEW_TAG, DLL, Set::MaterializeNew, 192),
                ],
                sets: vec![Set::MaterializeOld, Set::MaterializeNew],
            },
        ];
        // The new content for offset 0, which only the new side references.
        let (d, db, dr) = chunk(0xd4, 64, 0xddb3_1a9f_d853_46c4);
        let mut blobs = blobs;
        blobs.push(Blob {
            chunk: d.clone(),
            raw: dr,
            refs: vec![
                reference(NEW_TAG, DLL, Set::MaterializeNew, 0),
                reference(NEW_TAG, DLL, Set::PatchFetch, 0),
            ],
            sets: vec![Set::MaterializeNew, Set::PatchFetch],
        });

        let old_bytes: Vec<u8> = [ab.as_slice(), bb.as_slice(), cb.as_slice()].concat();
        let new_bytes: Vec<u8> = [db.as_slice(), bb.as_slice(), cb.as_slice()].concat();
        let old_md5 = hash::md5_hex(&old_bytes);
        let new_md5 = hash::md5_hex(&new_bytes);

        // `chunks/` - the blobs and the index that describes them.
        let mut entries = Vec::new();
        for b in &blobs {
            let rel = BlobProps::stored_path_for(&b.chunk.id);
            let p = root.join(&rel);
            std::fs::create_dir_all(p.parent().unwrap()).expect("blob dir");
            std::fs::write(&p, &b.raw).expect("blob");
            let mut props = BlobProps::measure(&b.raw, &b.chunk).expect("measure");
            props.set_stored(rel);
            entries.push((
                IndexedBlob {
                    props,
                    referenced_by: b.refs.clone(),
                    sets: b.sets.clone(),
                },
                b.sets.clone(),
            ));
        }
        let index = ChunkIndex::build(entries);
        write_json(&root.join("chunks/index.json"), &index);

        // The two trees the blobs are supposed to reproduce.
        write_bytes(&root.join(format!("game/{DLL}")), &old_bytes);
        write_bytes(&root.join(format!("expected/{DLL}")), &new_bytes);
        // `config.ini` is written by the tool, not derived from a manifest, and
        // A13 re-reads it - so it has to be here for the check to mean anything.
        // Written with the tool's own writer, because a fixture that hand-rolls
        // the format would pass A13 even if the writer changed.
        write_bytes(
            &root.join("game/config.ini"),
            gircap::emit::gamedir::config_ini(&biz(), OLD_TAG).as_bytes(),
        );

        // `patchmaps/` - the plan and the ground truth.
        //
        // The plan is spelled out, never left null: a capture with a new side must
        // say what producing it costs, which is exactly what A20 checks. Three
        // new-side chunks: the first is new content (FETCH), the other two are in
        // the old file at the same offsets (REUSE, neither moved).
        use gircap::assemble::ranges::Range as PRange;
        use gircap::assemble::{Decision, PlanEntry, SourceRange, Totals};
        let entry = |i: usize, c: &SophonChunk, offset: i64, reuse_from: Option<i64>| PlanEntry {
            i,
            decision: if reuse_from.is_some() {
                Decision::Reuse
            } else {
                Decision::Fetch
            },
            target_range: PRange::at(offset, c.uncompressed_size),
            source_range: reuse_from.map(|s| SourceRange {
                start: s,
                end: s + c.uncompressed_size,
                file_tag: OLD_TAG.into(),
            }),
            chunk_id: c.id.clone(),
            url: None,
            compressed_size: c.compressed_size,
            uncompressed_size: c.uncompressed_size,
            uncompressed_md5: c.uncompressed_md5.clone(),
            note: None,
        };
        let plan = vec![
            entry(0, &d, 0, None),
            entry(1, &b, 64, Some(64)),
            entry(2, &c, 192, Some(192)),
        ];
        let fetch_bytes: i64 = plan
            .iter()
            .filter(|e| e.decision == Decision::Fetch)
            .map(|e| e.compressed_size)
            .sum();
        let totals = Totals {
            chunks: plan.len(),
            reuse: 2,
            fetch: 1,
            moved: 0,
            download_bytes: fetch_bytes,
            materialize_old_bytes: old_bytes.len() as i64,
            materialize_new_bytes: new_bytes.len() as i64,
        };
        let mut maps = BTreeMap::new();
        maps.insert(
            CAPTURE.to_string(),
            PatchMap {
                schema_version: gircap::SCHEMA_VERSION,
                capture_id: CAPTURE.into(),
                path: DLL.into(),
                matching_field: "game".into(),
                classes: vec![gircap::select::Class::PartialSameOffset],
                old: Some(Side {
                    tag: OLD_TAG.into(),
                    size: old_bytes.len() as i64,
                    md5: old_md5,
                    nchunks: 3,
                }),
                new: Some(Side {
                    tag: NEW_TAG.into(),
                    size: new_bytes.len() as i64,
                    md5: new_md5.clone(),
                    nchunks: 3,
                }),
                plan: Some(plan),
                totals: Some(totals),
                expectations: gircap::assemble::Expectations {
                    result_path: Some(format!("expected/{DLL}")),
                    result_md5: Some(new_md5.clone()),
                    absent_after: false,
                    download_bytes: fetch_bytes,
                    must_not_fetch: vec![b.id.clone(), c.id.clone()],
                },
            },
        );
        std::fs::create_dir_all(root.join("patchmaps")).expect("patchmaps dir");
        for (id, m) in &maps {
            write_json(&root.join(format!("patchmaps/{id}.json")), m);
        }

        // `api/error_shapes.json`. **Required**, and this fixture had to grow it:
        // it was building a bundle that no capture could produce, and `verify`
        // said so. A hand-built fixture that is easier than the real thing will
        // always find a way to be easier.
        write_json(
            &root.join("api/error_shapes.json"),
            &error_shapes(),
        );

        // `MANIFEST.json`, with one row per required class.
        let manifest = manifest();
        write_json(&root.join("MANIFEST.json"), &manifest);

        // `verify/selfcheck.json`, then `verify/checksums.json` last - the same
        // two-pass order `capture` uses, and for the same reason.
        let recorded = Selfcheck::build(
            vec![Assertion::pass(
                "A6",
                "every blob checks out",
                "chunks/",
                "ok",
            )],
            Vec::new(),
            &[],
        );
        write_json(&root.join("verify/selfcheck.json"), &recorded);
        let checksums = Checksums::scan(&root).expect("scan");
        write_json(&root.join("verify/checksums.json"), &checksums);

        Bundle { root }
    }

    /// Flip one byte inside a stored blob's frame, leaving the magic intact.
    fn corrupt_one_blob_byte(&self) -> String {
        let index: ChunkIndex = read_json(&self.root.join("chunks/index.json"));
        let b = index
            .blobs
            .iter()
            .find(|b| b.props.stored_path.is_some())
            .expect("a stored blob");
        let rel = b.props.stored_path.clone().unwrap();
        let p = self.root.join(&rel);
        let mut raw = std::fs::read(&p).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xFF;
        std::fs::write(&p, raw).unwrap();
        // Re-checksum so the failure a reader meets is the blob's, not the list's:
        // A16 firing first would hide what actually broke.
        let c = Checksums::scan(&self.root).expect("scan");
        write_json(&self.root.join("verify/checksums.json"), &c);
        rel
    }
}

impl Drop for Bundle {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A manifest naming this pair, with every class the matrix must carry.
fn manifest() -> Manifest {
    let rows = [
        ("identical", "unavailable", "identical"),
        ("zero_reuse_single", "unavailable", "zero_reuse_single"),
        ("zero_reuse_multi", "unavailable", "zero_reuse_multi"),
        ("partial_same_offset", "captured", "partial_same_offset"),
        (
            "partial_moved_offset",
            "unavailable",
            "partial_moved_offset",
        ),
        (
            "partial_size_changed",
            "unavailable",
            "partial_size_changed",
        ),
        ("added", "unavailable", "added"),
        ("removed", "unavailable", "removed"),
        (
            "full_reuse_reordered",
            "unavailable",
            "full_reuse_reordered",
        ),
    ];
    Manifest {
        schema_version: gircap::SCHEMA_VERSION,
        tool: gircap::emit::index::Tool {
            name: "gircap".into(),
            version: "0.1.0".into(),
        },
        capture: gircap::emit::index::Capture {
            id: "20261014T000000Z".into(),
            captured_at: "2026-10-14T00:00:00Z".into(),
            host_os: "test".into(),
            tool_argv: vec!["gircap".into()],
            complete: true,
        },
        biz: gircap::emit::index::Biz_ {
            id: "hk4e_global".into(),
            launcher_id: "VYTpXlbWo8".into(),
            game_id: "gopR6Cufr3".into(),
            channel: "1".into(),
            sub_channel: "0".into(),
            cps: "none".into(),
            hyp_base: "http://127.0.0.1:1/hyp".into(),
            sophon_base: "http://127.0.0.1:1/sophon".into(),
        },
        versions: gircap::emit::index::Versions {
            new: build(NEW_TAG, "sMXGW2ll3Fuu"),
            old: build(OLD_TAG, "3XnPviMfoILi"),
            diff_tags: vec![OLD_TAG.into()],
        },
        field: "game".into(),
        audio: Vec::new(),
        gzip_threshold: 1_048_576,
        selection_pool: gircap::emit::index::SelectionPool {
            fields: vec!["game".into()],
            manifest_entries: BTreeMap::from([("game".to_string(), 2744)]),
            note: "the pool is game plus any --audio codes".into(),
        },
        totals: gircap::emit::index::Totals {
            files_captured: 1,
            chunks_blobs: 4,
            bytes_downloaded: 0,
            bytes_on_disk: 0,
        },
        matrix: rows
            .iter()
            .map(|(class, status, _)| ManifestMatrixRow {
                class: (*class).into(),
                status: (*status).into(),
                capture_id: None,
                path: Some(DLL.into()),
                matching_field: Some("game".into()),
                covered_by: None,
                also_satisfies: None,
                reason: Some("no candidate in the pool".into()),
                candidates_considered: Some(0),
            })
            .collect(),
        files: Vec::new(),
        totals_by_set: BTreeMap::new(),
    }
}

fn build(tag: &str, id: &str) -> gircap::emit::index::Build {
    gircap::emit::index::Build {
        tag: tag.into(),
        build_id: id.into(),
        source: "measured".into(),
    }
}

/// The one required document this fixture has no measurement behind.
///
/// It is synthesised rather than probed, because probing needs a live API and
/// this fixture is the point of being offline. What it has to be is **valid** —
/// the same type `capture` writes, with the field shapes a reader relies on —
/// because the requirement under test is "the document is there and readable",
/// not "the numbers are right". The numbers are `capture_layout`'s job, against
/// a real probe.
fn error_shapes() -> gircap::http::probe::ErrorShapes {
    use gircap::http::probe::ErrorShape;
    gircap::http::probe::ErrorShapes {
        schema_version: gircap::SCHEMA_VERSION,
        shapes: vec![ErrorShape {
            id: "chunk_missing".into(),
            family: gircap::http::probe::Family::Blob,
            request: "a chunk id the build does not have".into(),
            url: "https://cdn.example/chunks/0000000000000000_0".into(),
            status: 404,
            content_type: Some("application/xml".into()),
            retcode: None,
            error: true,
            body: "<?xml version=\"1.0\"?><Error><Code>NoSuchKey</Code></Error>".into(),
            volatile_fields: vec!["RequestId".into(), "HostId".into(), "Key".into()],
            retryable: false,
        }],
    }
}

/// The region, resolved the way `capture` resolves it from `--biz hk4e_global`.
fn biz() -> gircap::biz::Biz {
    gircap::biz::Biz::resolve(gircap::cli::Biz::Hk4eGlobal)
}

fn write_json<T: serde::Serialize>(path: &Path, v: &T) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).expect("parent");
    }
    let body = serde_json::to_vec_pretty(v).expect("serialize");
    std::fs::write(path, body).expect("write json");
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> T {
    let raw = std::fs::read(path).expect("read");
    serde_json::from_slice(&raw).expect("parse")
}

fn write_bytes(path: &Path, bytes: &[u8]) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).expect("parent");
    }
    std::fs::write(path, bytes).expect("write");
}

fn reverify(root: &Path) -> gircap::verify::Report {
    let b = gircap::verify::Bundle::open(root).expect("the bundle opens");
    gircap::verify::reverify(b)
}

// --------------------------------------------------------------- acceptance

/// `verify` on a good artifact exits 0.
#[test]
fn verify_passes_a_good_bundle() {
    let b = Bundle::build("good");
    let r = reverify(&b.root);
    let (total, passed, failed) = r.counts();
    assert_eq!(failed, 0, "failures: {:?}", r.failures());
    assert!(r.closure.is_none(), "{:?}", r.closure);
    assert!(r.agrees, "the recomputation disagreed with the recording");
    assert_eq!(r.result(), "pass");
    // A6 is one row per blob, A7/A8/A11/A18 one per side, so the count is a
    // function of the fixture and cannot drift without something changing.
    assert_eq!(total, passed);
    let by = r.by_id();
    assert_eq!(by["A6"], 4, "one row per stored blob");
    assert_eq!(by["A7"], 1, "one row per old side");
    assert_eq!(by["A8"], 1, "one row per new side");
    assert_eq!(by["A11"], 2, "one row per side");
    assert_eq!(by["A18"], 2, "one row per side");
    assert_eq!(by["A13"], 1);
    assert_eq!(by["A16"], 1);
    assert_eq!(by["A17"], 1);
    assert!(!r.findings.is_empty() || r.findings.is_empty());
}

/// After corrupting one byte of one chunk blob, `verify` reports `3` and
/// names the blob and the assertion.
#[test]
fn verify_reports_a_damaged_blob_by_name() {
    let b = Bundle::build("damaged");
    let rel = b.corrupt_one_blob_byte();
    let r = reverify(&b.root);
    assert_eq!(r.result(), "fail");
    let head = r.headline().expect("a headline for a failed bundle");
    assert!(head.contains(&rel), "names the blob: {head}");
    assert!(
        r.rows
            .iter()
            .any(|x| x.id == "A6" && !x.ok && x.subject == rel),
        "has a red A6 row for the blob: {:?}",
        r.failures()
    );
    // A16 was re-checksummed, so it must *not* be what fires - otherwise the
    // bundle's digest list would be a moving target and A6 would never be read.
    assert!(
        r.rows.iter().any(|x| x.id == "A16" && x.ok),
        "A16 holds: the list is current"
    );
    assert!(
        !r.agrees,
        "a bundle that changed under its own recording does not agree"
    );
}

/// A blob deleted outright is a different fault from a damaged one, and says so.
#[test]
fn verify_reports_a_deleted_blob_by_name() {
    let b = Bundle::build("deleted");
    let index: ChunkIndex = read_json(&b.root.join("chunks/index.json"));
    let rel = index.blobs[0].props.stored_path.clone().unwrap();
    std::fs::remove_file(b.root.join(&rel)).unwrap();
    let r = reverify(&b.root);
    assert_eq!(r.result(), "fail");
    let a6 = r
        .rows
        .iter()
        .find(|x| x.id == "A6" && !x.ok)
        .expect("a red A6");
    assert_eq!(a6.subject, rel);
    assert!(a6.actual.contains("unreadable"), "{}", a6.actual);
    assert!(r.closure.is_some(), "and the closure failed too");
}

/// `materialize` rebuilds both files from `chunks/` and matches `game/` and
/// `expected/`.
#[test]
fn materialize_rebuilds_both_files_byte_for_byte() {
    let b = Bundle::build("materialize");
    let r = reverify(&b.root);
    let pair = r.rebuilt.first().expect("one capture rebuilt");
    assert_eq!(
        pair.old.as_ref().unwrap().bytes,
        std::fs::read(b.root.join(format!("game/{DLL}"))).unwrap()
    );
    assert_eq!(
        pair.new.as_ref().unwrap().bytes,
        std::fs::read(b.root.join(format!("expected/{DLL}"))).unwrap()
    );
}

/// `materialize` writes beside the bundle by default, and writing inside it is
/// refused loudly - because doing so would break A16 on the next `verify`.
#[test]
fn materialize_defaults_beside_the_bundle() {
    let b = Bundle::build("beside");
    let out = b.root.with_file_name(format!(
        "{}-materialized",
        b.root.file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&out);
    let plan = gircap::config::MaterializePlan {
        artifact: b.root.clone(),
        out: None,
    };
    let (summary, code) = block_on(gircap::run::materialize(plan)).expect("materialize runs");
    assert_eq!(code, gircap::exit::ExitCode::Ok, "{:?}", summary);
    assert!(
        out.join(format!("game/{DLL}")).is_file(),
        "wrote beside the bundle"
    );
    assert!(out.join(format!("expected/{DLL}")).is_file());
    assert!(!b.root.join("materialized").exists(), "and not inside it");
    let _ = std::fs::remove_dir_all(&out);
}

/// `list` finds a bundle by walking, not by knowing the layout.
#[test]
fn list_finds_a_bundle_from_the_root() {
    let b = Bundle::build("list");
    // A sibling that is not a bundle must be stepped over, not refused.
    let sibling = b.root.with_file_name("not-a-bundle");
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("readme.txt"), b"nothing here").unwrap();

    let found = gircap::verify::list(b.root.parent().unwrap()).expect("lists");
    let hit = found.iter().find(|f| {
        f.path
            .ends_with(b.root.file_name().unwrap().to_string_lossy().as_ref())
    });
    let hit = hit.unwrap_or_else(|| panic!("the bundle was not found in {found:#?}"));
    assert_eq!(hit.capture_id, "20261014T000000Z");
    assert_eq!(hit.tags, format!("{OLD_TAG} -> {NEW_TAG}"));
    assert_eq!(hit.patchmaps, 1);
    assert_eq!(hit.blobs, 4);
    assert_eq!(hit.selfcheck, "pass");
    assert!(hit.complete);
    let _ = std::fs::remove_dir_all(&sibling);
}

/// A directory that *looks* like a bundle - it has a `MANIFEST.json` - but is
/// truncated is skipped, not fatal. `list` runs over trees that may contain a
/// capture in progress.
#[test]
fn a_bundle_that_does_not_open_is_skipped_rather_than_fatal() {
    let b = Bundle::build("skip");
    let parent = b.root.parent().unwrap();
    let broken = parent.join("broken-bundle");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("MANIFEST.json"), b"{ not json").unwrap();
    let found = gircap::verify::list(parent).expect("lists");
    assert!(found.iter().all(|f| !f.path.contains("broken-bundle")));
    assert!(
        found.iter().any(|f| f.capture_id == "20261014T000000Z"),
        "the good one is still there"
    );
    let _ = std::fs::remove_dir_all(&broken);
}

// ------------------------------------------------------------------ plumbing

/// Deriving `Deserialize` on every bundle document was necessary for `verify` to
/// read them back. The risk it carries is the opposite direction: that the wire
/// format silently shifted while the reader and writer were being brought into
/// agreement. This pins it — what this build writes for a document it read must
/// be byte-identical to what it read.
///
/// Through the *typed* round-trip, not through `serde_json::Value`: a `Value` is
/// a `BTreeMap`, so it re-serializes in sorted key order and would report a
/// difference that is really only a reordering. Struct field order is part of
/// what a human-readable document is.
#[test]
fn reading_and_writing_a_document_back_is_byte_identical() {
    let b = Bundle::build("roundtrip");
    round_trip::<Manifest>(&b.root.join("MANIFEST.json"));
    round_trip::<ChunkIndex>(&b.root.join("chunks/index.json"));
    round_trip::<Checksums>(&b.root.join("verify/checksums.json"));
    round_trip::<Selfcheck>(&b.root.join("verify/selfcheck.json"));
    // `error_shapes.json` is here because it is now **required** — `verify`
    // reads it through the type that wrote it, so a `Serialize`-only struct would
    // have been a compile error rather than a discovery. The round-trip is what
    // says the two halves still agree on the wire format.
    round_trip::<gircap::http::probe::ErrorShapes>(&b.root.join("api/error_shapes.json"));
    round_trip::<PatchMap>(&b.root.join(format!("patchmaps/{CAPTURE}.json")));
}

/// Read a document into its type and write it back; require the same bytes.
fn round_trip<T: serde::de::DeserializeOwned + serde::Serialize>(path: &Path) {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let on_disk = std::fs::read(path).unwrap();
    let doc: T = serde_json::from_slice(&on_disk).unwrap();
    let again = serde_json::to_vec_pretty(&doc).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&on_disk).trim_end(),
        String::from_utf8_lossy(&again).trim_end(),
        "{name} changed shape"
    );
}

/// `run::*` is `async` for `capture`'s sake; nothing here awaits anything, so a
/// poll-once executor is enough and keeps the test free of a runtime dependency.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    use std::task::{Context, Poll};
    let mut cx = Context::from_waker(std::task::Waker::noop());
    let mut f = std::pin::pin!(f);
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::hint::spin_loop(),
        }
    }
}

/// The rebuild helper is public because `materialize` uses it; this pins the
/// one thing a caller is most likely to get wrong - that the two sides differ.
#[test]
fn the_two_sides_of_the_fixture_really_do_differ() {
    let b = Bundle::build("sides");
    let r = reverify(&b.root);
    let p = &r.rebuilt[0];
    let o = &p.old.as_ref().unwrap().bytes;
    let n = &p.new.as_ref().unwrap().bytes;
    assert_eq!(
        o.len(),
        n.len(),
        "the fixture only varies content, not size"
    );
    assert_ne!(o, n, "offset 0 holds different content on each side");
    assert_eq!(&o[64..192], &n[64..192], "and the middle chunk is shared");
    // The rebuild must agree with what `materialize::rebuild` says, not with
    // itself: a second implementation of the same wrong thing is not a check.
    let index: ChunkIndex = read_json(&b.root.join("chunks/index.json"));
    let direct = materialize::rebuild(
        &materialize::ChunksByFile::build(&index),
        &gircap::assemble::ChunkSource::from_index(&b.root, &index),
        OLD_TAG,
        CAPTURE,
        DLL,
        Set::MaterializeOld,
    )
    .expect("rebuilds");
    assert_eq!(&direct.bytes, o);
}
