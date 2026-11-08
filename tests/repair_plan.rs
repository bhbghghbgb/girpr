//! `--print-plan` and `--plan-out` cases.
//!
//! The plan is read-only work: what a run would cost and would touch, without
//! doing it. Each case here runs against the offline mock server.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{
    ChunkSpec, FileSpec, FixtureSpec, Lies, MockOpts, MockServer, build_fixture,
    build_fixture_spec, ctx_for, field, load_counters, read_game_file, run, run_binary, temp_dir,
    write_game_file,
};
use girpr::RunExit;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn the_plan_line_says_what_the_repair_would_cost_in_disk() {
    // The `PLAN` line carries the two disk figures: the bytes a repair
    // writes into the install, and the free space it needs beyond it.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("plan_disk");
    let fx2 = build_fixture();
    // bar.dat is 48 bytes in the manifest and 32 on disk: it has to grow by
    // 16. foo.dat is not there: it has to be created, so it needs all 128.
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data[..32]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.print_plan = true;
    let (summary, code) = run(ctx).await.expect("print-plan runs");
    assert_eq!(code, RunExit::Success);
    // The run reported the plan, not a repair.
    assert!(summary.files_repaired.load(Ordering::Relaxed) == 0);

    // The plan line itself is on stdout; the driver only keeps the summary
    // here, so the figures are checked through the plan-out document.
    let out = dir.join("plan.json");
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.print_plan = true;
    ctx.plan_out = Some(out.clone());
    run(ctx).await.expect("print-plan runs");
    let written = std::fs::read_to_string(&out).expect("--plan-out file");
    let v: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
    let foo = v["repair"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["rel"] == "game/foo.dat")
        .expect("foo.dat is planned");
    assert_eq!(foo["mode"], "inplace");
    assert_eq!(
        foo["extra_bytes"], 128,
        "a file that is not there has to be created"
    );
    let bar = v["repair"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["rel"] == "game/bar.dat")
        .expect("bar.dat is planned");
    assert_eq!(bar["mode"], "inplace");
    assert_eq!(bar["extra_bytes"], 16, "bar.dat has to grow by 16");

    mock.stop();
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

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.print_plan = true;
    ctx.purge_after = true;
    let (summary, code) = run(ctx).await.expect("print-plan runs");
    assert_eq!(code, RunExit::Success);

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
}

#[tokio::test]
async fn print_plan_with_audio_writes_no_scan_file_and_names_the_language() {
    // `--print-plan` is documented as read-only by construction: it never
    // writes the scan file or creates `game_dir`.
    //
    // The report must still name the requested language: resolving the audio
    // set is not the write.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            scan_dir: "Audio/scan.txt".to_string(),
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("print_plan_audio");
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
            "--audio",
            "en-us",
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(!game.exists(), "--print-plan must not create the game dir");
    assert!(
        !game.join("Audio").exists(),
        "--print-plan must not create the scan dir"
    );
    assert!(
        stdout.contains("audio_langs=en-us"),
        "the report must name the requested language:\n{stdout}"
    );

    mock.stop();
}

#[tokio::test]
async fn print_plan_audio_outside_the_scope_is_not_refused_and_writes_nothing() {
    // An explicit `--audio` under an active scope whose scan file is *not*
    // included is a
    // usage error (exit `1`) — unless the run is `--print-plan`, which never
    // writes the scan file at all. `--print-plan` therefore has to exempt the
    // scope gate while still deferring the write: exit `0`, a `PLAN`, and no
    // scan file on disk prove both halves.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            scan_dir: "Audio/scan.txt".to_string(),
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("print_plan_audio_scope_out");
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
            "--audio",
            "en-us",
            "--include",
            "game/foo.dat",
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.lines().any(|l| l.starts_with("PLAN ")),
        "the plan must still be reported:\n{stdout}"
    );
    assert!(
        !game.join("Audio").join("scan.txt").exists(),
        "the scan file is out of scope and print-plan must not write it"
    );

    mock.stop();
}

#[tokio::test]
async fn print_plan_audio_inside_the_scope_still_defers_the_write() {
    // The companion case: include the scan file's own rel, so the scope gate
    // would admit the write. `--print-plan` alone still defers it — exit `0`,
    // the `PLAN` is printed, and the file the scope admitted is not created.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            scan_dir: "Audio/scan.txt".to_string(),
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("print_plan_audio_scope_in");
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
            "--audio",
            "en-us",
            "--include",
            "Audio/scan.txt",
            "--include",
            "game/foo.dat",
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.lines().any(|l| l.starts_with("PLAN ")),
        "the plan must still be reported:\n{stdout}"
    );
    assert!(
        !game.join("Audio").join("scan.txt").exists(),
        "print-plan must defer the scan write even when the scope admits it"
    );

    mock.stop();
}

#[tokio::test]
async fn a_hostile_manifest_saturates_the_plan_figures_instead_of_panicking() {
    // `size` and `compressed_size` are server manifest data. Three files at
    // `i64::MAX` and one file with three chunks at `i64::MAX` each overflow the
    // plan's aggregates: the plan phase must saturate, not panic a debug build
    // (or wrap a smaller figure onto the PLAN line in a release build).
    let big =
        |rel: &str, id: &str| FileSpec::new(rel, vec![ChunkSpec::new(id, vec![0x41u8; 16], 0)]);
    let fx = build_fixture_spec(
        &FixtureSpec::new(vec![
            big("game/big1.dat", "chunk_b1"),
            big("game/big2.dat", "chunk_b2"),
            big("game/big3.dat", "chunk_b3"),
            FileSpec::new(
                "game/chunky.dat",
                vec![
                    ChunkSpec::new("chunk_h0", vec![0x10u8; 16], 0),
                    ChunkSpec::new("chunk_h1", vec![0x11u8; 16], 16),
                    ChunkSpec::new("chunk_h2", vec![0x12u8; 16], 32),
                ],
            ),
        ])
        .with_lies(Lies {
            huge_file_size: vec![
                "game/big1.dat".into(),
                "game/big2.dat".into(),
                "game/big3.dat".into(),
            ],
            huge_compressed_size: vec!["chunk_h0".into(), "chunk_h1".into(), "chunk_h2".into()],
            ..Default::default()
        }),
    );
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("plan_overflow");
    let out = dir.join("plan.json");
    let game = dir.to_string_lossy().into_owned();
    let out_arg = out.to_string_lossy().into_owned();
    let args = [
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
        "--print-plan",
        "--plan-out",
        &out_arg,
    ];
    let (code, stdout, stderr) = run_binary(&args, &[]).await;
    assert_eq!(code, 0, "{stderr}");
    let plan_line = stdout
        .lines()
        .find(|l| l.starts_with("PLAN "))
        .expect("the PLAN line");
    assert_eq!(field(plan_line, "est_write_bytes"), u64::MAX, "{plan_line}");
    assert_eq!(
        field(plan_line, "est_download_bytes"),
        u64::MAX,
        "{plan_line}"
    );

    let written = std::fs::read_to_string(&out).expect("--plan-out file");
    let v: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
    let entry = v["repair"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["rel"] == "game/chunky.dat")
        .expect("chunky.dat is planned");
    assert_eq!(entry["est_download_bytes"], u64::MAX);
    let sum = v["repair"].as_array().unwrap().iter().fold(0u64, |n, e| {
        n.saturating_add(e["est_download_bytes"].as_u64().unwrap())
    });
    assert_eq!(sum, u64::MAX, "the document agrees with the PLAN line");

    mock.stop();
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
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.print_plan = true;
    ctx.purge_after = true;
    ctx.plan_out = Some(out.clone());
    let (_, code) = run(ctx).await.expect("print-plan runs");
    assert_eq!(code, RunExit::Success);

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
}

#[tokio::test]
async fn plan_out_without_print_plan_writes_the_document_of_the_run() {
    // The help says the flag works on its own: the document is written before
    // execution, through the real binary.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("plan_out_alone");
    let fx2 = build_fixture();
    // bar is intact and foo is missing, so this is a run that does work.
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);

    let out = dir.join("reports").join("plan.json");
    let out_arg = out.to_string_lossy().into_owned();
    let game = dir.to_string_lossy().into_owned();
    let (code, stdout, stderr) = run_binary(
        &[
            "--game-path",
            &game,
            "--biz",
            "hk4e_global",
            "--jobs",
            "1",
            "--plan-out",
            &out_arg,
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("SUMMARY "), "the run executed: {stdout}");
    let written = std::fs::read_to_string(&out)
        .expect("--plan-out without --print-plan must create the documented file");
    let v: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
    assert_eq!(
        v["repair"].as_array().unwrap().len(),
        2,
        "the document describes the run: {written}"
    );
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data,
        "the run still did the work it planned"
    );

    mock.stop();
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
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.print_plan = true;
    ctx.limits.download_bytes = Some(est - 1);
    let (summary, code) = run(ctx).await.expect("a refusal is not an error");
    assert_eq!(
        code,
        RunExit::Limit,
        "the assert comes before the plan report"
    );
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

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.print_plan = true;
    let (summary, code) = run(ctx).await.expect("print-plan runs");
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.files_failed.load(Ordering::Relaxed), 0);
    assert_eq!(summary.files_skipped.load(Ordering::Relaxed), 0);

    // The same tree under --check-only does report damage.
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.check_only = true;
    let (_, code) = run(ctx).await.expect("check runs");
    assert_eq!(code, RunExit::CheckFailed);

    mock.stop();
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
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.print_plan = true;
    ctx.dry_run = true;
    let (summary, code) = run(ctx).await.expect("print-plan + dry-run runs");
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.files_total, 2);
    assert!(!dir.join("config.ini").exists(), "dry-run writes nothing");

    mock.stop();
}
