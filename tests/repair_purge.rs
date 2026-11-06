//! Purge cases: orphan deletion before and after a repair, and `--purge-only`.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{
    ChunkSpec, FileSpec, FixtureSpec, MockOpts, MockServer, build_fixture, build_fixture_spec,
    ctx_for, load_counters, logs_of, read_game_file, scope, temp_dir, write_game_file,
};
use girpr::test_support::summary_record;
use girpr::{RunCtx, RunExit, run};
use std::sync::atomic::Ordering;

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

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.purge_after = true;
    let (summary, code) = run(ctx).await.expect("purge-after runs");
    assert_eq!(code, RunExit::Success);
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
}

#[tokio::test]
async fn purge_after_keeps_the_case_variant_it_just_verified() {
    // N-5: the purge walks the **on-disk** spelling, which on Windows can
    // differ in case from the manifest. Before the purge key mirrored the
    // plan's dedup key, `--purge-after` saw `game/foo.dat`, missed the
    // expected `Game/Foo.dat` and deleted the file the run had just verified.
    let spec = FixtureSpec::new(vec![FileSpec::new(
        "Game/Foo.dat",
        vec![ChunkSpec::new("case_c0", vec![0x41u8; 64], 0)],
    )]);
    let expected = build_fixture_spec(&spec);
    let data = expected.files[0].data.clone();
    let mock = MockServer::start(build_fixture_spec(&spec), MockOpts::default()).await;
    let dir = temp_dir("purge_case");
    write_game_file(&dir, "game/foo.dat", &data);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.purge_after = true;
    let (_summary, code) = run(ctx).await.expect("case-differing purge run");
    assert_eq!(code, RunExit::Success);

    #[cfg(windows)]
    assert_eq!(
        read_game_file(&dir, "game/foo.dat").as_deref(),
        Some(data.as_slice()),
        "Windows: the case variant is the file the manifest expected, and survives"
    );
    #[cfg(unix)]
    {
        assert_eq!(
            read_game_file(&dir, "Game/Foo.dat").as_deref(),
            Some(data.as_slice()),
            "Unix: the manifest spelling is a distinct file and is kept"
        );
        assert!(
            read_game_file(&dir, "game/foo.dat").is_none(),
            "Unix: the lower-case sibling is an orphan and still purges"
        );
    }

    mock.stop();
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

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.purge_before = true;
    let (summary, code) = run(ctx).await.expect("purge-before runs");
    assert_eq!(code, RunExit::Success);
    let (_, repaired, failed, _, deleted) = load_counters(&summary);
    assert_eq!((repaired, failed), (1, 0));
    assert!(deleted >= 64);
    assert!(read_game_file(&dir, "stray/orphan.dat").is_none());
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data
    );

    mock.stop();
}

#[tokio::test]
async fn purge_before_and_after_share_the_keys_and_delete_the_orphan_once() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("purge_both");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 96]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.purge_before = true;
    ctx.purge_after = true;
    let (summary, code) = run(ctx).await.expect("both purge passes run");
    assert_eq!(code, RunExit::Success);
    let (_, repaired, failed, _, deleted) = load_counters(&summary);
    assert_eq!((repaired, failed), (0, 0));
    assert_eq!(deleted, 96, "the orphan is deleted exactly once");
    assert!(read_game_file(&dir, "stray/orphan.dat").is_none());
    for f in &fx2.files {
        assert_eq!(read_game_file(&dir, &f.rel).unwrap(), f.data);
    }
    assert!(dir.join("config.ini").exists(), "config.ini kept");

    mock.stop();
}

#[tokio::test]
async fn a_dry_run_with_both_purge_flags_lists_each_candidate_once() {
    // The before pass sees the orphan on disk and names it; a dry run deletes
    // nothing, so the after pass must recognise it as already listed rather
    // than name it a second time. Both flags together are the case where the
    // two walks see the same tree.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("purge_both_dry");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 96]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.purge_before = true;
    ctx.purge_after = true;
    ctx.dry_run = true;
    let (result, logs) = logs_of(run(ctx)).await;
    let (summary, code) = result.expect("a dry run still returns");
    assert_eq!(code, RunExit::Success);
    assert_eq!(
        logs.matches("would purge extra").count(),
        1,
        "one candidate, one listing: {logs}"
    );
    // The chosen per-phase contract: the after pass returns before the delete
    // step when it has nothing new to report, so it emits no second byte
    // summary. The before line already covers this candidate.
    assert_eq!(
        logs.matches("bytes would be freed").count(),
        1,
        "only the before pass reached the delete step, so only it emits a byte summary: {logs}"
    );
    assert!(
        logs.contains("every after-pass candidate was already listed before the repair"),
        "the after pass took the dedup path: {logs}"
    );
    assert_eq!(
        summary.deleted_extra_bytes.load(Ordering::Relaxed),
        0,
        "a dry run counts no freed bytes"
    );
    assert!(
        read_game_file(&dir, "stray/orphan.dat").is_some(),
        "a dry run deletes nothing"
    );
    for f in &fx2.files {
        assert_eq!(read_game_file(&dir, &f.rel).unwrap(), f.data);
    }

    mock.stop();
}

#[tokio::test]
async fn blacklisted_files_survive_every_purge_spelling() {
    // The blacklist excludes a file from the *work list*; it does not remove
    // it from the expected install. docs/01 says blacklisted entries are
    // "never deleted" and docs/02 defines the cleanup's expected set from the
    // manifest, so the blacklist must also be a keep input.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            blacklist_dir: "blacklist.txt".to_string(),
            enable_resource_blacklist: true,
            ..Default::default()
        },
    )
    .await;

    for (name, apply) in [
        (
            "after",
            (|c: &mut RunCtx| c.purge_after = true) as fn(&mut RunCtx),
        ),
        ("before", |c: &mut RunCtx| c.purge_before = true),
        ("only", |c: &mut RunCtx| c.purge_only = true),
    ] {
        let dir = temp_dir(&format!("purge_blacklist_{name}"));
        let fx2 = build_fixture();
        for f in &fx2.files {
            write_game_file(&dir, &f.rel, &f.data);
        }
        // The blacklisted file is a manifest path (so it is part of the
        // expected install) and stays on disk unchanged.
        write_game_file(&dir, "blacklist.txt", br#"{"fileName":"game/foo.dat"}"#);
        // A genuinely stale file still has to be deleted.
        write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 32]);

        let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
        apply(&mut ctx);
        let (summary, code) = run(ctx)
            .await
            .unwrap_or_else(|e| panic!("{name}: run failed: {e:#}"));
        assert_eq!(code, RunExit::Success, "{name}: exit");
        assert!(
            read_game_file(&dir, "game/foo.dat").is_some(),
            "{name}: a blacklisted manifest file must never be deleted"
        );
        assert!(
            read_game_file(&dir, "stray/orphan.dat").is_none(),
            "{name}: the stale file is still purged"
        );
        assert!(
            summary.deleted_extra_bytes.load(Ordering::Relaxed) >= 32,
            "{name}: the stale file's bytes are counted"
        );
    }

    mock.stop();
}

#[tokio::test]
async fn purge_only_deletes_a_scoped_path_that_is_absent_from_the_new_manifest() {
    // The `removed` capture class: a path on disk that the new manifest no
    // longer contains. It is in no repair plan by construction, so the only
    // work that can touch it is the cleanup — which is exactly what
    // `--purge-only --include <that path>` asks for.
    //
    // An empty repair plan with non-empty purge candidates is a legitimate
    // request, not a typo: the guard that says "an `--include` matching
    // nothing is a typo" tests the *repair* plan's emptiness, and
    // `--purge-only` has no repair plan to be empty.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("purge_only_removed");
    let fx2 = build_fixture();
    // Intact manifest files, so the only candidate is the removed one.
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "game/removed.dat", &[0x7Eu8; 512]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.purge_only = true;
    ctx.path_scope = scope(&["game/removed.dat"], &[]);
    let (summary, code) = run(ctx).await.expect("a cleanup run must not be refused");
    assert_eq!(
        code,
        RunExit::Success,
        "an empty repair plan is not a typo when there is cleanup to do"
    );
    assert!(
        read_game_file(&dir, "game/removed.dat").is_none(),
        "the removed path is what --purge-only exists to delete"
    );
    for f in &fx2.files {
        assert_eq!(
            read_game_file(&dir, &f.rel).unwrap(),
            f.data,
            "{} is out of scope and must survive",
            f.rel
        );
    }
    assert!(mock.chunk_hits().is_empty(), "nothing may be downloaded");
    assert_eq!(summary.files_total, 0, "the repair plan really was empty");
    assert_eq!(summary.files_repaired.load(Ordering::Relaxed), 0);
    assert_eq!(summary.deleted_extra_bytes.load(Ordering::Relaxed), 512);
    let line = summary_record(&summary, code.code()).text();
    assert!(line.contains("config_bumped=0"), "{line}");
    assert!(line.contains("scoped=1"), "{line}");

    mock.stop();
}

#[tokio::test]
async fn purge_only_cleans_up_without_repairing() {
    // The `removed` matrix class is reachable only through the purge path, and
    // `gircap`'s own procedure said not to pass a purge flag — so it needed
    // this. `check_only` inverted: "is it intact?" vs "make it match".
    //
    // Note the absence of `purge_after`: `--purge-only` **implies** the
    // cleanup, and a flag that cleaned up nothing would be a lie.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("purge_only");
    let fx2 = build_fixture();
    // One damaged file: purge-only must NOT repair it, but must not fail either.
    write_game_file(&dir, &fx2.files[0].rel, b"not the game data");
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 777]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.purge_only = true;
    let (summary, code) = run(ctx).await.expect("purge-only runs");
    assert_eq!(code, RunExit::Success);
    assert_eq!(
        summary.files_repaired.load(Ordering::Relaxed),
        0,
        "purge-only never repairs"
    );
    assert!(mock.chunk_hits().is_empty(), "no chunk may be fetched");
    assert_eq!(
        summary.deleted_extra_bytes.load(Ordering::Relaxed),
        777,
        "the orphan was deleted with no purge flag at all"
    );
    assert!(read_game_file(&dir, "stray/orphan.dat").is_none());
    // The damaged file was left exactly as it was.
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        b"not the game data"
    );
    // A cleanup puts no bytes in place, so it does not advance config.ini.
    assert!(
        !dir.join("config.ini").exists(),
        "purge-only must not bump config.ini"
    );
    let line = summary_record(&summary, code.code()).text();
    assert!(line.contains("config_bumped=0"), "{line}");

    mock.stop();
}

#[tokio::test]
async fn purge_only_with_dry_run_deletes_and_moves_nothing() {
    // `--dry-run` is documented as writing nothing. `post::run` has two call
    // sites: the repair run, which reaches it only through `if !readonly`, and
    // the `--purge-only` early return, which is not behind that gate — so the
    // three destructive steps of the post-phase (deprecated deletes, the audio
    // cache move, the cleanup pass) have to check the flag themselves.
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
    let dir = temp_dir("purge_only_dry");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "old/legacy.dll", &[0x7Eu8; 50]);
    write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 777]);
    write_game_file(&dir, "Audio/cache/track_1.pcm", &[0x3Bu8; 128]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.purge_only = true;
    ctx.dry_run = true;
    let (summary, code) = run(ctx).await.expect("purge-only --dry-run runs");
    assert_eq!(code, RunExit::Success);

    // Nothing moved, nothing deleted, nothing promoted.
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
        "byte counters stay 0 in dry-run by design"
    );
    assert!(!dir.join("config.ini").exists());

    mock.stop();
}

#[tokio::test]
async fn a_plan_out_inside_the_install_is_an_ordinary_purge_candidate() {
    // `--plan-out` lands before the purge runs, and the cleanup keeps only the
    // manifest, the metadata files, the blacklist and the audio bookkeeping.
    // An in-install plan document is therefore an unexpected file and a real
    // purge deletes it. Writing it outside `--game-path` is the way to keep it.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("plan_out_inside_purge");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 32]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.purge_after = true;
    ctx.plan_out = Some(dir.join("plan.json"));
    let (_summary, code) = run(ctx).await.expect("the purge run succeeds");
    assert_eq!(code, RunExit::Success);

    assert!(
        read_game_file(&dir, "plan.json").is_none(),
        "the in-install plan document is purged like any other extra file"
    );
    assert!(read_game_file(&dir, "stray/orphan.dat").is_none());
    for f in &fx2.files {
        assert_eq!(read_game_file(&dir, &f.rel).unwrap(), f.data);
    }

    mock.stop();
}

#[tokio::test]
async fn purge_only_and_check_only_are_the_two_halves_of_the_cleanup_contract() {
    // Read: "is it intact?" Write: "make it match". Both exit 0/3/4 by their own
    // rules, and neither repairs.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("purge_only_pair");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[0].rel, b"not the game data");
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");
    write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 100]);

    // check-only: damage found, nothing written, orphan kept.
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.check_only = true;
    let (_, code) = run(ctx).await.expect("check runs");
    assert_eq!(code, RunExit::CheckFailed);
    assert!(read_game_file(&dir, "stray/orphan.dat").is_some());

    // purge-only: the orphan is gone, the damage is untouched.
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.purge_only = true;
    let (_, code) = run(ctx).await.expect("purge-only runs");
    assert_eq!(code, RunExit::Success);
    assert!(read_game_file(&dir, "stray/orphan.dat").is_none());
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        b"not the game data"
    );

    mock.stop();
}
