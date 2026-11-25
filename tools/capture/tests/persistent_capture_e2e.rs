#![allow(unsafe_code)]
//! Persistent capture, end to end through the real `run::capture_with_http`.
//!
//! C3's `tests/persistent_capture.rs` drives the producer leaf with a canned
//! `DispatchContext`. This test drives the whole command instead: one loopback
//! mock answers the Collapse metadata, `query_cur_region`, `base_revision`, the
//! three mirrored manifests and the hotfix blobs, and the *real* producer fetches
//! and decodes the byte-exact committed envelope itself. Nothing is copied into
//! a record by the test, and no production key or endpoint is touched.
//!
//! `GIRCAP_TEST_DISPATCH_PIN=0` is the test-only master-pin hook from C3. It is
//! set here for the life of each capture and restored on drop. Nextest runs one
//! process per test, so the process-global variable cannot leak into another
//! case; the hand-run producer uses the same guard.
//!
//! The `#[ignore]`d case at the bottom writes a small complete bundle to the
//! gitignored `artifacts/persistent_v4/`. It is the only fixture the later
//! manual bridge cases consume, via `GIRPR_ARTIFACT_BUNDLE`.

mod common;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use common::{
    Dispatch, DispatchProfile, Mock, PERSISTENT_LATEST, dispatch_with, persistent_fixture,
};
use gircap::emit::index::Manifest;
use gircap::hash::{md5_hex, sha256_hex};
use gircap::persistent::{DispatchRecord, PersistentIndex};
use gircap::report::Val;
use gircap::select::Class;
use gircap::verify::{Bundle, list, reverify};
use girpr_dispatch::{
    DispatchDecodeParams, channel_prefix, decode_dispatch_response, dispatch_host,
    dispatch_info_from_decoded, private_key_fingerprint, private_key_from_bytes,
};
use girpr_model::Region;
use girpr_tempdir::TempDir;

/// Restore the pin hook when a capture (or the hand-run producer) finishes.
struct PinGuard(Option<OsString>);

impl PinGuard {
    /// Install `GIRCAP_TEST_DISPATCH_PIN=0` for this test process.
    fn install() -> Self {
        let previous = std::env::var_os("GIRCAP_TEST_DISPATCH_PIN");
        // Edition 2024 marks process-global environment writes unsafe. The
        // harness deliberately keeps this in one test binary and nextest keeps
        // each case in its own process; no other test reads the hook.
        unsafe { std::env::set_var("GIRCAP_TEST_DISPATCH_PIN", "0") };
        Self(previous)
    }
}

impl Drop for PinGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.0 {
                Some(value) => std::env::set_var("GIRCAP_TEST_DISPATCH_PIN", value),
                None => std::env::remove_var("GIRCAP_TEST_DISPATCH_PIN"),
            }
        }
    }
}

/// The one bundle under `root`, whichever way the layout nests it.
fn find_bundle(root: &Path) -> PathBuf {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if dir.join("MANIFEST.json").is_file() {
            return dir;
        }
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut subs: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            subs.sort();
            stack.extend(subs.into_iter().rev());
        }
    }
    panic!("no bundle under {}", root.display());
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> T {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_slice(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn capture(mock: &Mock, out: &Path, extra: &[&str]) -> Dispatch {
    let out_arg = out.to_string_lossy().into_owned();
    let mut args = vec![
        "capture".to_string(),
        "--out".to_string(),
        out_arg,
        "--hyp-base".to_string(),
        mock.hyp_base(),
        "--sophon-base".to_string(),
        mock.sophon_base(),
        "--per-class".to_string(),
        "3".to_string(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    dispatch_with(&borrowed, Some(mock.transport()))
}

fn summary_int(d: &Dispatch, key: &str) -> u64 {
    match d.summary().get(key) {
        Some(Val::Int(n)) => *n as u64,
        other => panic!("SUMMARY has no integer {key}: {other:?}"),
    }
}

/// One capture's `dispatch/`, `persistent/` and verification documents.
struct BundleFacts {
    manifest: Manifest,
    record: DispatchRecord,
    index: PersistentIndex,
    envelope: Vec<u8>,
    key_material: Vec<u8>,
}

fn facts(bundle: &Path) -> BundleFacts {
    let manifest: Manifest = read_json(&bundle.join("MANIFEST.json"));
    let record: DispatchRecord = read_json(&bundle.join("dispatch/record.json"));
    let index: PersistentIndex = read_json(&bundle.join("persistent/index.json"));
    let envelope = std::fs::read(bundle.join(&record.envelope)).expect("envelope on disk");
    let key_material = std::fs::read(bundle.join(&record.key_file)).expect("key file on disk");
    BundleFacts {
        manifest,
        record,
        index,
        envelope,
        key_material,
    }
}

/// Re-derive the record's facts from the bytes it carries, independently of
/// the producer: the envelope is decrypted with the key stored in this bundle.
fn assert_self_contained(f: &BundleFacts) -> String {
    assert_eq!(f.record.schema_version, 4, "the record carries schema 4");
    assert_eq!(
        f.record.key_file, "dispatch/key.pem",
        "historical extension stays"
    );
    assert_eq!(f.index.schema_version, 4);
    assert_eq!(f.record.ciphertext_bytes, f.envelope.len() as u64);
    assert_eq!(f.record.envelope_md5, md5_hex(&f.envelope));
    assert_eq!(f.record.envelope_sha256, sha256_hex(&f.envelope));

    let key = private_key_from_bytes(&f.key_material).expect("the bundle's own key parses");
    let fingerprint = private_key_fingerprint(&key).expect("fingerprint");
    assert_eq!(f.record.key_fingerprint, fingerprint);

    let decoded = decode_dispatch_response(
        &f.envelope,
        DispatchDecodeParams {
            biz: Region::Hk4eGlobal,
            latest: PERSISTENT_LATEST,
            host: dispatch_host(Region::Hk4eGlobal),
            channel_prefix: channel_prefix(Region::Hk4eGlobal),
            exe_stem: "GenshinImpact",
            key: &key,
            block_bytes: 256,
        },
    )
    .expect("the bundle's own envelope decrypts and decodes");
    let info = dispatch_info_from_decoded(decoded.clone(), &f.record.decoded.base_revision)
        .expect("the recorded base_revision parses");

    assert_eq!(f.record.decoded.game_version, decoded.game_version);
    assert_eq!(f.record.decoded.channel_name, decoded.channel_name);
    assert_eq!(f.record.decoded.codename, decoded.codename);
    assert_eq!(f.record.decoded.res_revision, decoded.res_revision);
    assert_eq!(f.record.decoded.data_revision, decoded.data_revision);
    assert_eq!(f.record.decoded.silence_revision, decoded.silence_revision);
    assert_eq!(f.record.decoded.audio_revision, info.audio_revision);
    assert_eq!(f.record.decoded.res_hash, decoded.res_hash);
    assert_eq!(f.record.decoded.data_hash, decoded.data_hash);
    assert_eq!(f.record.decoded.silence_hash, decoded.silence_hash);
    assert_eq!(f.record.decoded.audio_hash, info.audio_hash);
    assert_eq!(
        f.record.decoded.repo_res_version_url,
        decoded.repo_res_version_url
    );
    assert_eq!(
        f.record.decoded.repo_design_data_url,
        decoded.repo_design_data_url
    );
    assert_eq!(f.record.decoded.base_revision, "99000000 001122334455");

    for (row, descriptor, remote_name) in f
        .record
        .decoded
        .descriptors
        .iter()
        .zip(&decoded.manifest_digests)
        .zip(&decoded.descriptor_remote_names)
        .map(|((row, descriptor), remote_name)| (row, descriptor, remote_name))
    {
        assert_eq!(row.kind, descriptor.kind.as_str());
        assert_eq!(row.remote_name, *remote_name);
        assert_eq!(row.url, descriptor.url);
        assert_eq!(row.md5, descriptor.expected_md5);
        assert_eq!(row.file_size, descriptor.expected_size);
    }
    fingerprint
}

#[test]
fn persistent_capture_is_end_to_end_against_one_loopback_mock() {
    let _pin = PinGuard::install();
    let mock = Mock::persistent(DispatchProfile::Fixture);
    let routes = mock.persistent_routes();
    // The mock's own view of the dispatcher must be the committed bytes.
    assert_eq!(
        routes.envelope(),
        persistent_fixture("dispatch_response.json")
    );
    assert_eq!(routes.seed(), "0011223344556677");

    let scratch = TempDir::new("gircap-persistent-e2e", "primary");
    let out = scratch.join("artifacts");
    let d = capture(&mock, &out, &[]);
    assert_eq!(d.code, 0, "capture failed: {:?}", d.detail.as_deref());
    let bundle = find_bundle(&out);
    let f = facts(&bundle);

    // Nine rows, in `REQUIRED_ORDER`, and the three record classes captured.
    let ids: Vec<&str> = f.manifest.matrix.iter().map(|r| r.class.as_str()).collect();
    assert_eq!(ids, Class::REQUIRED_ORDER.map(|c| c.id()));
    let persistent_rows: Vec<_> = f
        .manifest
        .matrix
        .iter()
        .filter(|r| Class::parse(&r.class).is_ok_and(Class::is_record))
        .collect();
    assert_eq!(persistent_rows.len(), 3);
    for row in &persistent_rows {
        assert_eq!(row.status, "captured", "{row:?}");
        assert!(row.capture_id.is_some() && row.path.is_some(), "{row:?}");
    }

    // Cheapest-first is visible in the admitted costs: within each manifest the
    // 48-byte row is admitted before the 64-byte row, and extras are bought
    // after all three cheapest rows.
    let costs: Vec<u64> = f.index.blobs.iter().map(|b| b.bytes).collect();
    assert_eq!(
        costs,
        [48, 48, 64, 48, 64],
        "five committed rows, cheapest first"
    );
    let res_order: Vec<&str> = f
        .index
        .blobs
        .iter()
        .filter(|b| b.source_kind == "res")
        .map(|b| b.remote_name.as_str())
        .collect();
    assert_eq!(
        res_order,
        ["English(US)/VO_TEST.pck", "blocks/00/11111111.blk"],
        "the 48 B row precedes the 64 B row in the same manifest"
    );
    assert!(costs.iter().any(|c| *c == 48) && costs.iter().any(|c| *c == 64));
    for kind in ["res", "data", "silence_data"] {
        assert!(
            f.index.blobs.iter().any(|b| b.source_kind == kind),
            "every source_kind has at least one blob"
        );
    }

    // The record is fetched, decoded and self-contained.
    let fingerprint = assert_self_contained(&f);
    assert_eq!(f.envelope, persistent_fixture("dispatch_response.json"));
    assert_eq!(f.record.version, PERSISTENT_LATEST);
    assert_eq!(f.record.query_version, "OSRELWin9.9.0");
    assert_eq!(f.record.dispatch_url, routes.dispatch_url());
    assert_eq!(f.record.seed, routes.seed());
    assert_eq!(f.key_material, routes.key_material());

    // The index has exactly three manifests, byte-exact against the fixtures.
    let files = [
        ("res", "res.ndjson"),
        ("data", "data.ndjson"),
        ("silence_data", "silence.ndjson"),
    ];
    for (m, (kind, file)) in f.index.manifests.iter().zip(files) {
        let committed = persistent_fixture(file);
        assert_eq!(m.kind, kind);
        assert_eq!(m.bytes, committed.len() as u64);
        assert_eq!(m.md5, md5_hex(&committed));
        assert_eq!(
            std::fs::read(bundle.join(&m.path)).expect("manifest body"),
            committed,
            "the raw manifest is byte-exact"
        );
    }
    for blob in &f.index.blobs {
        let body = std::fs::read(bundle.join(&blob.path)).expect("blob body");
        assert_eq!(body.len() as u64, blob.bytes);
        assert_eq!(md5_hex(&body), blob.md5);
    }

    // Every new path joins both inventories.
    let listed: Vec<&str> = f.manifest.files.iter().map(|e| e.path.as_str()).collect();
    let sums: gircap::emit::index::Checksums = read_json(&bundle.join("verify/checksums.json"));
    let checked: Vec<&str> = sums.entries.iter().map(|e| e.path.as_str()).collect();
    let mut new_paths = vec![f.record.envelope.as_str(), f.record.key_file.as_str()];
    new_paths.extend(f.index.manifests.iter().map(|m| m.path.as_str()));
    new_paths.extend(f.index.blobs.iter().map(|b| b.path.as_str()));
    new_paths.push("persistent/index.json");
    for path in new_paths {
        assert!(
            listed.contains(&path),
            "{path} missing from MANIFEST.json files[]"
        );
        assert!(
            checked.contains(&path),
            "{path} missing from verify/checksums.json"
        );
    }

    // In-process re-verification: A17 and both persistent rows pass.
    let report = reverify(Bundle::open(&bundle).expect("open the captured bundle"));
    assert_eq!(report.result(), "pass", "failures: {:?}", report.failures());
    assert!(report.agrees);
    for id in ["A17", "P1", "P2"] {
        let row = report
            .rows
            .iter()
            .find(|r| r.id == id)
            .expect("the row exists");
        assert!(row.ok, "{id} failed: {:?}", row);
    }

    // `gircap list` sees the new documents in its per-bundle record.
    let index = list(&out).expect("list the capture root");
    assert_eq!(index.bundles.len(), 1);
    let listed_bundle = &index.bundles[0];
    assert!(listed_bundle.dispatch_record);
    assert_eq!(listed_bundle.persistent_manifests, 3);
    assert_eq!(listed_bundle.persistent_blobs, 5);

    // A budget boundary: pay the fixed cost plus only the two cheapest blobs.
    // The silence row is over budget, so neither its 64 B blob nor any extra is
    // admitted, and the ledger stops at exactly the sum of what was paid for.
    let fixed = f.envelope.len() as u64 + f.index.manifests.iter().map(|m| m.bytes).sum::<u64>();
    let unlimited = summary_int(&d, "capture_bytes_admitted");
    let content_spend = unlimited - fixed - costs.iter().sum::<u64>();
    let boundary_spend = content_spend + fixed + 48 + 48;
    let scratch2 = TempDir::new("gircap-persistent-e2e", "budget");
    let out2 = scratch2.join("artifacts");
    let d2 = capture(
        &mock,
        &out2,
        &["--max-download-bytes", &boundary_spend.to_string()],
    );
    assert_eq!(d2.code, 0, "a best-effort cap is not a capture failure");
    let bundle2 = find_bundle(&out2);
    let f2 = facts(&bundle2);
    let row2: Vec<_> = f2
        .manifest
        .matrix
        .iter()
        .filter(|r| Class::parse(&r.class).is_ok_and(Class::is_record))
        .collect();
    assert_eq!(row2.len(), 3);
    assert!(
        row2.iter().any(|r| r.status == "over_budget"),
        "the class whose cheapest row no longer fits is recorded, not dropped"
    );
    assert_eq!(
        f2.index.blobs.len(),
        2,
        "extras are refused over the boundary"
    );
    assert!(
        f2.index.blobs.iter().all(|b| b.bytes == 48),
        "only the two 48 B cheapest rows were paid for"
    );
    assert_eq!(summary_int(&d2, "budget_limit"), boundary_spend);
    assert_eq!(summary_int(&d2, "capture_bytes_admitted"), boundary_spend);
    let ledger = content_spend + fixed + f2.index.blobs.iter().map(|b| b.bytes).sum::<u64>();
    assert_eq!(summary_int(&d2, "capture_bytes_admitted"), ledger);
    assert_eq!(ledger, boundary_spend, "bytes_admitted == budget.spent()");

    // A second capture with a different synthetic key and seed verifies its own
    // bundle: records are self-contained, not keyed to the first run.
    let mock2 = Mock::persistent(DispatchProfile::Rotated);
    assert_ne!(
        mock2.persistent_routes().key_material(),
        routes.key_material()
    );
    assert_ne!(mock2.persistent_routes().seed(), routes.seed());
    let scratch3 = TempDir::new("gircap-persistent-e2e", "rotated");
    let out3 = scratch3.join("artifacts");
    let d3 = capture(&mock2, &out3, &[]);
    assert_eq!(
        d3.code,
        0,
        "rotated capture failed: {:?}",
        d3.detail.as_deref()
    );
    let bundle3 = find_bundle(&out3);
    let f3 = facts(&bundle3);
    let fingerprint3 = assert_self_contained(&f3);
    assert_ne!(fingerprint3, fingerprint, "a genuinely different key");
    assert_ne!(f3.envelope, f.envelope, "a genuinely different envelope");
    assert_eq!(
        f3.record.dispatch_url,
        mock2.persistent_routes().dispatch_url()
    );
    assert_eq!(f3.record.seed, mock2.persistent_routes().seed());
    let report3 = reverify(Bundle::open(&bundle3).expect("open the rotated bundle"));
    assert_eq!(
        report3.result(),
        "pass",
        "failures: {:?}",
        report3.failures()
    );
    for id in ["A17", "P1", "P2"] {
        assert!(
            report3.rows.iter().any(|r| r.id == id && r.ok),
            "{id} must pass"
        );
    }
}

/// Hand-run producer for the later manual bridge cases.
///
/// ```powershell
/// cargo nextest run -p gircap --test persistent_capture_e2e --run-ignored only \
///   -E 'test(write_persistent_v4_bundle)'
/// ```
///
/// Writes the bundle to `artifacts/persistent_v4/` itself (gitignored), verifies
/// it in-process, and prints the exact `GIRPR_ARTIFACT_BUNDLE` value. No bundle
/// is committed and no writer is handcrafted: this is the same
/// `run::capture_with_http` path the always-on test drives. The file is called
/// `dispatch/key.pem` for historical reasons; C3 writes `RSAKeyValue` XML there
/// and girpr accepts either PEM or XML.
#[test]
#[ignore = "hand-run: produces the gitignored artifacts/persistent_v4 bundle"]
fn write_persistent_v4_bundle() {
    let _pin = PinGuard::install();
    let artifacts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts");
    std::fs::create_dir_all(&artifacts).expect("create the gitignored artifacts root");
    let staging = TempDir::new("gircap-persistent-v4", "producer");
    let out = staging.join("out");

    let mock = Mock::persistent(DispatchProfile::Fixture);
    let d = capture(&mock, &out, &[]);
    assert_eq!(d.code, 0, "capture failed: {:?}", d.detail.as_deref());
    let built = find_bundle(&out);
    let final_dir = artifacts.join("persistent_v4");
    if final_dir.exists() {
        std::fs::remove_dir_all(&final_dir).expect("replace the previous bundle");
    }
    std::fs::rename(&built, &final_dir).expect("publish the bundle at a stable path");

    let report = reverify(Bundle::open(&final_dir).expect("open the hand-run bundle"));
    assert_eq!(report.result(), "pass", "failures: {:?}", report.failures());
    assert!(report.agrees);
    // The stable, repo-relative value a later bridge session sets. Absolute
    // canonicalization on Windows would print a `\\?\` prefix nobody needs.
    eprintln!("GIRPR_ARTIFACT_BUNDLE=artifacts/persistent_v4");
}
