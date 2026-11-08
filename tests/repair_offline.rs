//! Offline integration / e2e tests for `girpr`.
//!
//! Everything runs against a local mock HoYoPlay + Sophon server
//! (`mod common`, `127.0.0.1:<ephemeral>`). No external network is touched:
//! every `RunCtx` sets `hyp_base_override` / `sophon_base_override` to the
//! mock, so a sandbox without internet still passes. Three cases drive the
//! real binary instead, over those same overrides.
//!
//! This file covers the offline repair run and the run-level contract:
//! - the end-to-end repair of missing and corrupt files, and that a failed
//!   audio move is best-effort while the version still advances;
//! - `--dry-run` writes nothing;
//! - a manifest path that escapes the game dir, and a chunk range that
//!   overflows, are refused before any write;
//! - the per-file write counters and the `--repair-mode` group;
//! - the single-writer lock: a second run is refused, a scoped run takes the
//!   same lock, read-only modes skip it, and a legacy in-install lock is left
//!   alone;
//! - the non-chunk download mode is a usage refusal, including when the branch
//!   call also fails;
//! - manifest selection: no local build, build-order selection, an ignored
//!   category, and an unserved extra manifest.
//!
//! The scope, ceiling, plan-output, write-stop, read-only, and endpoint-override
//! cases live in their own integration files (`tests/repair_scope.rs`,
//! `tests/repair_limits.rs`, `tests/repair_plan.rs`, and so on).

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{
    BranchFailure, ChunkSpec, ExtraManifestSpec, FileSpec, FixtureLatestManifest, FixtureSpec,
    GameFixture, LATEST_TAG, Lies, MockOpts, MockServer, build_fixture, build_fixture_spec,
    ctx_for, field, load_counters, read_game_file, remove_run_lock, run, run_binary, scope,
    temp_dir, write_game_file,
};
use girpr::RunExit;
use girpr::test_support::{Lock, RepairMode, join_url, summary_record};
use std::sync::atomic::Ordering;

/// The extra latest-build manifest the fixture advertises under `field`.
///
/// Selection is by the field `getBuild` advertises, never by position: a
/// fixture that reorders its extra manifests must not swap the identities a
/// test asserts on.
fn extra_manifest<'a>(fx: &'a GameFixture, field: &str) -> &'a FixtureLatestManifest {
    fx.extra_latest_manifests
        .iter()
        .find(|m| m.matching_field == field)
        .unwrap_or_else(|| panic!("the fixture describes the {field} manifest"))
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

    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 2))
        .await
        .expect("run must succeed");
    assert_eq!(code, RunExit::Success, "repair exit");
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
    assert!(cfg.contains(&format!("game_version={LATEST_TAG}")), "{cfg}");

    mock.stop();
}

#[tokio::test]
async fn a_failed_audio_move_is_best_effort_and_still_bumps_the_version() {
    // The audio move is housekeeping: a destination that refuses both the
    // rename and the copy must not fail the run, must keep the source, and
    // must not publish a partial file. The post-phase still reaches its
    // `config.ini` bump, because the repair itself completed.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            cache_dir: "Audio/cache".to_string(),
            res_dir: "Audio/res".to_string(),
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("audio_move_failed");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "Audio/cache/a.pck", b"cache bytes");
    // A directory at the destination makes both the rename and the copy fail.
    std::fs::create_dir_all(dir.join("Audio/res/a.pck")).unwrap();

    let (_summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 2))
        .await
        .expect("a refused audio move is not a run failure");
    assert_eq!(code, RunExit::Success);
    assert_eq!(
        read_game_file(&dir, "Audio/cache/a.pck").unwrap(),
        b"cache bytes",
        "the refused move keeps its source"
    );
    assert!(
        dir.join("Audio/res/a.pck").is_dir(),
        "the destination that refused the publish is untouched"
    );
    let cfg = std::fs::read_to_string(dir.join("config.ini")).expect("config.ini");
    assert!(
        cfg.contains(&format!("game_version={LATEST_TAG}")),
        "the completed repair still advances the version: {cfg}"
    );

    mock.stop();
}

#[tokio::test]
async fn dry_run_writes_nothing() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("dry_run");
    let fx2 = build_fixture();
    // foo missing entirely, bar intact.
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.dry_run = true;
    let (_summary, code) = run(ctx).await.expect("dry-run runs");
    assert_eq!(code, RunExit::Success);
    assert!(
        read_game_file(&dir, &fx2.files[0].rel).is_none(),
        "dry-run must not create the missing file"
    );
    assert!(!dir.join("config.ini").exists());

    mock.stop();
}

#[tokio::test]
async fn a_manifest_path_that_escapes_the_game_dir_is_refused_before_any_write() {
    // The manifest is server data. Every spelling that `Path::join` would use
    // to replace the base — a parent traversal, a drive-relative `C:`, an
    // absolute POSIX path — must fail the plan (metadata, exit 2) before a
    // directory is created or a byte is written.
    for bad in ["../outside.dat", "C:evil.dat", "/abs/evil.dat"] {
        let fx = build_fixture_spec(&FixtureSpec::new(vec![FileSpec::new(
            bad,
            vec![ChunkSpec::new("chunk_evil", vec![0x45u8; 16], 0)],
        )]));
        let mock = MockServer::start(fx, MockOpts::default()).await;
        let dir = temp_dir("escape_manifest");
        let game = dir.join("install");

        let err = run(ctx_for(game.clone(), &mock, 1))
            .await
            .expect_err("an escaping manifest path is metadata");
        assert_eq!(err.exit_code, RunExit::Metadata, "{bad}");
        assert!(!game.exists(), "{bad}: the game dir must not be created");
        assert!(!dir.join("outside.dat").exists(), "{bad}: escaped write");
        assert!(!dir.join("evil.dat").exists(), "{bad}: escaped write");

        mock.stop();
    }
}

#[tokio::test]
async fn a_manifest_offset_overflow_refuses_the_plan_instead_of_panicking() {
    // A malformed range is a **plan** error (metadata, exit `2`) before any
    // mode runs, so there is no unrunnable plan to print and no file task to
    // panic or seek to `i64::MAX`.
    let fx = build_fixture_spec(
        &FixtureSpec::new(vec![FileSpec::new(
            "game/foo.dat",
            vec![ChunkSpec::new("chunk_foo0", vec![0x41u8; 64], 0)],
        )])
        .with_lies(Lies {
            huge_offset: vec!["chunk_foo0".into()],
            ..Default::default()
        }),
    );
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("huge_offset");
    let game = dir.join("install");
    let game_arg = game.to_string_lossy().into_owned();

    let (code, stdout, stderr) = run_binary(
        &[
            "--game-path",
            &game_arg,
            "--biz",
            "hk4e_global",
            "--jobs",
            "1",
            "--print-plan",
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 2, "a corrupt chunk range is metadata: {stderr}");
    assert!(
        !game.exists(),
        "the plan refusal must not create the game dir"
    );
    assert!(
        !dir.join("config.ini").exists(),
        "the version must not be bumped for a refused plan"
    );
    assert!(
        !stdout.contains("PLAN "),
        "an unrunnable plan must not be printed: {stdout}"
    );

    mock.stop();
}

#[tokio::test]
async fn the_run_counts_the_paths_it_used() {
    // The disk claim of the run, in two numbers: how many files were written
    // where they lie, and how many went through a `_tmp`.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("mode_counts");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    let mut half_bad = fx2.files[0].data.clone();
    for b in half_bad.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &half_bad);

    // The default: in place.
    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("run");
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.files_inplace.load(Ordering::Relaxed), 1);
    assert_eq!(summary.files_temp.load(Ordering::Relaxed), 0);

    // The same file, forced through the temp path.
    let dir2 = temp_dir("mode_counts_temp");
    write_game_file(&dir2, &fx2.files[1].rel, &fx2.files[1].data);
    write_game_file(&dir2, &fx2.files[0].rel, &half_bad);
    let mut ctx = ctx_for(dir2.to_path_buf(), &mock, 1);
    ctx.repair_mode = RepairMode::Temp;
    let (summary, code) = run(ctx).await.expect("temp run");
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.files_inplace.load(Ordering::Relaxed), 0);
    assert_eq!(summary.files_temp.load(Ordering::Relaxed), 1);
    // The temp path wrote the whole file; the in-place path wrote one chunk.
    assert_eq!(summary.write.settled(), 128);

    mock.stop();
}

#[tokio::test]
async fn the_repair_mode_group_is_emitted_only_when_the_flag_was_passed() {
    // The disk claim, on the data plane. The four keys travel together and
    // appear only when `--repair-mode` was passed, and `ram_bytes=` only
    // when `--max-ram-bytes` was: an unflagged run's line stays
    // byte-identical.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("mode_group");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    let mut half_bad = fx2.files[0].data.clone();
    for b in half_bad.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &half_bad);

    // Unflagged: nothing.
    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("run");
    assert_eq!(code, RunExit::Success);
    let line = summary_record(&summary, code.code()).text();
    for k in ["repair_mode", "inplace", "temp", "kept_bytes", "ram_bytes"] {
        assert!(
            !line.contains(k),
            "{k} must be absent when unflagged: {line}"
        );
    }

    // Flagged: the mode, the paths it used, and the bytes it kept.
    let dir2 = temp_dir("mode_group_flagged");
    write_game_file(&dir2, &fx2.files[1].rel, &fx2.files[1].data);
    write_game_file(&dir2, &fx2.files[0].rel, &half_bad);
    let mut ctx = ctx_for(dir2.to_path_buf(), &mock, 1);
    ctx.repair_mode = RepairMode::Auto;
    ctx.repair_mode_explicit = true;
    let (summary, code) = run(ctx).await.expect("run");
    assert_eq!(code, RunExit::Success);
    let line = summary_record(&summary, code.code()).text();
    assert!(line.contains("repair_mode=auto"), "{line}");
    assert!(line.contains("inplace=1"), "{line}");
    assert!(line.contains("temp=0"), "{line}");
    // The file is 128 bytes and 64 of them were written: the other 64 were
    // already correct and were left alone.
    assert!(line.contains("kept_bytes=64"), "{line}");
    assert!(!line.contains("ram_bytes"), "{line}");

    // The RAM peak, with a RAM budget asked for.
    let dir3 = temp_dir("mode_group_ram");
    write_game_file(&dir3, &fx2.files[1].rel, &fx2.files[1].data);
    write_game_file(&dir3, &fx2.files[0].rel, &half_bad);
    let mut ctx = ctx_for(dir3.to_path_buf(), &mock, 1);
    ctx.limits.ram_bytes = Some(0);
    let (summary, code) = run(ctx).await.expect("run");
    assert_eq!(code, RunExit::Success);
    let line = summary_record(&summary, code.code()).text();
    assert!(line.contains("ram_bytes=0"), "{line}");

    mock.stop();
}

#[tokio::test]
async fn a_second_run_against_the_same_install_is_refused() {
    // Two runs against one install can write conflicting bytes into the same
    // file. The lock is what stops the second one — and it stops it
    // loudly, by naming the file and the pid that holds it.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("lock");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    // Hold the lock, as a first run would.
    let _lock = Lock::acquire(&dir).expect("the lock is taken");
    let lock_path = _lock.path().to_path_buf();

    let err = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect_err("the second run is refused");
    assert_eq!(
        err.exit_code,
        RunExit::Usage,
        "a live lock is a usage refusal"
    );
    let msg = format!("{:#}", err.source);
    assert!(
        msg.contains(lock_path.to_string_lossy().as_ref()),
        "the refusal names the file: {msg}"
    );
    // An exclusive OS lock can hide the body from another process; either
    // the pid or that fact is named.
    assert!(
        msg.contains(&std::process::id().to_string()) || msg.contains("body is unreadable"),
        "{msg}"
    );

    // A **scoped** run takes the same lock even though the lock's path is
    // outside its include list; otherwise it could write beside the first run.
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.path_scope = scope(&["game/foo.dat"], &[]);
    let err = run(ctx).await.expect_err("a scoped run is refused too");
    assert_eq!(
        err.exit_code,
        RunExit::Usage,
        "a live lock is a usage refusal"
    );

    // The lock is released when the run that holds it ends; the file is left
    // for the next run to reuse rather than unlinked under a racing opener.
    drop(_lock);
    assert!(lock_path.exists(), "the unlocked file stays in place");
    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("the next run is not refused");
    assert_eq!(code, RunExit::Success);
    // foo.dat was missing and is repaired; bar.dat was intact.
    assert_eq!(summary.files_skipped.load(Ordering::Relaxed), 1);
    assert_eq!(summary.files_repaired.load(Ordering::Relaxed), 1);

    mock.stop();
}

#[tokio::test]
async fn read_only_runs_do_not_take_the_lock() {
    // A held lock only refuses a run that mutates the install. Verification,
    // a dry run and a printed plan stay lock-free.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("lock_readonly");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let _lock = Lock::acquire(&dir).expect("the lock is taken");

    let mut check = ctx_for(dir.to_path_buf(), &mock, 1);
    check.check_only = true;
    let (_, code) = run(check).await.expect("check-only ignores the lock");
    assert_eq!(code, RunExit::Success);

    let mut dry = ctx_for(dir.to_path_buf(), &mock, 1);
    dry.dry_run = true;
    let (_, code) = run(dry).await.expect("dry-run ignores the lock");
    assert_eq!(code, RunExit::Success);

    let mut plan = ctx_for(dir.to_path_buf(), &mock, 1);
    plan.print_plan = true;
    let (_, code) = run(plan).await.expect("print-plan ignores the lock");
    assert_eq!(code, RunExit::Success);

    drop(_lock);
    remove_run_lock(&dir);
    mock.stop();
}

#[tokio::test]
async fn a_legacy_in_install_lock_is_left_alone() {
    // Older runs put the lock at the game dir's root. This run locks outside
    // the install and must not delete the old file behind the user's back;
    // the files-cleanup keeps it for the same reason.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("lock_purge");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 64]);
    let legacy = dir.join("girpr.lock");
    std::fs::write(&legacy, b"pid=1\n").unwrap();

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.purge_after = true;
    let (_, code) = run(ctx).await.expect("the purge runs");
    assert_eq!(code, RunExit::Success);
    // The old lock is an ordinary unexpected file that the cleanup keeps;
    // only the new temp-dir lock is the run's own.
    assert!(legacy.exists(), "the legacy in-install lock is left alone");
    assert!(
        read_game_file(&dir, "stray/orphan.dat").is_none(),
        "the orphan is still purged"
    );

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
        let err = run(ctx_for(dir.to_path_buf(), &mock, 1))
            .await
            .expect_err("non-chunk mode must fail");
        assert_eq!(
            err.exit_code,
            RunExit::Usage,
            "mode {mode:?} must be a usage error"
        );
        assert!(
            err.source.to_string().contains("not chunk mode"),
            "mode {mode:?}: {err}"
        );
        // No manifests were fetched and nothing was written.
        assert!(
            mock.first_target_with("getBuild").is_none(),
            "mode {mode:?}"
        );
        assert!(!dir.join("config.ini").exists(), "mode {mode:?}");
        mock.stop();
    }
}

#[tokio::test]
async fn non_chunk_mode_wins_over_a_failed_branch_call() {
    // The first wave issues both calls. A branch endpoint that fails must not
    // turn the authoritative mode refusal into a metadata error: the mode
    // check runs after the config answer but before the branch result is used,
    // and a still-pending branch call is dropped, so exit 1 wins over exit 2.
    //
    // Holding the config answer until the branch answer is chosen pins the
    // branch as issued and failed, rather than leaving that to which request
    // the selector happens to poll first.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            download_mode: Some("DOWNLOAD_MODE_FILE".to_string()),
            branch_failure: BranchFailure::NonJson,
            config_after_branches: true,
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("dlmode_branch");
    let err = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect_err("non-chunk mode must fail");
    assert_eq!(
        err.exit_code,
        RunExit::Usage,
        "the mode refusal wins: {err:#}"
    );
    assert!(
        err.source.to_string().contains("not chunk mode"),
        "the refusal is the mode one: {err:#}"
    );
    assert!(
        mock.first_target_with("getGameBranches").is_some(),
        "the branch call was issued and then discarded"
    );
    assert!(
        mock.first_target_with("getBuild").is_none(),
        "no build call may follow a refused mode"
    );

    mock.stop();
}

#[tokio::test]
async fn non_chunk_mode_wins_over_a_terminal_branch_failure() {
    // A branch answer with a terminal retcode is final on the first attempt,
    // and the config's mode is still the authoritative answer: the run is a
    // usage refusal, not a metadata failure. Holding the config until the
    // branch has answered pins the branch as issued and already failed.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            download_mode: Some("DOWNLOAD_MODE_FILE".to_string()),
            branch_failure: BranchFailure::terminal(-1001),
            config_after_branches: true,
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("dlmode_branch_terminal");
    let err = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect_err("non-chunk mode must fail");
    assert_eq!(
        err.exit_code,
        RunExit::Usage,
        "the mode refusal wins over a terminal branch failure: {err:#}"
    );
    assert!(
        err.source.to_string().contains("not chunk mode"),
        "the refusal is the mode one, not the branch one: {err:#}"
    );
    assert!(
        !err.source.to_string().contains("getGameBranches"),
        "the terminal branch error must not replace the mode refusal: {err:#}"
    );
    assert!(
        mock.first_target_with("getGameBranches").is_some(),
        "the branch call was issued and then discarded"
    );
    assert!(
        mock.first_target_with("getBuild").is_none(),
        "no build call may follow a refused mode"
    );

    mock.stop();
}

#[tokio::test]
async fn a_non_json_branch_body_is_metadata_under_chunk_mode() {
    // Under the mode this tool implements, a branch answer whose body is not
    // JSON fails to parse on the first attempt, and that fatal parse error is a
    // metadata failure that names the branch call. The run must stop before any
    // build or write.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            branch_failure: BranchFailure::NonJson,
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("branch_terminal_metadata");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }

    let err = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect_err("a terminal branch failure must stop the run");
    assert_eq!(
        err.exit_code,
        RunExit::Metadata,
        "a refused branch call is metadata: {err:#}"
    );
    let text = format!("{:#}", err);
    assert!(text.contains("getGameBranches"), "{text}");
    assert!(
        !text.contains("getGameConfigs"),
        "the config call succeeded, so it must not be blamed: {text}"
    );
    assert!(
        mock.first_target_with("getBuild").is_none(),
        "no build call may follow a failed branch call"
    );
    let branch_requests = mock
        .request_targets()
        .iter()
        .filter(|target| target.contains("getGameBranches"))
        .count();
    assert_eq!(
        branch_requests, 1,
        "a non-JSON failure is fatal, so the call is not retried"
    );
    assert!(
        !dir.join("config.ini").exists(),
        "the version must not be bumped for a failed metadata phase"
    );

    mock.stop();
}

#[tokio::test]
async fn a_retryable_branch_failure_is_retried_then_succeeds() {
    // A JSON wrapper with a non-terminal retcode is transient: the client
    // retries it and takes the next (successful) answer, so the run proceeds.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            branch_failure: BranchFailure::Retryable { calls: 2 },
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("branch_retry_succeeds");

    let (_, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("the third branch answer succeeds");
    assert_eq!(code, RunExit::Success);

    let branch_requests = mock
        .request_targets()
        .iter()
        .filter(|target| target.contains("getGameBranches"))
        .count();
    assert_eq!(
        branch_requests, 3,
        "two retryable answers and then the successful one"
    );
    assert!(
        mock.first_target_with("getBuild").is_some(),
        "the run must proceed once the branch answers"
    );

    mock.stop();
}

#[tokio::test]
async fn a_retryable_branch_failure_exhausts_the_retry_budget() {
    // The same retryable body on every attempt: the client gives up after its
    // five attempts and the run reports a metadata failure naming the branch
    // call, before any build request or write.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            branch_failure: BranchFailure::Retryable { calls: 5 },
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("branch_retry_exhausted");

    let err = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect_err("a persistently retryable branch answer must stop the run");
    assert_eq!(err.exit_code, RunExit::Metadata);
    let text = format!("{:#}", err);
    assert!(text.contains("getGameBranches"), "{text}");
    assert!(text.contains("after 5 attempts"), "{text}");
    assert!(text.contains("-1"), "the last retcode is reported: {text}");

    let branch_requests = mock
        .request_targets()
        .iter()
        .filter(|target| target.contains("getGameBranches"))
        .count();
    assert_eq!(
        branch_requests, 5,
        "the client's whole retry budget is used"
    );
    assert!(
        mock.first_target_with("getBuild").is_none(),
        "no build call may follow a failed branch call"
    );
    assert!(
        !dir.join("config.ini").exists(),
        "the version must not be bumped for a failed metadata phase"
    );

    mock.stop();
}

#[tokio::test]
async fn a_retried_config_call_after_the_branch_still_answers() {
    // The branch answer is a fact that stays true, not a one-shot permit. A
    // config call that is retried after the branch was already served must
    // answer instead of waiting for another branch response that never comes.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            config_after_branches: true,
            ..Default::default()
        },
    )
    .await;
    let client = reqwest::Client::new();
    let branch = client
        .get(format!("{}/getGameBranches", mock.hyp_base))
        .send()
        .await
        .expect("branch answers");
    assert!(branch.status().is_success(), "{:?}", branch.status());

    let config_url = format!("{}/getGameConfigs", mock.hyp_base);
    for attempt in 0..2 {
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            client.get(&config_url).send(),
        )
        .await
        .unwrap_or_else(|_| panic!("config attempt {attempt} hung"));
        let response = response.expect("config answers");
        assert!(
            response.status().is_success(),
            "attempt {attempt}: {:?}",
            response.status()
        );
    }

    mock.stop();
}

#[tokio::test]
async fn a_run_without_a_local_version_issues_only_the_latest_build() {
    // A fresh install has no `config.ini`, so there is no local tag to ask
    // for: the second build call is not merely allowed to fail, it must not
    // be issued at all.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("no_local_build");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.print_plan = true;
    let (summary, code) = run(ctx).await.expect("print-plan runs");
    assert_eq!(code, RunExit::Success);
    let builds = mock
        .request_targets()
        .into_iter()
        .filter(|t| t.contains("getBuild"))
        .collect::<Vec<_>>();
    assert_eq!(
        builds.len(),
        1,
        "only the latest build is fetched: {builds:?}"
    );
    assert!(
        !builds[0].contains("tag="),
        "the one build request is the latest (tag omitted): {builds:?}"
    );
    assert!(
        summary.flags.local_manifest.is_none(),
        "no local version means no lookup to report"
    );

    mock.stop();
}

#[tokio::test]
async fn several_latest_manifests_are_selected_in_build_order() {
    // The build advertises four manifests: the primary `game`, an unselected
    // audio field, and two ordinary fields that describe the same path. Every
    // selected manifest contributes its own files, but when two describe the
    // same path the earlier selection wins — and an unselected audio field is
    // never fetched at all.
    let fx = build_fixture_spec(
        &FixtureSpec::new(vec![FileSpec::new(
            "game/base.dat",
            vec![ChunkSpec::new("chunk_base", vec![0x41u8; 32], 0)],
        )])
        .with_extra_manifest(ExtraManifestSpec::new(
            "en-us",
            vec![FileSpec::new(
                "audio/english.dat",
                vec![ChunkSpec::new("chunk_en", vec![0x45u8; 16], 0)],
            )],
        ))
        .with_extra_manifest(ExtraManifestSpec::new(
            "alpha",
            vec![FileSpec::new(
                "game/shared.dat",
                vec![ChunkSpec::new("chunk_alpha_shared", vec![0xAAu8; 48], 0)],
            )],
        ))
        .with_extra_manifest(ExtraManifestSpec::new(
            "beta",
            vec![
                FileSpec::new(
                    "game/shared.dat",
                    vec![ChunkSpec::new("chunk_beta_shared", vec![0xBBu8; 48], 0)],
                ),
                FileSpec::new(
                    "game/beta_only.dat",
                    vec![ChunkSpec::new("chunk_beta_only", vec![0xCCu8; 16], 0)],
                ),
            ],
        )),
    );
    let en_id = extra_manifest(&fx, "en-us").id.clone();
    let alpha_id = extra_manifest(&fx, "alpha").id.clone();
    let beta_id = extra_manifest(&fx, "beta").id.clone();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("multi_manifest_order");

    // Explicit `--audio none` makes the audio manifest's exclusion a fact of
    // the fixture rather than of the absent scan file's default.
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.audio_explicit = true;
    ctx.audio.clear();
    let (summary, code) = run(ctx).await.expect("the multi-manifest run succeeds");
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.files_total, 3, "base + shared + beta_only");
    assert_eq!(summary.files_repaired.load(Ordering::Relaxed), 3);

    // Both selected manifests contributed, and the earlier selection's claim
    // about the shared path is the one that was written.
    assert_eq!(
        read_game_file(&dir, "game/base.dat").unwrap(),
        vec![0x41u8; 32]
    );
    assert_eq!(
        read_game_file(&dir, "game/shared.dat").unwrap(),
        vec![0xAAu8; 48],
        "alpha is selected before beta, so alpha's shared path wins"
    );
    assert_eq!(
        read_game_file(&dir, "game/beta_only.dat").unwrap(),
        vec![0xCCu8; 16]
    );
    assert!(
        read_game_file(&dir, "audio/english.dat").is_none(),
        "an unselected audio manifest must not contribute files"
    );

    // Only the selected manifests were requested, and only the chunks of the
    // files the plan kept were downloaded.
    let targets = mock.request_targets();
    assert!(
        targets.iter().any(|t| t.ends_with(&alpha_id)),
        "{targets:?}"
    );
    assert!(targets.iter().any(|t| t.ends_with(&beta_id)), "{targets:?}");
    assert!(
        !targets.iter().any(|t| t.ends_with(&en_id)),
        "the unselected audio manifest was fetched: {targets:?}"
    );
    let hits = mock.chunk_hits();
    assert!(hits.iter().any(|id| id == "chunk_alpha_shared"), "{hits:?}");
    assert!(hits.iter().any(|id| id == "chunk_beta_only"), "{hits:?}");
    assert!(
        !hits.iter().any(|id| id == "chunk_beta_shared"),
        "the duplicate path from the later manifest was dropped: {hits:?}"
    );
    assert!(
        !hits.iter().any(|id| id == "chunk_en"),
        "the unselected audio chunk was fetched: {hits:?}"
    );

    mock.stop();
}

#[tokio::test]
async fn a_failed_selected_manifest_fails_the_binary_without_a_fallback() {
    // The build advertises the primary `game` manifest and a second selected
    // field whose body is gone. The run must fail as metadata (exit 2) instead
    // of quietly planning only the manifest that did answer.
    let fx = build_fixture_spec(
        &FixtureSpec::new(vec![FileSpec::new(
            "game/base.dat",
            vec![ChunkSpec::new("chunk_base", vec![0x41u8; 32], 0)],
        )])
        .with_extra_manifest(
            ExtraManifestSpec::new(
                "extra",
                vec![FileSpec::new(
                    "game/extra.dat",
                    vec![ChunkSpec::new("chunk_extra", vec![0x5Au8; 16], 0)],
                )],
            )
            .not_served(),
        ),
    );
    let game_id = fx.manifest_id.clone();
    let extra_id = extra_manifest(&fx, "extra").id.clone();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("manifest_fallback");
    let game = dir.join("install");
    let game_arg = game.to_string_lossy().into_owned();

    let (code, stdout, stderr) = run_binary(
        &[
            "--game-path",
            &game_arg,
            "--biz",
            "hk4e_global",
            "--jobs",
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
        code, 2,
        "a selected manifest that is gone is metadata: {stderr}"
    );
    assert!(
        stderr.contains("manifest extra"),
        "the failing manifest is named: {stderr}"
    );
    assert!(
        stderr.contains("404"),
        "the failure is the missing body: {stderr}"
    );
    let fatal = stderr
        .lines()
        .find(|line| line.starts_with("FATAL"))
        .unwrap_or_else(|| panic!("a fatal error line: {stderr}"));
    assert!(
        !fatal.contains("manifest game"),
        "the successful manifest must not be blamed: {fatal}"
    );
    assert!(
        !stdout.contains("SUMMARY"),
        "a failed plan must not emit a success summary: {stdout}"
    );
    assert!(!game.exists(), "the plan failed before any write");
    assert!(
        !dir.join("config.ini").exists(),
        "the version was not bumped"
    );
    assert!(
        read_game_file(&dir, "game/extra.dat").is_none(),
        "nothing from the failed manifest was written"
    );

    // Both selected manifests were requested: the run did not skip the failing
    // one, and did not fall back to the one that succeeded.
    let targets = mock.request_targets();
    assert!(targets.iter().any(|t| t.ends_with(&game_id)), "{targets:?}");
    assert!(
        targets.iter().any(|t| t.ends_with(&extra_id)),
        "{targets:?}"
    );

    mock.stop();
}

#[tokio::test]
async fn an_ignored_manifest_category_is_not_planned_by_the_binary() {
    // The `res_category_dir` ignore list drops a manifest by its
    // `category_id` as well as by its `matching_field`. This fixture advertises
    // the category under a field the ignore file does not name, so only the
    // category id can be what excludes it.
    let fx = build_fixture_spec(
        &FixtureSpec::new(vec![FileSpec::new(
            "game/base.dat",
            vec![ChunkSpec::new("chunk_base", vec![0x41u8; 32], 0)],
        )])
        .with_extra_manifest(
            ExtraManifestSpec::new(
                "extra",
                vec![FileSpec::new(
                    "game/skipped.dat",
                    vec![ChunkSpec::new("chunk_skipped", vec![0x5Au8; 16], 0)],
                )],
            )
            .with_category_id("10302"),
        ),
    );
    let extra_id = extra_manifest(&fx, "extra").id.clone();
    let mock = MockServer::start(
        fx,
        MockOpts {
            res_category_dir: "res_category.txt".to_string(),
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("ignored_manifest");
    write_game_file(
        &dir,
        "res_category.txt",
        br#"{"category":"10302","is_delete":true}"#,
    );
    let game_arg = dir.to_string_lossy().into_owned();

    let (code, stdout, stderr) = run_binary(
        &[
            "--game-path",
            &game_arg,
            "--biz",
            "hk4e_global",
            "--jobs",
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
        code, 0,
        "an ignored manifest is not an error: {stderr}\n{stdout}"
    );

    let summary = stdout
        .lines()
        .find(|line| line.starts_with("SUMMARY "))
        .unwrap_or_else(|| panic!("the run ends with a SUMMARY: {stdout}"));
    assert_eq!(
        field(summary, "total"),
        1,
        "only game/base.dat is planned: {summary}"
    );
    assert_eq!(field(summary, "repaired"), 1, "{summary}");
    assert_eq!(field(summary, "failed"), 0, "{summary}");

    assert_eq!(
        read_game_file(&dir, "game/base.dat").unwrap(),
        vec![0x41u8; 32]
    );
    assert!(
        read_game_file(&dir, "game/skipped.dat").is_none(),
        "the ignored manifest contributes no files"
    );

    let targets = mock.request_targets();
    assert!(
        !targets.iter().any(|t| t.ends_with(&extra_id)),
        "the ignored manifest was fetched: {targets:?}"
    );
    assert!(
        !mock.chunk_hits().iter().any(|id| id == "chunk_skipped"),
        "the ignored manifest's chunk was fetched"
    );

    mock.stop();
}

#[tokio::test]
async fn an_unserved_extra_manifest_contributes_no_chunk_bytes() {
    // A manifest that `getBuild` advertises but the origin does not serve is
    // a `404` for its body, so nothing can ever reference its chunks. Its
    // manifest body is still built (it is advertised), but the chunk route
    // must not publish bytes no reachable manifest names — including when it
    // names an id the served side already uses with different bytes.
    let fx = build_fixture_spec(
        &FixtureSpec::new(vec![FileSpec::new(
            "game/base.dat",
            vec![ChunkSpec::new("chunk_shared", vec![0x41u8; 16], 0)],
        )])
        .with_extra_manifest(
            ExtraManifestSpec::new(
                "gone",
                vec![FileSpec::new(
                    "game/gone.dat",
                    vec![
                        // The same id as the served manifest, with different
                        // bytes: merging the absent manifest's chunks would
                        // replace the bytes the served manifest published.
                        ChunkSpec::new("chunk_shared", vec![0x47u8; 16], 0),
                        ChunkSpec::new("chunk_gone", vec![0x47u8; 16], 16),
                    ],
                )],
            )
            .not_served(),
        )
        .with_extra_manifest(ExtraManifestSpec::new(
            "kept",
            vec![FileSpec::new(
                "game/kept.dat",
                vec![ChunkSpec::new("chunk_kept", vec![0x4Bu8; 16], 0)],
            )],
        )),
    );
    let gone_id = extra_manifest(&fx, "gone").id.clone();
    let kept_id = extra_manifest(&fx, "kept").id.clone();

    // The store is what the mock serves: the shared id must still hold the
    // served manifest's bytes, not the absent one's, and the absent-only id
    // must not be published at all.
    let served = fx
        .chunk_store
        .get("chunk_shared")
        .expect("the served manifest publishes its chunk");
    let served = zstd::stream::decode_all(std::io::Cursor::new(served.clone()))
        .expect("the served chunk is a zstd frame");
    assert_eq!(
        served,
        vec![0x41u8; 16],
        "the unserved manifest must not overwrite a served chunk"
    );
    assert!(fx.chunk_store.contains_key("chunk_kept"));
    assert!(
        !fx.chunk_store.contains_key("chunk_gone"),
        "an absent manifest's chunks are unreachable"
    );
    assert!(!fx.corrupt_chunk_store.contains_key("chunk_gone"));

    // The advertisement is the mock's own `getBuild` answer, not a restatement
    // of the fixture description: both extras are present under the identities
    // the build publishes.
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let client = reqwest::Client::new();
    let body: serde_json::Value = client
        .get(format!("{}/getBuild", mock.sophon_base))
        .send()
        .await
        .expect("getBuild answers")
        .json()
        .await
        .expect("getBuild is JSON");
    let manifests = body["data"]["manifests"]
        .as_array()
        .expect("a manifests array");
    let advertised: Vec<(&str, &str)> = manifests
        .iter()
        .map(|m| {
            (
                m["matching_field"]
                    .as_str()
                    .expect("a matching_field per manifest"),
                m["manifest"]["id"].as_str().expect("an id per manifest"),
            )
        })
        .collect();
    assert!(
        advertised.contains(&("gone", gone_id.as_str())),
        "the absent manifest is still advertised: {advertised:?}"
    );
    assert!(
        advertised.contains(&("kept", kept_id.as_str())),
        "the served extra manifest is advertised: {advertised:?}"
    );

    // Servability follows the advertisement: asking the URL the build
    // publishes for each manifest gets the body the origin has, so the absent
    // one is a `404` and the kept one is a `200`. The URL is production's join
    // of a manifest prefix and id.
    let body_url = |field: &str| {
        let manifest = manifests
            .iter()
            .find(|m| m["matching_field"].as_str() == Some(field))
            .unwrap_or_else(|| panic!("the build advertises {field}: {advertised:?}"));
        let prefix = manifest["manifest_download"]["url_prefix"]
            .as_str()
            .expect("a manifest url_prefix per manifest");
        let id = manifest["manifest"]["id"]
            .as_str()
            .expect("an id per manifest");
        join_url(prefix, id)
    };
    let kept_body = client
        .get(body_url("kept"))
        .send()
        .await
        .expect("the kept manifest route answers");
    assert_eq!(
        kept_body.status(),
        reqwest::StatusCode::OK,
        "the advertised kept manifest is served"
    );
    let gone_body = client
        .get(body_url("gone"))
        .send()
        .await
        .expect("the gone manifest route answers");
    assert_eq!(
        gone_body.status(),
        reqwest::StatusCode::NOT_FOUND,
        "the advertised gone manifest is not served"
    );

    mock.stop();
}
