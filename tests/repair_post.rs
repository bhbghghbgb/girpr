//! Post-phase cases: deprecated files, explicit audio selection, and the
//! dry run that lists post-phase work.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "common/mod.rs"]
mod common;

use common::{
    MockOpts, MockServer, absent_game_dir, build_fixture, ctx_for, ctx_for_biz, load_counters,
    read_game_file, run_binary, temp_dir, write_game_file,
};
use girpr::{Biz, run};
use std::collections::HashSet;
use std::sync::atomic::Ordering;

/// Run the binary with an explicit `--audio` and one extra refusing flag.
///
/// R-7's reproducer: the scan write lives in the plan half, so a run that then
/// refuses may already have rewritten the user's audio selection.
async fn run_audio_refusal(
    mock: &MockServer,
    game: &std::path::Path,
    extra: &[&str],
) -> (i32, String, String) {
    let game_arg = game.to_string_lossy().into_owned();
    let mut args: Vec<&str> = vec![
        "--game-path",
        &game_arg,
        "--biz",
        "hk4e_global",
        "--io-threads",
        "1",
        "--audio",
        "en-us",
        "--hyp-base",
        &mock.hyp_base,
        "--sophon-base",
        &mock.sophon_base,
    ];
    args.extend_from_slice(extra);
    run_binary(&args, &[]).await
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

    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 2))
        .await
        .expect("run with deprecated list");
    assert_eq!(code, 0);
    let (_, _, _, _, deleted) = load_counters(&summary);
    assert_eq!(deleted, 50);
    assert!(read_game_file(&dir, "old/legacy.dll").is_none());

    mock.stop();
}

#[tokio::test]
async fn a_deprecated_path_that_escapes_the_game_dir_is_kept() {
    // The deprecated list is a **delete** sink. A `../` name must be skipped
    // with a warning, not resolved outside the install.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            deprecated_files: vec!["../victim.dll".to_string()],
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("escape_deprecated");
    let game = dir.join("install");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&game, &f.rel, &f.data);
    }
    std::fs::write(dir.join("victim.dll"), b"do not delete").unwrap();

    let (summary, code) = run(ctx_for(game, &mock, 2))
        .await
        .expect("the run itself is fine");
    assert_eq!(code, 0);
    assert_eq!(
        std::fs::read(dir.join("victim.dll")).unwrap(),
        b"do not delete",
        "the outside file must survive"
    );
    assert_eq!(
        summary.deleted_extra_bytes.load(Ordering::Relaxed),
        0,
        "nothing was deleted"
    );
    mock.stop();
}

#[tokio::test]
async fn an_audio_dir_that_escapes_the_game_dir_is_refused_as_metadata() {
    // The cache/res pair is a **mutation** sink fed by server strings. Both
    // are validated with the other config dirs at metadata time, so a `..`
    // value refuses the run (exit 2) before `config.ini` is bumped or a file
    // moves — it must not report success after skipping the move.
    for (cache, res) in [
        ("../audio_cache".to_string(), "Audio/res".to_string()),
        ("Audio/cache".to_string(), "../audio_res".to_string()),
    ] {
        let fx = build_fixture();
        let mock = MockServer::start(
            fx,
            MockOpts {
                cache_dir: cache,
                res_dir: res,
                ..Default::default()
            },
        )
        .await;
        let dir = temp_dir("escape_audio");
        let game = dir.join("install");
        let fx2 = build_fixture();
        for f in &fx2.files {
            write_game_file(&game, &f.rel, &f.data);
        }
        std::fs::create_dir_all(dir.join("audio_cache")).unwrap();
        std::fs::write(dir.join("audio_cache").join("a.pck"), b"cache bytes").unwrap();
        std::fs::create_dir_all(dir.join("audio_res")).unwrap();
        std::fs::write(dir.join("audio_res").join("keep.pck"), b"res bytes").unwrap();

        let err = run(ctx_for(game.clone(), &mock, 2))
            .await
            .expect_err("an escaping audio dir is metadata");
        assert_eq!(err.exit_code, 2, "an escaping audio dir is metadata");
        assert!(
            !game.join("config.ini").exists(),
            "the version must not be bumped for refused server data"
        );
        assert_eq!(
            std::fs::read(dir.join("audio_cache").join("a.pck")).unwrap(),
            b"cache bytes",
            "the outside cache must not move"
        );
        assert_eq!(
            std::fs::read(dir.join("audio_res").join("keep.pck")).unwrap(),
            b"res bytes",
            "the outside res dir must not be touched"
        );
        assert!(
            !game.join("Audio").join("res").exists(),
            "nothing may be moved into the install from outside it"
        );
        mock.stop();
    }
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
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.audio = HashSet::from(["ja-jp".to_string()]);
    ctx.audio_explicit = true;
    let (_, code) = run(ctx).await.expect("audio run");
    assert_eq!(code, 0);
    let scan = std::fs::read_to_string(dir.join("Audio").join("scan.txt")).unwrap();
    assert!(scan.contains("Japanese"), "{scan}");

    // Explicit game-only (empty set, `--audio none` equivalent) -> empty file.
    let dir2 = temp_dir("audio_none");
    for f in &fx2.files {
        write_game_file(&dir2, &f.rel, &f.data);
    }
    let mut ctx2 = ctx_for(dir2.to_path_buf(), &mock, 2);
    ctx2.audio = HashSet::new();
    ctx2.audio_explicit = true;
    let (_, code2) = run(ctx2).await.expect("audio-none run");
    assert_eq!(code2, 0);
    let scan2 = std::fs::read_to_string(dir2.join("Audio").join("scan.txt")).unwrap();
    assert!(scan2.is_empty(), "{scan2}");

    mock.stop();
}

#[tokio::test]
async fn an_unreadable_audio_scan_file_fails_before_any_repair() {
    // R-2: a scan file that exists but cannot be read is local state, not the
    // documented "absent → en-us" case. Treating it as absence silently
    // replaces the selection; this run refuses (exit 1) instead.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            scan_dir: "Audio/scan.txt".to_string(),
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("audio_unreadable");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    // A directory where the scan file belongs is readable-as-a-path but not
    // readable-as-a-file on every supported host.
    std::fs::create_dir_all(dir.join("Audio").join("scan.txt")).unwrap();

    let ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    let err = run(ctx)
        .await
        .expect_err("an unreadable scan file must not default to en-us");
    assert_eq!(err.exit_code, 1, "local config read failure: {err:#}");
    assert!(
        format!("{err:#}").contains("scan.txt"),
        "the report names the file: {err:#}"
    );
    assert!(
        mock.chunk_hits().is_empty(),
        "nothing may be planned or downloaded on a failed selection"
    );

    mock.stop();
}

#[tokio::test]
async fn explicit_audio_is_not_written_before_a_refusal() {
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            scan_dir: "Audio/scan.txt".to_string(),
            ..Default::default()
        },
    )
    .await;
    let parent = temp_dir("audio_refusal");

    // (a) a tripped ceiling: exit `5`.
    let game = absent_game_dir(&parent);
    let (code, _, stderr) = run_audio_refusal(&mock, &game, &["--max-files", "0"]).await;
    assert_eq!(code, 5, "a ceiling is exit 5: {stderr}");
    assert!(
        !game.exists(),
        "a refused run must not create game_dir (nor the scan file inside it): {stderr}"
    );
    assert!(
        !game.join("Audio").exists(),
        "a refused run must not create the scan dir: {stderr}"
    );

    // (b) an unwritable `--plan-out`: exit `3`, still before the first mutation.
    let game = absent_game_dir(&parent);
    let out_dir = parent.join("plan-out-is-a-directory");
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_arg = out_dir.to_string_lossy().into_owned();
    let (code, _, stderr) = run_audio_refusal(&mock, &game, &["--plan-out", &out_arg]).await;
    assert_eq!(code, 3, "an unwritable --plan-out is exit 3: {stderr}");
    assert!(
        !game.exists(),
        "the plan-out failure must not create game_dir: {stderr}"
    );

    // (c) an `--include` that selected nothing: exit `1`.
    let game = absent_game_dir(&parent);
    let (code, _, stderr) =
        run_audio_refusal(&mock, &game, &["--include", "does/not/exist/*"]).await;
    assert_eq!(code, 1, "an empty --include is exit 1: {stderr}");
    assert!(
        !game.exists(),
        "the include refusal must not create game_dir: {stderr}"
    );

    assert!(
        mock.chunk_hits().is_empty(),
        "no refusal may reach the download phase"
    );
    mock.stop();
}

#[tokio::test]
async fn an_all_unrecognized_audio_scan_file_is_refused_not_read_as_game_only() {
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            scan_dir: "Audio/scan.txt".to_string(),
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("audio_unknown");
    std::fs::create_dir_all(dir.join("Audio")).unwrap();
    let scan = dir.join("Audio").join("scan.txt");
    // A renamed display name: non-blank, understood by nothing.
    std::fs::write(&scan, "English (US)\n").unwrap();

    let game_arg = dir.to_string_lossy().into_owned();
    let (code, stdout, stderr) = run_binary(
        &[
            "--game-path",
            &game_arg,
            "--biz",
            "hk4e_global",
            "--io-threads",
            "1",
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;
    assert_eq!(
        code, 1,
        "an all-unrecognized scan file is a config error, not game-only:\n{stdout}\n{stderr}"
    );
    assert!(
        stderr.contains("scan.txt"),
        "the error names the file: {stderr}"
    );
    assert!(
        stderr.contains("English (US)"),
        "the error quotes the offending line: {stderr}"
    );
    assert!(
        mock.chunk_hits().is_empty(),
        "nothing may be planned or downloaded from a malformed selection"
    );
    assert_eq!(
        std::fs::read_to_string(&scan).unwrap(),
        "English (US)\n",
        "the scan file is reported, never rewritten"
    );
    mock.stop();
}

#[tokio::test]
async fn dry_run_lists_the_post_phase_and_changes_nothing() {
    // A dry run answers "what would this run do?" end to end, so it reaches
    // the post-phase's read-only paths: the deprecated deletes, the audio
    // cache move and the cleanup. It lists them, deletes and moves nothing,
    // counts no bytes, and stops before the `config.ini` bump.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            scan_dir: "Audio/scan.txt".to_string(),
            res_dir: "Audio/res".to_string(),
            cache_dir: "Audio/cache".to_string(),
            deprecated_files: vec!["old/legacy.dll".to_string()],
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("dry_run_post");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "old/legacy.dll", &[0x7Eu8; 50]);
    write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 777]);
    write_game_file(&dir, "Audio/cache/track_1.pcm", &[0x3Bu8; 128]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.dry_run = true;
    ctx.purge_after = true;
    let (summary, code) = run(ctx).await.expect("dry-run runs the post-phase");
    assert_eq!(code, 0);

    assert_eq!(
        read_game_file(&dir, "old/legacy.dll").unwrap(),
        &[0x7Eu8; 50],
        "dry-run must not delete deprecated files"
    );
    assert!(
        read_game_file(&dir, "stray/orphan.dat").is_some(),
        "dry-run must not delete orphans"
    );
    assert_eq!(
        read_game_file(&dir, "Audio/cache/track_1.pcm").unwrap(),
        &[0x3Bu8; 128],
        "dry-run must not move the audio cache"
    );
    assert!(
        read_game_file(&dir, "Audio/res/track_1.pcm").is_none(),
        "nothing may appear in the res dir"
    );
    assert_eq!(
        summary.deleted_extra_bytes.load(Ordering::Relaxed),
        0,
        "a dry run counts no deleted bytes"
    );
    assert!(
        !dir.join("config.ini").exists(),
        "a dry run must not bump config.ini"
    );
    assert!(
        mock.first_target_with("getGameDeprecatedFileConfigs")
            .is_some(),
        "the dry run fetches the list it lists"
    );

    mock.stop();
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
    run(ctx_for_biz(dir.to_path_buf(), &mock, 1, Biz::Hk4eBilibili))
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
}
