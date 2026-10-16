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
//! 7. partial `_tmp` prefix resumed by length, gated by final MD5 (S3)
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
//! - **read-only discipline** — no deprecated-list call, `download_bytes`
//!   counts only promoted files even after a retry
//! - **endpoint override** — the real binary over `--hyp-base` / `--sophon-base`,
//!   and the explicit-beats-env precedence

#[path = "common/mod.rs"]
mod common;

use common::{
    LATEST_TAG, MockOpts, MockServer, build_fixture, read_game_file, temp_dir, write_game_file,
};
use girpr::repair::scope::{Limits, PathScope};
use girpr::{Biz, OutputFormat, RunCtx, repair};
use std::collections::HashSet;
use std::sync::atomic::Ordering;

/// Compile a path scope for a test, the way `Args::into_ctx` does.
fn scope(include: &[&str], exclude: &[&str]) -> PathScope {
    let inc: Vec<String> = include.iter().map(|s| s.to_string()).collect();
    let exc: Vec<String> = exclude.iter().map(|s| s.to_string()).collect();
    PathScope::new(&inc, &exc).expect("test globs must compile")
}

fn ctx_for(game_dir: std::path::PathBuf, mock: &MockServer, jobs: usize) -> RunCtx {
    ctx_for_biz(game_dir, mock, jobs, Biz::Hk4eGlobal)
}

fn ctx_for_biz(game_dir: std::path::PathBuf, mock: &MockServer, jobs: usize, biz: Biz) -> RunCtx {
    RunCtx {
        game_dir,
        biz,
        audio: HashSet::new(),
        audio_explicit: false,
        path_scope: PathScope::default(),
        limits: Limits::default(),
        jobs,
        check_only: false,
        purge_only: false,
        fail_fast: false,
        dry_run: false,
        print_plan: false,
        plan_out: None,
        purge_after: false,
        purge_before: false,
        output: OutputFormat::Text,
        log_file: None,
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
        assert!(
            mock.first_target_with("getBuild").is_none(),
            "mode {mode:?}"
        );
        assert!(!dir.join("config.ini").exists(), "mode {mode:?}");
        mock.stop();
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[tokio::test]
async fn a_run_that_stops_before_acting_creates_no_game_dir() {
    // `create_dir_all` lives in the execution phase, not at the top of
    // the run, so a run that never reaches it (here: the non-chunk
    // download-mode refusal) leaves nothing on disk.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            download_mode: Some("DOWNLOAD_MODE_FILE".to_string()),
            ..Default::default()
        },
    )
    .await;
    let parent = temp_dir("nodir");
    let missing = parent.join("never-created");
    let err = repair::run(ctx_for(missing.clone(), &mock, 1))
        .await
        .expect_err("non-chunk mode must fail");
    assert_eq!(err.exit_code, 1);
    assert!(
        !missing.exists(),
        "a run that refuses must not create game_dir"
    );

    mock.stop();
    std::fs::remove_dir_all(&parent).ok();
}

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

    let mut ctx = ctx_for(dir.clone(), &mock, 1);
    ctx.path_scope = scope(&["game/foo.dat"], &[]);
    let (summary, code) = repair::run(ctx).await.expect("scoped run");
    assert_eq!(code, 0);
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
    let line = girpr::report::summary_record(&summary, code).text();
    assert!(line.contains("scoped=1"), "{line}");
    assert!(line.contains("config_bumped=0"), "{line}");
    assert!(line.contains("plan_files=1"), "{line}");

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
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

    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.path_scope = scope(&["game/foo.dat"], &[]);
    ctx.purge_after = true;
    let (summary, code) = repair::run(ctx).await.expect("scoped purge run");
    assert_eq!(code, 0);

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
    std::fs::remove_dir_all(&dir).ok();
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

    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.purge_after = true;
    let (summary, code) = repair::run(ctx).await.expect("plain purge run");
    assert_eq!(code, 0);
    assert!(
        read_game_file(&dir, "log/output.txt").is_none(),
        "orphan deleted"
    );
    assert!(!summary.flags.scoped);
    assert!(!summary.flags.limited);
    // And the conditional keys are absent, so an unflagged SUMMARY is
    // byte-identical to today's.
    let line = girpr::report::summary_record(&summary, code).text();
    for k in ["scoped", "config_bumped", "plan_files", "limited"] {
        assert!(
            !line.contains(k),
            "{k} must be absent when unflagged: {line}"
        );
    }

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
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
    let mut ctx = ctx_for(dir.clone(), &mock, 1);
    ctx.path_scope = scope(&["game/nope.dat"], &[]);
    let err = repair::run(ctx)
        .await
        .expect_err("an empty scope must fail");
    assert_eq!(err.exit_code, 1);
    let msg = format!("{:#}", err.source);
    assert!(msg.contains("game/nope.dat"), "must name the glob: {msg}");
    assert!(msg.contains("plan_files=0"), "{msg}");

    // `--exclude` matching nothing is unremarkable and must still succeed.
    let mut ctx = ctx_for(dir.clone(), &mock, 1);
    ctx.path_scope = scope(&[], &["no/such/thing"]);
    let (_, code) = repair::run(ctx)
        .await
        .expect("a non-matching exclude is fine");
    assert_eq!(code, 0);

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
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
    let mut ctx = ctx_for(dir.clone(), &mock, 1);
    ctx.path_scope = scope(&["game/foo.dat"], &[]);
    ctx.audio = HashSet::from(["ja-jp".to_string()]);
    ctx.audio_explicit = true;
    let err = repair::run(ctx).await.expect_err("--audio under a scope");
    assert_eq!(err.exit_code, 1);
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
    let mut ctx2 = ctx_for(dir2.clone(), &mock, 1);
    ctx2.path_scope = scope(&["Audio/scan.txt", "game/foo.dat"], &[]);
    ctx2.audio = HashSet::from(["ja-jp".to_string()]);
    ctx2.audio_explicit = true;
    let (_, code) = repair::run(ctx2)
        .await
        .expect("scan file in scope is allowed");
    assert_eq!(code, 0);
    let scan = std::fs::read_to_string(dir2.join("Audio").join("scan.txt")).unwrap();
    assert!(scan.contains("Japanese"), "{scan}");

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
    std::fs::remove_dir_all(&dir2).ok();
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

    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.path_scope = scope(&["old/*", "game/foo.dat", "game/bar.dat"], &[]);
    let (summary, code) = repair::run(ctx).await.expect("scoped deprecated run");
    assert_eq!(code, 0);
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
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// resource ceilings
// ---------------------------------------------------------------------------

/// A game dir that does not exist yet, pointed at by `parent/missing`.
fn absent_game_dir(parent: &std::path::Path) -> std::path::PathBuf {
    parent.join("not-created-yet")
}

#[tokio::test]
async fn max_files_refuses_before_touching_anything() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let parent = temp_dir("max_files");
    let dir = parent.join("game");
    let fx2 = build_fixture();
    // bar corrupt (would be repaired), foo missing (would be downloaded).
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");

    let mut ctx = ctx_for(dir.clone(), &mock, 1);
    ctx.limits.files = Some(1); // the plan is 2 files
    let (summary, code) = repair::run(ctx).await.expect("a refusal is not an error");
    assert_eq!(code, 5, "a tripped ceiling is exit 5");
    assert!(summary.flags.limited);
    assert_eq!(summary.files_total, 2, "plan_files is still reported");
    assert_eq!(
        summary.files_repaired.load(Ordering::Relaxed),
        0,
        "nothing was repaired"
    );
    assert_eq!(summary.deleted_extra_bytes.load(Ordering::Relaxed), 0);
    assert!(
        mock.chunk_hits().is_empty(),
        "not one chunk may be fetched on a refusal"
    );
    assert_eq!(
        read_game_file(&dir, &fx2.files[1].rel).unwrap(),
        b"not the game data",
        "the damaged file is untouched"
    );
    assert!(read_game_file(&dir, &fx2.files[0].rel).is_none());

    // The same refusal against a game dir that does not exist yet: not even
    // the directory may be created, because the ceiling asserts before
    // `create_dir_all`.
    let missing = absent_game_dir(&parent);
    let mut ctx = ctx_for(missing.clone(), &mock, 1);
    ctx.limits.files = Some(1);
    let (_, code) = repair::run(ctx).await.expect("a refusal is not an error");
    assert_eq!(code, 5);
    assert!(!missing.exists(), "a refusal must not even create game_dir");

    mock.stop();
    std::fs::remove_dir_all(&parent).ok();
}

#[tokio::test]
async fn max_files_at_the_boundary_does_not_refuse() {
    // "> N", not ">= N": a run that exactly fits must be allowed through.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_files_edge");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.limits.files = Some(2); // == plan.files.len()
    let (summary, code) = repair::run(ctx).await.expect("the boundary must pass");
    assert_eq!(code, 0);
    assert!(
        summary.flags.limited,
        "the flag was passed, so it is reported"
    );
    assert_eq!(summary.files_skipped.load(Ordering::Relaxed), 2);

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn max_files_zero_is_a_ceiling_not_a_usage_error() {
    // `0` is a perfectly meaningful ceiling: it is not an error on an empty
    // plan and it is on a 3000-file one. Only an unparseable value is
    // exit `1`, and that is clap's job.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_files_zero");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.limits.files = Some(0);
    let (_, code) = repair::run(ctx).await.expect("0 is a ceiling");
    assert_eq!(code, 5, "0 files allowed, 2 planned -> refuse");

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn max_files_is_checked_after_the_scope_filter() {
    // The pairing that gives the flag its purpose: `--include` selects,
    // `--max-files` verifies the selection.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_files_scoped");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[0].rel, b"not the game data");
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");

    let mut ctx = ctx_for(dir.clone(), &mock, 1);
    ctx.path_scope = scope(&["game/foo.dat"], &[]);
    ctx.limits.files = Some(1); // the *filtered* plan is 1 file
    let (summary, code) = repair::run(ctx).await.expect("scoped boundary passes");
    assert_eq!(code, 0);
    assert_eq!(summary.files_total, 1);
    assert!(summary.flags.scoped && summary.flags.limited);

    // Same scope, ceiling of 0: now it refuses.
    let dir2 = temp_dir("max_files_scoped_refuse");
    let mut ctx2 = ctx_for(dir2.clone(), &mock, 1);
    ctx2.path_scope = scope(&["game/foo.dat"], &[]);
    ctx2.limits.files = Some(0);
    let (_, code2) = repair::run(ctx2).await.expect("refusal is not an error");
    assert_eq!(code2, 5);

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
    std::fs::remove_dir_all(&dir2).ok();
}

#[tokio::test]
async fn max_delete_bytes_refuses_and_deletes_nothing() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_delete");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    // Two orphans totalling 170 bytes, which no purge flag would touch yet.
    write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 100]);
    write_game_file(&dir, "other/stale.dat", &[0x5Bu8; 70]);

    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.purge_after = true;
    ctx.limits.delete_bytes = Some(100);
    let (summary, code) = repair::run(ctx).await.expect("refusal is not an error");
    assert_eq!(code, 5, "170 > 100");
    assert_eq!(
        summary.deleted_extra_bytes.load(Ordering::Relaxed),
        0,
        "a refused purge deletes nothing"
    );
    assert!(read_game_file(&dir, "stray/orphan.dat").is_some(), "kept");
    assert!(read_game_file(&dir, "other/stale.dat").is_some(), "kept");

    // Exactly at the ceiling: the purge runs.
    let mut ctx2 = ctx_for(dir.clone(), &mock, 2);
    ctx2.purge_after = true;
    ctx2.limits.delete_bytes = Some(170);
    let (summary2, code2) = repair::run(ctx2).await.expect("the boundary passes");
    assert_eq!(code2, 0);
    assert_eq!(summary2.deleted_extra_bytes.load(Ordering::Relaxed), 170);

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_out_of_scope_path_never_counts_toward_the_delete_ceiling() {
    // The scope and the ceiling are complementary, not redundant: the glob
    // narrows what is *counted*, the ceiling bounds what is *done*.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_delete_scoped");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    // A 1 GB orphan that is deliberately out of scope.
    write_game_file(&dir, "huge/orphan.dat", &[0x5Au8; 4096]);

    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.path_scope = scope(&["game/foo.dat", "game/bar.dat"], &[]);
    ctx.purge_after = true;
    ctx.limits.delete_bytes = Some(1); // would be blown by the orphan
    let (_, code) = repair::run(ctx).await.expect("scoped run");
    assert_eq!(code, 0, "the out-of-scope orphan is not a candidate at all");
    assert!(read_game_file(&dir, "huge/orphan.dat").is_some(), "kept");

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_tripped_ceiling_still_emits_a_summary_tail() {
    // A refusal is *returned*, not raised, so every exit path ends with
    // one uniform SUMMARY.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_files_summary");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.limits.files = Some(1);
    let (summary, code) = repair::run(ctx).await.expect("refusal is not an error");
    let line = girpr::report::summary_record(&summary, code).text();
    assert_eq!(
        line,
        "SUMMARY total=2 skipped=0 repaired=0 failed=0 download_bytes=0 deleted_extra_bytes=0 \
         exit=5 limited=1 plan_files=2"
    );
    assert!(!line.contains("scoped"), "no scope was passed: {line}");

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn check_only_trips_a_ceiling_with_exit_five_not_four() {
    // A ceiling narrows *what is verified*; `4` (damage found) must not be
    // masked by narrowing — but a ceiling is a different fact entirely.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_files_check");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, b"damaged");
    }
    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.check_only = true;
    ctx.limits.files = Some(1);
    let (summary, code) = repair::run(ctx).await.expect("refusal is not an error");
    assert_eq!(code, 5, "the ceiling is asked before the verification");
    assert_eq!(
        summary.files_failed.load(Ordering::Relaxed),
        0,
        "nothing verified"
    );

    // Without the ceiling, the same run reports damage.
    let mut ctx2 = ctx_for(dir.clone(), &mock, 2);
    ctx2.check_only = true;
    let (_, code2) = repair::run(ctx2).await.expect("check runs");
    assert_eq!(code2, 4);

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn max_download_bytes_refuses_on_the_estimate_and_passes_at_the_boundary() {
    // With no local build the reuse map is empty, so the estimate is every
    // chunk's compressed size — the same quantity the runner would download.
    let fx = build_fixture();
    let est = fx.chunk_store.values().map(|b| b.len() as u64).sum::<u64>();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_dl_refuse");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[0].rel, b"not the game data");
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");

    // One byte under the estimate → refuse before fetching anything.
    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.limits.download_bytes = Some(est - 1);
    let (summary, code) = repair::run(ctx).await.expect("a refusal is not an error");
    assert_eq!(code, 5);
    assert_eq!(
        summary.flags.plan_download_bytes,
        Some(est),
        "SUMMARY reports the estimate it refused on"
    );
    assert!(mock.chunk_hits().is_empty(), "nothing may be fetched");
    assert!(!dir.join("game").join("foo.dat_tmp").exists(), "no _tmp");

    // Exactly the estimate → the boundary passes and the whole thing runs.
    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.limits.download_bytes = Some(est);
    let (summary, code) = repair::run(ctx).await.expect("the boundary passes");
    assert_eq!(code, 0);
    assert_eq!(summary.download_bytes.load(Ordering::Relaxed), est);
    let line = girpr::report::summary_record(&summary, code).text();
    assert!(
        line.contains(&format!("plan_download_bytes={est}")),
        "{line}"
    );

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_broken_reuse_path_still_trips_the_runtime_stop() {
    // The case that justifies B2 having *two* mechanisms. `config.ini` already
    // at the latest tag → the local build loads → the reuse map covers both of
    // foo's chunks → the pre-flight estimate is 0 and the run is allowed
    // through. Then foo's bytes are corrupted in *both* halves, so every local
    // slice fails its MD5 and every chunk falls back to a download. The
    // estimate was wrong; the runtime reservation is what stops the run.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_dl_runtime");
    std::fs::write(
        dir.join("config.ini"),
        "[General]\ngame_version=9.9.9-test\n",
    )
    .unwrap();
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data); // bar intact
    let mut all_bad = fx2.files[0].data.clone();
    for b in all_bad.iter_mut() {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &all_bad);

    let limit = 1u64; // any positive value: the estimate is 0, so it passes
    let mut ctx = ctx_for(dir.clone(), &mock, 1);
    ctx.limits.download_bytes = Some(limit);
    let (summary, code) = repair::run(ctx).await.expect("a stop is not an error");
    assert_eq!(code, 5, "the runtime hard stop is also exit 5");
    assert_eq!(
        summary.flags.stopped,
        Some("bytes"),
        "the stop must be distinguishable from a refusal"
    );
    assert_eq!(
        summary.flags.plan_download_bytes,
        Some(0),
        "the estimate really was zero — that is the point"
    );
    assert_eq!(
        summary.files_failed.load(Ordering::Relaxed),
        0,
        "not a failure"
    );
    assert_eq!(summary.files_repaired.load(Ordering::Relaxed), 0);
    assert_eq!(
        summary.files_skipped.load(Ordering::Relaxed),
        1,
        "bar was intact"
    );
    assert!(
        summary.download_bytes.load(Ordering::Relaxed) <= limit,
        "the ceiling must not be overshot"
    );
    // The stopped file keeps its `_tmp`; S3 means the next run resumes it.
    assert_ne!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data,
        "a stopped file is not promoted"
    );

    let line = girpr::report::summary_record(&summary, code).text();
    assert!(line.contains("stopped=bytes"), "{line}");
    assert!(line.contains("limited=1"), "{line}");

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_summary_growth_keys_appear_only_when_their_flag_is_active() {
    // Every scope/ceiling key is emitted only when its flag is active, so
    // an unflagged run stays byte-identical to one that passes no flags.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("summary_absent");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let (summary, code) = repair::run(ctx_for(dir.clone(), &mock, 2))
        .await
        .expect("plain run");
    assert_eq!(code, 0);
    for format in [OutputFormat::Text, OutputFormat::Json] {
        let line = girpr::report::summary_record(&summary, code).render(format);
        for k in [
            "limited",
            "scoped",
            "config_bumped",
            "plan_files",
            "plan_download_bytes",
            "write_bytes",
            "stopped",
            // A fresh install has no `config.ini`, so no local version is known
            // and the lookup is never attempted — the key must not appear.
            "local_manifest",
        ] {
            assert!(
                !line.contains(k),
                "{k} must be absent when unflagged: {line}"
            );
        }
    }
    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn print_plan_reports_the_plan_and_writes_nothing() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("print_plan");
    let fx2 = build_fixture();
    // foo missing (would be repaired), bar corrupt, plus an orphan that a purge
    // would delete and a `*_tmp` that would be swept.
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");
    write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 4096]);
    write_game_file(&dir, "game/bar.dat_tmp", &[0x6Bu8; 16]);

    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.print_plan = true;
    ctx.purge_after = true;
    let (summary, code) = repair::run(ctx).await.expect("print-plan runs");
    assert_eq!(code, 0);

    // The counters report a plan, not a repair.
    let (skipped, repaired, failed, dl, deleted) = load_counters(&summary);
    assert_eq!((skipped, repaired, failed, dl, deleted), (0, 0, 0, 0, 0));
    assert!(mock.chunk_hits().is_empty(), "no chunk may be fetched");

    // Read-only by construction: nothing on disk moved.
    assert!(
        read_game_file(&dir, "stray/orphan.dat").is_some(),
        "orphan kept"
    );
    assert!(dir.join("game").join("bar.dat_tmp").exists(), "_tmp kept");
    assert_eq!(
        read_game_file(&dir, &fx2.files[1].rel).unwrap(),
        b"not the game data",
        "the damaged file was not touched"
    );
    assert!(
        read_game_file(&dir, &fx2.files[0].rel).is_none(),
        "foo not created"
    );
    assert!(!dir.join("config.ini").exists(), "config.ini not bumped");

    // The numbers a real run would act on are visible without doing it: the
    // plan carried both files and the purge plan carried the orphan.
    assert_eq!(summary.files_total, 2);
    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn print_plan_out_writes_the_per_file_json_document() {
    let fx = build_fixture();
    let est: u64 = fx.chunk_store.values().map(|b| b.len() as u64).sum();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("plan_out");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 2048]);

    let out = dir.join("reports").join("plan.json");
    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.print_plan = true;
    ctx.purge_after = true;
    ctx.plan_out = Some(out.clone());
    let (_, code) = repair::run(ctx).await.expect("print-plan runs");
    assert_eq!(code, 0);

    let written = std::fs::read_to_string(&out).expect("--plan-out file");
    let v: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
    assert_eq!(v["repair"].as_array().unwrap().len(), 2);
    let rels: Vec<&str> = v["repair"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["rel"].as_str().unwrap())
        .collect();
    assert_eq!(rels, vec!["game/bar.dat", "game/foo.dat"], "plan order");
    // The per-file estimates must add up to the plan's own figure — the same
    // number `SUMMARY plan_download_bytes` and the ceiling are checked against.
    let sum: u64 = v["repair"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["est_download_bytes"].as_u64().unwrap())
        .sum();
    assert_eq!(sum, est, "document must agree with the plan total");
    assert_eq!(v["purge"][0]["rel"], "stray/orphan.dat");
    assert_eq!(v["purge"][0]["size"], 2048);

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_ceiling_trips_print_plan_like_any_other_mode() {
    let fx = build_fixture();
    let est: u64 = fx.chunk_store.values().map(|b| b.len() as u64).sum();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("print_plan_limit");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.print_plan = true;
    ctx.limits.download_bytes = Some(est - 1);
    let (summary, code) = repair::run(ctx).await.expect("a refusal is not an error");
    assert_eq!(code, 5, "the assert comes before the plan report");
    assert_eq!(summary.flags.plan_download_bytes, Some(est));
}

#[tokio::test]
async fn print_plan_does_not_imply_check_only() {
    // "what would you do", not "what is wrong": a damaged file is reported as
    // planned work, never as damage, and exit is `0` either way.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("print_plan_not_check");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[0].rel, b"corrupt");
    write_game_file(&dir, &fx2.files[1].rel, b"corrupt");

    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.print_plan = true;
    let (summary, code) = repair::run(ctx).await.expect("print-plan runs");
    assert_eq!(code, 0);
    assert_eq!(summary.files_failed.load(Ordering::Relaxed), 0);
    assert_eq!(summary.files_skipped.load(Ordering::Relaxed), 0);

    // The same tree under --check-only does report damage.
    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.check_only = true;
    let (_, code) = repair::run(ctx).await.expect("check runs");
    assert_eq!(code, 4);

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn print_plan_with_dry_run_is_coherent() {
    // The plan prints first; then `--dry-run` does what it was asked for —
    // walk the selected files and hash them, writing nothing.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("print_plan_dry");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.print_plan = true;
    ctx.dry_run = true;
    let (summary, code) = repair::run(ctx).await.expect("print-plan + dry-run runs");
    assert_eq!(code, 0);
    assert_eq!(summary.files_total, 2);
    assert!(!dir.join("config.ini").exists(), "dry-run writes nothing");

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// --max-write-bytes, --fail-fast, --purge-only
// ---------------------------------------------------------------------------

#[tokio::test]
async fn max_write_bytes_stops_mid_flight_and_leaves_the_tmp() {
    // The only ceiling that cannot be a pre-flight refusal: reuse success is
    // decided per chunk at runtime, against bytes that may not be there. This
    // run downloads both files, so almost every byte written into `*_tmp` is a
    // write the plan's estimate never counted.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_write");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[0].rel, b"not the game data");
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");

    let limit = 1u64;
    let mut ctx = ctx_for(dir.clone(), &mock, 1);
    ctx.limits.write_bytes = Some(limit);
    let (summary, code) = repair::run(ctx).await.expect("a stop is not an error");
    assert_eq!(code, 5);
    assert_eq!(summary.flags.stopped, Some("write"), "which ceiling bound");
    assert!(summary.flags.limited, "the ceiling was requested");
    assert_eq!(
        summary.files_failed.load(Ordering::Relaxed),
        0,
        "not a failure"
    );
    assert!(
        summary.write_bytes.load(Ordering::Relaxed) <= limit,
        "the ceiling must not be overshot"
    );

    let line = girpr::report::summary_record(&summary, code).text();
    assert!(line.contains("stopped=write"), "{line}");
    assert!(line.contains("write_bytes="), "{line}");

    // The un-promoted `_tmp` is left in place: resume-by-length makes the next
    // run a smaller run, not a corrupt one.
    assert!(!dir.join("config.ini").exists(), "post-phase is skipped");
    assert_ne!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data,
        "nothing was promoted"
    );

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn write_bytes_is_counted_only_when_the_flag_is_active() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("write_bytes_absent");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[0].rel, b"not the game data");
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");

    // Same run, no flag: the repair completes and the key is absent.
    let (summary, code) = repair::run(ctx_for(dir.clone(), &mock, 1))
        .await
        .expect("run");
    assert_eq!(code, 0);
    assert!(
        summary.write_bytes.load(Ordering::Relaxed) > 0,
        "it was counted"
    );
    let line = girpr::report::summary_record(&summary, code).text();
    assert!(!line.contains("write_bytes"), "{line}");
    // `download_bytes` is the network; `write_bytes` is the disk. They are not the
    // same quantity and never were — that is the whole reason `--max-write-bytes`
    // exists. Here every byte written came from a download, so write_bytes is
    // simply the *uncompressed* size, which is strictly larger.
    assert!(
        summary.write_bytes.load(Ordering::Relaxed)
            > summary.download_bytes.load(Ordering::Relaxed),
        "write_bytes counts uncompressed payload, download_bytes the compressed blob"
    );

    // A generous ceiling is reported but not tripped.
    let dir2 = temp_dir("write_bytes_generous");
    let fx3 = build_fixture();
    write_game_file(&dir2, &fx3.files[0].rel, b"not the game data");
    write_game_file(&dir2, &fx3.files[1].rel, b"not the game data");
    let mut ctx = ctx_for(dir2.clone(), &mock, 1);
    ctx.limits.write_bytes = Some(1 << 20);
    let (summary, code) = repair::run(ctx).await.expect("run");
    assert_eq!(code, 0);
    assert!(summary.flags.stopped.is_none());
    let line = girpr::report::summary_record(&summary, code).text();
    assert!(line.contains("write_bytes="), "{line}");
    assert!(!line.contains("stopped="), "{line}");

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
    std::fs::remove_dir_all(&dir2).ok();
}

#[tokio::test]
async fn fail_fast_reaches_its_summary_in_seconds_against_a_500_origin() {
    // Without `--fail-fast` each failing file costs `MAX_ATTEMPTS` × (the
    // 5× retry with 1-5 s sleeps inside `fetch_chunk_bytes` + 1-4 s of
    // file-level backoff) ≈ **125 s of pure sleeping**, measured — and that is
    // before `HypClient::new_with_bases` is even given a connect timeout. With
    // it, a file gets one attempt and the abort spares every file behind it.
    //
    // There is deliberately no "without the flag" control here: proving that
    // case means paying those 125 s, and it is already covered structurally —
    // `repairs_missing_and_corrupt_files_end_to_end` shows every planned file is
    // attempted when nothing is flagged.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            fail_chunks: true,
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("fail_fast");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[0].rel, b"not the game data");
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");

    let mut ctx = ctx_for(dir.clone(), &mock, 1);
    ctx.fail_fast = true;
    let started = std::time::Instant::now();
    let (summary, code) = repair::run(ctx)
        .await
        .expect("a failed repair is not an Err");
    let elapsed = started.elapsed();
    assert_eq!(code, 3, "a file failed, so the post-phase is skipped");
    assert_eq!(
        summary.files_failed.load(Ordering::Relaxed),
        1,
        "honest count"
    );
    assert_eq!(summary.files_repaired.load(Ordering::Relaxed), 0);
    assert!(!dir.join("config.ini").exists());
    // Every chunk request 500s, so nothing was ever delivered. A reservation
    // that the failed fetch never gave back would still be sitting in the
    // total — and with no ceiling set the sentinel is u64::MAX, so the leak is
    // not gated on `--max-download-bytes`.
    assert_eq!(
        summary.download_bytes.load(Ordering::Relaxed),
        0,
        "a fetch that delivered nothing must count nothing"
    );
    // The second file was never even asked for. The plan is sorted, so with
    // `jobs=1` `bar.dat` runs first, exhausts its retries and trips the abort;
    // `foo.dat` then drops out at its task entry without opening a connection.
    let hits = mock.chunk_hits();
    assert!(
        hits.iter().all(|id| id == "chunk_bar0"),
        "foo.dat must not be attempted after the abort: {hits:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "fail-fast must reach its SUMMARY in seconds, took {elapsed:?}"
    );
    assert!(
        summary.files_skipped.load(Ordering::Relaxed)
            + summary.files_repaired.load(Ordering::Relaxed)
            + summary.files_failed.load(Ordering::Relaxed)
            < summary.files_total,
        "the run was truncated before the plan was finished"
    );

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_404_is_asked_once_where_a_500_is_asked_five_times() {
    // The two halves of the retry policy, from the origin's side of the wire.
    //
    // A 404 cannot be fixed by asking again: the chunk id came out of this
    // build's own `getBuild` body, and the live blob routes answer a missing
    // object with 404 + `application/xml`, never 200-with-a-code. A 500 is a
    // server that may come back, so it is retried on the linear backoff.
    //
    // Measured against a real bundle before this policy existed: one 404'd chunk
    // cost 88.5 s and 25 requests, and still exited `3` — the same exit code it
    // reaches now without waiting at all, having asked once.
    //
    // `fail_fast` gives a file one attempt, so the two branches are directly
    // comparable: the 500 spends the chunk fetch's 5 retries on that one attempt,
    // and the 404 spends none of them.
    for (name, opts, want_hits, max_elapsed) in [
        (
            "404",
            MockOpts {
                not_found_chunks: true,
                ..Default::default()
            },
            1usize,
            std::time::Duration::from_secs(5),
        ),
        (
            "500",
            MockOpts {
                fail_chunks: true,
                ..Default::default()
            },
            ATTEMPTS,
            std::time::Duration::from_secs(30),
        ),
    ] {
        let fx = build_fixture();
        let mock = MockServer::start(fx, opts).await;
        let dir = temp_dir(&format!("retry_{name}"));
        let fx2 = build_fixture();
        write_game_file(&dir, &fx2.files[0].rel, b"not the game data");
        // `--include` so exactly one file is in scope, and `fail_fast` so a
        // single file's retry budget is the whole number being measured rather
        // than being multiplied by the rest of the plan.
        let mut ctx = ctx_for(dir.clone(), &mock, 1);
        ctx.fail_fast = true;
        ctx.path_scope = scope(&[&fx2.files[0].rel], &[]);
        let started = std::time::Instant::now();
        let (summary, code) = repair::run(ctx).await.expect("not an Err");
        let elapsed = started.elapsed();

        assert_eq!(
            code, 3,
            "{name}: a file failed, so the post-phase is skipped"
        );
        assert_eq!(summary.files_failed.load(Ordering::Relaxed), 1, "{name}");
        let hits = mock.chunk_hits();
        assert_eq!(
            hits.len(),
            want_hits,
            "{name}: the origin was asked {want_hits} time(s): {hits:?}"
        );
        assert!(hits.iter().all(|id| id == "chunk_foo0"), "{name}: {hits:?}");
        assert!(
            elapsed < max_elapsed,
            "{name} must not spend the budget in sleep, took {elapsed:?}"
        );
        mock.stop();
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// How many times `fetch_chunk_bytes` asks a *retryable* origin — the linear
/// 1+2+3+4 s backoff, five attempts. Kept next to the test that depends on it so
/// the two cannot drift.
const ATTEMPTS: usize = 5;

#[tokio::test]
async fn a_404_manifest_fails_the_run_without_waiting_for_it() {
    // The manifest half. Without a terminal status this costs ~15 s of backoff
    // to learn that the build's manifests are gone, and then exits `2` anyway —
    // metadata, not a repaired file, so the exit code is unchanged.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            not_found_manifests: true,
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("manifest_404");
    let started = std::time::Instant::now();
    let err = repair::run(ctx_for(dir.clone(), &mock, 1))
        .await
        .expect_err("no manifests means the run cannot plan");
    let elapsed = started.elapsed();
    assert_eq!(err.exit_code, 2, "metadata, not a write failure");
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "a 404 must not be retried at all, took {elapsed:?}"
    );
    // Asked once. The manifest route has no per-chunk log, so this reads the
    // server's own request log.
    let manifests = mock
        .request_targets()
        .into_iter()
        .filter(|t| t.contains("/manifests/"))
        .count();
    assert_eq!(manifests, 1, "the manifest was asked once");

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn purge_only_deletes_a_scoped_path_that_is_absent_from_the_new_manifest() {
    // The `removed` capture class: a path on disk that the new manifest no
    // longer contains. It is in no repair plan by construction, so the only
    // work that can touch it is the cleanup — which is exactly what
    // `--purge-only --include <that path>` asks for.
    //
    // The guard that says "an `--include` matching nothing is a typo" used to
    // test `plan.files.is_empty()` and run *before* the purge plan existed, so
    // it refused this as a typo. Empty repair plan + non-empty purge candidates
    // is a legitimate request, not a typo.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("purge_only_removed");
    let fx2 = build_fixture();
    // Intact manifest files, so the only candidate is the removed one.
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "game/removed.dat", &[0x7Eu8; 512]);

    let mut ctx = ctx_for(dir.clone(), &mock, 1);
    ctx.purge_only = true;
    ctx.path_scope = scope(&["game/removed.dat"], &[]);
    let (summary, code) = repair::run(ctx)
        .await
        .expect("a cleanup run must not be refused");
    assert_eq!(
        code, 0,
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
    let line = girpr::report::summary_record(&summary, code).text();
    assert!(line.contains("config_bumped=0"), "{line}");
    assert!(line.contains("scoped=1"), "{line}");

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
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
    let mut ctx = ctx_for(dir.clone(), &mock, 1);
    ctx.purge_only = true; // asks for the cleanup…
    ctx.path_scope = scope(&["game/never-existed.dat"], &[]); // …of nothing
    let err = repair::run(ctx)
        .await
        .expect_err("a scope that selects nothing is still a typo");
    assert_eq!(err.exit_code, 1);
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
    std::fs::remove_dir_all(&dir).ok();
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

    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.purge_only = true;
    let (summary, code) = repair::run(ctx).await.expect("purge-only runs");
    assert_eq!(code, 0);
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
    let line = girpr::report::summary_record(&summary, code).text();
    assert!(line.contains("config_bumped=0"), "{line}");

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn purge_only_with_dry_run_deletes_and_moves_nothing() {
    // `--dry-run` is documented as writing nothing, and `post::run` used to
    // rely on `repair::run` reaching it only through `if !readonly`. The
    // `--purge-only` early return is a second call site that is not behind that
    // gate, so the three destructive steps of the post-phase — deprecated
    // deletes, the audio cache move, the cleanup pass — all ran for real.
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

    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.purge_only = true;
    ctx.dry_run = true;
    let (summary, code) = repair::run(ctx).await.expect("purge-only --dry-run runs");
    assert_eq!(code, 0);

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
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_read_only_run_issues_no_deprecated_file_call() {
    // The deprecated list is fetched during the plan phase, and read-only
    // modes skip that phase's network calls: `--dry-run`, `--check-only` and
    // `--print-plan` must issue zero `getGameDeprecatedFileConfigs` requests.
    for mode in ["dry", "check", "plan"] {
        let fx = build_fixture();
        let mock = MockServer::start(
            fx,
            MockOpts {
                deprecated_files: vec!["old/legacy.dll".to_string()],
                ..Default::default()
            },
        )
        .await;
        let dir = temp_dir("readonly_deprecated");
        let fx2 = build_fixture();
        for f in &fx2.files {
            write_game_file(&dir, &f.rel, &f.data);
        }
        write_game_file(&dir, "old/legacy.dll", &[0x7Eu8; 50]);

        let mut ctx = ctx_for(dir.clone(), &mock, 2);
        match mode {
            "dry" => ctx.dry_run = true,
            "check" => ctx.check_only = true,
            _ => ctx.print_plan = true,
        }
        repair::run(ctx).await.expect("run");
        assert!(
            mock.first_target_with("getGameDeprecatedFileConfigs")
                .is_none(),
            "--{mode} must not fetch the deprecated list it would discard"
        );
        assert!(read_game_file(&dir, "old/legacy.dll").is_some());
        mock.stop();
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[tokio::test]
async fn download_bytes_counts_only_promoted_files_even_after_a_retry() {
    // `download_bytes` is the bytes of files that were actually
    // promoted. The counter is live per-chunk so the runtime hard stop can
    // read it mid-flight, which puts two ways of inflating it in reach of an
    // unflagged run — and both are exactly what an *abandoned* attempt looks
    // like:
    //
    //  - a reservation the `?` on a failed fetch never gave back, and
    //  - the bytes that attempt had already downloaded.
    //
    // The mock serves one corrupt-but-well-formed chunk, so the first attempt
    // really does transfer bytes and then fail its MD5.
    let fx = build_fixture();
    let est: u64 = fx.chunk_store.values().map(|b| b.len() as u64).sum();
    let mock = MockServer::start(
        fx,
        MockOpts {
            corrupt_first_chunks: 1,
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("download_retry");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");

    let (summary, code) = repair::run(ctx_for(dir.clone(), &mock, 1))
        .await
        .expect("the retry must recover");
    assert_eq!(code, 0);
    assert_eq!(
        read_game_file(&dir, &fx2.files[1].rel).unwrap(),
        fx2.files[1].data
    );
    // bar.dat sorts first and its single chunk is the corrupt one, so attempt 1
    // really transfers a byte and then fails the chunk MD5. The total is still
    // every chunk in the plan and nothing more: the abandoned attempt's byte is
    // given back.
    assert_eq!(
        summary.download_bytes.load(Ordering::Relaxed),
        est,
        "a failed attempt's bytes must not survive into the total"
    );
    assert_eq!(
        summary.files_repaired.load(Ordering::Relaxed),
        2,
        "both files were repaired, the first one on its second attempt"
    );

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
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
    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.check_only = true;
    let (_, code) = repair::run(ctx).await.expect("check runs");
    assert_eq!(code, 4);
    assert!(read_game_file(&dir, "stray/orphan.dat").is_some());

    // purge-only: the orphan is gone, the damage is untouched.
    let mut ctx = ctx_for(dir.clone(), &mock, 2);
    ctx.purge_only = true;
    let (_, code) = repair::run(ctx).await.expect("purge-only runs");
    assert_eq!(code, 0);
    assert!(read_game_file(&dir, "stray/orphan.dat").is_none());
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        b"not the game data"
    );

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_reuse_run_and_a_no_reuse_run_are_distinguishable_on_the_data_plane() {
    // A run that reused everything and a run that could not reuse anything are
    // otherwise identical on the data plane: both exit `0`, and the second's
    // `download_bytes` is simply larger. That matters most to
    // `--max-download-bytes`, whose pre-flight estimate is computed against the
    // same empty map — so a degraded run silently moves the ceiling and the
    // refusal a caller expected never fires.
    //
    // The mock answers `getBuild?tag=<local>` with the manifest only when the
    // tag is the latest one, which is exactly the "the local manifest loaded" /
    // "the server no longer serves that tag" pair.
    let est: u64 = build_fixture()
        .chunk_store
        .values()
        .map(|b| b.len() as u64)
        .sum();

    // (a) local version at the latest tag → the manifest loads → reuse works.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("local_manifest_ok");
    std::fs::write(
        dir.join("config.ini"),
        format!("[General]\ngame_version={}\n", LATEST_TAG),
    )
    .unwrap();
    let fx2 = build_fixture();
    // bar intact (skipped) and foo half-intact, so its first chunk is copied
    // from a verified local slice and only the second is fetched. That is a real
    // reuse, not an absent one.
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    let mut half_bad = fx2.files[0].data.clone();
    for b in half_bad.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &half_bad);
    let (summary, code) = repair::run(ctx_for(dir.clone(), &mock, 1))
        .await
        .expect("run");
    assert_eq!(code, 0);
    let line = girpr::report::summary_record(&summary, code).text();
    assert!(line.contains("local_manifest=ok"), "{line}");
    assert_eq!(mock.chunk_hits(), vec!["chunk_foo1".to_string()]);
    assert!(
        summary.download_bytes.load(Ordering::Relaxed) < est,
        "a run with a loaded local manifest reuses something: {} vs {est}",
        summary.download_bytes.load(Ordering::Relaxed)
    );
    mock.stop();
    std::fs::remove_dir_all(&dir).ok();

    // (b) a local version the server does not serve → lookup attempted and
    //     failed → the fallback is full-fetch, and the run says so.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("local_manifest_gone");
    std::fs::write(
        dir.join("config.ini"),
        "[General]\ngame_version=1.0.0-long-gone\n",
    )
    .unwrap();
    let fx2 = build_fixture();
    // The same tree as (a), so the only difference between the two runs is
    // whether the local manifest loaded.
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    let mut half_bad = fx2.files[0].data.clone();
    for b in half_bad.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &half_bad);
    let (summary, code) = repair::run(ctx_for(dir.clone(), &mock, 1))
        .await
        .expect("a degraded run still succeeds");
    assert_eq!(code, 0, "the fallback is not an error");
    let line = girpr::report::summary_record(&summary, code).text();
    assert!(line.contains("local_manifest=unavailable"), "{line}");
    assert_eq!(
        mock.chunk_hits(),
        vec!["chunk_foo0".to_string(), "chunk_foo1".to_string()],
        "with no local manifest there is nothing to reuse from, so even the \
         half-intact file downloads both of its chunks"
    );
    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// endpoint overrides, through the real binary
// ---------------------------------------------------------------------------

/// Run the shipped binary. The endpoint overrides are the only seam a
/// *binary-level* test can use, and this is where that seam is exercised: the
/// library tests inject `RunCtx::hyp_base_override` directly,
/// which proves nothing about whether a flag reaches `HypClient`.
///
/// Async on purpose: a blocking `Command` inside `#[tokio::test]` would occupy
/// the single runtime thread the mock server needs to answer on.
async fn run_binary(args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_girpr"));
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().await.expect("run girpr");
    (
        out.status.code().expect("process must exit normally"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[tokio::test]
async fn the_endpoint_override_flags_serve_the_whole_pipeline() {
    // A mock at 127.0.0.1 standing in for the whole CDN, driven by the real
    // binary through `--hyp-base` / `--sophon-base`. This is the shape
    // `gircap`'s artifact-serving shim will take.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("base_flags");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    // A purge flag *and* a scope, so the run exercises the whole narrowed
    // pipeline rather than the trivial path.
    write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 64]);
    let game = dir.to_string_lossy().into_owned();

    let (code, stdout, stderr) = run_binary(
        &[
            "--game-path",
            &game,
            "--biz",
            "hk4e_global",
            "--io-threads",
            "1",
            "--include",
            "game/foo.dat",
            "--purge-after",
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("REPORT "), "{stdout}");
    assert!(stdout.contains("SUMMARY "), "{stdout}");
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data,
        "the included file was repaired against the mock"
    );
    assert!(
        read_game_file(&dir, "stray/orphan.dat").is_some(),
        "the out-of-scope orphan survives the purge"
    );
    // The flags reached `HypClient::new_with_bases`: a miss would have been a
    // connection failure against production, i.e. exit 2.
    assert!(mock.first_target_with("getBuild").is_some());

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_scoped_cleanup_of_a_removed_path_works_through_the_binary() {
    // The same case as the library-level case, through the real binary and its
    // flags — because the guard this exercises is a *usage* guard, so exit 1 is
    // what a caller sees, and only the process boundary shows that.
    //
    // This is `gircap`'s `removed` capture class verbatim: one `--include` for a
    // path that is on disk and absent from the new manifest, `--purge-only`, and
    // an assertion that the path is gone and nothing was downloaded.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("binary_removed");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    write_game_file(&dir, "game/removed.dat", &[0x7Eu8; 512]);
    let game = dir.to_string_lossy().into_owned();

    let (code, stdout, stderr) = run_binary(
        &[
            "--game-path",
            &game,
            "--biz",
            "hk4e_global",
            "--include",
            "game/removed.dat",
            "--purge-only",
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "must not be refused as a typo: {stderr}");
    assert!(
        read_game_file(&dir, "game/removed.dat").is_none(),
        "the removed path is gone"
    );
    for f in &fx2.files {
        assert_eq!(
            read_game_file(&dir, &f.rel).unwrap(),
            f.data,
            "{} must survive",
            f.rel
        );
    }
    assert!(mock.chunk_hits().is_empty(), "nothing was downloaded");
    // The machine contract for this shape, from the process's own stdout.
    assert!(stdout.contains("SUMMARY "), "{stdout}");
    assert!(stdout.contains("total=0"), "{stdout}");
    assert!(stdout.contains("scoped=1"), "{stdout}");
    assert!(stdout.contains("config_bumped=0"), "{stdout}");

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn output_json_is_pure_ndjson_and_output_text_is_the_default() {
    // The property downstream tooling depends on: under `--output json` *every*
    // stdout line is a JSON object, so the stream can be parsed line by line
    // without a fallback. This needs a server, which is why it lives here rather
    // than in `cli_contract.rs` — that file has never had one, and the bridge
    // test will keep it that way.
    //
    // `--print-plan` is here deliberately: `--print-plan --output json` must
    // still be pure NDJSON, so the plan record is exercised on the JSON plane
    // too. One record per line is what keeps that true.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("output_json");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let game = dir.to_string_lossy().into_owned();
    let base: Vec<&str> = vec![
        "--game-path",
        &game,
        "--biz",
        "hk4e_global",
        "--io-threads",
        "1",
        "--hyp-base",
        &mock.hyp_base,
        "--sophon-base",
        &mock.sophon_base,
    ];

    let mut json_args = base.clone();
    json_args.extend_from_slice(&["--print-plan", "--output", "json"]);
    let (code, stdout, stderr) = run_binary(&json_args, &[]).await;
    assert_eq!(code, 0, "stderr: {stderr}");

    let events: Vec<String> = stdout
        .lines()
        .map(|l| {
            let v: serde_json::Value =
                serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON: {e:?}\n{l}"));
            assert!(v.get("event").is_some(), "every record names itself: {l}");
            v["event"].as_str().unwrap().to_string()
        })
        .collect();
    assert_eq!(
        events,
        vec!["report", "plan", "summary"],
        "one record per event, in order, and nothing else on stdout:\n{stdout}"
    );
    // No `KEY=value` text anywhere: that substring is what a text-plane leak
    // would look like, and `total=0` is a substring of nothing in JSON.
    assert!(
        !stdout.contains("total=") && !stdout.contains("REPORT ") && !stdout.contains("PLAN "),
        "text plane leaked into the JSON stream:\n{stdout}"
    );
    // And the counts are still there, by name, as numbers.
    let last: serde_json::Value = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
    assert_eq!(last["total"], fx2.files.len() as u64);
    assert_eq!(last["exit"], 0);
    let plan: serde_json::Value = serde_json::from_str(stdout.lines().nth(1).unwrap()).unwrap();
    assert_eq!(plan["files"], fx2.files.len() as u64);
    assert_eq!(plan["scoped"], 0);

    // The default is the text plane, and it is unchanged by any of this.
    let mut text_args = base.clone();
    text_args.push("--print-plan");
    let (code, stdout, stderr) = run_binary(&text_args, &[]).await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.starts_with("REPORT local_version=")
            && stdout.contains(&format!("PLAN files={}", fx2.files.len())),
        "{stdout}"
    );
    assert!(stdout.trim_end().ends_with("exit=0"), "{stdout}");

    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_explicit_base_beats_the_env_var() {
    // The env seams are pointed at a **dead** port, so if the env won the run
    // would fail with exit 2 instead of exiting 0.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("base_precedence");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let game = dir.to_string_lossy().into_owned();
    let (code, _, stderr) = run_binary(
        &[
            "--game-path",
            &game,
            "--biz",
            "hk4e_global",
            "--check-only",
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[
            ("GIRPR_HYP_BASE", "http://127.0.0.1:1"),
            ("GIRPR_SOPHON_BASE", "http://127.0.0.1:1"),
        ],
    )
    .await;
    assert_eq!(code, 0, "the flag must win over the env var: {stderr}");
    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_env_seam_still_works_without_the_flags() {
    // The env seam every offline e2e run already uses must keep working.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("base_env");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let game = dir.to_string_lossy().into_owned();
    let (code, _, stderr) = run_binary(
        &["--game-path", &game, "--biz", "hk4e_global", "--check-only"],
        &[
            ("GIRPR_HYP_BASE", &mock.hyp_base),
            ("GIRPR_SOPHON_BASE", &mock.sophon_base),
        ],
    )
    .await;
    assert_eq!(code, 0, "env overrides must still work: {stderr}");
    mock.stop();
    std::fs::remove_dir_all(&dir).ok();
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
