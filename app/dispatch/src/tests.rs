//! Offline tests: recorded live summaries plus the synthetic fixture set.
//!
//! No test opens a socket. The few RSA keys here are either the committed
//! test-only `test_key.pem` or a synthetic 1024-bit master generated once and
//! pasted as DER; no production key or ciphertext is in this crate.

use crate::crypto::decode_base64;
use crate::dispatch::{BytesFetcher, DispatchQuery, check_minor_version, check_response_channel};
use crate::metadata::{
    MAGIC_SIGNATURE, MASTER_KEY_BLOB_LEN, MASTER_KEY_BLOB_SHA256, config_file_from_stamp,
    load_dispatch_keys, load_dispatch_keys_with_pin, master_key_from_json, pinned_seed,
    rsa_key_from_xml, unwrap_magic_blob,
};
use crate::proto::{WireValue, parse_descriptor, parse_dispatch_protobuf, walk_fields};
use crate::url::{
    DEFAULT_URL_TEMPLATES, build_dispatcher_url, channel_prefix, dispatch_host, query_version,
    region_id, template_for, validate_dispatcher_url,
};
use crate::{DispatchError, ManifestKind};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use girpr_digest::{decode_hex, md5_hex, sha256_hex};
use girpr_model::Region;
use prost::Message as _;
use rsa::{
    PrivateKey, PublicKey, pkcs1::DecodeRsaPrivateKey, pkcs1::EncodeRsaPrivateKey,
    pkcs1v15::Pkcs1v15Encrypt, pkcs8::EncodePublicKey, traits::PublicKeyParts,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;

const SAMPLES: &str = "../../../tests/fixtures/persistent_inputs/samples/";
const FIXTURES: &str = "../../../tests/fixtures/persistent_inputs/samples/fixtures/";

const METADATA_SUMMARY: &str =
    include_str!("../../../tests/fixtures/persistent_inputs/samples/metadata_summary.json");
const GLOBAL_SUMMARY: &str =
    include_str!("../../../tests/fixtures/persistent_inputs/samples/dispatch_summary.json");
const CN_SUMMARY: &str =
    include_str!("../../../tests/fixtures/persistent_inputs/samples/hk4e_cn.dispatch_summary.json");
const BILI_SUMMARY: &str = include_str!(
    "../../../tests/fixtures/persistent_inputs/samples/hk4e_bilibili.dispatch_summary.json"
);
const FIELD_DUMP: &str =
    include_str!("../../../tests/fixtures/persistent_inputs/samples/dispatch_fields.txt");
const FIXTURE_DOC: &str =
    include_str!("../../../tests/fixtures/persistent_inputs/samples/fixtures/fixtures.json");
const TEST_KEY_PEM: &str =
    include_str!("../../../tests/fixtures/persistent_inputs/samples/fixtures/test_key.pem");
const TEST_KEY_XML: &str = include_str!("test_key.xml");
const DISPATCH_RESPONSE: &str = include_str!(
    "../../../tests/fixtures/persistent_inputs/samples/fixtures/dispatch_response.json"
);
const RES_NDJSON: &str =
    include_str!("../../../tests/fixtures/persistent_inputs/samples/fixtures/res.ndjson");
const DATA_NDJSON: &str =
    include_str!("../../../tests/fixtures/persistent_inputs/samples/fixtures/data.ndjson");
const SILENCE_NDJSON: &str =
    include_str!("../../../tests/fixtures/persistent_inputs/samples/fixtures/silence.ndjson");

// Synthetic RSA-1024 PKCS#1 DER master key (test only), generated once. It is
// not in the release repo and is never used outside these tests.
const SYNTHETIC_MASTER_DER_HEX: &str = "3082025d02010002818100eabdc5013a54eeac1753ebc7ff278343a265b940eb4f91239805f117eb14a266e73ae6512e23da9598d34183cc09ff72d78b1421d6f38a82c78699e24576cdecb28b9f5be384a19135ce94ab5745fdd3326a0616c3b1a34029931d396a7da4083afe9d27eaaaa65e6e935099b4729554958e6e7420247e173a05326f65eb9e1702030100010281800e8e2f9d2260839fba522c86e07c7fd510e24d427b5b339369227ac8b230d541820332d9d0ca4dbf5deddab6044ff6caaee3d0520fa4f07c0ee2947325715089940ac838975407418315a3b35668c6bdfe4fe2d7a5a237adb55801018c407dd796b9856ff085b3140d5f6491db2922e56d6f2afd7e865a84ac0413c753fdd795024100edc9c57aecec0c3a41e6ccf4e6a0c6e01de704a54bc497fa389f32a3a5c81c58240b55710525fdb467e33fcdaef397a77affa97db37277a6b774e3d24058a025024100fcb8424dccb8894c65f3962fa055431f56972132a4ae5c89945728886596791620ab8ab38dd1638dae4002b5322137758849280a3e0feb4c82f1e3b204fce28b024100eaca8b72f8538f334af521354430468eef17b80f9b03d068a2d616c35da37cd14f2dddbcce2f9816356a3f0ad20ff693c9b07de3ff772c4bf2e3b93c568aebfd024100c63f796bf4a3f2b8e6e56e34904f32679c12d46d18f8db08e64075408288c7164faca212f6b682a53b43659c64e5dfcea889d30a2de3690f4d4d99f425ab87bf02406fca8228f6028547f570b1e71a53820cf61dec3aa01e1ca4d158ec259a808046fcc184e74c04345851754c162d8d92354f380703724b2a748cdb30c98dbd1bda";

struct MemoryFetcher(HashMap<String, Vec<u8>>);

impl BytesFetcher for MemoryFetcher {
    fn get(&self, url: &str) -> crate::Result<Vec<u8>> {
        self.0
            .get(url)
            .cloned()
            .ok_or_else(|| DispatchError::Transport(format!("unexpected URL {url:?}")))
    }
}

fn compress(data: &[u8]) -> Vec<u8> {
    let mut writer = brotli::CompressorWriter::new(Vec::new(), 4096, 5, 22);
    writer.write_all(data).expect("compress");
    writer.into_inner()
}

fn magic_sized(
    flags: i64,
    payload: Vec<u8>,
    compressed_size: usize,
    decompressed_size: usize,
) -> Vec<u8> {
    let mut raw = Vec::with_capacity(32 + payload.len());
    raw.extend_from_slice(&MAGIC_SIGNATURE.to_le_bytes());
    raw.extend_from_slice(&flags.to_le_bytes());
    raw.extend_from_slice(&(compressed_size as i64).to_le_bytes());
    raw.extend_from_slice(&(decompressed_size as i64).to_le_bytes());
    raw.extend_from_slice(&payload);
    raw
}

fn magic(flags: i64, payload: Vec<u8>, decompressed_size: usize) -> Vec<u8> {
    let compressed_size = payload.len();
    magic_sized(flags, payload, compressed_size, decompressed_size)
}

/// flags 0x1: Brotli only. Returns the encoded base64 metadata field.
fn wrap_plain(text: &[u8]) -> String {
    let compressed = compress(text);
    STANDARD.encode(magic(1, compressed, text.len()))
}

/// flags 0x101: Brotli + RSA in 128-byte master-key blocks.
fn wrap_encrypted(text: &[u8], master: &PrivateKey, block: usize) -> String {
    let compressed = compress(text);
    let mut ciphertext = Vec::new();
    for chunk in compressed.chunks(block - 11) {
        ciphertext.extend(
            master
                .to_public_key()
                .encrypt(&mut rand::rng(), Pkcs1v15Encrypt, chunk)
                .expect("master encrypt"),
        );
    }
    // Collapse's header records the *compressed* (post-RSA) size, not the
    // ciphertext size: unwrap_magic checks it after decryption.
    STANDARD.encode(magic_sized(0x101, ciphertext, compressed.len(), text.len()))
}

fn synthetic_master() -> PrivateKey {
    PrivateKey::from_pkcs1_der(&decode_hex(SYNTHETIC_MASTER_DER_HEX).expect("hex")).expect("key")
}

fn stamp_json() -> Vec<u8> {
    serde_json::to_vec(&json!([
        {"GameName":"Genshin Impact","GameRegion":"Global","MetadataType":"PresetConfigV2","MetadataPath":"config_GIGlb.json"},
        {"GameName":"Genshin Impact","GameRegion":"Mainland China","MetadataType":"PresetConfigV2","MetadataPath":"config_GICN.json"},
        {"GameName":"Genshin Impact","GameRegion":"Bilibili","MetadataType":"PresetConfigV2","MetadataPath":"config_GICNBilibili.json"},
    ]))
    .expect("stamp")
}

fn master_json(master: &PrivateKey) -> Vec<u8> {
    let der = master.to_pkcs1_der().expect("der").as_bytes().to_vec();
    let blob = magic(1, compress(&der), der.len());
    serde_json::to_vec(&json!({"Key": STANDARD.encode(&blob), "BitSize": 128}))
        .expect("master json")
}

/// Per-biz config with the test dispatcher key and a live-shaped seed.
fn config_json(biz: Region, master: &PrivateKey) -> Vec<u8> {
    let (profile, template, seed, channel, sub_channel, exe) = match biz {
        Region::Hk4eGlobal => (
            "GIGlb",
            template_for(Region::Hk4eGlobal),
            "018bc9560abc707f",
            1u64,
            0u64,
            "GenshinImpact.exe",
        ),
        Region::Hk4eCn => (
            "GICN",
            template_for(Region::Hk4eCn),
            "e17f04ce947d859a",
            1,
            1,
            "YuanShen.exe",
        ),
        Region::Hk4eBilibili => (
            "GICNBilibili",
            template_for(Region::Hk4eBilibili),
            "e17f04ce947d859a",
            14,
            0,
            "YuanShen.exe",
        ),
    };
    serde_json::to_vec(&json!({
        "ProfileName": profile,
        "GameDispatchURL": wrap_plain(template.as_bytes()),
        "ProtoDispatchKey": wrap_plain(seed.as_bytes()),
        "DispatcherKeyBitLength": 256,
        "DispatcherKey": wrap_encrypted(TEST_KEY_XML.as_bytes(), master, 128),
        "ChannelID": channel,
        "SubChannelID": sub_channel,
        "GameExecutableName": exe,
    }))
    .expect("config json")
}

fn fixture_key() -> PrivateKey {
    PrivateKey::from_pkcs1_pem(TEST_KEY_PEM).expect("test key")
}

fn encrypted_envelope(plaintext: &[u8]) -> Vec<u8> {
    let public: PublicKey = fixture_key().to_public_key();
    let mut ciphertext = Vec::new();
    for chunk in plaintext.chunks(245) {
        ciphertext.extend(
            public
                .encrypt(&mut rand::rng(), Pkcs1v15Encrypt, chunk)
                .expect("encrypt"),
        );
    }
    let content = STANDARD.encode(ciphertext);
    let sign = STANDARD.encode([0u8; 256]);
    format!("{{\"content\":\"{content}\",\"sign\":\"{sign}\"}}").into_bytes()
}

/// The descriptor embedded in the protobuf describes the whole NDJSON body.
fn descriptor_line(remote_name: &str, body: &str) -> String {
    format!(
        "{{\"remoteName\":\"{remote_name}\",\"md5\":\"{}\",\"fileSize\":{}}}",
        md5_hex(body.as_bytes()),
        body.len()
    )
}

fn synthetic_protobuf() -> Vec<u8> {
    let res = crate::RepoResVersionProperties {
        res_revision: 99_000_001,
        res_descriptor: descriptor_line("res_versions_external", RES_NDJSON),
        res_hash: "aabbccddeeff".into(),
        version: "7.2_live".into(),
    };
    let props = crate::GatewayProperties {
        properties_2: 22_101,
        server_name: "TEST".into(),
        repo_res_version_url: "https://mock.girpr.test/client_game_res/test".into(),
        repo_design_data_url: "https://mock.girpr.test/client_design_data/test".into(),
        game_version: "7.2_live".into(),
        data_revision: 99_000_002,
        silence_revision: 99_000_003,
        data_descriptor: descriptor_line("data_versions", DATA_NDJSON),
        silence_descriptor: descriptor_line("silence_data_versions", SILENCE_NDJSON),
        res_version_properties: Some(res),
        data_hash: "112233445566".into(),
        silence_hash: "ffeeddccbbaa".into(),
        codename: "hk4e_global".into(),
    };
    crate::proto::DispatchResponse {
        properties: Some(props),
    }
    .encode_to_vec()
}

#[test]
fn recorded_summaries_pin_the_master_blob_and_all_three_urls() {
    let metadata: Value = serde_json::from_str(METADATA_SUMMARY).expect("metadata summary");
    assert_eq!(
        metadata["master"]["master_key_blob_sha256"].as_str(),
        Some(MASTER_KEY_BLOB_SHA256)
    );
    assert_eq!(
        metadata["master"]["master_key_blob_len"].as_u64(),
        Some(MASTER_KEY_BLOB_LEN as u64)
    );

    for (biz, config_file, summary, seed, region, prefix, host) in [
        (
            Region::Hk4eGlobal,
            "config_GIGlb.json",
            GLOBAL_SUMMARY,
            "018bc9560abc707f",
            0,
            "OSRELWin",
            "osusadispatch",
        ),
        (
            Region::Hk4eCn,
            "config_GICN.json",
            CN_SUMMARY,
            "e17f04ce947d859a",
            4,
            "CNRELWin",
            "cngfdispatch",
        ),
        (
            Region::Hk4eBilibili,
            "config_GICNBilibili.json",
            BILI_SUMMARY,
            "e17f04ce947d859a",
            5,
            "CNRELWin",
            "cnqddispatch",
        ),
    ] {
        let recorded: Value = serde_json::from_str(summary).expect("dispatch summary");
        let per_biz = &metadata["biz"][biz.as_str()];
        assert_eq!(per_biz["config_file"].as_str(), Some(config_file));
        assert_eq!(per_biz["region_id"].as_u64(), Some(region));
        let template = per_biz["fields"]["GameDispatchURL"]
            .as_str()
            .expect("template");
        assert_eq!(template, template_for(biz));
        assert_eq!(per_biz["fields"]["ProtoDispatchKey"].as_str(), Some(seed));
        assert_eq!(pinned_seed(biz, "7.2.0").expect("seed"), seed);
        assert_eq!(region_id(biz), region as u8);
        assert_eq!(channel_prefix(biz), prefix);
        assert_eq!(dispatch_host(biz), host);
        let query = query_version(biz, "7.2.0");
        assert_eq!(query, format!("{prefix}7.2.0"));
        let url = build_dispatcher_url(template, host, &query, seed).expect("url");
        assert_eq!(url, recorded["used"]["url"].as_str().expect("recorded url"));
        validate_dispatcher_url(&url, biz).expect("query parameters");
        assert_eq!(recorded["used"]["seed"].as_str(), Some(seed));
        assert_eq!(recorded["used"]["channel"].as_str(), Some(prefix));
        assert_eq!(
            recorded["values"]["game_version_from_props"].as_str(),
            Some("7.2_live")
        );
        assert_eq!(
            recorded["values"]["res_revision"].as_u64(),
            Some(49_530_578)
        );
        assert_eq!(recorded["values"]["res_hash"].as_str(), Some("62f1ec6cdb"));
        assert_eq!(
            recorded["derived_urls"]["client_game_res_url"].as_str(),
            Some(format!("https://autopatch{}.yuanshen.com/client_game_res/7.2_live/output_49530578_62f1ec6cdb/client", if biz == Region::Hk4eGlobal { "hk" } else { "cn" }).as_str())
        );
    }
}

#[test]
fn synthetic_metadata_blob_unwraps_and_the_production_pin_is_loud() {
    let master = synthetic_master();
    let master_bytes = master_json(&master);
    let (unwrapped, block) = master_key_from_json(&master_bytes, false).expect("master");
    assert_eq!(block, 128);
    assert_eq!(unwrapped.size(), 128);
    // The committed pin belongs to the live blob, not this test key.
    assert!(master_key_from_json(&master_bytes, true).is_err());

    assert!(master_key_from_json(
        &serde_json::to_vec(&json!({
            "Key": STANDARD.encode(magic(1, vec![0u8; MASTER_KEY_BLOB_LEN], MASTER_KEY_BLOB_LEN)),
            "BitSize": 128,
        }))
        .expect("wrong master"),
        true,
    )
    .is_err());

    // Header checks: signature, both sizes, and encryption needs a key.
    let mut wrong_size = magic(1, compress(b"payload"), b"payload".len() + 1);
    wrong_size[16..24].copy_from_slice(&(compress(b"payload").len() as i64).to_le_bytes());
    assert!(unwrap_magic_blob(&wrong_size, None, 0).is_err());
    assert!(unwrap_magic_blob(&magic(0x101, vec![0u8; 128], 10), None, 128).is_err());

    let stamp = stamp_json();
    for (biz, file) in [
        (Region::Hk4eGlobal, "config_GIGlb.json"),
        (Region::Hk4eCn, "config_GICN.json"),
        (Region::Hk4eBilibili, "config_GICNBilibili.json"),
    ] {
        assert_eq!(
            config_file_from_stamp(&stamp, biz).expect("config file"),
            file
        );
    }

    let config = config_json(Region::Hk4eGlobal, &master);
    let mut urls = HashMap::new();
    urls.insert(crate::metadata::MASTER_URL.to_owned(), master_bytes.clone());
    urls.insert(crate::metadata::STAMP_URL.to_owned(), stamp);
    urls.insert(
        format!("{}/config_GIGlb.json", crate::metadata::METADATA_BASE),
        config,
    );
    let fetcher = MemoryFetcher(urls);
    let keys = load_dispatch_keys_with_pin(Region::Hk4eGlobal, "7.2.0", &fetcher, false)
        .expect("unwrapped keys");
    assert_eq!(keys.host, "osusadispatch");
    assert_eq!(keys.channel_prefix, "OSRELWin");
    assert_eq!(keys.region_id, 0);
    assert_eq!(keys.seed, "018bc9560abc707f");
    assert_eq!(keys.block_bytes, 256);
    assert_eq!(keys.key.size(), 256);
    assert_eq!(keys.exe_stem, "GenshinImpact");
    assert_eq!(keys.template, template_for(Region::Hk4eGlobal));
    // The pinned product path refuses the synthetic key blob.
    assert!(load_dispatch_keys(Region::Hk4eGlobal, "7.2.0", &fetcher).is_err());
}

#[test]
fn xml_key_ignores_inverse_q_and_tampered_components_fail() {
    // `TEST_KEY_XML` carries a seventh component that is not q^-1 mod p;
    // loading only (n,e,d,p,q) must still succeed.
    let key = rsa_key_from_xml(TEST_KEY_XML).expect("XML key");
    assert_eq!(key.size(), 256);
    let public_der = key.to_public_key().to_public_key_der().expect("public DER");
    assert_eq!(sha256_hex(public_der.as_bytes()), {
        let doc: Value = serde_json::from_str(FIXTURE_DOC).unwrap();
        doc["test_key"]["sha256_public_der"]
            .as_str()
            .unwrap()
            .to_owned()
    });
    let tampered =
        TEST_KEY_XML.replacen("<Exponent>AQAB</Exponent>", "<Exponent>AQAD</Exponent>", 1);
    assert!(rsa_key_from_xml(&tampered).is_err());
    assert!(rsa_key_from_xml("<RSAKeyValue><Modulus>AA==</Modulus></RSAKeyValue>").is_err());
}

#[test]
fn committed_synthetic_dispatch_response_decrypts() {
    let doc: Value = serde_json::from_str(FIXTURE_DOC).expect("fixtures.json");
    let response: Value = serde_json::from_str(DISPATCH_RESPONSE).expect("dispatch response");
    let mut keys: Vec<&str> = response
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["content", "sign"]);
    let ciphertext = decode_base64(response["content"].as_str().unwrap()).expect("base64");
    assert_eq!(
        ciphertext.len(),
        doc["dispatch_response"]["ciphertext_bytes"]
            .as_u64()
            .unwrap() as usize
    );
    let plaintext = crate::decrypt_pkcs1_blocks(&fixture_key(), &ciphertext, 256).expect("decrypt");
    assert_eq!(plaintext.len(), 633);
    assert_eq!(
        sha256_hex(&plaintext),
        doc["dispatch_response"]["plaintext_sha256"]
            .as_str()
            .unwrap()
    );

    let (decoded, opaque) = parse_dispatch_protobuf(&plaintext).expect("protobuf");
    let props = decoded.properties.expect("properties");
    let res = props
        .res_version_properties
        .expect("res version properties");
    assert_eq!(props.game_version, "9.9_live");
    assert_eq!(props.codename, "hk4e_global");
    assert_eq!(props.properties_2, 22101);
    assert_eq!(res.res_revision, 99_000_001);
    assert_eq!(res.res_hash, "aabbccddeeff");
    assert_eq!(props.data_revision, 99_000_002);
    assert_eq!(props.silence_revision, 99_000_003);
    // Root 11/14 are opaque; field 37 in properties is unknown to the typed
    // schema but must survive the generic walk.
    assert_eq!(opaque.root, vec![(11, 16), (14, 8)]);
    assert!(opaque.properties.is_empty());
    let fields = walk_fields(&plaintext).expect("walk");
    assert!(fields.iter().any(|(tag, _)| *tag == 3));
    let props_bytes = fields.iter().find(|(tag, _)| *tag == 3).unwrap().1;
    let WireValue::Bytes(props_bytes) = props_bytes else {
        panic!("field 3 is bytes")
    };
    let props_fields = walk_fields(props_bytes).expect("walk properties");
    assert!(props_fields.iter().any(|(tag, _)| *tag == 37));
    assert!(crate::decrypt_pkcs1_blocks(&fixture_key(), &ciphertext[..255], 256).is_err());
}

#[test]
fn proto_field_numbers_match_recorded_dump() {
    let section = |header: &str| -> Vec<u32> {
        let start = FIELD_DUMP.find(header).expect("section") + header.len();
        let rest = &FIELD_DUMP[start..];
        let end = rest.find("=== ").unwrap_or(rest.len());
        rest[..end]
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .filter_map(|token| token.parse::<u32>().ok())
            .collect()
    };
    let root = section("=== ROOT (all fields) ===");
    let props = section("=== FIELD 3 GatewayProperties (all fields) ===");
    let resver = section("=== FIELD 22 RepoResVersionProperties (all fields) ===");
    for tag in [3, 11, 12, 14] {
        assert!(root.contains(&tag), "root tag {tag}");
    }
    for tag in [1, 2, 3, 7, 8, 9, 12, 14, 18, 19, 20, 22, 26, 27, 36, 38, 47] {
        assert!(props.contains(&tag), "property tag {tag}");
    }
    for tag in [1, 3, 5, 6] {
        assert!(resver.contains(&tag), "res-version tag {tag}");
    }
    for value in [
        "7.2_live",
        "USA",
        "hk4e_global",
        "49530578",
        "62f1ec6cdb",
        "49419538",
        "604495f667",
        "49291486",
        "c25ba650a6",
        "autopatchhk.yuanshen.com",
    ] {
        assert!(FIELD_DUMP.contains(value), "recorded value {value:?}");
    }
}

#[test]
fn descriptor_and_base_revision_values_match_the_live_samples() {
    let summary: Value = serde_json::from_str(GLOBAL_SUMMARY).expect("summary");
    let res = &summary["manifests"]["res_versions_external"]["sample"][0];
    let rows = format!(
        "{{\"remoteName\":{},\"md5\":{},\"hash\":{},\"fileSize\":{}}}\n",
        res["remoteName"], res["md5"], res["hash"], res["fileSize"]
    );
    let row = parse_descriptor(&rows).expect("descriptor");
    assert_eq!(row.remote_name, "res_versions_external");
    assert_eq!(row.md5, "9a1b5efc2e2b11475e74c88789d0f339");
    assert_eq!(row.file_size, 370_958);

    let fixture: Value = serde_json::from_str(FIXTURE_DOC).expect("fixtures.json");
    for (name, body) in [
        ("res", RES_NDJSON),
        ("data", DATA_NDJSON),
        ("silence", SILENCE_NDJSON),
    ] {
        // The descriptor embedded in the protobuf describes the *whole*
        // manifest body; a manifest's own first row describes a hotfix blob.
        // Both gates are checked here, against the committed digests.
        assert_eq!(
            md5_hex(body.as_bytes()),
            fixture["manifests"][name]["md5"].as_str().unwrap()
        );
        assert_eq!(
            body.len() as u64,
            fixture["manifests"][name]["bytes"].as_u64().unwrap()
        );
        let first = body.lines().find(|line| !line.trim().is_empty()).unwrap();
        let expected: Value = serde_json::from_str(first).expect("first NDJSON row");
        let row = parse_descriptor(body).expect("fixture descriptor");
        assert_eq!(row.md5, expected["md5"].as_str().unwrap());
        assert_eq!(row.file_size, expected["fileSize"].as_u64().unwrap());
    }
    assert_eq!(
        md5_hex(RES_NDJSON.as_bytes()),
        "ce5fe1b283fa2b97929eabb86452429f"
    );
    assert_eq!(
        md5_hex(DATA_NDJSON.as_bytes()),
        "bbc05fb62bbde346e56368bf77b122d3"
    );
    assert_eq!(
        md5_hex(SILENCE_NDJSON.as_bytes()),
        "d8d8b615534bc1c66c7bb042d1b4603c"
    );
    assert!(parse_descriptor("data.ndjson\n").is_err());
    assert_eq!(
        crate::parse_base_revision("49008628 644b3d946e\r\n").expect("base_revision"),
        (49_008_628, "644b3d946e".to_owned())
    );
}

#[test]
fn version_minor_match_and_mismatch() {
    assert_eq!(crate::minor_version("7.2.0"), Some((7, 2)));
    assert_eq!(crate::minor_version("7.2_live"), Some((7, 2)));
    assert_eq!(crate::minor_version("9.9_live"), Some((9, 9)));
    check_minor_version("7.2.0", "7.2_live").expect("same minor");
    check_minor_version("7.2.1", "7.2_live").expect("same minor");
    let err = check_minor_version("7.3.0", "7.2_live").expect_err("real disagreement");
    assert!(matches!(err, DispatchError::Version { .. }));
    assert!(check_minor_version("not-a-version", "7.2_live").is_err());
}

#[test]
fn codename_and_channel_authority_accept_bili_cn_codename() {
    check_response_channel(Region::Hk4eGlobal, "hk4e_global", 22_101, "osusadispatch")
        .expect("global");
    check_response_channel(Region::Hk4eCn, "hk4e_cn", 22_101, "cngfdispatch").expect("cn");
    // Bili answers codename hk4e_cn; 22102 and the cnqd host are authoritative.
    check_response_channel(Region::Hk4eBilibili, "hk4e_cn", 22_102, "cnqddispatch")
        .expect("bili accepted by properties/host");
    check_response_channel(Region::Hk4eBilibili, "hk4e_cn", 22_101, "cnqddispatch")
        .expect("bili via host");
    check_response_channel(
        Region::Hk4eBilibili,
        "hk4e_bilibili",
        22_102,
        "cnqddispatch",
    )
    .expect("bili native codename");
    assert!(
        check_response_channel(Region::Hk4eBilibili, "hk4e_global", 22_102, "cnqddispatch")
            .is_err()
    );
    assert!(
        check_response_channel(Region::Hk4eBilibili, "hk4e_cn", 22_101, "cngfdispatch").is_err()
    );
    assert!(check_response_channel(Region::Hk4eCn, "hk4e_cn", 22_102, "cnqddispatch").is_err());
}

#[test]
fn full_query_with_injected_transport_returns_dispatch_info() {
    let master = synthetic_master();
    let master_bytes = master_json(&master);
    let config = config_json(Region::Hk4eGlobal, &master);
    let plaintext = synthetic_protobuf();
    let envelope = encrypted_envelope(&plaintext);

    let res_base = "https://mock.girpr.test/client_game_res/test";
    let data_base = "https://mock.girpr.test/client_design_data/test";
    let dispatch_url = build_dispatcher_url(
        template_for(Region::Hk4eGlobal),
        "osusadispatch",
        "OSRELWin7.2.0",
        "018bc9560abc707f",
    )
    .expect("dispatch URL");
    let base_revision_url =
        format!("{res_base}/output_99000001_aabbccddeeff/client/StandaloneWindows64/base_revision");
    let mut urls = HashMap::new();
    urls.insert(crate::metadata::MASTER_URL.to_owned(), master_bytes);
    urls.insert(crate::metadata::STAMP_URL.to_owned(), stamp_json());
    urls.insert(
        format!("{}/config_GIGlb.json", crate::metadata::METADATA_BASE),
        config,
    );
    urls.insert(dispatch_url.clone(), envelope);
    let fetcher = MemoryFetcher(urls);
    let revision = |url: &str| -> crate::Result<String> {
        assert_eq!(url, base_revision_url);
        Ok("99000004 aabbccdd\n".to_owned())
    };

    let info = crate::query_dispatch_with_master_pin(
        DispatchQuery {
            biz: Region::Hk4eGlobal,
            latest: "7.2.0",
        },
        &fetcher,
        &revision,
        false,
    )
    .expect("dispatch info");
    assert_eq!(info.game_version, "7.2_live");
    assert_eq!(info.channel_name, "OSRELWin");
    assert_eq!(info.server_name, "TEST");
    assert_eq!(info.codename, "hk4e_global");
    assert_eq!(
        (info.res_revision, info.data_revision, info.silence_revision),
        (99_000_001, 99_000_002, 99_000_003)
    );
    assert_eq!(
        (info.audio_revision, info.audio_hash.as_str()),
        (99_000_004, "aabbccdd")
    );
    assert_eq!(info.res_hash, "aabbccddeeff");
    assert_eq!(info.data_hash, "112233445566");
    assert_eq!(info.silence_hash, "ffeeddccbbaa");
    assert_eq!(
        info.client_game_res_url,
        format!("{res_base}/output_99000001_aabbccddeeff/client")
    );
    assert_eq!(
        info.client_design_data_url,
        format!("{data_base}/output_99000002_112233445566/client/General")
    );
    assert_eq!(
        info.client_silence_data_url,
        format!("{data_base}/output_99000003_ffeeddccbbaa/client_silence/General")
    );
    assert_eq!(
        info.client_audio_assets_url,
        format!("{res_base}/output_99000004_aabbccdd/client")
    );
    assert_eq!(info.manifest_digests.len(), 3);
    let [res, data, silence] = &info.manifest_digests;
    assert_eq!(res.kind, ManifestKind::Res);
    assert_eq!(
        res.url,
        format!(
            "{}/StandaloneWindows64/res_versions_external",
            info.client_game_res_url
        )
    );
    assert_eq!(res.expected_md5, "ce5fe1b283fa2b97929eabb86452429f");
    assert_eq!(res.expected_size, RES_NDJSON.len() as u64);
    assert_eq!(
        res.mirror_rel,
        "GenshinImpact_Data/Persistent/res_versions_persist"
    );
    assert_eq!(data.kind, ManifestKind::Data);
    assert_eq!(
        data.url,
        format!("{}/AssetBundles/data_versions", info.client_design_data_url)
    );
    assert_eq!(
        data.mirror_rel,
        "GenshinImpact_Data/Persistent/data_versions_persist"
    );
    assert_eq!(silence.kind, ManifestKind::Silence);
    assert_eq!(
        silence.url,
        format!(
            "{}/AssetBundles/silence_data_versions",
            info.client_silence_data_url
        )
    );
    assert_eq!(
        silence.mirror_rel,
        "GenshinImpact_Data/Persistent/silence_data_versions_persist"
    );
    assert_eq!(
        sha256_hex(&decode_base64(&STANDARD.encode(&plaintext)).unwrap()),
        sha256_hex(&plaintext)
    );
}

#[test]
fn no_network_constants_are_referenced_by_path_only() {
    // A compile-time check that the committed fixtures stay in the tree: the
    // include_str! constants above are the only way tests see sample bytes.
    assert!(METADATA_SUMMARY.len() > 1000);
    assert!(GLOBAL_SUMMARY.len() > 1000);
    assert!(CN_SUMMARY.len() > 1000);
    assert!(BILI_SUMMARY.len() > 1000);
    assert!(SAMPLES.ends_with("samples/"));
    assert!(FIXTURES.ends_with("fixtures/"));
    assert_eq!(DEFAULT_URL_TEMPLATES.len(), 3);
    assert!(pinned_seed(Region::Hk4eGlobal, "8.1.0").is_err());
}
