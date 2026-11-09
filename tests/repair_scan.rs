//! Scan and `--check-only` cases: reporting damage without repairing it.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{
    ChunkSpec, FileSpec, FixtureSpec, LATEST_TAG, LOCAL_TAG, Lies, MockOpts, MockServer,
    PLAN_SCAN_BYTES_KEY, build_fixture, build_fixture_spec, build_moved_fixture, ctx_for, field,
    load_counters, read_game_file, run, run_binary, temp_dir, write_game_file,
};
use girpr::RunExit;
use std::sync::atomic::Ordering;

// ---------------------------------------------------------------------------
// zero-length manifest entries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn check_only_clean_reports_success() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("check_clean");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.check_only = true;
    let (summary, code) = run(ctx).await.expect("check-only runs");
    assert_eq!(code, RunExit::Success);
    let (skipped, _rep, failed, _, _) = load_counters(&summary);
    assert_eq!(failed, 0);
    assert_eq!(skipped, 2);

    mock.stop();
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

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.check_only = true;
    let (summary, code) = run(ctx).await.expect("check-only runs");
    assert_eq!(code, RunExit::CheckFailed, "check-only damage exit");
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
}

#[tokio::test]
async fn check_only_matches_between_one_and_many_jobs() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let fx2 = build_fixture();
    let foo = &fx2.files[0];
    let bar = &fx2.files[1];

    let mut runs = Vec::new();
    for jobs in [1usize, 4] {
        let dir = temp_dir(&format!("check_parity_{jobs}"));
        write_game_file(&dir, &bar.rel, &bar.data);
        // Same length, different bytes: a digest failure, not a size one.
        let mut bad_foo = foo.data.clone();
        for b in bad_foo.iter_mut().skip(64) {
            *b ^= 0xFF;
        }
        write_game_file(&dir, &foo.rel, &bad_foo);

        let mut ctx = ctx_for(dir.to_path_buf(), &mock, jobs);
        ctx.check_only = true;
        let (summary, code) = run(ctx).await.expect("check-only runs");
        runs.push((load_counters(&summary), code));
    }

    assert_eq!(
        runs[0], runs[1],
        "jobs must not change the check's verdict or counters"
    );
    assert_eq!(
        runs[0].1,
        RunExit::CheckFailed,
        "the corrupt file is still damage"
    );
    let (skipped, _repaired, failed, _, _) = runs[0].0;
    assert_eq!((skipped, failed), (1, 1));

    mock.stop();
}

#[tokio::test]
async fn check_only_scan_keeps_the_damaged_chunk_total_on_the_summary() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("check_damaged_chunks_summary");
    let fx2 = build_fixture();
    let foo = &fx2.files[0];
    let bar = &fx2.files[1];
    write_game_file(&dir, &bar.rel, &bar.data);
    // Same length, different bytes: a digest failure the scan can locate
    // chunk by chunk.
    let mut bad_foo = foo.data.clone();
    for b in bad_foo.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &foo.rel, &bad_foo);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.check_only = true;
    ctx.scan = true;
    let (summary, code) = run(ctx).await.expect("check-only runs");
    assert_eq!(
        code,
        RunExit::CheckFailed,
        "the corrupt file is still damage"
    );

    let checked = summary
        .check_damaged_chunks()
        .expect("the check measured the tally");
    assert!(checked > 0, "the check carries the chunks it found wrong");
    assert_eq!(
        Some(checked),
        summary.flags.damaged_chunks,
        "the check's tally matches the scan figure it was given"
    );

    mock.stop();
}

#[tokio::test]
async fn check_only_scan_treats_an_absent_empty_file_as_damage() {
    // The scan's verdict for an absent file carries the empty digest, and a
    // legitimate empty file's manifest entry has exactly that digest. The
    // md5 alone therefore cannot tell them apart.
    let spec = FixtureSpec::new(vec![FileSpec::new("game/empty.dat", vec![])]);
    let mock = MockServer::start(build_fixture_spec(&spec), MockOpts::default()).await;
    let dir = temp_dir("check_empty_absent");

    for scan in [false, true] {
        let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
        ctx.check_only = true;
        ctx.scan = scan;
        let (summary, code) = run(ctx).await.expect("check-only runs");
        assert_eq!(
            code,
            RunExit::CheckFailed,
            "scan={scan}: an absent file is damage"
        );
        assert_eq!(
            summary.files_failed.load(Ordering::Relaxed),
            1,
            "scan={scan}"
        );
    }

    // The same manifest entry, with the file present and empty: clean.
    write_game_file(&dir, "game/empty.dat", &[]);
    for scan in [false, true] {
        let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
        ctx.check_only = true;
        ctx.scan = scan;
        let (_summary, code) = run(ctx).await.expect("check-only runs");
        assert_eq!(
            code,
            RunExit::Success,
            "scan={scan}: an empty file that exists is clean"
        );
    }

    mock.stop();
}

#[tokio::test]
async fn the_scanned_estimate_knows_which_sources_the_ordered_tier_loses() {
    // N-17: the scan finds both moved sources intact, but the ordered tier's
    // write order keeps only one direction's worth. The estimate must model
    // the writer's order, not "the source verified", or the PLAN line is below
    // the bytes the same run downloads.
    let fx = build_moved_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("scan_ordered");
    let (old_rel, old_bytes) = build_moved_fixture().old_data.expect("the old file");
    std::fs::write(
        dir.join("config.ini"),
        format!("[General]\ngame_version={LOCAL_TAG}\n"),
    )
    .unwrap();
    write_game_file(&dir, &old_rel, &old_bytes);
    let game = dir.to_string_lossy().into_owned();
    let base: Vec<&str> = vec![
        "--game-path",
        &game,
        "--biz",
        "hk4e_global",
        "--jobs",
        "1",
        "--hyp-base",
        &mock.hyp_base,
        "--sophon-base",
        &mock.sophon_base,
        "--max-ram-bytes",
        "0",
    ];

    // 1. the plan, scanned: the ordered tier is selected by the RAM budget.
    let mut args = base.clone();
    args.extend_from_slice(&["--scan", "--print-plan"]);
    let (code, stdout, stderr) = run_binary(&args, &[]).await;
    assert_eq!(code, 0, "{stderr}");
    let estimate = field(
        stdout
            .lines()
            .find(|l| l.starts_with("PLAN "))
            .expect("the PLAN line"),
        "est_download_bytes",
    );

    // 2. the repair: the same budget, so the same tier and order.
    let mut args = base.clone();
    args.push("--scan");
    let (code, stdout, stderr) = run_binary(&args, &[]).await;
    assert_eq!(code, 0, "{stderr}");
    let downloaded = field(
        stdout
            .lines()
            .find(|l| l.starts_with("SUMMARY "))
            .expect("the SUMMARY line"),
        "download_bytes",
    );
    assert_eq!(
        estimate, downloaded,
        "the scanned estimate is the ordered tier's real cost: {estimate} vs {downloaded}"
    );
    assert_eq!(
        mock.chunk_hits(),
        vec!["chunk_p".to_string(), "chunk_t".to_string()],
        "the source the losing direction needed is downloaded"
    );

    mock.stop();
}

#[tokio::test]
async fn the_scanned_estimate_is_the_bytes_the_repair_really_downloads() {
    // The assertion that did not exist before `--scan`: the plan's estimate
    // and the repair's actual cost are the same number.
    //
    // Without the scan the estimate counts every chunk the reuse map cannot
    // supply — which is what the *temp* path downloads, but not what the
    // in-place path does: it keeps the chunk that is already correct where
    // it belongs. So the unscanned estimate is an upper bound, and the
    // scanned one is the figure.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("scan_exact");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    let mut half_bad = fx2.files[0].data.clone();
    for b in half_bad.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &half_bad);
    let game = dir.to_string_lossy().into_owned();
    let base: Vec<&str> = vec![
        "--game-path",
        &game,
        "--biz",
        "hk4e_global",
        "--jobs",
        "1",
        "--hyp-base",
        &mock.hyp_base,
        "--sophon-base",
        &mock.sophon_base,
    ];

    // 1. the plan, unscanned: the upper bound.
    let mut args = base.clone();
    args.extend_from_slice(&["--print-plan"]);
    let (code, stdout, stderr) = run_binary(&args, &[]).await;
    assert_eq!(code, 0, "{stderr}");
    let unscanned = field(
        stdout
            .lines()
            .find(|l| l.starts_with("PLAN "))
            .expect("the PLAN line"),
        "est_download_bytes",
    );

    // 2. the plan, scanned: the exact figure, and the per-file document must
    //    carry the same split the total was folded from.
    let out = dir.join("plan.json");
    let out_arg = out.to_string_lossy().into_owned();
    let mut args = base.clone();
    args.extend_from_slice(&["--scan", "--print-plan", "--plan-out", &out_arg]);
    let (code, stdout, stderr) = run_binary(&args, &[]).await;
    assert_eq!(code, 0, "{stderr}");
    let plan_line = stdout
        .lines()
        .find(|l| l.starts_with("PLAN "))
        .expect("the PLAN line");
    let scanned = field(plan_line, "est_download_bytes");
    assert!(
        scanned < unscanned,
        "the scan knows the intact chunk is kept: {scanned} vs {unscanned}"
    );
    assert!(
        plan_line.contains(&format!("{PLAN_SCAN_BYTES_KEY}={}", 128 + 48)),
        "the scan reports the bytes it read: {plan_line}"
    );
    let written = std::fs::read_to_string(&out).expect("--plan-out file");
    let v: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
    let doc_sum = v["repair"].as_array().unwrap().iter().fold(0u64, |n, e| {
        n.saturating_add(e["est_download_bytes"].as_u64().unwrap())
    });
    assert_eq!(
        doc_sum, scanned,
        "the document's per-file halves sum to the scanned PLAN total: {doc_sum} vs {scanned}"
    );

    // 3. the repair: what it really downloaded.
    let mut args = base.clone();
    args.push("--scan");
    let (code, stdout, stderr) = run_binary(&args, &[]).await;
    assert_eq!(code, 0, "{stderr}");
    let summary = stdout
        .lines()
        .find(|l| l.starts_with("SUMMARY "))
        .expect("the SUMMARY line");
    let downloaded = field(summary, "download_bytes");
    assert_eq!(
        scanned, downloaded,
        "the scanned estimate is the real cost: {scanned} vs {downloaded}"
    );
    assert!(summary.contains("damaged_chunks=1"), "{summary}");

    mock.stop();
}

#[tokio::test]
async fn the_scanned_temp_estimate_is_the_bytes_the_repair_really_downloads() {
    // Both of foo's chunks
    // have a candidate entry (config.ini names the installed build), but
    // chunk 1 is corrupt on disk: the temp path re-hashes the candidate
    // slice, refuses it and fetches `chunk_foo1`. The scanned plan must price
    // that chunk, and the price must be what the repair really spends.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("scan_exact_temp");
    let fx2 = build_fixture();
    std::fs::write(
        dir.join("config.ini"),
        format!("[General]\ngame_version={LATEST_TAG}\n"),
    )
    .unwrap();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    let mut half_bad = fx2.files[0].data.clone();
    for b in half_bad.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &half_bad);
    let game = dir.to_string_lossy().into_owned();
    let base: Vec<&str> = vec![
        "--game-path",
        &game,
        "--biz",
        "hk4e_global",
        "--jobs",
        "1",
        "--hyp-base",
        &mock.hyp_base,
        "--sophon-base",
        &mock.sophon_base,
    ];

    // 1. the plan: the scan's own figure for the mode that was asked for.
    let out = dir.join("plan.json");
    let out_arg = out.to_string_lossy().into_owned();
    let mut args = base.clone();
    args.extend_from_slice(&[
        "--scan",
        "--repair-mode",
        "temp",
        "--print-plan",
        "--plan-out",
        &out_arg,
    ]);
    let (code, stdout, stderr) = run_binary(&args, &[]).await;
    assert_eq!(code, 0, "{stderr}");
    let plan_line = stdout
        .lines()
        .find(|l| l.starts_with("PLAN "))
        .expect("the PLAN line");
    let scanned = field(plan_line, "est_download_bytes");
    assert!(
        scanned > 0,
        "the corrupt candidate cannot be reused, so the chunk is a download: {plan_line}"
    );
    let written = std::fs::read_to_string(&out).expect("--plan-out file");
    let v: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
    let doc_sum = v["repair"].as_array().unwrap().iter().fold(0u64, |n, e| {
        n.saturating_add(e["est_download_bytes"].as_u64().unwrap())
    });
    assert_eq!(
        doc_sum, scanned,
        "the document's per-file halves sum to the scanned PLAN total: {doc_sum} vs {scanned}"
    );

    // 2. the repair in the same mode: the same figure.
    let mut args = base.clone();
    args.extend_from_slice(&["--scan", "--repair-mode", "temp"]);
    let (code, stdout, stderr) = run_binary(&args, &[]).await;
    assert_eq!(code, 0, "{stderr}");
    let summary = stdout
        .lines()
        .find(|l| l.starts_with("SUMMARY "))
        .expect("the SUMMARY line");
    let downloaded = field(summary, "download_bytes");
    assert_eq!(
        scanned, downloaded,
        "the scanned temp estimate is the real cost: {scanned} vs {downloaded}"
    );
    assert_eq!(
        mock.chunk_hits(),
        vec!["chunk_foo1".to_string()],
        "the corrupt candidate is fetched, the intact one is reused"
    );

    mock.stop();
}

#[tokio::test]
async fn check_only_trips_a_ceiling_not_damage() {
    // A ceiling narrows *what is verified*; `CheckFailed` must not be masked by
    // narrowing — but a ceiling is a different fact entirely.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_files_check");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, b"damaged");
    }
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.check_only = true;
    ctx.limits.files = Some(1);
    let (summary, code) = run(ctx).await.expect("refusal is not an error");
    assert_eq!(
        code,
        RunExit::Limit,
        "the ceiling is asked before the verification"
    );
    assert_eq!(
        summary.files_failed.load(Ordering::Relaxed),
        0,
        "nothing verified"
    );

    // Without the ceiling, the same run reports damage.
    let mut ctx2 = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx2.check_only = true;
    let (_, code2) = run(ctx2).await.expect("check runs");
    assert_eq!(code2, RunExit::CheckFailed);

    mock.stop();
}

#[tokio::test]
async fn check_only_and_print_plan_issue_no_deprecated_file_call() {
    // The deprecated list is fetched during the plan phase, and these modes
    // never reach the post-phase, so they skip that network call:
    // `--check-only` and `--print-plan` must issue zero
    // `getGameDeprecatedFileConfigs` requests. (`--dry-run` does fetch it,
    // because it lists the deprecated deletes — but `--print-plan` returns
    // before the execution phase and is not widened by the combination.)
    for mode in ["check", "plan", "plan-dry"] {
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

        let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
        match mode {
            "check" => ctx.check_only = true,
            "plan-dry" => {
                ctx.print_plan = true;
                ctx.dry_run = true;
            }
            _ => ctx.print_plan = true,
        }
        run(ctx).await.expect("run");
        assert!(
            mock.first_target_with("getGameDeprecatedFileConfigs")
                .is_none(),
            "--{mode} must not fetch the deprecated list it would discard"
        );
        assert!(read_game_file(&dir, "old/legacy.dll").is_some());
        mock.stop();
    }
}

#[tokio::test]
async fn a_hostile_chunk_range_refuses_every_read_only_mode_before_it_runs() {
    // Option (b): the range gate is in `build_plan`, so `--check-only`,
    // `--dry-run` and `--print-plan` all refuse the manifest at exit `2`
    // instead of one of them reporting a false clean or printing an unrunnable
    // plan. Nothing is written in any mode.
    for mode in ["--check-only", "--dry-run", "--print-plan"] {
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
        // The file on disk is the one the manifest's size and MD5 describe, so
        // without the plan gate `--check-only` would call it intact and exit
        // `0` for the wrong reason.
        let expected = fx.files[0].data.clone();
        let mock = MockServer::start(fx, MockOpts::default()).await;
        let dir = temp_dir("readonly_hostile_range");
        let game = dir.join("install");
        write_game_file(&game, "game/foo.dat", &expected);
        let game_arg = game.to_string_lossy().into_owned();

        let (code, stdout, stderr) = run_binary(
            &[
                "--game-path",
                &game_arg,
                "--biz",
                "hk4e_global",
                "--jobs",
                "1",
                mode,
                "--hyp-base",
                &mock.hyp_base,
                "--sophon-base",
                &mock.sophon_base,
            ],
            &[],
        )
        .await;
        assert_eq!(code, 2, "{mode}: a hostile range is metadata: {stderr}");
        assert_eq!(
            read_game_file(&game, "game/foo.dat").as_deref(),
            Some(expected.as_slice()),
            "{mode}: the read-only run must not touch the target"
        );
        assert!(
            !dir.join("config.ini").exists(),
            "{mode}: config.ini must not be written"
        );
        assert!(
            !stdout.contains("PLAN "),
            "{mode}: no plan may be printed: {stdout}"
        );
        assert!(
            !stdout.contains("CHECK"),
            "{mode}: no CHECK line may call the file clean: {stdout}"
        );
        assert!(
            mock.chunk_hits().is_empty(),
            "{mode}: no chunk may be fetched"
        );

        mock.stop();
    }
}

#[tokio::test]
async fn a_legitimate_empty_file_passes_every_mode_untouched() {
    // A `size = 0` entry whose digest is `EMPTY_MD5` describes a real, empty
    // file. The fixture builder derives both from the (empty) layout, so this
    // is the honest shape. Every mode must accept it, and the repair must not
    // rewrite a file that is already correct.
    let fx = build_fixture_spec(&FixtureSpec::new(vec![FileSpec::new(
        "game/empty.dat",
        vec![],
    )]));
    assert_eq!(fx.files[0].data, Vec::<u8>::new());
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("empty_legit");
    let game = dir.join("install");
    // Present and empty, exactly as the manifest describes it.
    write_game_file(&game, "game/empty.dat", b"");
    let game_arg = game.to_string_lossy().into_owned();
    let base = [
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
    ];

    let mut args = base.to_vec();
    args.push("--check-only");
    let (code, _, stderr) = run_binary(&args, &[]).await;
    assert_eq!(
        code, 0,
        "--check-only must call the empty file clean: {stderr}"
    );
    assert_eq!(
        read_game_file(&game, "game/empty.dat"),
        Some(Vec::new()),
        "the check must not touch the file"
    );

    let mut args = base.to_vec();
    args.push("--dry-run");
    let (code, _, stderr) = run_binary(&args, &[]).await;
    assert_eq!(code, 0, "--dry-run: {stderr}");

    let mut args = base.to_vec();
    args.push("--print-plan");
    let (code, stdout, stderr) = run_binary(&args, &[]).await;
    assert_eq!(code, 0, "--print-plan: {stderr}");
    assert!(
        stdout.lines().any(|l| l.starts_with("PLAN ")),
        "an empty file is still a planned file:\n{stdout}"
    );

    let (code, _, stderr) = run_binary(&base, &[]).await;
    assert_eq!(
        code, 0,
        "the repair must accept the correct empty file: {stderr}"
    );
    assert_eq!(
        read_game_file(&game, "game/empty.dat"),
        Some(Vec::new()),
        "the repair must leave the empty file empty"
    );
    assert!(!game.join("game").join("empty.dat_tmp").exists());

    mock.stop();
}

#[tokio::test]
async fn an_empty_file_with_a_non_empty_digest_is_refused_by_the_md5_gate() {
    // A zero-length entry whose declared digest is not `EMPTY_MD5` cannot
    // describe any byte string, so the size gate passes and the digest gate
    // is the one that has to refuse. The read-only modes disagree on purpose:
    // `--check-only` reports the file bad (exit `4`), `--dry-run` changes
    // nothing (exit `0`), `--print-plan` still prints the plan (exit `0`); a
    // real repair fails the file (exit `3`) without publishing a wrong one.
    let fx = build_fixture_spec(
        &FixtureSpec::new(vec![FileSpec::new("game/empty.dat", vec![])]).with_lies(Lies {
            wrong_file_md5: vec!["game/empty.dat".into()],
            ..Default::default()
        }),
    );
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("empty_bad_md5");
    let game = dir.join("install");
    write_game_file(&game, "game/empty.dat", b"");
    let game_arg = game.to_string_lossy().into_owned();
    let base = [
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
    ];

    let mut args = base.to_vec();
    args.push("--check-only");
    let (code, _, stderr) = run_binary(&args, &[]).await;
    assert_eq!(
        code, 4,
        "--check-only must report the digest mismatch: {stderr}"
    );
    assert_eq!(
        read_game_file(&game, "game/empty.dat"),
        Some(Vec::new()),
        "the check must not touch the file"
    );

    let mut args = base.to_vec();
    args.push("--dry-run");
    let (code, _, stderr) = run_binary(&args, &[]).await;
    assert_eq!(code, 0, "--dry-run: {stderr}");
    assert_eq!(
        read_game_file(&game, "game/empty.dat"),
        Some(Vec::new()),
        "--dry-run must not touch the file"
    );

    let mut args = base.to_vec();
    args.push("--print-plan");
    let (code, stdout, stderr) = run_binary(&args, &[]).await;
    assert_eq!(code, 0, "--print-plan: {stderr}");
    assert!(
        stdout.lines().any(|l| l.starts_with("PLAN ")),
        "the plan is printed even though the digest is wrong:\n{stdout}"
    );

    // The digest mismatch is deterministic: the file has no chunks, so the
    // next attempt would assemble the same empty bytes and fail the same gate.
    // It is therefore terminal, not retried — exactly one attempt is made even
    // without `--fail-fast` — and the failure names the whole-file digest gate
    // rather than some other write error.
    let (code, stdout, stderr) = run_binary(&base, &[]).await;
    assert_eq!(
        code, 3,
        "the repair must fail the one file it cannot make true: {stderr}"
    );
    assert!(
        stderr.contains("final md5 mismatch for game/empty.dat"),
        "the repair must fail on the whole-file digest gate: {stderr}"
    );
    assert!(
        stderr.contains("(the bytes outside the chunks are wrong)"),
        "the failure must name the part the plan cannot write: {stderr}"
    );
    assert_eq!(
        stderr.matches("attempt failed").count(),
        1,
        "a deterministic digest mismatch must not be attempted again: {stderr}"
    );
    assert!(
        !stderr.contains("after 5 attempts"),
        "the terminal failure must not be reported as exhausted retries: {stderr}"
    );
    assert!(
        stdout.contains("failed=1") && stdout.contains("exit=3"),
        "the summary must carry the failed file and the write exit: {stdout}"
    );
    assert_eq!(
        read_game_file(&game, "game/empty.dat"),
        Some(Vec::new()),
        "no wrong bytes may be published in its place"
    );
    assert!(
        !game.join("game").join("empty.dat_tmp").exists(),
        "a failed empty-file repair must leave no artefact"
    );

    mock.stop();
}

#[tokio::test]
async fn a_manifest_that_does_not_match_its_checksum_is_refused() {
    // The checksum is the MD5 of the **decompressed** bytes, and it is
    // verified before the protobuf parse. A manifest that does not match it
    // cannot be planned from: the run fails as metadata, before a single byte
    // is written.
    //
    // The blob the mock serves is unchanged — only the checksum the API
    // advertises is wrong — so the checksum gate is the only thing that can
    // catch it, and the decompressed bytes are the only thing it can be
    // compared against.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            wrong_manifest_checksum: true,
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("manifest_checksum");
    let err = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect_err("a manifest that does not match its checksum cannot be planned from");
    assert_eq!(
        err.exit_code,
        RunExit::Metadata,
        "metadata, not a write failure"
    );
    assert!(!dir.join("config.ini").exists(), "nothing was written");
    mock.stop();
}
