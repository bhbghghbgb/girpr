//! One-off generator for the committed synthetic persistent bundle.
//!
//! The bundle is built from the small, test-only material in
//! `tools/persistent-sophon-research/samples/fixtures/` and from the ordinary
//! offline capture fixture (`gircap_test_support::shared_bundle`). Nothing here
//! touches the network or a production key. Run it once, by hand, when the
//! fixture needs rebuilding:
//!
//! ```text
//! cargo nextest run -p gircap --test persistent_fixture --run-ignored only
//! ```
//!
//! Every new file is written through `gircap::emit`'s writers, so
//! `MANIFEST.json → files[]` and `verify/checksums.json` are derived from the
//! bytes actually on disk rather than repeated by hand.

use std::path::{Path, PathBuf};

use gircap::emit;
use gircap::persistent::{
    BlobFile, Decoded, Descriptor, DispatchRecord, ManifestFile, PersistentIndex,
};
use gircap_test_support::{BundleVariant, shared_bundle};

const RES_BASE: &str = "https://mock.girpr.test/client_game_res/test";
const DATA_BASE: &str = "https://mock.girpr.test/client_design_data/test";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../persistent-sophon-research/samples/fixtures")
        .join(name)
}

fn target() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/artifact_persistent")
}

fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create the fixture root");
    for entry in std::fs::read_dir(src).expect("read the source bundle") {
        let entry = entry.expect("a source entry");
        let to = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), &to).expect("copy a bundle file");
        }
    }
}

fn body(name: &str) -> Vec<u8> {
    std::fs::read(fixture(name)).unwrap_or_else(|e| panic!("read fixture {name}: {e}"))
}

#[test]
#[ignore = "one-off generator for the committed fixture; run by hand"]
fn write_the_synthetic_persistent_bundle() {
    let source = shared_bundle(BundleVariant::Default);
    let root = target();
    if root.exists() {
        std::fs::remove_dir_all(&root).expect("clear the previous fixture");
    }
    copy_tree(&source, &root);
    // C16's source for `base_res_version_hash`: a tiny, deterministic base file
    // so the bridge can assert every marker byte, not just eight of nine.
    let streaming = root
        .join("game")
        .join("GenshinImpact_Data")
        .join("StreamingAssets")
        .join("res_versions_streaming");
    std::fs::write(&streaming, b"res_versions_streaming bytes").expect("write the hash source");

    // The shared capture's mock names the audio scan file with a file literally
    // named `Persistent`. The product has to create a `Persistent/` *directory*,
    // so move that content under `Persistent/audio_lang_14` and say so in the
    // API body — the same scan path the offline repair tests use.
    let persistent = root
        .join("game")
        .join("GenshinImpact_Data")
        .join("Persistent");
    if persistent.is_file() {
        let bytes = std::fs::read(&persistent).expect("read the scan stand-in");
        std::fs::remove_file(&persistent).expect("remove the scan stand-in");
        std::fs::create_dir_all(&persistent).expect("create Persistent/");
        std::fs::write(persistent.join("audio_lang_14"), bytes).expect("write the scan file");
    }
    {
        let path = root.join("api").join("getGameConfigs.json");
        let mut body: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("getGameConfigs")).expect("json");
        body["data"]["launch_configs"][0]["audio_pkg_scan_dir"] =
            serde_json::Value::String("GenshinImpact_Data/Persistent/audio_lang_14".into());
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&body).expect("serialize"),
        )
        .expect("rewrite getGameConfigs");
    }

    // The shared capture's mock uses a path shape whose second-to-last segment
    // is the word `manifest`, not the build id. The replay server takes the
    // last two segments as `(build_id, id)` exactly as production does, so give
    // the synthetic bodies production-shaped paths. Bytes are still the
    // capture's own; only the two `url_prefix` paths are normalised.
    for tag in ["7.0.0", "7.1.0"] {
        let path = root.join("api").join(format!("getBuild.{tag}.json"));
        let mut body: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("getBuild")).expect("json");
        let build_id = body["data"]["build_id"]
            .as_str()
            .expect("build id")
            .to_string();
        for m in body["data"]["manifests"]
            .as_array_mut()
            .expect("manifests[]")
        {
            m["manifest_download"]["url_prefix"] = serde_json::Value::String(format!(
                "https://autopatch.example/client_app/sophon/manifests/gopR6Cufr3/{build_id}"
            ));
            m["chunk_download"]["url_prefix"] = serde_json::Value::String(format!(
                "https://autopatch.example/client_app/sophon/chunks/gopR6Cufr3/{build_id}"
            ));
        }
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&body).expect("serialize getBuild"),
        )
        .expect("rewrite getBuild");
    }

    let res = body("res.ndjson");
    let data = body("data.ndjson");
    let silence = body("silence.ndjson");
    let hotfix_a = body("hotfix_a.blk");
    let hotfix_b = body("hotfix_b.pck");
    let manifest_doc: serde_json::Value =
        serde_json::from_slice(&body("fixtures.json")).expect("fixtures.json");
    let seed = manifest_doc["seed"].as_str().expect("seed").to_string();
    let fingerprint = manifest_doc["test_key"]["sha256_public_der"]
        .as_str()
        .expect("key fingerprint")
        .to_string();
    let version = manifest_doc["query_version"]
        .as_str()
        .expect("query version")
        .strip_prefix("OSRELWin")
        .expect("channel prefix")
        .to_string();

    let md5 = |b: &[u8]| gircap::hash::md5_hex(b);
    let descriptors = [
        Descriptor {
            kind: "res".into(),
            remote_name: "res_versions_external".into(),
            url: format!(
                "{RES_BASE}/output_99000001_aabbccddeeff/client/StandaloneWindows64/res_versions_external"
            ),
            md5: md5(&res),
            file_size: res.len() as u64,
        },
        Descriptor {
            kind: "data".into(),
            remote_name: "data_versions".into(),
            url: format!(
                "{DATA_BASE}/output_99000002_112233445566/client/General/AssetBundles/data_versions"
            ),
            md5: md5(&data),
            file_size: data.len() as u64,
        },
        Descriptor {
            kind: "silence_data".into(),
            remote_name: "silence_data_versions".into(),
            url: format!(
                "{DATA_BASE}/output_99000003_ffeeddccbbaa/client_silence/General/AssetBundles/data_versions"
            ),
            md5: md5(&silence),
            file_size: silence.len() as u64,
        },
    ];
    let record = DispatchRecord {
        schema_version: gircap::SCHEMA_VERSION,
        version,
        query_version: manifest_doc["query_version"].as_str().unwrap().to_string(),
        key_fingerprint: fingerprint,
        seed,
        envelope: "dispatch/response.json".into(),
        envelope_md5: String::new(),
        envelope_sha256: String::new(),
        ciphertext_bytes: 0,
        decoded: Decoded {
            game_version: "9.9_live".into(),
            channel_name: "OSRELWin".into(),
            codename: "hk4e_global".into(),
            res_revision: 99_000_001,
            data_revision: 99_000_002,
            silence_revision: 99_000_003,
            audio_revision: 99_000_000,
            res_hash: "aabbccddeeff".into(),
            data_hash: "112233445566".into(),
            silence_hash: "ffeeddccbbaa".into(),
            audio_hash: "001122334455".into(),
            repo_res_version_url: RES_BASE.into(),
            repo_design_data_url: DATA_BASE.into(),
            base_revision: "99000000 001122334455".into(),
            descriptors,
        },
    };
    let written = emit::write_dispatch(&root, &record, &body("dispatch_response.json"))
        .expect("write dispatch/");
    println!("wrote {}", written.join(", "));

    let blobs = [
        ("blocks/00/11111111.blk", hotfix_a.as_slice()),
        ("blocks/00/22222222.blk", hotfix_b.as_slice()),
        ("blocks/00/33333333.blk", hotfix_a.as_slice()),
        ("English(US)/VO_TEST.pck", hotfix_b.as_slice()),
        ("hardware_model_config.json", hotfix_b.as_slice()),
    ]
    .map(|(name, bytes)| {
        (
            format!("persistent/blobs/{name}"),
            name,
            bytes.len() as u64,
            md5(bytes),
        )
    });
    let manifests = [
        (
            "res",
            "persistent/res_versions_external.ndjson",
            res.as_slice(),
        ),
        ("data", "persistent/data_versions.ndjson", data.as_slice()),
        (
            "silence_data",
            "persistent/silence_data_versions.ndjson",
            silence.as_slice(),
        ),
    ]
    .map(|(kind, path, bytes)| ManifestFile {
        kind: kind.into(),
        path: path.into(),
        bytes: bytes.len() as u64,
        md5: md5(bytes),
    });
    let index = PersistentIndex {
        schema_version: gircap::SCHEMA_VERSION,
        manifests: manifests.clone(),
        blobs: blobs
            .iter()
            .map(|(path, name, bytes, md5)| BlobFile {
                remote_name: (*name).into(),
                path: path.clone(),
                bytes: *bytes,
                md5: md5.clone(),
            })
            .collect(),
    };
    let mut bodies: Vec<(String, Vec<u8>)> = manifests
        .iter()
        .zip([res, data, silence])
        .map(|(f, bytes)| (f.path.clone(), bytes))
        .collect();
    bodies.extend(
        blobs
            .iter()
            .zip([
                hotfix_a.clone(),
                hotfix_b.clone(),
                hotfix_a,
                hotfix_b.clone(),
                hotfix_b,
            ])
            .map(|((path, _, _, _), bytes)| (path.clone(), bytes)),
    );
    let written = emit::write_persistent(&root, &index, &bodies).expect("write persistent/");
    println!("wrote {}", written.join(", "));

    // The close-out: files[] and checksums are derived from the tree, not
    // restated. MANIFEST.json itself is excluded from files[] just as the real
    // close-out excludes the documents it renders.
    let entries: Vec<_> = emit::write_manifest_files(&root)
        .expect("scan the tree")
        .into_iter()
        .filter(|e| e.path != "MANIFEST.json" && e.path != "verify/checksums.json")
        .collect();
    let manifest_path = root.join("MANIFEST.json");
    let manifest: gircap::emit::index::Manifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).expect("read MANIFEST.json"))
            .expect("parse MANIFEST.json");
    let mut value = serde_json::to_value(manifest).expect("serialize MANIFEST.json");
    value["files"] = serde_json::to_value(entries).expect("serialize files[]");
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&value).unwrap() + "\n",
    )
    .expect("rewrite MANIFEST.json");
    let (path, checksums) = emit::write_checksums(&root).expect("write checksums");
    println!("wrote {path}: {} files", checksums.entries.len());
    println!("fixture at {}", root.display());
}
