//! Path-scope cases: an include/exclude selection applied to the repair,
//! purge, deprecated-file and audio paths.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{
    MockOpts, MockServer, build_fixture, ctx_for, read_game_file, scope, temp_dir, write_game_file,
};
use girpr::test_support::summary_record;
use girpr::{RunExit, run};
use std::collections::HashSet;
use std::sync::atomic::Ordering;

// ---------------------------------------------------------------------------
// path scope
// ---------------------------------------------------------------------------

#[tokio::test]
async fn include_repairs_exactly_the_selected_file() {
    // The `gircap` use case: one run per fixture path, with the
    // exact chunk download set as the assertion.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("scope_include");
    let fx2 = build_fixture();
    let foo = &fx2.files[0];
    let bar = &fx2.files[1];
    // bar is corrupt and must NOT be repaired; foo is missing and must be.
    write_game_file(&dir, &bar.rel, b"not the game data");

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.path_scope = scope(&["game/foo.dat"], &[]);
    let (summary, code) = run(ctx).await.expect("scoped run");
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.files_total, 1, "only the selected file is planned");
    assert!(summary.flags.scoped, "scoped run reports scoped=1");

    assert_eq!(
        read_game_file(&dir, &foo.rel).unwrap(),
        foo.data,
        "selected file repaired"
    );
    assert_eq!(
        read_game_file(&dir, &bar.rel).unwrap(),
        b"not the game data",
        "an out-of-scope file is not even verified"
    );
    assert_eq!(
        mock.chunk_hits(),
        vec!["chunk_foo0".to_string(), "chunk_foo1".to_string()],
        "only the selected file's chunks are fetched"
    );
    // docs/04 §2.1: a scoped run must not advance config.ini.
    assert!(
        !dir.join("config.ini").exists(),
        "a scoped run does not bump config.ini"
    );
    // ...and says so in the machine contract, so a test can assert the fact
    // instead of reading the file.
    let line = summary_record(&summary, code.code()).text();
    assert!(line.contains("scoped=1"), "{line}");
    assert!(line.contains("config_bumped=0"), "{line}");
    assert!(line.contains("plan_files=1"), "{line}");

    mock.stop();
}

#[tokio::test]
async fn a_purge_under_a_scope_leaves_every_other_fixture_file_alone() {
    // The case the whole thing exists for: narrowing the plan must NOT narrow
    // the cleanup's keep set, or `--purge-*` eats the fixture.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("scope_purge");
    let fx2 = build_fixture();
    // Everything intact, plus neighbours that are not in the manifest and would
    // normally be eaten by a full-install purge.
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "log/output.txt", &[0x5Au8; 100]);
    write_game_file(&dir, "ScreenShot/shot.png", &[0x5Bu8; 100]);
    write_game_file(&dir, "old/legacy.dll", &[0x7Eu8; 50]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.path_scope = scope(&["game/foo.dat"], &[]);
    ctx.purge_after = true;
    let (summary, code) = run(ctx).await.expect("scoped purge run");
    assert_eq!(code, RunExit::Success);

    // Everything survives: the fixture files, and the orphans that were out of
    // scope even though they are not in the manifest.
    for f in &fx2.files {
        assert_eq!(
            read_game_file(&dir, &f.rel).unwrap(),
            f.data,
            "{} must survive",
            f.rel
        );
    }
    for rel in ["log/output.txt", "ScreenShot/shot.png", "old/legacy.dll"] {
        assert!(read_game_file(&dir, rel).is_some(), "{rel} must survive");
    }
    assert_eq!(
        summary.deleted_extra_bytes.load(Ordering::Relaxed),
        0,
        "a purge under a scope has nothing to delete here"
    );
    assert!(dir.join("log").is_dir(), "emptied dirs must not be swept");

    mock.stop();
}

#[tokio::test]
async fn a_scoped_purge_keeps_an_empty_out_of_scope_directory() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("scope_purge_empty_dir");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    std::fs::create_dir_all(dir.join("mods/nested")).unwrap();

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.path_scope = scope(&["game/foo.dat"], &[]);
    ctx.purge_only = true;
    let (summary, code) = run(ctx).await.expect("scoped purge-only run");
    assert_eq!(code, RunExit::Success);

    assert!(
        dir.join("mods/nested").is_dir(),
        "an out-of-scope empty dir must survive the sweep"
    );
    assert!(dir.join("mods").is_dir());
    assert_eq!(
        summary.deleted_extra_bytes.load(Ordering::Relaxed),
        0,
        "nothing in scope was deleted"
    );

    mock.stop();
}

#[tokio::test]
async fn an_unscoped_purge_still_deletes_orphans() {
    // The control: with no scope, the cleanup is unchanged.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("scope_unscoped_purge");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "log/output.txt", &[0x5Au8; 100]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.purge_after = true;
    let (summary, code) = run(ctx).await.expect("plain purge run");
    assert_eq!(code, RunExit::Success);
    assert!(
        read_game_file(&dir, "log/output.txt").is_none(),
        "orphan deleted"
    );
    assert!(!summary.flags.scoped);
    assert!(!summary.flags.limited);
    // And the conditional keys are absent, so an unflagged `SUMMARY` carries
    // only the always-on ones.
    let line = summary_record(&summary, code.code()).text();
    for k in ["scoped", "config_bumped", "plan_files", "limited"] {
        assert!(
            !line.contains(k),
            "{k} must be absent when unflagged: {line}"
        );
    }

    mock.stop();
}

#[tokio::test]
async fn an_include_matching_nothing_is_a_usage_error() {
    // Guard 1: a scope that selected nothing is a typo, and exit
    // 1 — not exit 5, which means "well-formed but the answer was too big".
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("scope_empty");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.path_scope = scope(&["game/nope.dat"], &[]);
    let err = run(ctx).await.expect_err("an empty scope must fail");
    assert_eq!(err.exit_code, RunExit::Usage);
    let msg = format!("{:#}", err.source);
    assert!(msg.contains("game/nope.dat"), "must name the glob: {msg}");
    assert!(msg.contains("plan_files=0"), "{msg}");

    // `--exclude` matching nothing is unremarkable and must still succeed.
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.path_scope = scope(&[], &["no/such/thing"]);
    let (_, code) = run(ctx).await.expect("a non-matching exclude is fine");
    assert_eq!(code, RunExit::Success);

    mock.stop();
}

#[tokio::test]
async fn explicit_audio_under_a_scope_is_refused_unless_the_scan_file_is_in_scope() {
    // Guard 2: two flags asking for opposite scopes is a
    // contradiction, not a merge. The scan file's rel path comes from
    // getGameConfigs and is never hardcoded.
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

    let dir = temp_dir("audio_scope_refuse");
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.path_scope = scope(&["game/foo.dat"], &[]);
    ctx.audio = HashSet::from(["ja-jp".to_string()]);
    ctx.audio_explicit = true;
    let err = run(ctx).await.expect_err("--audio under a scope");
    assert_eq!(err.exit_code, RunExit::Usage);
    let msg = format!("{:#}", err.source);
    assert!(
        msg.contains("Audio/scan.txt"),
        "must name the scan file: {msg}"
    );
    assert!(
        !dir.join("Audio").exists(),
        "the refused run must not write the scan file"
    );

    // The escape hatch: include the scan file's own rel path.
    let dir2 = temp_dir("audio_scope_allowed");
    for f in &fx2.files {
        write_game_file(&dir2, &f.rel, &f.data);
    }
    let mut ctx2 = ctx_for(dir2.to_path_buf(), &mock, 1);
    ctx2.path_scope = scope(&["Audio/scan.txt", "game/foo.dat"], &[]);
    ctx2.audio = HashSet::from(["ja-jp".to_string()]);
    ctx2.audio_explicit = true;
    let (_, code) = run(ctx2).await.expect("scan file in scope is allowed");
    assert_eq!(code, RunExit::Success);
    let scan = std::fs::read_to_string(dir2.join("Audio").join("scan.txt")).unwrap();
    assert!(scan.contains("Japanese"), "{scan}");

    mock.stop();
}

#[tokio::test]
async fn deprecated_files_outside_the_scope_are_not_deleted() {
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            deprecated_files: vec!["old/legacy.dll".to_string(), "other/stale.dll".to_string()],
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("scope_deprecated");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "old/legacy.dll", &[0x7Eu8; 50]);
    write_game_file(&dir, "other/stale.dll", &[0x7Fu8; 70]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.path_scope = scope(&["old/*", "game/foo.dat", "game/bar.dat"], &[]);
    let (summary, code) = run(ctx).await.expect("scoped deprecated run");
    assert_eq!(code, RunExit::Success);
    assert!(
        read_game_file(&dir, "old/legacy.dll").is_none(),
        "in-scope deprecated file deleted"
    );
    assert_eq!(
        read_game_file(&dir, "other/stale.dll").unwrap(),
        &[0x7Fu8; 70],
        "out-of-scope deprecated file kept"
    );
    assert_eq!(summary.deleted_extra_bytes.load(Ordering::Relaxed), 50);

    mock.stop();
}

#[tokio::test]
async fn a_dot_spelled_deprecated_path_hidden_by_the_scope_is_not_deleted() {
    // The server may spell the same file `./old/legacy.dll`. The scope
    // matches the *cleaned* rel (`old/legacy.dll`), so an exclude that hides
    // the plain spelling must hide this one too: the gate cleans before it
    // compares.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            deprecated_files: vec!["./old/legacy.dll".to_string()],
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("dot_scope_deprecated");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "old/legacy.dll", &[0x7Eu8; 50]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.path_scope = scope(&[], &["old/*"]);
    let (summary, code) = run(ctx).await.expect("scoped deprecated run");
    assert_eq!(code, RunExit::Success);
    assert_eq!(
        read_game_file(&dir, "old/legacy.dll").unwrap(),
        &[0x7Eu8; 50],
        "the excluded `./`-spelled path must survive"
    );
    assert_eq!(
        summary.deleted_extra_bytes.load(Ordering::Relaxed),
        0,
        "nothing was deleted"
    );

    mock.stop();
}

#[tokio::test]
async fn a_dot_spelled_explicit_audio_scan_file_in_scope_is_accepted() {
    // The scan file's own rel path is the documented escape hatch for
    // `--audio` under a scope. A `./` spelling is the same file, so the
    // include that names the plain path must admit it; the gate cleans before
    // it compares, so a `./` spelling cannot refuse an in-scope run.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            scan_dir: "./Audio/scan.txt".to_string(),
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("audio_scope_dot_allowed");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.path_scope = scope(&["Audio/scan.txt", "game/foo.dat"], &[]);
    ctx.audio = HashSet::from(["ja-jp".to_string()]);
    ctx.audio_explicit = true;
    let (_, code) = run(ctx).await.expect("the cleaned scan rel is in scope");
    assert_eq!(code, RunExit::Success);
    let scan = std::fs::read_to_string(dir.join("Audio").join("scan.txt")).unwrap();
    assert!(scan.contains("Japanese"), "{scan}");

    mock.stop();
}

#[tokio::test]
async fn a_dot_spelled_audio_cache_hidden_by_the_scope_is_not_moved() {
    // `./Audio/cache` and `Audio/cache` are one directory: the move sink
    // cleans before it gates. An exclude that hides the cache must leave it
    // where it is.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            cache_dir: "./Audio/cache".to_string(),
            res_dir: "./Audio/res".to_string(),
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("dot_scope_audio_move");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "Audio/cache/track_1.pcm", &[0x3Bu8; 128]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.path_scope = scope(&[], &["Audio/*"]);
    let (_, code) = run(ctx).await.expect("scoped audio run");
    assert_eq!(code, RunExit::Success);
    assert_eq!(
        read_game_file(&dir, "Audio/cache/track_1.pcm").unwrap(),
        &[0x3Bu8; 128],
        "the excluded cache file must not move"
    );
    assert!(
        read_game_file(&dir, "Audio/res/track_1.pcm").is_none(),
        "nothing may appear in the res dir"
    );

    mock.stop();
}

#[tokio::test]
async fn an_include_that_selects_nothing_at_all_is_still_a_typo() {
    // The guard survives the relaxation: a scope that matches neither a manifest
    // path nor a purge candidate really is a typo, and a typo that costs 130 GB
    // to discover is what the guard is for.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("scope_nothing_typed");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.purge_only = true; // asks for the cleanup…
    ctx.path_scope = scope(&["game/never-existed.dat"], &[]); // …of nothing
    let err = run(ctx)
        .await
        .expect_err("a scope that selects nothing is still a typo");
    assert_eq!(err.exit_code, RunExit::Usage);
    let msg = format!("{:#}", err.source);
    assert!(
        msg.contains("game/never-existed.dat"),
        "must name the glob: {msg}"
    );

    // The message names both plans when a cleanup was asked for, so the error
    // says which of the two came back empty.
    assert!(msg.contains("plan_files=0"), "{msg}");
    assert!(msg.contains("cleanup"), "{msg}");

    mock.stop();
}
