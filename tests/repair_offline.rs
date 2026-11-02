//! Offline integration / e2e tests for `girpr`.
//!
//! Everything runs against a local mock HoYoPlay + Sophon server
//! (`mod common`, `127.0.0.1:<ephemeral>`). No external network is touched:
//! every `RunCtx` sets `hyp_base_override` / `sophon_base_override` to the
//! mock, so a sandbox without internet still passes. Three cases run the
//! real binary instead, over those same overrides.
//!
//! The first ten cases are the repair pipeline (each a full `repair::run`):
//! 1. repair of missing + corrupt files (exit 0, bytes verified, tmp swept)
//! 2. `--check-only` clean (exit 0) and dirty (exit 4, writes nothing)
//! 3. `--dry-run` writes nothing
//! 4. `--purge-after` / `--purge-before` delete orphans into one counter
//! 5. deprecated files deleted in the post phase
//! 6. verified local-slice reuse downloads only the bad chunk (S2)
//! 7. partial `_tmp` prefix resumed by length, gated by final MD5
//! 8. explicit audio selection overwrites the scan file
//! 9. a non-chunk `default_download_mode` is refused with exit 1
//! 10. the deprecated-files call carries the per-launcher channel
//!
//! The rest are grouped by the tier they cover (see
//! [`docs/04-scope-limitation-spec.md`](../docs/04-scope-limitation-spec.md)):
//! - **path scope** — include-only repairs exactly one file, a purge under a
//!   scope leaves the fixture alone, deprecated files out of scope survive, an
//!   `--include` matching nothing and `--audio` under a scope are exit `1`
//! - **resource ceilings** — `--max-files`, `--max-delete-bytes` and
//!   `--max-download-bytes` refuse (exit `5`) or pass on the boundary, a broken
//!   reuse path trips the runtime stop, a refused run leaves no `game_dir`
//! - **`--print-plan`** — reports the plan, writes nothing, `--plan-out` JSON
//!   agrees with the plan total, a ceiling still trips it
//! - **`--max-write-bytes`, `--fail-fast`, `--purge-only`** — the write stop, a
//!   500 origin, and `--purge-only` alone and with `--dry-run`
//! - **read-only discipline** — `--check-only`/`--print-plan` issue no
//!   deprecated-list call (a dry run does, to list its deletes), `download_bytes`
//!   counts only promoted files even after a retry
//! - **endpoint override** — the real binary over `--hyp-base` / `--sophon-base`,
//!   and the explicit-beats-env precedence

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "common/mod.rs"]
mod common;

use common::{
    ChunkSpec, FileSpec, FixtureSpec, Lies, MockOpts, MockServer, build_fixture,
    build_fixture_spec, ctx_for, load_counters, read_game_file, run_binary, scope, temp_dir,
    write_game_file,
};
use girpr::run;
use girpr::test_support::{Lock, RepairMode, summary_record};
use std::sync::atomic::Ordering;

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
    assert_eq!(code, 0);
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
        assert_eq!(err.exit_code, 2, "{bad}");
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
            "--io-threads",
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
    assert_eq!(code, 0);
    assert_eq!(summary.files_inplace.load(Ordering::Relaxed), 1);
    assert_eq!(summary.files_temp.load(Ordering::Relaxed), 0);

    // The same file, forced through the temp path.
    let dir2 = temp_dir("mode_counts_temp");
    write_game_file(&dir2, &fx2.files[1].rel, &fx2.files[1].data);
    write_game_file(&dir2, &fx2.files[0].rel, &half_bad);
    let mut ctx = ctx_for(dir2.to_path_buf(), &mock, 1);
    ctx.repair_mode = RepairMode::Temp;
    let (summary, code) = run(ctx).await.expect("temp run");
    assert_eq!(code, 0);
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
    assert_eq!(code, 0);
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
    assert_eq!(code, 0);
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
    assert_eq!(code, 0);
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
    assert_eq!(err.exit_code, 1, "a live lock is a usage refusal");
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
    assert_eq!(err.exit_code, 1, "a live lock is a usage refusal");

    // The lock is released when the run that holds it ends.
    drop(_lock);
    assert!(!lock_path.exists());
    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("the next run is not refused");
    assert_eq!(code, 0);
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
    assert_eq!(code, 0);

    let mut dry = ctx_for(dir.to_path_buf(), &mock, 1);
    dry.dry_run = true;
    let (_, code) = run(dry).await.expect("dry-run ignores the lock");
    assert_eq!(code, 0);

    let mut plan = ctx_for(dir.to_path_buf(), &mock, 1);
    plan.print_plan = true;
    let (_, code) = run(plan).await.expect("print-plan ignores the lock");
    assert_eq!(code, 0);

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
    assert_eq!(code, 0);
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
        assert_eq!(err.exit_code, 1, "mode {mode:?} must be a usage error");
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
    // check runs after both futures have settled but before the branch result
    // is used, so exit 1 wins over exit 2.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            download_mode: Some("DOWNLOAD_MODE_FILE".to_string()),
            fail_branches: true,
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("dlmode_branch");
    let err = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect_err("non-chunk mode must fail");
    assert_eq!(err.exit_code, 1, "the mode refusal wins: {err:#}");
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
    assert_eq!(code, 0);
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
