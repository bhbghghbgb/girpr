//! Post-phase cases: deprecated files, explicit audio selection, and the
//! dry run that lists post-phase work.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{
    ChunkSpec, FileSpec, FixtureSpec, MockOpts, MockServer, absent_game_dir, build_fixture,
    build_fixture_spec, ctx_for, ctx_for_biz, load_counters, read_game_file, run_binary, temp_dir,
    write_game_file,
};
use girpr::{Biz, RunExit, run};
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
    assert_eq!(code, RunExit::Success);
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
    assert_eq!(code, RunExit::Success);
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
        assert_eq!(
            err.exit_code,
            RunExit::Metadata,
            "an escaping audio dir is metadata"
        );
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
    assert_eq!(code, RunExit::Success);
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
    assert_eq!(code2, RunExit::Success);
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
    assert_eq!(
        err.exit_code,
        RunExit::Usage,
        "local config read failure: {err:#}"
    );
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
    assert_eq!(code, RunExit::Success);

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

/// A cache file whose name is not UTF-8 moves under its own bytes: the scope
/// keys the lossy `/` rel, but the destination name is the source's name.
///
/// The name is built with the platform's own byte constructor, so a host that
/// cannot create one skips rather than failing.
#[cfg(any(unix, windows))]
#[tokio::test]
async fn a_non_utf8_audio_cache_name_moves_byte_for_byte() {
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            res_dir: "Audio/res".to_string(),
            cache_dir: "Audio/cache".to_string(),
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("audio_non_utf8");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let cache = dir.join("Audio").join("cache");
    std::fs::create_dir_all(&cache).unwrap();

    #[cfg(unix)]
    let name = {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::OsStr::from_bytes(b"track_\xff.pcm").to_os_string()
    };
    #[cfg(windows)]
    let name = {
        use std::os::windows::ffi::OsStringExt;
        // An unpaired surrogate has no UTF-8 encoding.
        std::ffi::OsString::from_wide(&[
            0x74, 0x72, 0x61, 0x63, 0x6B, 0x5F, 0xD800, 0x2E, 0x70, 0x63, 0x6D,
        ])
    };

    let src = cache.join(&name);
    if let Err(e) = std::fs::write(&src, b"non-utf8 cache bytes") {
        eprintln!("skipping: this host cannot create a non-UTF-8 name: {e}");
        mock.stop();
        return;
    }

    let (_, code) = run(ctx_for(dir.to_path_buf(), &mock, 2))
        .await
        .expect("run with a non-UTF-8 cache name");
    assert_eq!(code, RunExit::Success);
    assert!(!src.exists(), "the source is gone after the move");

    let res = dir.join("Audio").join("res");
    let entries: Vec<std::ffi::OsString> = std::fs::read_dir(&res)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(entries.len(), 1, "the one cache file moved: {entries:?}");

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(entries[0].as_bytes(), b"track_\xff.pcm");
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        assert_eq!(
            entries[0].encode_wide().collect::<Vec<u16>>(),
            name.encode_wide().collect::<Vec<u16>>()
        );
    }
    assert_eq!(
        std::fs::read(res.join(&entries[0])).unwrap(),
        b"non-utf8 cache bytes",
        "the moved file keeps its content"
    );

    mock.stop();
}

#[tokio::test]
async fn a_game_dir_that_vanishes_before_the_config_bump_is_a_write_failure() {
    // The install can disappear between `prepare_install` and the post-phase
    // bump: a user unmounts or cleans the tree while the repair is running.
    // The repaired file is still written through the handle the repair opened,
    // so the run reaches the bump; the bump cannot create a game dir, and a
    // missing install there is fatal (exit 3), not a best-effort post step.
    //
    // The first chunk request is the deterministic seam: `prepare_install` has
    // already created the dir and the file task has opened its target, so
    // removing the tree now leaves the repair with a valid handle and the
    // post-phase with a vanished game dir.
    let fx = build_fixture_spec(&FixtureSpec::new(vec![FileSpec::new(
        "game/only.dat",
        vec![ChunkSpec::new("chunk_only", vec![0x41u8; 64], 0)],
    )]));
    let dir = temp_dir("game_dir_vanished");
    let game = dir.join("install");
    let hook_game = game.clone();
    let mock = MockServer::start(
        fx,
        MockOpts {
            on_first_chunk: Some(std::sync::Arc::new(move || {
                std::fs::remove_dir_all(&hook_game).expect("remove the game dir before the bump");
            })),
            ..Default::default()
        },
    )
    .await;

    let game_arg = game.to_string_lossy().into_owned();
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
        code, 3,
        "a vanished game dir at the bump is a write failure:\n{stdout}\n{stderr}"
    );
    assert!(
        stderr.contains("FATAL write config.ini"),
        "the bump is the fatal step: {stderr}"
    );
    assert!(
        stderr.contains("does not exist"),
        "the refusal names the missing game dir: {stderr}"
    );
    assert!(
        !stdout.contains("SUMMARY"),
        "a fatal post-phase failure must not emit a success summary: {stdout}"
    );
    assert!(
        !game.exists(),
        "the test's vanishing game dir must stay gone"
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
            biz: Some(Biz::Hk4eBilibili),
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
