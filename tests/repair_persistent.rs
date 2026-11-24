//! Offline contract cases for the plan half of `--with-persistent` (commit 6).
//!
//! The dispatcher is replaced by the `DispatchOverride` seam; the three
//! manifests come from the committed synthetic fixtures served on a loopback
//! origin. No case opens a production URL and no case writes a marker, a
//! mirror or a hotfix file.

use std::sync::Arc;

use crate::common::{
    MockOpts, MockServer, build_fixture, build_fixture_spec, ctx_for, run, scope, temp_dir,
    write_game_file,
};
use girpr_loopback_mock::{Reply, ScriptedServer};

use girpr::test_support::{
    DispatchOverride, ManifestKind, failing_dispatch_override, persistent_dispatch,
    persistent_override, summary_record,
};
use girpr::{Biz, RunCtx, RunExit, RunFailure, Summary};

const RES: &[u8] =
    include_bytes!("../tools/persistent-sophon-research/samples/fixtures/res.ndjson");
const DATA: &[u8] =
    include_bytes!("../tools/persistent-sophon-research/samples/fixtures/data.ndjson");
const SILENCE: &[u8] =
    include_bytes!("../tools/persistent-sophon-research/samples/fixtures/silence.ndjson");

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

/// C13: `--scan` reads only base files. The plan-out section says so instead
/// of leaving the exclusion implicit.
#[tokio::test]
async fn scan_exclusion_is_visible_in_plan_out() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let server = manifest_server(RES);
    let dir = temp_dir("persistent_scan_exclusion");
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
