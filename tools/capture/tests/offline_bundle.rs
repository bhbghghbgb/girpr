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
use std::path::Path;

use gircap::assemble::{PatchMap, Side};
use gircap::blobs::chunks::{Reference, Set};
use gircap::blobs::props::BlobProps;
use gircap::blobs::{ChunkIndex, IndexedBlob};
use gircap::emit::index::{Checksums, Manifest, ManifestMatrixRow};
use gircap::emit::selfcheck::{Assertion, Selfcheck};
use gircap::hash;
use gircap::manifest::proto::SophonChunk;
use gircap::verify::materialize;
use girpr_tempdir::TempDir;

const DLL: &str = "GenshinImpact_Data/Plugins/HoYoChannel.dll";
const OLD_TAG: &str = "7.0.0";
const NEW_TAG: &str = "7.1.0";
const CAPTURE: &str = "reuse_in_place";
/// The business the bundle belongs to: the first level of the tree `list` walks.
const BIZ: &str = "hk4e_global";

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
/// The guard rather than an explicit teardown, because the interesting tests are
/// the ones that fail partway through and a fixture that only cleans up on the
/// happy path leaks into the next test.
///
/// The bundle does **not** sit at the root of the scratch tree. It sits where a
/// real capture puts it — `<scratch>/artifacts/<biz>/<old>__<new>/<capture_id>/`
/// — so that `list`, which is a *walk*, is pointed at a tree the test owns
/// (`<scratch>/artifacts`) rather than at a directory that belongs to the whole
/// machine. A walk rooted at the system temp directory is the system temp
/// directory's cost to pay, and it grows with every other program on it.
struct Bundle {
    /// The scratch tree that owns everything this test made.
    scratch: TempDir,
    /// The tree `list` is pointed at: `<scratch>/artifacts`.
    artifacts: std::path::PathBuf,
    /// The bundle itself: `<scratch>/artifacts/<biz>/<old>__<new>/<capture_id>`.
    root: std::path::PathBuf,
}

impl Bundle {
    fn build(name: &str) -> Bundle {
        let scratch = TempDir::new("gircap-offline", name);
        // The layout a capture really produces, one level below the root the
        // walk is given: a tree of pairs, each pair holding a capture.
        let artifacts = scratch.join("artifacts");
        let root = scratch.subdir(&format!(
            "artifacts/{}/{OLD_TAG}__{NEW_TAG}/20261014T000000Z",
            BIZ
        ));

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
                classes: vec![gircap::select::Class::ReuseInPlace],
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
        write_json(&root.join("api/error_shapes.json"), &error_shapes());

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

        Bundle {
            scratch,
            artifacts,
            root,
        }
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

/// A manifest naming this pair, with every class the matrix must carry.
fn manifest() -> Manifest {
    let rows = [
        ("identical", "unavailable", "identical"),
        ("no_reuse", "unavailable", "no_reuse"),
        ("reuse_in_place", "captured", "reuse_in_place"),
        ("reuse_moved", "unavailable", "reuse_moved"),
        ("added", "unavailable", "added"),
        ("removed", "unavailable", "removed"),
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
        max_decompressed_bytes: gircap::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
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

/// **O2.** `verify` is the command for a bundle someone else produced, so an
/// incomplete matrix must be a red A17 rather than a clean pass. The document is
/// mutated after `Bundle::open` so the test exercises the re-derivation path
/// without also making A16 report the edited file; the point is that A17 sees
/// the claim, not that the checksum catches the edit.
#[test]
fn verify_surfaces_an_incomplete_matrix_as_a_failed_a17() {
    let b = Bundle::build("a17-incomplete");
    let mut bundle = gircap::verify::Bundle::open(&b.root).expect("the bundle opens");
    bundle.manifest.matrix.retain(|r| r.class != "added");
    // The fixture records only A6; a real capture also records A17 as holding,
    // which is the recording `verify` must contradict once the matrix is short.
    bundle.recorded.assertions.push(Assertion::pass(
        "A17",
        "every required class is present",
        "MANIFEST.json matrix",
        "6 classes",
    ));
    let r = gircap::verify::reverify(bundle);
    assert_eq!(r.result(), "fail");
    let a17 = r
        .rows
        .iter()
        .find(|x| x.id == "A17")
        .expect("A17 is re-derived");
    assert!(!a17.ok, "A17 must fail: {:?}", a17);
    assert!(a17.actual.contains("added"), "{}", a17.actual);
    assert!(
        !r.agrees,
        "the capture recorded A17 as holding; re-derivation must disagree"
    );
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
    // The tool picks this path itself when `--out` is absent, so the assertion
    // can only be made against it. It is a *sibling* of the bundle root, hence a
    // guard of its own: the bundle's guard does not own it.
    let out = TempDir::at(b.root.with_file_name(format!(
        "{}-materialized",
        b.root.file_name().unwrap().to_string_lossy()
    )));
    let plan = gircap::config::MaterializePlan {
        artifact: b.root.to_path_buf(),
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
    // "Beside the bundle" is a sibling of the bundle *root*, and the bundle
    // root is inside the tree this test owns — so the rebuild lands there too,
    // and not in whatever directory the scratch tree happens to sit in.
    assert!(
        out.starts_with(b.scratch.as_ref()),
        "the rebuild is inside the tree the test owns: {}",
        out.display()
    );
}

/// A crafted index whose `stored_path` leaves the root is refused when the
/// bundle is opened, not measured as an A6 fact.
///
/// Without the check in `Bundle::open`, `check_blobs` joins the field and
/// publishes the length, md5 and sha-256 of any file the operator can read. The
/// assertion is on the open, because that is the boundary an operator reaches
/// before a byte is read.
#[test]
fn a_bundle_whose_stored_path_leaves_the_root_is_refused() {
    let b = Bundle::build("escaping-stored-path");
    let index_path = b.root.join("chunks/index.json");
    let mut index: ChunkIndex = read_json(&index_path);
    index.blobs[0].props.stored_path = Some("../../../outside.zst".into());
    write_json(&index_path, &index);

    let e = gircap::verify::Bundle::open(&b.root).unwrap_err();
    assert!(e.contains("stored_path"), "{e}");
    assert!(e.contains("outside.zst"), "{e}");

    // And the command refuses at exit `1`, rather than reporting a measured
    // escape as an assertion failure.
    let plan = gircap::config::VerifyPlan {
        artifact: b.root.clone(),
    };
    let failure = block_on(gircap::run::verify(plan)).expect_err("verify refuses the bundle");
    assert_eq!(failure.code, gircap::exit::ExitCode::Usage);
    assert!(failure.detail.contains("stored_path"), "{failure}");
}

/// **K1.** A crafted index whose offset cannot be added to its blob's size is
/// refused when the bundle is opened, before any command reaches the layout
/// arithmetic that would have aborted on it in debug.
#[test]
fn a_bundle_whose_index_offset_overflows_is_refused() {
    let b = Bundle::build("overflowing-index-offset");
    let index_path = b.root.join("chunks/index.json");
    let mut index: ChunkIndex = read_json(&index_path);
    let blob = &mut index.blobs[0];
    let id = blob.props.chunk_id.clone();
    blob.referenced_by[0].offset = i64::MAX;
    write_json(&index_path, &index);

    let e = gircap::verify::Bundle::open(&b.root).unwrap_err();
    assert!(e.contains(&id), "names the blob: {e}");
    assert!(
        e.contains(&i64::MAX.to_string()) && e.contains("overflows i64"),
        "names the value and the arithmetic: {e}"
    );

    let plan = gircap::config::VerifyPlan {
        artifact: b.root.clone(),
    };
    let failure = block_on(gircap::run::verify(plan)).expect_err("verify refuses the bundle");
    assert_eq!(failure.code, gircap::exit::ExitCode::Usage);
    assert!(failure.detail.contains("overflows i64"), "{failure}");
}

/// `materialize` on a crafted patchmap path writes nothing outside `--out`.
///
/// The index's `referenced_by[].path` is moved with the patchmap so the blob
/// closure still resolves; otherwise the rebuild would fail for an unrelated
/// reason and the destination join would never run. The `out` tree is nested so
/// `../../../escaped.dll` lands in the scratch root the test owns rather than in
/// the machine's temp directory.
#[test]
fn materialize_refuses_a_patchmap_path_that_leaves_the_output() {
    let b = Bundle::build("escaping-patchmap");
    let escape = "../../../escaped.dll";

    let index_path = b.root.join("chunks/index.json");
    let mut index: ChunkIndex = read_json(&index_path);
    for blob in &mut index.blobs {
        for reference in &mut blob.referenced_by {
            reference.path = escape.into();
        }
    }
    write_json(&index_path, &index);

    let map_path = b.root.join(format!("patchmaps/{CAPTURE}.json"));
    let mut map: PatchMap = read_json(&map_path);
    map.path = escape.into();
    write_json(&map_path, &map);

    let out = b.scratch.join("nested/out");
    let plan = gircap::config::MaterializePlan {
        artifact: b.root.clone(),
        out: Some(out.clone()),
    };
    let failure =
        block_on(gircap::run::materialize(plan)).expect_err("materialize refuses the bundle");
    assert_eq!(failure.code, gircap::exit::ExitCode::Usage);
    assert!(failure.detail.contains("path"), "{failure}");

    let escaped = b.scratch.join("escaped.dll");
    assert!(
        !escaped.exists(),
        "the rebuild escaped --out: {}",
        escaped.display()
    );
}

/// `MANIFEST.json`'s `capture.id` names the default output when the artifact is
/// a filesystem root, so it is a file-name component and not free text.
#[test]
fn a_bundle_whose_capture_id_is_not_one_file_name_component_is_refused() {
    let b = Bundle::build("escaping-capture-id");
    let manifest_path = b.root.join("MANIFEST.json");
    let mut manifest: Manifest = read_json(&manifest_path);
    manifest.capture.id = "../../escape".into();
    write_json(&manifest_path, &manifest);

    let e = gircap::verify::Bundle::open(&b.root).unwrap_err();
    assert!(e.contains("capture.id"), "{e}");
    assert!(e.contains("escape"), "{e}");
}

/// Two `patchmaps/*.json` that declare one `capture_id` are refused, not
/// silently collapsed.
///
/// The map is keyed by the id the document declares, so the second `insert` used
/// to replace the first: `verify` rebuilt one capture, never examined the other
/// capture's blobs, and exited `0` on a bundle that had already lost a capture.
/// The filenames are deliberately different, because the identity is the field.
#[test]
fn two_patchmaps_declaring_one_capture_id_are_refused() {
    let b = Bundle::build("duplicate-patchmap-id");
    let original = b.root.join(format!("patchmaps/{CAPTURE}.json"));
    let map: PatchMap = read_json(&original);
    write_json(&b.root.join("patchmaps/added+1.json"), &map);

    let e = gircap::verify::Bundle::open(&b.root).unwrap_err();
    assert!(e.contains("two patchmaps"), "{e}");
    assert!(e.contains("declare capture_id"), "{e}");
    assert!(e.contains(CAPTURE), "the repeated id is named: {e}");
    assert!(
        e.contains(&format!("{CAPTURE}.json")) && e.contains("added+1.json"),
        "both files are named, so a reader can find them: {e}"
    );

    let plan = gircap::config::VerifyPlan {
        artifact: b.root.clone(),
    };
    let failure = block_on(gircap::run::verify(plan)).expect_err("verify refuses the bundle");
    assert_eq!(failure.code, gircap::exit::ExitCode::Usage);
    assert!(failure.detail.contains("two patchmaps"), "{failure}");
}

/// `list` finds a bundle by walking, not by knowing the layout.
///
/// The walk is rooted at **the tree the test owns** — `<scratch>/artifacts` —
/// and not at the directory the scratch tree happens to sit in. `list` is a
/// recursive walk: rooted at the system temp directory it would descend into
/// every other program's leftovers, and the cost of the test would be the
/// machine's cost rather than the code's.
#[test]
fn list_finds_a_bundle_from_the_root() {
    let b = Bundle::build("list");
    // A sibling that is not a bundle must be stepped over, not refused.
    let sibling = b.artifacts.join("not-a-bundle");
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("readme.txt"), b"nothing here").unwrap();

    let found = gircap::verify::list(&b.artifacts).expect("lists");
    let hit = found.bundles.iter().find(|f| {
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
    // The sibling goes with the guard, not by hand: a directory removed by hand
    // is a directory left behind when the test panics before the removal.
}

/// The published order is the relative path, **not** depth.
///
/// `list`'s doc used to promise "deepest first"; the walk is a depth-first
/// traversal but both lists are re-sorted by the relative-path string, so a
/// shallower bundle whose leading component sorts earlier comes first. This
/// pins the order the summary's `first` and `last` report.
#[test]
fn list_orders_bundles_by_relative_path_not_by_depth() {
    let shallow = Bundle::build("order-shallow");
    let deep = Bundle::build("order-deep");
    // `z_region/hk4e_global/...` sorts after the shallow bundle's
    // `hk4e_global/...` even though it is one directory deeper.
    let pair = shallow
        .artifacts
        .join("z_region")
        .join(BIZ)
        .join(format!("{OLD_TAG}__{NEW_TAG}"));
    std::fs::create_dir_all(&pair).unwrap();
    std::fs::rename(&deep.root, pair.join("20261014T000000Z")).unwrap();

    let found = gircap::verify::list(&shallow.artifacts).expect("lists");
    assert_eq!(found.bundles.len(), 2, "{found:#?}");
    assert_eq!(
        found.bundles[0].path,
        format!("{BIZ}/{OLD_TAG}__{NEW_TAG}/20261014T000000Z"),
        "the shallower bundle sorts first because `hk4e_global` < `z_region`"
    );
    assert_eq!(
        found.bundles[1].path,
        format!("z_region/{BIZ}/{OLD_TAG}__{NEW_TAG}/20261014T000000Z"),
        "the deeper bundle is last: depth is not an ordering key"
    );
}

/// A directory that *looks* like a bundle - it has a `MANIFEST.json` - but is
/// truncated is skipped, not fatal. `list` runs over trees that may contain a
/// capture in progress.
///
/// The broken directory is a child of the tree the test owns, for the same
/// reason the walk is rooted there: a fixture that puts its litter in a
/// directory it does not own is a fixture that collides with the next run.
#[test]
fn a_bundle_that_does_not_open_is_skipped_rather_than_fatal() {
    let b = Bundle::build("skip");
    let broken = b.artifacts.join("broken-bundle");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("MANIFEST.json"), b"{ not json").unwrap();
    let found = gircap::verify::list(&b.artifacts).expect("lists");
    assert!(
        found
            .bundles
            .iter()
            .all(|f| !f.path.contains("broken-bundle"))
    );
    assert!(
        found
            .bundles
            .iter()
            .any(|f| f.capture_id == "20261014T000000Z"),
        "the good one is still there"
    );
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
        &gircap::assemble::ChunkSource::from_index(
            &b.root,
            &index,
            gircap::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
        ),
        OLD_TAG,
        CAPTURE,
        DLL,
        Set::MaterializeOld,
    )
    .expect("rebuilds");
    assert_eq!(&direct.bytes, o);
}
