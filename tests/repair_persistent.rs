//! Offline contract cases for the plan half of `--with-persistent` (commit 6).
//!
//! The dispatcher is replaced by the `DispatchOverride` seam; the three
//! manifests come from the committed synthetic fixtures served on a loopback
//! origin. No case opens a production URL and no case writes a marker, a
//! mirror or a hotfix file.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::common::{
    LATEST_TAG, LOCAL_TAG, MockOpts, MockServer, build_fixture, build_fixture_spec, ctx_for,
    logs_of, md5hex, run, scope, temp_dir, write_game_file,
};
use girpr_digest::{md5_hex, sha256_hex};
use girpr_loopback_mock::{Reply, ScriptedServer};
use serde_json::{Value, json};

use girpr::test_support::{
    DispatchOverride, ManifestDescriptor, ManifestKind, failing_dispatch_override,
    persistent_dispatch, persistent_override, summary_record,
};
use girpr::{Biz, RunCtx, RunExit, RunFailure, Summary};

const RES: &[u8] = include_bytes!("fixtures/persistent_inputs/samples/fixtures/res.ndjson");
const DATA: &[u8] = include_bytes!("fixtures/persistent_inputs/samples/fixtures/data.ndjson");
const SILENCE: &[u8] = include_bytes!("fixtures/persistent_inputs/samples/fixtures/silence.ndjson");
const HOTFIX_A: &[u8] = include_bytes!("fixtures/persistent_inputs/samples/fixtures/hotfix_a.blk");
const HOTFIX_B: &[u8] = include_bytes!("fixtures/persistent_inputs/samples/fixtures/hotfix_b.pck");
const TEST_KEY_PEM: &str = include_str!("fixtures/persistent_inputs/samples/fixtures/test_key.pem");
const ENVELOPE: &[u8] =
    include_bytes!("fixtures/persistent_inputs/samples/fixtures/dispatch_response.json");

/// Serve the three manifests in descriptor order. `ScriptedServer` repeats the
/// last reply, so a retry of the last body still has the advertised digest.
fn manifest_server(res: &'static [u8]) -> ScriptedServer {
    ScriptedServer::start(vec![
        Reply::new(200, res),
        Reply::new(200, DATA),
        Reply::new(200, SILENCE),
    ])
}

/// A dispatcher result whose descriptors name `server`'s manifest routes.
fn dispatch_for(server: &ScriptedServer, res: &'static [u8]) -> girpr::test_support::DispatchInfo {
    persistent_dispatch(
        server.base(),
        "GenshinImpact",
        [
            (ManifestKind::Res, "res.ndjson", res),
            (ManifestKind::Data, "data.ndjson", DATA),
            (ManifestKind::Silence, "silence.ndjson", SILENCE),
        ],
    )
}

fn line(s: &Summary) -> String {
    summary_record(s, 0).text()
}

fn flagged(ctx: &mut RunCtx, info: girpr::test_support::DispatchInfo) {
    ctx.with_persistent = true;
    ctx.persistent_dispatch_override = Some(persistent_override(info));
}

// ---------------------------------------------------------------------------
// Out-of-band key + record inputs (the path the bridge will use next).
// ---------------------------------------------------------------------------

/// Swap a recorded authority for the loopback base, leaving the path alone —
/// exactly the rewrite the bridge applies to a scratch copy of the record.
fn rewrite_authority(url: &str, base: &str) -> String {
    let after_scheme = url.split_once("://").expect("absolute URL").1;
    let path = &after_scheme[after_scheme.find('/').expect("path")..];
    format!("{}{path}", base.trim_end_matches('/'))
}

const RES_URL: &str = "https://mock.girpr.test/client_game_res/test/output_99000001_aabbccddeeff/client/StandaloneWindows64/res_versions_external";
const DATA_URL: &str = "https://mock.girpr.test/client_design_data/test/output_99000002_112233445566/client/General/AssetBundles/data_versions";
const SILENCE_URL: &str = "https://mock.girpr.test/client_design_data/test/output_99000003_ffeeddccbbaa/client_silence/General/AssetBundles/silence_data_versions";

/// The record that a future gircap writer will emit for the committed fixture,
/// with its CDN authorities rewritten to `base`.
fn record_value(base: &str) -> Value {
    json!({
        "schema_version": 3,
        "version": "9.9.0",
        "query_version": "OSRELWin9.9.0",
        "dispatch_url": format!("{base}/query"),
        "key_fingerprint": "e0fcb3321f5dfd7a6e5d208699782e44c40da897b6ee6813c3d81ff8bae303c0",
        "key_file": "dispatch/key.pem",
        "seed": "0011223344556677",
        "envelope": "dispatch/response.json",
        "envelope_md5": md5_hex(ENVELOPE),
        "envelope_sha256": sha256_hex(ENVELOPE),
        "ciphertext_bytes": ENVELOPE.len(),
        "decoded": {
            "game_version": "9.9_live",
            "channel_name": "OSRELWin",
            "codename": "hk4e_global",
            "res_revision": 99000001u64,
            "data_revision": 99000002u64,
            "silence_revision": 99000003u64,
            "res_hash": "aabbccddeeff",
            "data_hash": "112233445566",
            "silence_hash": "ffeeddccbbaa",
            "base_revision": "99000000 001122334455",
            "repo_res_version_url": rewrite_authority("https://mock.girpr.test/client_game_res/test", base),
            "repo_design_data_url": rewrite_authority("https://mock.girpr.test/client_design_data/test", base),
            "descriptors": [
                {
                    "kind": "res",
                    "remote_name": "res_versions_external",
                    "url": rewrite_authority(RES_URL, base),
                    "md5": md5_hex(RES),
                    "file_size": RES.len()
                },
                {
                    "kind": "data",
                    "remote_name": "data_versions",
                    "url": rewrite_authority(DATA_URL, base),
                    "md5": md5_hex(DATA),
                    "file_size": DATA.len()
                },
                {
                    "kind": "silence_data",
                    "remote_name": "silence_data_versions",
                    "url": rewrite_authority(SILENCE_URL, base),
                    "md5": md5_hex(SILENCE),
                    "file_size": SILENCE.len()
                }
            ]
        }
    })
}

/// Write a scratch bundle: `dispatch/record.json`, `dispatch/key.pem` and the
/// byte-exact envelope the record names.
fn write_dispatch_inputs(dir: &Path, record: &Value) -> (PathBuf, PathBuf) {
    let dispatch = dir.join("dispatch");
    std::fs::create_dir_all(&dispatch).expect("scratch dispatch dir");
    let key = dispatch.join("key.pem");
    let record_path = dispatch.join("record.json");
    std::fs::write(&key, TEST_KEY_PEM).expect("scratch key");
    std::fs::write(&record_path, record.to_string()).expect("scratch record");
    std::fs::write(dispatch.join("response.json"), ENVELOPE).expect("scratch envelope");
    (key, record_path)
}

/// The override-path equivalent of `record_value`: same facts, same URLs, so a
/// comparison of the two runs' publications has only the input path as a
/// variable.
fn fixture_dispatch(server: &ScriptedServer) -> girpr::test_support::DispatchInfo {
    let descriptor = |kind, body: &[u8], url: &str| ManifestDescriptor {
        kind,
        url: rewrite_authority(url, server.base()),
        expected_md5: md5_hex(body),
        expected_size: body.len() as u64,
        mirror_rel: format!(
            "{}_Data/Persistent/{}_versions_persist",
            "GenshinImpact",
            kind.as_str()
        ),
    };
    girpr::test_support::DispatchInfo {
        game_version: "9.9_live".into(),
        channel_name: "OSRELWin".into(),
        server_name: "TEST".into(),
        codename: "hk4e_global".into(),
        res_revision: 99_000_001,
        data_revision: 99_000_002,
        silence_revision: 99_000_003,
        audio_revision: 99_000_000,
        res_hash: "aabbccddeeff".into(),
        data_hash: "112233445566".into(),
        silence_hash: "ffeeddccbbaa".into(),
        audio_hash: "001122334455".into(),
        client_game_res_url: rewrite_authority(
            "https://mock.girpr.test/client_game_res/test/output_99000001_aabbccddeeff/client",
            server.base(),
        ),
        client_design_data_url: rewrite_authority(
            "https://mock.girpr.test/client_design_data/test/output_99000002_112233445566/client/General",
            server.base(),
        ),
        client_silence_data_url: rewrite_authority(
            "https://mock.girpr.test/client_design_data/test/output_99000003_ffeeddccbbaa/client_silence/General",
            server.base(),
        ),
        client_audio_assets_url: rewrite_authority(
            "https://mock.girpr.test/client_game_res/test/output_99000000_001122334455/client",
            server.base(),
        ),
        manifest_digests: [
            descriptor(ManifestKind::Res, RES, RES_URL),
            descriptor(ManifestKind::Data, DATA, DATA_URL),
            descriptor(ManifestKind::Silence, SILENCE, SILENCE_URL),
        ],
    }
}

/// The marker/mirror/hotfix bytes a flagged run publishes, as one comparable
/// snapshot. The two runs below differ only in how `DispatchInfo` was obtained.
fn published_snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut names: Vec<String> = [
        "base_res_version_hash",
        "res_revision",
        "res_revision_eternal",
        "data_revision",
        "silence_revision",
        "audio_revision",
        "PatchDone",
        "ChannelName",
        "ScriptVersion",
        "res_versions_persist",
        "data_versions_persist",
        "silence_data_versions_persist",
    ]
    .iter()
    .map(|n| format!("Persistent/{n}"))
    .collect();
    names.extend(
        [
            "StreamingAssets/AssetBundles/blocks/00/11111111.blk",
            "StreamingAssets/AssetBundles/blocks/00/22222222.blk",
            "StreamingAssets/AssetBundles/blocks/00/33333333.blk",
            "StreamingAssets/AudioAssets/English(US)/VO_TEST.pck",
            "StreamingAssets/hardware_model_config.json",
        ]
        .iter()
        .map(|n| (*n).to_owned()),
    );
    names
        .into_iter()
        .map(|rel| {
            let full = dir.join(format!("GenshinImpact_Data/{rel}"));
            let bytes =
                std::fs::read(&full).unwrap_or_else(|e| panic!("missing published {rel}: {e}"));
            (rel, bytes)
        })
        .collect()
}

/// Put the four in-scope fixture rows on disk already intact, so a scope test
/// exercises the scope verdict rather than the download path. The excluded
/// `Persistent/...11111111.blk` hotfix is deliberately left absent.
fn write_intact_fixture_rows(dir: &std::path::Path) {
    for (rel, bytes) in [
        (
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/22222222.blk",
            HOTFIX_B,
        ),
        (
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/33333333.blk",
            HOTFIX_A,
        ),
        (
            "GenshinImpact_Data/StreamingAssets/AudioAssets/English(US)/VO_TEST.pck",
            HOTFIX_B,
        ),
        (
            "GenshinImpact_Data/StreamingAssets/hardware_model_config.json",
            HOTFIX_B,
        ),
    ] {
        write_game_file(dir, rel, bytes);
    }
}

// ---------------------------------------------------------------------------
// failure classes
// ---------------------------------------------------------------------------

/// A key/seed/decrypt/decode failure is a metadata failure with the pinned log
/// prefix; no manifest request follows it.
#[tokio::test]
async fn a_dispatcher_failure_exits_two_without_fetching_manifests() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("persistent_dispatch_failure");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.with_persistent = true;
    ctx.persistent_dispatch_override = Some(failing_dispatch_override("bad key blob"));

    let err: RunFailure = run(ctx).await.expect_err("the query is fatal");
    assert_eq!(err.exit_code, RunExit::Metadata);
    assert!(
        format!("{err:#}").starts_with("dispatcher query failed:"),
        "{err:#}"
    );
    assert!(format!("{err:#}").contains("bad key blob"));
    mock.stop();
}

/// B9: a version disagreement is the same exit-2 class, not a degraded run.
#[tokio::test]
async fn a_version_disagreement_exits_two() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("persistent_version_mismatch");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.with_persistent = true;
    ctx.persistent_dispatch_override = Some(failing_dispatch_override(
        "version mismatch: queried 7.2.0, response 7.3_live",
    ));

    let err = run(ctx).await.expect_err("a real disagreement is fatal");
    assert_eq!(err.exit_code, RunExit::Metadata);
    let text = format!("{err:#}");
    assert!(text.starts_with("dispatcher query failed:"), "{text}");
    assert!(text.contains("version mismatch"), "{text}");
    mock.stop();
}

/// The descriptor MD5 is a required gate: a wrong body fails the run instead
/// of producing a degraded report.
#[tokio::test]
async fn a_manifest_md5_failure_exits_two() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let server = manifest_server(RES);
    let dir = temp_dir("persistent_manifest_md5");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    let mut info = dispatch_for(&server, RES);
    info.manifest_digests[0].expected_md5 = "00000000000000000000000000000000".into();
    flagged(&mut ctx, info);

    let err = run(ctx).await.expect_err("the manifest gate is fatal");
    assert_eq!(err.exit_code, RunExit::Metadata);
    let text = format!("{:?}", err.source);
    assert!(text.contains("manifest MD5 mismatch"), "{text}");
    assert!(!dir.join("GenshinImpact_Data/Persistent").exists());
    mock.stop();
}

/// A corrupt key is a metadata failure with the pinned prefix. No manifest
/// server is even started, so a fetch would have failed differently.
#[tokio::test]
async fn a_corrupt_key_exits_two_without_fetching_manifests() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("persistent_bad_key_file");
    let inputs = temp_dir("persistent_bad_key_inputs");
    let (key, record) = write_dispatch_inputs(&inputs, &record_value("http://127.0.0.1:1"));
    std::fs::write(&key, b"not a key").expect("corrupt key");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.with_persistent = true;
    ctx.dispatch_key_path = Some(key);
    ctx.dispatch_record_path = Some(record);

    let err = run(ctx).await.expect_err("the key is fatal");
    assert_eq!(err.exit_code, RunExit::Metadata);
    let text = format!("{err:#}");
    assert!(text.starts_with("dispatcher query failed:"), "{text}");
    assert!(text.contains("dispatch key"), "{text}");
    mock.stop();
}

/// Matching the record's pinned fingerprint is the first fact check: a valid
/// key from a different bundle cannot be substituted by editing the record.
#[tokio::test]
async fn a_fingerprint_mismatch_exits_two() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("persistent_fingerprint_mismatch");
    let inputs = temp_dir("persistent_fingerprint_inputs");
    let mut value = record_value("http://127.0.0.1:1");
    value["key_fingerprint"] =
        json!("0000000000000000000000000000000000000000000000000000000000000000");
    let (key, record) = write_dispatch_inputs(&inputs, &value);
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.with_persistent = true;
    ctx.dispatch_key_path = Some(key);
    ctx.dispatch_record_path = Some(record);

    let err = run(ctx).await.expect_err("a fingerprint mismatch is fatal");
    assert_eq!(err.exit_code, RunExit::Metadata);
    let text = format!("{err:#}");
    assert!(text.starts_with("dispatcher query failed:"), "{text}");
    assert!(text.contains("key_fingerprint"), "{text}");
    mock.stop();
}

/// Editing only the record must not invent a revision: the decoded envelope
/// is the authority, and disagreements are a metadata failure.
#[tokio::test]
async fn a_tampered_record_fact_exits_two() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("persistent_tampered_fact");
    let inputs = temp_dir("persistent_tampered_inputs");
    let mut value = record_value("http://127.0.0.1:1");
    value["decoded"]["res_revision"] = json!(12345678u64);
    let (key, record) = write_dispatch_inputs(&inputs, &value);
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.with_persistent = true;
    ctx.dispatch_key_path = Some(key);
    ctx.dispatch_record_path = Some(record);

    let err = run(ctx).await.expect_err("a tampered fact is fatal");
    assert_eq!(err.exit_code, RunExit::Metadata);
    let text = format!("{err:#}");
    assert!(text.starts_with("dispatcher query failed:"), "{text}");
    assert!(text.contains("record facts disagree"), "{text}");
    assert!(!dir.join("GenshinImpact_Data/Persistent").exists());
    mock.stop();
}

/// The key+record path publishes exactly what the override path publishes:
/// every marker, all three mirrors and the downloaded hotfix. The only
/// difference between the two runs is how `DispatchInfo` was obtained.
#[tokio::test]
async fn a_key_and_record_publish_the_same_state_as_the_override() {
    async fn run_flagged(
        dir: &Path,
        inputs: &Path,
        mock: &MockServer,
        server: &ScriptedServer,
        from_record: bool,
    ) -> (Summary, RunExit) {
        write_game_file(
            dir,
            "GenshinImpact_Data/StreamingAssets/res_versions_streaming",
            b"res_versions_streaming bytes",
        );
        std::fs::write(
            dir.join("config.ini"),
            format!("[General]\ngame_version={LOCAL_TAG}\n"),
        )
        .expect("config.ini");
        write_intact_fixture_rows(dir);
        let mut ctx = ctx_for(dir.to_path_buf(), mock, 1);
        if from_record {
            let (key, record) = write_dispatch_inputs(inputs, &record_value(server.base()));
            ctx.with_persistent = true;
            ctx.dispatch_key_path = Some(key);
            ctx.dispatch_record_path = Some(record);
        } else {
            flagged(&mut ctx, fixture_dispatch(server));
        }
        run(ctx).await.expect("the flagged run succeeds")
    }

    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    // Manifests first, then the single hotfix row left absent above.
    let replies = || {
        vec![
            Reply::new(200, RES),
            Reply::new(200, DATA),
            Reply::new(200, SILENCE),
            Reply::new(200, HOTFIX_A),
        ]
    };
    let override_server = ScriptedServer::start(replies());
    let record_server = ScriptedServer::start(replies());
    let override_dir = temp_dir("persistent_override_parity");
    let record_dir = temp_dir("persistent_record_parity");
    let inputs = temp_dir("persistent_record_parity_inputs");

    let (override_summary, override_code) =
        run_flagged(&override_dir, &inputs, &mock, &override_server, false).await;
    let (record_summary, record_code) =
        run_flagged(&record_dir, &inputs, &mock, &record_server, true).await;
    assert_eq!(override_code, RunExit::Success);
    assert_eq!(record_code, RunExit::Success);
    assert_eq!(
        record_summary.flags.persistent,
        override_summary.flags.persistent
    );
    assert_eq!(
        record_summary.flags.persistent_files,
        override_summary.flags.persistent_files
    );
    assert_eq!(
        record_summary.flags.persistent_repaired,
        override_summary.flags.persistent_repaired
    );
    assert_eq!(
        record_summary.flags.persistent_failed,
        override_summary.flags.persistent_failed
    );
    assert_eq!(
        published_snapshot(&record_dir),
        published_snapshot(&override_dir)
    );
    mock.stop();
}

// ---------------------------------------------------------------------------
// plan build and read-only modes
// ---------------------------------------------------------------------------

/// The three NDJSON bodies parse (blank lines and extra keys included) into a
/// sorted plan, and `--plan-out` carries the flag-only section. The read-only
/// plan writes no marker, mirror or hotfix file.
#[tokio::test]
async fn three_manifests_are_listed_in_plan_out_without_writes() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let server = manifest_server(RES);
    let dir = temp_dir("persistent_plan_out");
    let out = temp_dir("persistent_plan_out_doc").join("plan.json");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, dispatch_for(&server, RES));
    ctx.print_plan = true;
    ctx.scan = true;
    ctx.plan_out = Some(out.clone());

    let (summary, code) = run(ctx).await.expect("a listed plan succeeds");
    assert_eq!(code, RunExit::Success);
    assert!(
        line(&summary).contains("persistent=listed"),
        "{}",
        line(&summary)
    );
    assert_eq!(summary.flags.persistent_files, Some(5));
    assert_eq!(summary.flags.persistent_repaired, Some(0));
    assert_eq!(summary.flags.persistent_failed, Some(0));
    assert_eq!(summary.flags.persistent_base_hash, Some("omitted"));

    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&out).expect("plan-out is written")).unwrap();
    let p = &doc["persistent"];
    assert_eq!(p["files"], 5);
    assert_eq!(p["bytes"], 64 + 48 + 48 + 48 + 64);
    assert_eq!(p["scan_excludes_persistent"], true);
    assert_eq!(p["scope"], "listed");
    assert_eq!(p["assets"].as_array().unwrap().len(), 5);
    assert_eq!(p["mirrors"].as_array().unwrap().len(), 3);
    assert_eq!(p["markers"].as_array().unwrap().len(), 8);
    assert!(
        !dir.join("GenshinImpact_Data/Persistent").exists(),
        "the plan half writes nothing under Persistent"
    );
    mock.stop();
}

/// `--check-only` and `--dry-run` also query and list, but write no markers.
#[tokio::test]
async fn check_only_and_dry_run_list_the_persistent_set_only() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("persistent_readonly");
    for (check_only, dry_run) in [(true, false), (false, true)] {
        let server = manifest_server(RES);
        let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
        flagged(&mut ctx, dispatch_for(&server, RES));
        ctx.check_only = check_only;
        ctx.dry_run = dry_run;
        let (summary, _) = run(ctx).await.expect("a read-only run completes");
        assert_eq!(summary.flags.persistent, Some("listed"));
        assert_eq!(summary.flags.persistent_files, Some(5));
        assert!(!dir.join("GenshinImpact_Data/Persistent").exists());
    }
    mock.stop();
}

/// C20: a cleanup-only run does no persistent work at all, even with the flag.
#[tokio::test]
async fn purge_only_never_calls_the_dispatcher() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("persistent_purge_only");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.with_persistent = true;
    ctx.purge_only = true;
    ctx.persistent_dispatch_override = Some(DispatchOverride::new(|_, _| {
        panic!("--purge-only must not query the dispatcher")
    }));

    let (summary, code) = run(ctx).await.expect("the cleanup still runs");
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.flags.persistent, Some("skipped"));
    assert_eq!(summary.flags.persistent_files, Some(0));
    mock.stop();
}

/// Without the flag, a configured failing provider is never reached and the
/// summary keeps its unflagged shape byte for byte.
#[tokio::test]
async fn an_unflagged_run_never_calls_the_provider_and_adds_no_keys() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("persistent_flag_off");
    let fx = build_fixture();
    write_game_file(&dir, &fx.files[0].rel, &fx.files[0].data);
    write_game_file(&dir, &fx.files[1].rel, &fx.files[1].data);
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.persistent_dispatch_override = Some(DispatchOverride::new(|_, _| {
        panic!("an unflagged run must not call the dispatcher")
    }));

    let (summary, code) = run(ctx).await.expect("the base repair is unchanged");
    assert_eq!(code, RunExit::Success);
    let text = line(&summary);
    assert!(!text.contains("persistent"), "{text}");
    for key in [
        "persistent",
        "persistent_files",
        "persistent_repaired",
        "persistent_failed",
        "persistent_base_hash",
    ] {
        assert!(
            !summary_record(&summary, 0).json().contains(key),
            "{key} leaked into an unflagged JSON line"
        );
    }
    mock.stop();
}

// ---------------------------------------------------------------------------
// scope guards and ceilings
// ---------------------------------------------------------------------------

/// C7(d): a marker file outside an active scope is a usage error, in the
/// wording style of the existing `--audio` scan-file guard.
#[tokio::test]
async fn a_marker_file_out_of_scope_exits_one() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let server = manifest_server(RES);
    let dir = temp_dir("persistent_marker_scope");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, dispatch_for(&server, RES));
    ctx.path_scope = scope(&["game/*"], &[]);

    let err = run(ctx).await.expect_err("the marker path is install-wide");
    assert_eq!(err.exit_code, RunExit::Usage);
    let text = format!("{err:#}");
    assert!(text.contains("marker file"), "{text}");
    assert!(text.contains("outside the active path scope"), "{text}");
    mock.stop();
}

/// C7(f): a base collision forces a dispatcher row into `Persistent/`. Hiding
/// that hotfix path while admitting every marker still repairs the in-scope
/// work, but suppresses the marker block and reports it.
#[tokio::test]
async fn a_hotfix_path_out_of_scope_suppresses_markers() {
    let fx = build_fixture_spec(&girpr_test_fixtures::FixtureSpec::new(vec![
        girpr_test_fixtures::FileSpec::new(
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/11111111.blk",
            vec![girpr_test_fixtures::ChunkSpec::new(
                "chunk_block",
                vec![0x5a; 64],
                0,
            )],
        ),
    ]));
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let server = manifest_server(RES);
    let dir = temp_dir("persistent_hotfix_scope");
    write_intact_fixture_rows(&dir);
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, dispatch_for(&server, RES));
    ctx.path_scope = scope(
        &[],
        &["GenshinImpact_Data/Persistent/AssetBundles/blocks/00/11111111.blk"],
    );

    let (summary, code) = run(ctx).await.expect("the in-scope files still repair");
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.flags.persistent, Some("suppressed"));
    assert_eq!(summary.flags.persistent_files, Some(4));
    assert!(!dir.join("GenshinImpact_Data/Persistent/PatchDone").exists());
    mock.stop();
}

/// C11/B7: persistent full sizes are part of the pre-flight byte ceiling, so
/// a zero download budget refuses before any write.
#[tokio::test]
async fn persistent_bytes_are_counted_by_the_download_ceiling() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let server = manifest_server(RES);
    let dir = temp_dir("persistent_ceiling");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, dispatch_for(&server, RES));
    ctx.limits.download_bytes = Some(0);

    let (summary, code) = run(ctx).await.expect("a ceiling is a returned exit code");
    assert_eq!(code, RunExit::Limit);
    assert!(summary.flags.limited);
    assert_eq!(summary.flags.persistent_files, Some(5));
    assert!(
        !dir.join("GenshinImpact_Data").exists(),
        "nothing was written"
    );
    mock.stop();
}

/// C11/B7: `--max-files` joins the filtered base plan and the persistent plan.
/// Both halves fit under 5 on their own (2 base, 5 persistent is equal, not
/// over); the joined 7 does not. The separate checks let this run through.
#[tokio::test]
async fn max_files_joins_base_and_persistent_files() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let server = manifest_server(RES);
    let dir = temp_dir("persistent_joined_files_ceiling");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, dispatch_for(&server, RES));
    ctx.limits.files = Some(5); // 2 base + 5 persistent = 7

    let (result, logs) = logs_of(run(ctx)).await;
    let (summary, code) = result.expect("a refusal is a returned exit code");
    assert_eq!(code, RunExit::Limit);
    assert!(summary.flags.limited);
    assert_eq!(
        summary.files_total, 2,
        "the base plan is still reported on its own"
    );
    assert_eq!(summary.flags.persistent_files, Some(5));
    assert_eq!(summary.files_repaired.load(Ordering::Relaxed), 0);
    assert!(
        logs.contains("plan_files=7 limit=5"),
        "one summed hit names the joined figure: {logs}"
    );
    assert!(
        !dir.join("GenshinImpact_Data/Persistent/PatchDone").exists(),
        "nothing was published"
    );
    mock.stop();
}

/// C13: `--scan` reads only base files. The plan-out section says so instead
/// of leaving the exclusion implicit.
#[tokio::test]
async fn scan_exclusion_is_visible_in_plan_out() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let server = manifest_server(RES);
    let dir = temp_dir("persistent_scan_exclusion");
    write_intact_fixture_rows(&dir);
    let out = temp_dir("persistent_scan_exclusion_doc").join("plan.json");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, dispatch_for(&server, RES));
    ctx.plan_out = Some(out.clone());
    ctx.scan = true;

    run(ctx).await.expect("the scan succeeds");
    let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(doc["persistent"]["scan_excludes_persistent"], true);
    mock.stop();
}

/// The dispatcher version comes from `meta.latest`, not a stale local version:
/// even a fresh install (no config.ini) must make the same query.
#[tokio::test]
async fn a_fresh_install_still_queries_with_the_latest_version() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let server = manifest_server(RES);
    let dir = temp_dir("persistent_fresh_install");
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let info = dispatch_for(&server, RES);
    let log = seen.clone();
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.with_persistent = true;
    ctx.persistent_dispatch_override = Some(DispatchOverride::new(move |biz, latest| {
        log.lock().unwrap().push((biz, latest.to_owned()));
        Ok(info.clone())
    }));
    ctx.print_plan = true;

    let (summary, code) = run(ctx).await.expect("a fresh install can be planned");
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.flags.persistent, Some("listed"));
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        &[(Biz::Hk4eGlobal, mock_latest(&mock))]
    );
    mock.stop();
}

/// The latest tag is the fixture's fixed LATEST_TAG, restated here so the
/// version-authority assertion names the value the mock actually served.
fn mock_latest(_mock: &MockServer) -> String {
    girpr_test_fixtures::LATEST_TAG.to_owned()
}

// ---------------------------------------------------------------------------
// execute half
// ---------------------------------------------------------------------------

/// One manifest row naming the committed 64-byte hotfix blob. All three
/// manifests carry the same row, so the plan dedupes it to a single asset and
/// the tests can pin one HTTP exchange at a time.
fn single_manifest() -> Vec<u8> {
    format!(
        "{{\"remoteName\":\"blocks/00/11111111.blk\",\
         \"md5\":\"b2d3f56bc197fd985d5965079b5e7148\",\
         \"hash\":\"f7c67301db6713f0\",\"fileSize\":64}}\n"
    )
    .into_bytes()
}

/// A dispatcher whose manifests come from `manifest_server` and whose four CDN
/// prefixes all point at `hotfix_base`. The plan resolves one asset at
/// `GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/11111111.blk`.
fn single_asset_dispatch(
    manifest_server: &ScriptedServer,
    hotfix_base: &str,
) -> girpr::test_support::DispatchInfo {
    let body = single_manifest();
    let mut info = persistent_dispatch(
        manifest_server.base(),
        "GenshinImpact",
        [
            (ManifestKind::Res, "res.ndjson", &body),
            (ManifestKind::Data, "data.ndjson", &body),
            (ManifestKind::Silence, "silence.ndjson", &body),
        ],
    );
    info.client_game_res_url = format!("{hotfix_base}/game_res");
    info.client_audio_assets_url = format!("{hotfix_base}/audio_assets");
    info.client_design_data_url = format!("{hotfix_base}/design_data");
    info.client_silence_data_url = format!("{hotfix_base}/silence_data");
    info
}

fn manifest_server_for(body: &[u8]) -> ScriptedServer {
    ScriptedServer::start(vec![
        Reply::new(200, body),
        Reply::new(200, body),
        Reply::new(200, body),
    ])
}

fn hotfix_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/11111111.blk")
}

/// A primary 404 is the only status that switches to the alternate. Both URLs
/// name the same canned server; its first reply is the 404, its second the
/// verified blob, so the two requests must both happen and the file must land.
#[tokio::test]
async fn a_hotfix_download_falls_back_to_the_alternate_on_404() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let manifests = manifest_server_for(&single_manifest());
    let hotfix = ScriptedServer::start(vec![Reply::new(404, ""), Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_exec_404_fallback");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));

    let (summary, code) = run(ctx).await.expect("the run completes");
    assert_eq!(code, RunExit::Success);
    assert_eq!(hotfix.hits(), 2, "primary then alternate, nothing else");
    assert_eq!(std::fs::read(hotfix_path(&dir)).unwrap(), HOTFIX_A);
    assert_eq!(summary.flags.persistent_files, Some(1));
    assert_eq!(summary.flags.persistent_repaired, Some(1));
    assert_eq!(summary.flags.persistent_failed, Some(0));
    assert!(summary.download.settled() >= 64, "the hotfix bytes settled");
    assert!(summary.write.settled() >= 64, "the hotfix bytes landed");
    assert!(line(&summary).contains("persistent=published"));
    // C19: a clean run publishes the whole block and all three mirrors, even
    // though the hotfix needed a download (the alternate delivered it here).
    let persistent = dir.join("GenshinImpact_Data/Persistent");
    assert!(persistent.join("PatchDone").is_file());
    for mirror in [
        "res_versions_persist",
        "data_versions_persist",
        "silence_data_versions_persist",
    ] {
        assert_eq!(
            std::fs::read(persistent.join(mirror)).unwrap(),
            single_manifest(),
            "{mirror} is the raw verified manifest"
        );
    }
    mock.stop();
}

/// B3: the write ceiling is charged before the downloader publishes, so a zero
/// budget leaves no hotfix behind and the run stops with exit 5 rather than
/// counting a failed file (which would exit 3).
#[tokio::test]
async fn a_zero_write_ceiling_publishes_no_persistent_file() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let manifests = manifest_server_for(&single_manifest());
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_write_ceiling_zero");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));
    ctx.limits.write_bytes = Some(0);

    let (summary, code) = run(ctx).await.expect("a ceiling is an exit code");
    assert_eq!(code, RunExit::Limit, "a runtime hard stop is exit 5");
    assert_eq!(summary.flags.stopped.as_deref(), Some("--max-write-bytes"));
    assert_eq!(summary.flags.persistent_repaired, Some(0));
    assert_eq!(
        summary.flags.persistent_failed,
        Some(0),
        "a stop, not a failure"
    );
    assert_eq!(
        summary.files_failed.load(Ordering::Relaxed),
        0,
        "a failed file would outrank the ceiling"
    );
    assert!(
        !hotfix_path(&dir).exists(),
        "no hotfix bytes were published"
    );
    assert!(
        !dir.join("GenshinImpact_Data/Persistent/PatchDone").exists(),
        "no marker block was published either"
    );
    mock.stop();
}

/// The settled write ceiling uses the same pre-publish seam and names its own
/// flag in `stopped=`; it too publishes nothing.
#[tokio::test]
async fn a_zero_verified_write_ceiling_publishes_no_persistent_file() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let manifests = manifest_server_for(&single_manifest());
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_verified_write_ceiling_zero");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));
    ctx.limits.verified_write_bytes = Some(0);

    let (summary, code) = run(ctx).await.expect("a ceiling is an exit code");
    assert_eq!(code, RunExit::Limit);
    assert_eq!(
        summary.flags.stopped.as_deref(),
        Some("--max-verified-write-bytes")
    );
    assert_eq!(summary.files_failed.load(Ordering::Relaxed), 0);
    assert!(
        !hotfix_path(&dir).exists(),
        "no hotfix bytes were published"
    );
    mock.stop();
}

/// At the boundary the bytes are charged exactly once and the hotfix lands.
#[tokio::test]
async fn a_write_ceiling_at_the_boundary_charges_the_hotfix_once() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let manifests = manifest_server_for(&single_manifest());
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_write_ceiling_boundary");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));
    ctx.limits.write_bytes = Some(64);

    let (summary, code) = run(ctx).await.expect("the boundary passes");
    assert_eq!(code, RunExit::Success);
    assert!(summary.flags.stopped.is_none());
    assert_eq!(summary.flags.persistent_repaired, Some(1));
    assert_eq!(summary.write.settled(), 64);
    assert_eq!(summary.write.live(), 64);
    assert_eq!(
        std::fs::read(hotfix_path(&dir)).unwrap(),
        HOTFIX_A,
        "the charged bytes are the published ones"
    );
    mock.stop();
}

/// A body that passes the size check but fails XXH64 must not fall back to the
/// alternate: the gate is decided by the URL that delivered the bytes. The run
/// exits 3, skips the post phase and leaves no published file.
#[tokio::test]
async fn a_corrupt_hotfix_fails_the_gate_and_publishes_nothing() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let manifests = manifest_server_for(&single_manifest());
    let corrupt = vec![0xabu8; 64];
    let hotfix = ScriptedServer::start(vec![Reply::new(200, &corrupt)]);
    let dir = temp_dir("persistent_exec_corrupt");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));

    let (summary, code) = run(ctx)
        .await
        .expect("a failed file is a returned exit code");
    assert_eq!(code, RunExit::Write);
    assert_eq!(
        hotfix.hits(),
        1,
        "a digest failure never tries the alternate"
    );
    assert!(!hotfix_path(&dir).exists(), "nothing was published");
    assert_eq!(summary.flags.persistent, Some("failed"));
    assert_eq!(summary.flags.persistent_repaired, Some(0));
    assert_eq!(summary.flags.persistent_failed, Some(1));
    let persistent = dir.join("GenshinImpact_Data/Persistent");
    assert!(!persistent.join("PatchDone").exists());
    assert!(!persistent.join("res_versions_persist").exists());
    mock.stop();
}

/// Gap rows 9-11: both trees, stale `ctable.dat`, `DXSETUP/` and the copy-back
/// of `ctable.dat` → `ctable_streaming.dat`; plus the Collapse move of a
/// stranded non-patch `.pck` out of `Persistent/`.
#[tokio::test]
async fn prerequisites_clean_stale_state_and_restore_ctable_streaming() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let manifests = manifest_server_for(&single_manifest());
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_exec_prereqs");
    let data = dir.join("GenshinImpact_Data");
    write_game_file(&dir, "GenshinImpact_Data/Persistent/ctable.dat", b"stale");
    write_game_file(&dir, "DXSETUP/setup.exe", b"junk");
    let source = b"launcher-ctable";
    write_game_file(
        &dir,
        "GenshinImpact_Data/StreamingAssets/ctable.dat",
        source,
    );
    write_game_file(
        &dir,
        "GenshinImpact_Data/StreamingAssets/AudioAssets/English(US)/stranded.pck",
        b"old",
    );
    write_game_file(
        &dir,
        "GenshinImpact_Data/Persistent/AudioAssets/English(US)/stranded.pck",
        b"stranded",
    );
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));

    let (summary, code) = run(ctx).await.expect("the run completes");
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.flags.persistent_repaired, Some(1));
    assert!(data.join("Persistent").is_dir());
    assert!(data.join("StreamingAssets").is_dir());
    assert!(!data.join("Persistent/ctable.dat").exists());
    assert!(!dir.join("DXSETUP").exists());
    assert_eq!(
        std::fs::read(data.join("StreamingAssets/ctable_streaming.dat")).unwrap(),
        source
    );
    assert!(
        !data
            .join("Persistent/AudioAssets/English(US)/stranded.pck")
            .exists()
    );
    assert_eq!(
        std::fs::read(data.join("StreamingAssets/AudioAssets/English(US)/stranded.pck")).unwrap(),
        b"stranded"
    );
    mock.stop();
}

/// A missing `StreamingAssets/ctable.dat` is a no-op, and an existing
/// `ctable_streaming.dat` is never replaced by nothing.
#[tokio::test]
async fn a_missing_ctable_source_leaves_the_verified_target_alone() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let manifests = manifest_server_for(&single_manifest());
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_exec_no_ctable_source");
    let sentinel = b"verified-sophon-file";
    write_game_file(
        &dir,
        "GenshinImpact_Data/StreamingAssets/ctable_streaming.dat",
        sentinel,
    );
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));

    let (_, code) = run(ctx).await.expect("the run completes");
    assert_eq!(code, RunExit::Success);
    assert_eq!(
        std::fs::read(dir.join("GenshinImpact_Data/StreamingAssets/ctable_streaming.dat")).unwrap(),
        sentinel
    );
    mock.stop();
}

/// C17(b): a read-only hotfix target is cleared on that one path immediately
/// before publish, not stripped tree-wide. The verified bytes land at the
/// target itself, with write permission restored.
#[tokio::test]
async fn a_read_only_hotfix_target_is_overwritten() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let manifests = manifest_server_for(&single_manifest());
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_exec_read_only_target");
    let target = hotfix_path(&dir);
    write_game_file(
        &dir,
        "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/11111111.blk",
        b"wrong-bytes",
    );
    let mut permissions = std::fs::metadata(&target).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&target, permissions).unwrap();
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));

    let (summary, code) = run(ctx).await.expect("the run completes");
    assert_eq!(code, RunExit::Success);
    assert_eq!(std::fs::read(&target).unwrap(), HOTFIX_A);
    assert!(!std::fs::metadata(&target).unwrap().permissions().readonly());
    assert_eq!(summary.flags.persistent_repaired, Some(1));
    mock.stop();
}

/// C4: `--dry-run` and `--check-only` list the persistent work but open no
/// hotfix request and create no tree.
#[tokio::test]
async fn read_only_modes_list_without_a_single_hotfix_request() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("persistent_exec_readonly");
    for (check_only, dry_run) in [(true, false), (false, true)] {
        let manifests = manifest_server_for(&single_manifest());
        let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
        flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));
        ctx.check_only = check_only;
        ctx.dry_run = dry_run;

        let (summary, _) = run(ctx).await.expect("a read-only run completes");
        assert_eq!(summary.flags.persistent, Some("listed"));
        assert_eq!(summary.flags.persistent_files, Some(1));
        assert_eq!(hotfix.hits(), 0, "no hotfix byte was requested");
        assert!(!dir.join("GenshinImpact_Data/Persistent").exists());
    }
    mock.stop();
}

// ---------------------------------------------------------------------------
// post phase: publish the full state last, then bump the version (C9)
// ---------------------------------------------------------------------------

fn persistent_file(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("GenshinImpact_Data/Persistent/{name}"))
}

/// A flagged run whose single hotfix is already intact: the post phase must
/// publish the whole block (C19) without spending a download byte.
async fn publish_clean(
    dir: &Path,
    mock: &MockServer,
    manifests: &ScriptedServer,
    hotfix: &ScriptedServer,
) -> (Summary, RunExit) {
    write_game_file(dir, &hotfix_rel(), HOTFIX_A);
    let mut ctx = ctx_for(dir.to_path_buf(), mock, 1);
    flagged(&mut ctx, single_asset_dispatch(manifests, hotfix.base()));
    run(ctx).await.expect("the clean flagged run succeeds")
}

fn hotfix_rel() -> String {
    "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/11111111.blk".to_owned()
}

/// Every marker path and byte, including the `res_revision_eternal` twin and
/// `PatchDone = audio_revision`, with no trailing newline. All three mirrors
/// carry the raw manifest bytes, and the version bump still happens last.
#[tokio::test]
async fn markers_and_all_three_mirrors_are_published_byte_exact() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let body = single_manifest();
    let manifests = manifest_server_for(&body);
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_post_bytes");
    let source = b"res_versions_streaming bytes";
    write_game_file(
        &dir,
        "GenshinImpact_Data/StreamingAssets/res_versions_streaming",
        source,
    );
    std::fs::write(
        dir.join("config.ini"),
        format!("[General]\ngame_version={LOCAL_TAG}\n"),
    )
    .unwrap();

    let (summary, code) = publish_clean(&dir, &mock, &manifests, &hotfix).await;
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.flags.persistent, Some("published"));
    assert_eq!(summary.flags.persistent_repaired, Some(0));
    assert_eq!(summary.flags.persistent_failed, Some(0));
    assert_eq!(summary.flags.persistent_base_hash, None);
    assert_eq!(
        hotfix.hits(),
        0,
        "nothing needed repair, yet all bytes published"
    );

    for (name, value) in [
        ("base_res_version_hash", md5hex(source)),
        ("res_revision", "49530578".to_owned()),
        ("res_revision_eternal", "49530578".to_owned()),
        ("data_revision", "49419538".to_owned()),
        ("silence_revision", "49291486".to_owned()),
        ("audio_revision", "49008628".to_owned()),
        ("PatchDone", "49008628".to_owned()),
        ("ChannelName", "OSRELWin".to_owned()),
        ("ScriptVersion", "7.2_live".to_owned()),
    ] {
        assert_eq!(
            std::fs::read(persistent_file(&dir, name)).unwrap(),
            value.as_bytes(),
            "{name} is the exact plain-text value with no trailing newline"
        );
    }
    // The two twins are written from one dispatcher value, never each other's
    // stale copy, and the base hash is the MD5 of the local source.
    assert_eq!(
        std::fs::read(persistent_file(&dir, "res_revision_eternal")).unwrap(),
        std::fs::read(persistent_file(&dir, "res_revision")).unwrap()
    );
    assert_eq!(
        std::fs::read(persistent_file(&dir, "PatchDone")).unwrap(),
        std::fs::read(persistent_file(&dir, "audio_revision")).unwrap()
    );

    for name in [
        "res_versions_persist",
        "data_versions_persist",
        "silence_data_versions_persist",
    ] {
        assert_eq!(
            std::fs::read(persistent_file(&dir, name)).unwrap(),
            body,
            "the {name} mirror is the raw verified manifest"
        );
    }

    // Markers are before the bump: the bump still ran on a clean publish.
    let cfg = std::fs::read_to_string(dir.join("config.ini")).unwrap();
    assert!(cfg.contains(&format!("game_version={LATEST_TAG}")), "{cfg}");
    mock.stop();
}

/// C16: the absent streaming source leaves exactly one marker out, and the
/// omission is visible in `SUMMARY` rather than failing the run.
#[tokio::test]
async fn a_missing_base_hash_source_omits_only_that_marker() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let body = single_manifest();
    let manifests = manifest_server_for(&body);
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_post_omit_hash");

    let (summary, code) = publish_clean(&dir, &mock, &manifests, &hotfix).await;
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.flags.persistent, Some("published"));
    assert_eq!(summary.flags.persistent_base_hash, Some("omitted"));
    assert!(
        line(&summary).contains("persistent_base_hash=omitted"),
        "{}",
        line(&summary)
    );
    assert!(
        !persistent_file(&dir, "base_res_version_hash").exists(),
        "C16 omits the marker entirely"
    );
    for name in [
        "res_revision",
        "res_revision_eternal",
        "data_revision",
        "silence_revision",
        "audio_revision",
        "PatchDone",
        "ChannelName",
        "ScriptVersion",
    ] {
        assert!(
            persistent_file(&dir, name).is_file(),
            "{name} is still published"
        );
    }
    mock.stop();
}

/// C4: a dry run logs one listing line per marker, writes no marker or mirror,
/// and leaves the launcher version untouched.
#[tokio::test]
async fn a_dry_run_lists_every_marker_and_writes_nothing() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let body = single_manifest();
    let manifests = manifest_server_for(&body);
    let dir = temp_dir("persistent_post_dry_run");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(
        &mut ctx,
        single_asset_dispatch(&manifests, "http://127.0.0.1:1"),
    );
    ctx.dry_run = true;

    let (result, logs) = logs_of(run(ctx)).await;
    let (summary, code) = result.expect("a dry run completes");
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.flags.persistent, Some("listed"));

    let listed: Vec<&str> = logs
        .lines()
        .filter(|l| l.contains("would write marker"))
        .collect();
    assert_eq!(
        listed.len(),
        8,
        "one line per marker, none per mirror: {logs}"
    );
    for name in [
        "res_revision",
        "res_revision_eternal",
        "data_revision",
        "silence_revision",
        "audio_revision",
        "PatchDone",
        "ChannelName",
        "ScriptVersion",
    ] {
        assert!(
            listed
                .iter()
                .any(|l| l.contains(&format!("/Persistent/{name}"))),
            "{name} was not listed: {logs}"
        );
    }
    assert!(!dir.join("GenshinImpact_Data/Persistent").exists());
    assert!(!dir.join("config.ini").exists());
    mock.stop();
}

/// C20: a cleanup-only run puts no bytes in place, so it publishes no state.
#[tokio::test]
async fn purge_only_publishes_no_markers_or_mirrors() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("persistent_post_purge_only");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.with_persistent = true;
    ctx.purge_only = true;
    ctx.persistent_dispatch_override = Some(DispatchOverride::new(|_, _| {
        panic!("--purge-only must not query the dispatcher")
    }));

    let (summary, code) = run(ctx).await.expect("the cleanup still runs");
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.flags.persistent, Some("skipped"));
    assert!(
        !dir.join("GenshinImpact_Data/Persistent").exists(),
        "no revision claim without bytes in place"
    );
    assert!(!dir.join("config.ini").exists());
    mock.stop();
}

/// A refused marker publish is a write failure (exit 3) that skips the version
/// bump: the config file is absent even though mirrors were written first.
#[tokio::test]
async fn a_failed_marker_publish_exits_three_without_bumping_the_version() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let body = single_manifest();
    let manifests = manifest_server_for(&body);
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_post_publish_failure");
    write_game_file(&dir, &hotfix_rel(), HOTFIX_A);
    // The first marker is the write that fails; a directory cannot be
    // atomically replaced by a file.
    std::fs::create_dir_all(persistent_file(&dir, "res_revision")).unwrap();

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));
    let err = run(ctx).await.expect_err("the marker publish is fatal");
    assert_eq!(err.exit_code, RunExit::Write);
    assert!(
        !dir.join("config.ini").exists(),
        "a failed marker publish must skip the config.ini bump"
    );
    assert_eq!(
        std::fs::read(persistent_file(&dir, "res_versions_persist")).unwrap(),
        body,
        "mirrors are written before the marker block"
    );
    assert!(
        !persistent_file(&dir, "PatchDone").exists(),
        "the marker block stops at the first failed write"
    );
    mock.stop();
}

/// C5: with the flag, a clean check of a published install exits 0 and still
/// reports `persistent=listed`.
#[tokio::test]
async fn a_persistent_check_of_an_intact_install_succeeds() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let body = single_manifest();
    let manifests = manifest_server_for(&body);
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_check_clean");
    let (_, code) = publish_clean(&dir, &mock, &manifests, &hotfix).await;
    assert_eq!(code, RunExit::Success);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));
    ctx.check_only = true;
    let (summary, code) = run(ctx).await.expect("the check completes");
    assert_eq!(code, RunExit::Success, "the published install is intact");
    assert_eq!(summary.flags.persistent, Some("listed"));
    mock.stop();
}

/// C5(a): a damaged persistent file is damage, even though it is not part of
/// the Sophon plan.
#[tokio::test]
async fn a_damaged_persistent_file_makes_check_only_exit_four() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let body = single_manifest();
    let manifests = manifest_server_for(&body);
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_check_file_damage");
    publish_clean(&dir, &mock, &manifests, &hotfix).await;
    std::fs::write(hotfix_path(&dir), vec![0xabu8; 64]).unwrap();

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));
    ctx.check_only = true;
    let (summary, code) = run(ctx).await.expect("the check completes");
    assert_eq!(code, RunExit::CheckFailed);
    assert_eq!(summary.flags.persistent, Some("listed"));
    mock.stop();
}

/// C5(b): a replaced mirror is damage, even with every hotfix byte intact.
#[tokio::test]
async fn a_damaged_mirror_makes_check_only_exit_four() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let body = single_manifest();
    let manifests = manifest_server_for(&body);
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_check_mirror_damage");
    publish_clean(&dir, &mock, &manifests, &hotfix).await;
    std::fs::write(
        persistent_file(&dir, "res_versions_persist"),
        b"not the manifest",
    )
    .unwrap();

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));
    ctx.check_only = true;
    let (summary, code) = run(ctx).await.expect("the check completes");
    assert_eq!(code, RunExit::CheckFailed);
    assert_eq!(summary.flags.persistent, Some("listed"));
    mock.stop();
}

/// C5(c): a wrong marker value is damage, including a missing newline or an
/// old revision left behind by a previous run.
#[tokio::test]
async fn a_damaged_marker_makes_check_only_exit_four() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let body = single_manifest();
    let manifests = manifest_server_for(&body);
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_check_marker_damage");
    publish_clean(&dir, &mock, &manifests, &hotfix).await;
    std::fs::write(persistent_file(&dir, "PatchDone"), "49008628\n").unwrap();

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));
    ctx.check_only = true;
    let (summary, code) = run(ctx).await.expect("the check completes");
    assert_eq!(code, RunExit::CheckFailed);
    assert_eq!(summary.flags.persistent, Some("listed"));
    mock.stop();
}

// ---------------------------------------------------------------------------
// review fixes: post-repair base hash, mirror scope, exit precedence
// ---------------------------------------------------------------------------

/// C7(d) also covers the mirrors post writes: excluding a mirror while all
/// markers remain in scope is a usage error (exit 1), not an out-of-scope write.
#[tokio::test]
async fn a_mirror_file_out_of_scope_exits_one() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let server = manifest_server(RES);
    let dir = temp_dir("persistent_mirror_scope");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, dispatch_for(&server, RES));
    ctx.path_scope = scope(
        &["GenshinImpact_Data/Persistent/*"],
        &["GenshinImpact_Data/Persistent/*_versions_persist"],
    );

    let err = run(ctx)
        .await
        .expect_err("a mirror post would write must be in scope");
    assert_eq!(err.exit_code, RunExit::Usage);
    let text = format!("{err:#}");
    assert!(text.contains("mirror file"), "{text}");
    assert!(text.contains("outside the active path scope"), "{text}");
    mock.stop();
}

/// A fixture whose one base file is the streaming source the persistent marker
/// hashes. The repair loop can therefore correct or create it before post.
fn streaming_source_fixture(source: &[u8]) -> girpr_test_fixtures::GameFixture {
    build_fixture_spec(&girpr_test_fixtures::FixtureSpec::new(vec![
        girpr_test_fixtures::FileSpec::new(
            "GenshinImpact_Data/StreamingAssets/res_versions_streaming",
            vec![girpr_test_fixtures::ChunkSpec::new(
                "stream_chunk",
                source.to_vec(),
                0,
            )],
        ),
    ]))
}

fn streaming_source_path(dir: &Path) -> PathBuf {
    dir.join("GenshinImpact_Data/StreamingAssets/res_versions_streaming")
}

/// A damaged streaming source is corrected by the base repair. The published
/// hash must describe the repaired bytes, not the pre-repair bytes the plan
/// read while rendering the marker.
#[tokio::test]
async fn a_repaired_streaming_source_publishes_the_fresh_base_hash() {
    let source = b"res_versions_streaming bytes";
    let mock = MockServer::start(streaming_source_fixture(source), MockOpts::default()).await;
    let body = single_manifest();
    let manifests = manifest_server_for(&body);
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_fresh_hash_repaired_source");
    let stale = vec![b'x'; source.len()];
    write_game_file(&dir, &hotfix_rel(), HOTFIX_A);
    std::fs::create_dir_all(streaming_source_path(&dir).parent().unwrap()).unwrap();
    std::fs::write(streaming_source_path(&dir), &stale).unwrap();
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));

    let (summary, code) = run(ctx).await.expect("the run completes");
    assert_eq!(code, RunExit::Success);
    assert_eq!(
        std::fs::read(streaming_source_path(&dir)).unwrap(),
        source,
        "the base repair corrected the damaged source"
    );
    assert_eq!(
        std::fs::read(persistent_file(&dir, "base_res_version_hash")).unwrap(),
        md5hex(source).as_bytes(),
        "the marker describes the repaired bytes"
    );
    assert_ne!(
        std::fs::read(persistent_file(&dir, "base_res_version_hash")).unwrap(),
        md5hex(&stale).as_bytes(),
        "the pre-repair hash is never published"
    );
    assert_eq!(
        summary.flags.persistent_base_hash, None,
        "the source exists, so nothing is omitted"
    );
    assert!(
        !line(&summary).contains("persistent_base_hash=omitted"),
        "{}",
        line(&summary)
    );
    mock.stop();
}

/// On a fresh install the plan omits the base hash marker (C16), but the base
/// repair creates the source. Post adds the marker and clears the omit flag.
#[tokio::test]
async fn a_streaming_source_created_by_repair_adds_the_base_hash_marker() {
    let source = b"created by the base repair";
    let mock = MockServer::start(streaming_source_fixture(source), MockOpts::default()).await;
    let body = single_manifest();
    let manifests = manifest_server_for(&body);
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_fresh_hash_created_source");
    write_game_file(&dir, &hotfix_rel(), HOTFIX_A);
    // No streaming source on disk: the plan renders only the eight C16 markers.
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));

    let (summary, code) = run(ctx).await.expect("the run completes");
    assert_eq!(code, RunExit::Success);
    assert_eq!(
        std::fs::read(streaming_source_path(&dir)).unwrap(),
        source,
        "the repair created the source"
    );
    assert_eq!(
        std::fs::read(persistent_file(&dir, "base_res_version_hash")).unwrap(),
        md5hex(source).as_bytes(),
        "the marker that C16 omitted at plan time is published"
    );
    assert_eq!(
        summary.flags.persistent_base_hash, None,
        "the omit status changed and SUMMARY follows it"
    );
    assert!(
        !line(&summary).contains("persistent_base_hash=omitted"),
        "{}",
        line(&summary)
    );
    mock.stop();
}

/// Check-only re-reads the same source, so it agrees with the freshly published
/// block instead of comparing against the bytes the plan saw.
#[tokio::test]
async fn check_only_agrees_with_the_published_fresh_base_hash() {
    let source = b"res_versions_streaming bytes";
    let mock = MockServer::start(streaming_source_fixture(source), MockOpts::default()).await;
    let body = single_manifest();
    let manifests = manifest_server_for(&body);
    let hotfix = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
    let dir = temp_dir("persistent_check_fresh_hash");
    let stale = vec![b'x'; source.len()];
    write_game_file(&dir, &hotfix_rel(), HOTFIX_A);
    std::fs::create_dir_all(streaming_source_path(&dir).parent().unwrap()).unwrap();
    std::fs::write(streaming_source_path(&dir), &stale).unwrap();
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));
    let (_, code) = run(ctx).await.expect("the repair completes");
    assert_eq!(code, RunExit::Success);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));
    ctx.check_only = true;
    let (summary, code) = run(ctx).await.expect("the check completes");
    assert_eq!(code, RunExit::Success, "the published block is intact");
    assert_eq!(summary.flags.persistent, Some("listed"));
    mock.stop();
}

/// A failed persistent file and a later ceiling stop are both visible, but the
/// failure wins the exit code: 3, not 5. `stopped=` still names the ceiling.
#[tokio::test]
async fn a_failed_file_outranks_a_later_ceiling_stop() {
    let fx = build_fixture();
    let mock = MockServer::start(fx.clone(), MockOpts::default()).await;
    let body = single_manifest();
    let manifests = manifest_server_for(&body);
    let corrupt = vec![0xabu8; 64];
    let hotfix = ScriptedServer::start(vec![Reply::new(200, &corrupt)]);
    let dir = temp_dir("persistent_failure_beats_ceiling");
    // A corrupt hotfix fails before the base repair; those base files need
    // writes, so the write ceiling stops them a moment later.
    for f in &fx.files {
        write_game_file(&dir, &f.rel, b"not the game data");
    }
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, single_asset_dispatch(&manifests, hotfix.base()));
    ctx.limits.write_bytes = Some(1);

    let (summary, code) = run(ctx).await.expect("a stop is not an error");
    assert_eq!(code, RunExit::Write, "a failed file is exit 3");
    assert_eq!(summary.flags.persistent, Some("failed"));
    assert_eq!(summary.flags.persistent_failed, Some(1));
    assert_eq!(
        summary.flags.stopped,
        Some("--max-write-bytes"),
        "the ceiling is still recorded"
    );
    assert!(
        line(&summary).contains("stopped=--max-write-bytes"),
        "{}",
        line(&summary)
    );
    assert!(
        !dir.join("config.ini").exists(),
        "the post-phase is skipped on failure or ceiling"
    );
    mock.stop();
}
