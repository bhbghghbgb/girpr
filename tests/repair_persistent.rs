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
use girpr_loopback_mock::{Reply, ScriptedServer};

use girpr::test_support::{
    DispatchOverride, ManifestKind, failing_dispatch_override, persistent_dispatch,
    persistent_override, summary_record,
};
use girpr::{Biz, RunCtx, RunExit, RunFailure, Summary};

const RES: &[u8] = include_bytes!("fixtures/persistent_inputs/samples/fixtures/res.ndjson");
const DATA: &[u8] = include_bytes!("fixtures/persistent_inputs/samples/fixtures/data.ndjson");
const SILENCE: &[u8] = include_bytes!("fixtures/persistent_inputs/samples/fixtures/silence.ndjson");
const HOTFIX_A: &[u8] = include_bytes!("fixtures/persistent_inputs/samples/fixtures/hotfix_a.blk");
const HOTFIX_B: &[u8] = include_bytes!("fixtures/persistent_inputs/samples/fixtures/hotfix_b.pck");

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

/// A cleanup-only run still queries the dispatcher: the manifests widen the
/// purge keep set so existing hotfix bytes under `Persistent/` survive. A
/// forced-persistent row is kept only by that set — no filename rule protects
/// it — while an ordinary orphan beside it is still purged.
#[tokio::test]
async fn purge_only_queries_the_dispatcher_and_keeps_manifest_files() {
    // A base file with different bytes at the row's default rel forces the
    // dispatcher row into `Persistent/`, where a plain name has no keep rule.
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
    let dir = temp_dir("persistent_purge_only_keeps_manifest_files");
    let hotfix_rel = "GenshinImpact_Data/Persistent/AssetBundles/blocks/00/11111111.blk";
    write_game_file(&dir, hotfix_rel, b"hotfix bytes");
    write_game_file(&dir, "GenshinImpact_Data/Persistent/orphan.bin", b"orphan");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, dispatch_for(&server, RES));
    ctx.purge_only = true;

    let (summary, code) = run(ctx).await.expect("the cleanup still runs");
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.flags.persistent, Some("skipped"));
    // The dispatcher was queried: its five planned rows are visible in SUMMARY
    // even though none of them is downloaded or published.
    assert_eq!(summary.flags.persistent_files, Some(5));
    assert_eq!(
        std::fs::read(dir.join(hotfix_rel)).unwrap(),
        b"hotfix bytes",
        "a manifest file under Persistent survives a cleanup-only run"
    );
    assert!(
        !dir.join("GenshinImpact_Data/Persistent/orphan.bin")
            .exists(),
        "an orphan with no keep rule is still purged"
    );
    assert!(!dir.join("GenshinImpact_Data/Persistent/PatchDone").exists());
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
    "{\"remoteName\":\"blocks/00/11111111.blk\",\
         \"md5\":\"b2d3f56bc197fd985d5965079b5e7148\",\
         \"hash\":\"f7c67301db6713f0\",\"fileSize\":64}\n"
        .to_string()
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
    assert_eq!(summary.flags.stopped, Some("--max-write-bytes"));
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
    assert_eq!(summary.flags.stopped, Some("--max-verified-write-bytes"));
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

/// A stranded AV file whose `StreamingAssets` destination is in a manifest is
/// relocated before `--purge-before` acts. The purge plan was walked in the
/// plan half, before the move, and names the stranded source as an orphan;
/// hoisting the move out of `prerequisites` and ahead of that purge keeps the
/// moved file, which the repair pass then finds intact. Previously the purge
/// deleted the source first and no destination was ever created.
#[tokio::test]
async fn a_stranded_av_file_with_a_manifest_destination_survives_purge_before() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let server = manifest_server(RES);
    let dir = temp_dir("persistent_av_move_before_purge");
    let source_rel = "GenshinImpact_Data/Persistent/AudioAssets/English(US)/VO_TEST.pck";
    let dest_rel = "GenshinImpact_Data/StreamingAssets/AudioAssets/English(US)/VO_TEST.pck";
    // Every other planned row is already intact, so the run opens no hotfix
    // request; the only work is the relocation itself.
    for (rel, bytes) in [
        (
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/11111111.blk",
            HOTFIX_A,
        ),
        (
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/22222222.blk",
            HOTFIX_B,
        ),
        (
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/33333333.blk",
            HOTFIX_A,
        ),
        (
            "GenshinImpact_Data/StreamingAssets/hardware_model_config.json",
            HOTFIX_B,
        ),
    ] {
        write_game_file(&dir, rel, bytes);
    }
    // HOTFIX_B is the manifest's exact 48-byte body for this .pck row.
    write_game_file(&dir, source_rel, HOTFIX_B);
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, dispatch_for(&server, RES));
    ctx.purge_before = true;

    let (summary, code) = run(ctx).await.expect("the run completes");
    assert_eq!(code, RunExit::Success);
    assert!(
        !dir.join(source_rel).exists(),
        "the stranded source was moved, not left behind"
    );
    assert_eq!(
        std::fs::read(dir.join(dest_rel)).unwrap(),
        HOTFIX_B,
        "the destination is in the manifest, so the purge kept it"
    );
    assert_eq!(summary.flags.persistent_repaired, Some(0));
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

/// A cleanup-only run queries the dispatcher for the keep set but puts no
/// bytes in place, so it publishes no marker block, no mirror and no version.
#[tokio::test]
async fn purge_only_publishes_no_markers_or_mirrors() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let server = manifest_server(RES);
    let dir = temp_dir("persistent_post_purge_only");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    flagged(&mut ctx, dispatch_for(&server, RES));
    ctx.purge_only = true;

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
