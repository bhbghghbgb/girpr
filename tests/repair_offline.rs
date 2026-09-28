//! Offline integration / e2e tests for `girpr`.
//!
//! Everything runs against a local mock HoYoPlay + Sophon server
//! (`mod common`, `127.0.0.1:<ephemeral>`). No external network is touched:
//! every `RunCtx` sets `hyp_base_override` / `sophon_base_override` to the
//! mock, so a sandbox without internet still passes.
//!
//! Coverage (full `repair::run` pipeline per test):
//! 1. repair of missing + corrupt files (exit 0, bytes verified, tmp swept)
//! 2. `--check-only` clean (exit 0) and dirty (exit 4, writes nothing)
//! 3. `--dry-run` writes nothing
//! 4. `--purge-after` / `--purge-before` delete orphans into one counter
//! 5. deprecated files deleted in the post phase
//! 6. verified local-slice reuse downloads only the bad chunk (S2)
//! 7. partial `_tmp` prefix resumed by length, gated by final MD5 (S3)
//! 8. explicit audio selection overwrites the scan file
//! 9. a non-chunk `default_download_mode` is refused with exit 1
//! 10. the deprecated-files call carries the per-launcher channel

#[path = "common/mod.rs"]
mod common;

use common::{build_fixture, read_game_file, temp_dir, write_game_file, MockOpts, MockServer};
use girpr::{repair, Biz, RunCtx};
use std::collections::HashSet;
use std::sync::atomic::Ordering;

fn ctx_for(game_dir: std::path::PathBuf, mock: &MockServer, jobs: usize) -> RunCtx {
    ctx_for_biz(game_dir, mock, jobs, Biz::Hk4eGlobal)
}

fn ctx_for_biz(
    game_dir: std::path::PathBuf,
    mock: &MockServer,
    jobs: usize,
    biz: Biz,
) -> RunCtx {
    RunCtx {
        game_dir,
        biz,
        audio: HashSet::new(),
        audio_explicit: false,
        jobs,
        check_only: false,
        dry_run: false,
        purge_after: false,
        purge_before: false,
        json_summary: false,
        hyp_base_override: Some(mock.hyp_base.clone()),
        sophon_base_override: Some(mock.sophon_base.clone()),
    }
}

fn load_counters(s: &girpr::Summary) -> (u64, u64, u64, u64, u64) {
    (
        s.files_skipped.load(Ordering::Relaxed),
        s.files_repaired.load(Ordering::Relaxed),
        s.files_failed.load(Ordering::Relaxed),
        s.download_bytes.load(Ordering::Relaxed),
        s.deleted_extra_bytes.load(Ordering::Relaxed),
    )
}

#[tokio::test]
async fn repairs_missing_and_corrupt_files_end_to_end() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("repair_ok");

    // Rebuild the expected bytes from a fresh fixture (same deterministic data).
    let fx2 = build_fixture();
    let foo = &fx2.files[0];
    let bar = &fx2.files[1];

    // bar intact -> skip; foo corrupt (same size, flipped tail) -> repair.
    write_game_file(&dir, &bar.rel, &bar.data);
    let mut bad_foo = foo.data.clone();
    for b in bad_foo.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &foo.rel, &bad_foo);

    let (summary, code) = repair::run(ctx_for(dir.clone(), &mock, 2))
        .await
        .expect("run must succeed");
    assert_eq!(code, 0, "repair exit");
    assert_eq!(summary.files_total, 2);
    let (skipped, repaired, failed, dl, _del) = load_counters(&summary);
    assert_eq!((skipped, repaired, failed), (1, 1, 0));
    assert!(dl > 0, "must have downloaded the bad chunk");

    // Bytes on disk now match the manifest; no `_tmp` leftovers.
    assert_eq!(read_game_file(&dir, &foo.rel).unwrap(), foo.data);
    assert_eq!(read_game_file(&dir, &bar.rel).unwrap(), bar.data);
    assert!(
        !dir.join("game").join("foo.dat_tmp").exists(),
        "tmp must be promoted"
    );
    // Version bumped to the mock latest tag.
    let cfg = std::fs::read_to_string(dir.join("config.ini")).expect("config.ini");
    assert!(cfg.contains("game_version=9.9.9-test"), "{cfg}");

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn check_only_clean_reports_zero() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("check_clean");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }

    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.check_only = true;
    let (summary, code) = repair::run(ctx).await.expect("check-only runs");
    assert_eq!(code, 0);
    let (skipped, _rep, failed, _, _) = load_counters(&summary);
    assert_eq!(failed, 0);
    assert_eq!(skipped, 2);

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn check_only_dirty_reports_damage_and_writes_nothing() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("check_dirty");
    let fx2 = build_fixture();
    let foo = &fx2.files[0];
    let bar = &fx2.files[1];
    write_game_file(&dir, &bar.rel, &bar.data);
    write_game_file(&dir, &foo.rel, b"definitely not the game data");

    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.check_only = true;
    let (summary, code) = repair::run(ctx).await.expect("check-only runs");
    assert_eq!(code, 4, "check-only damage exit");
    let (_sk, _rep, failed, _, _) = load_counters(&summary);
    assert_eq!(failed, 1);

    // Read-only contract: corrupt bytes untouched, no config bump, no tmps.
    assert_eq!(
        read_game_file(&dir, &foo.rel).unwrap(),
        b"definitely not the game data"
    );
    assert!(!dir.join("config.ini").exists());
    assert!(!dir.join("game").join("foo.dat_tmp").exists());

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn dry_run_writes_nothing() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("dry_run");
    let fx2 = build_fixture();
    // foo missing entirely, bar intact.
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);

    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.dry_run = true;
    let (_summary, code) = repair::run(ctx).await.expect("dry-run runs");
    assert_eq!(code, 0);
    assert!(
        read_game_file(&dir, &fx2.files[0].rel).is_none(),
        "dry-run must not create the missing file"
    );
    assert!(!dir.join("config.ini").exists());

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn purge_after_deletes_orphans_keeps_manifest_and_config() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("purge_after");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 100]);

    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.purge_after = true;
    let (summary, code) = repair::run(ctx).await.expect("purge-after runs");
    assert_eq!(code, 0);
    let (_, _, _, _, deleted) = load_counters(&summary);
    assert!(deleted >= 100, "orphan bytes counted, got {deleted}");
    assert!(
        read_game_file(&dir, "stray/orphan.dat").is_none(),
        "orphan deleted"
    );
    for f in &fx2.files {
        assert_eq!(read_game_file(&dir, &f.rel).unwrap(), f.data);
    }
    assert!(dir.join("config.ini").exists(), "config.ini kept");

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn purge_before_deletes_orphans_then_repairs() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("purge_before");
    let fx2 = build_fixture();
    // bar intact, foo missing, plus an orphan taking space.
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 64]);

    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.purge_before = true;
    let (summary, code) = repair::run(ctx).await.expect("purge-before runs");
    assert_eq!(code, 0);
    let (_, repaired, failed, _, deleted) = load_counters(&summary);
    assert_eq!((repaired, failed), (1, 0));
    assert!(deleted >= 64);
    assert!(read_game_file(&dir, "stray/orphan.dat").is_none());
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data
    );

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn deprecated_files_deleted_in_post_phase() {
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            deprecated_files: vec!["old/legacy.dll".to_string()],
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("deprecated");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "old/legacy.dll", &[0x7Eu8; 50]);

    let (summary, code) = repair::run(ctx_for(dir.clone(), &mock, 2))
        .await
        .expect("run with deprecated list");
    assert_eq!(code, 0);
    let (_, _, _, _, deleted) = load_counters(&summary);
    assert_eq!(deleted, 50);
    assert!(read_game_file(&dir, "old/legacy.dll").is_none());

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn verified_local_slice_is_reused_without_redownload() {
    // S2: same-file reuse. config.ini already at latest so the local build
    // loads; foo keeps chunk0 intact and corrupts chunk1 -> only chunk_foo1
    // may be fetched.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("reuse");
    let fx2 = build_fixture();
    std::fs::write(
        dir.join("config.ini"),
        "[General]\ngame_version=9.9.9-test\n",
    )
    .unwrap();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    let mut half_bad = fx2.files[0].data.clone();
    for b in half_bad.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &half_bad);

    let (summary, code) = repair::run(ctx_for(dir.clone(), &mock, 1))
        .await
        .expect("reuse run");
    assert_eq!(code, 0);
    let (_, repaired, failed, _, _) = load_counters(&summary);
    assert_eq!((repaired, failed), (1, 0));
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data
    );
    assert_eq!(mock.chunk_hits(), vec!["chunk_foo1".to_string()]);

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn partial_tmp_prefix_is_resumed() {
    // S3: resume-by-length. foo missing but foo_tmp holds the first chunk;
    // only the second chunk is downloaded, then tmp is promoted.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("resume");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    write_game_file(&dir, "game/foo.dat_tmp", &fx2.files[0].data[..64]);

    let (summary, code) = repair::run(ctx_for(dir.clone(), &mock, 1))
        .await
        .expect("resume run");
    assert_eq!(code, 0);
    let (_, repaired, failed, _, _) = load_counters(&summary);
    assert_eq!((repaired, failed), (1, 0));
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data
    );
    assert!(
        !dir.join("game").join("foo.dat_tmp").exists(),
        "tmp promoted"
    );
    assert_eq!(mock.chunk_hits(), vec!["chunk_foo1".to_string()]);

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn explicit_audio_selection_overwrites_scan_file() {
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            scan_dir: "Audio/scan.txt".to_string(),
            ..Default::default()
        },
    )
    .await;
    let fx2 = build_fixture();

    // Explicit ja-jp -> scan file written with the display name.
    let dir = temp_dir("audio_explicit");
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.audio = HashSet::from(["ja-jp".to_string()]);
    ctx.audio_explicit = true;
    let (_, code) = repair::run(ctx).await.expect("audio run");
    assert_eq!(code, 0);
    let scan = std::fs::read_to_string(dir.join("Audio").join("scan.txt")).unwrap();
    assert!(scan.contains("Japanese"), "{scan}");
    std::fs::remove_dir_all(&dir).ok();

    // Explicit game-only (empty set, `--audio none` equivalent) -> empty file.
    let dir2 = temp_dir("audio_none");
    for f in &fx2.files {
        write_game_file(&dir2, &f.rel, &f.data);
    }
    let mut ctx2 = ctx_for(dir2.clone(), &mock, 2);
    ctx2.audio = HashSet::new();
    ctx2.audio_explicit = true;
    let (_, code2) = repair::run(ctx2).await.expect("audio-none run");
    assert_eq!(code2, 0);
    let scan2 = std::fs::read_to_string(dir2.join("Audio").join("scan.txt")).unwrap();
    assert!(scan2.is_empty(), "{scan2}");
    std::fs::remove_dir_all(&dir2).ok();

    mock.stop();
}

#[tokio::test]
async fn non_chunk_download_mode_is_a_usage_error() {
    // Only DOWNLOAD_MODE_CHUNK is patchable; FILE and LDIFF need the 7z/hdiff
    // paths v1 does not have, so the run must stop with exit 1 (not let getBuild
    // -202 turn it into a metadata error).
    for mode in ["DOWNLOAD_MODE_FILE", "DOWNLOAD_MODE_LDIFF", ""] {
        let fx = build_fixture();
        let mock = MockServer::start(
            fx,
            MockOpts {
                download_mode: Some(mode.to_string()),
                ..Default::default()
            },
        )
        .await;
        let dir = temp_dir("dlmode");
        let fx2 = build_fixture();
        for f in &fx2.files {
            write_game_file(&dir, &f.rel, &f.data);
        }
        let err = repair::run(ctx_for(dir.clone(), &mock, 1))
            .await
            .expect_err("non-chunk mode must fail");
        assert_eq!(err.exit_code, 1, "mode {mode:?} must be a usage error");
        assert!(
            err.source.to_string().contains("not chunk mode"),
            "mode {mode:?}: {err}"
        );
        // No manifests were fetched and nothing was written.
        assert!(mock.first_target_with("getBuild").is_none(), "mode {mode:?}");
        assert!(!dir.join("config.ini").exists(), "mode {mode:?}");
        mock.stop();
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[tokio::test]
async fn deprecated_files_call_carries_the_per_launcher_channel() {
    // `getGameDeprecatedFileConfigs` is the one call that needs
    // channel/sub_channel, and the values are per launcher: bilibili is 14/0,
    // not the global 1/0.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            deprecated_files: vec!["old/legacy.dll".to_string()],
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("channel");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "old/legacy.dll", &[0x7Eu8; 50]);
    repair::run(ctx_for_biz(dir.clone(), &mock, 1, Biz::Hk4eBilibili))
        .await
        .expect("bilibili run");

    let dep = mock
        .first_target_with("getGameDeprecatedFileConfigs")
        .expect("deprecated call made");
    assert!(dep.contains("channel=14"), "{dep}");
    assert!(dep.contains("sub_channel=0"), "{dep}");
    assert!(dep.contains("game_ids[]=T2S0Gz4Dr2"), "{dep}");
    // The two channel-less APIs must not carry the params.
    for api in ["getGameConfigs", "getGameBranches"] {
        let t = mock.first_target_with(api).expect("call made");
        assert!(!t.contains("channel="), "{api} must omit channel: {t}");
    }

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}
