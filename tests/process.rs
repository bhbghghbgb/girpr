//! Process-level cases: the shipped binary's endpoints, output, logging and
//! closed-stdout behaviour.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{
    ChunkSpec, FileSpec, FixtureSpec, Lies, MockOpts, MockServer, build_fixture,
    build_fixture_spec, read_game_file, run_binary, temp_dir, write_game_file,
};

/// Run the shipped binary with the reading end of its stdout pipe gone.
///
/// The pipe is dropped immediately after the child exists, so the run's first
/// data-plane write meets the closed consumer. stderr is still collected; stdout
/// is deliberately not, because there is no reader left to collect it.
async fn run_binary_with_closed_stdout(args: &[&str], env: &[(&str, &str)]) -> (i32, String) {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_girpr"));
    cmd.args(args);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn girpr");
    drop(child.stdout.take());
    let out = child.wait_with_output().await.expect("wait for girpr");
    common::remove_run_lock_for_args(args);
    (
        out.status.code().expect("process must exit normally"),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

// ---------------------------------------------------------------------------
// endpoint overrides, through the real binary
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_closed_stdout_does_not_change_the_exit_code_or_the_repair() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("closed_stdout");
    let fx2 = build_fixture();
    // bar.dat is intact; foo.dat is missing and must be repaired.
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    let game = dir.to_string_lossy().into_owned();
    let log = dir.join("run.log");
    let log_arg = log.to_string_lossy().into_owned();

    let (code, stderr) = run_binary_with_closed_stdout(
        &[
            "--game-path",
            &game,
            "--biz",
            "hk4e_global",
            "--io-threads",
            "1",
            "--log-file",
            &log_arg,
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;

    assert_eq!(code, 0, "a closed stdout is not a failed run: {stderr}");
    assert!(
        !stderr.contains("panicked"),
        "the closed pipe must not panic: {stderr}"
    );

    // The run earned its install state: the missing file was repaired, the
    // intact one was left alone, and the version was bumped.
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).as_deref(),
        Some(fx2.files[0].data.as_slice()),
        "the missing file is still repaired"
    );
    assert_eq!(
        read_game_file(&dir, &fx2.files[1].rel).as_deref(),
        Some(fx2.files[1].data.as_slice()),
        "the intact file is untouched"
    );
    let cfg = std::fs::read_to_string(dir.join("config.ini")).expect("config.ini");
    assert!(cfg.contains("game_version=9.9.9-test"), "{cfg}");

    // The log mirror is written before stdout, so the run's record survives the
    // closed pipe. The closed-stdout note also proves the test exercised the
    // branch rather than writing into a live pipe.
    let log_body = std::fs::read_to_string(&log).expect("log file");
    assert!(
        log_body.contains("stdout closed") || log_body.contains("stdout write failed"),
        "the run met the closed stdout and carried on: {log_body}"
    );
    assert!(
        log_body
            .lines()
            .any(|line| line.contains(" SUMMARY total=") && line.contains("exit=0")),
        "the final record still reaches the log: {log_body}"
    );

    mock.stop();
}

#[tokio::test]
async fn a_successful_run_flushes_its_summary_to_the_log_file() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("success_log_tail");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let game = dir.to_string_lossy().into_owned();
    let log = dir.join("run.log");
    let log_arg = log.to_string_lossy().into_owned();

    let (code, stdout, stderr) = run_binary(
        &[
            "--game-path",
            &game,
            "--biz",
            "hk4e_global",
            "--io-threads",
            "1",
            "--check-only",
            "--log-file",
            &log_arg,
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");

    let summary = stdout
        .lines()
        .find(|line| line.starts_with("SUMMARY "))
        .expect("the run ends with a SUMMARY");
    let body = std::fs::read_to_string(&log).expect("the log file was written");
    let tail = body
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .expect("the log is not empty");
    assert!(
        tail.contains(summary),
        "the log's tail must be the same SUMMARY the run returned:\n{body}"
    );

    mock.stop();
}

#[tokio::test]
async fn a_fatal_run_flushes_its_last_line_to_the_log_file() {
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
    let dir = temp_dir("fatal_log_tail");
    let game = dir.join("install");
    let log = dir.join("run.log");
    let game_arg = game.to_string_lossy().into_owned();
    let log_arg = log.to_string_lossy().into_owned();

    let (code, _, stderr) = run_binary(
        &[
            "--game-path",
            &game_arg,
            "--biz",
            "hk4e_global",
            "--io-threads",
            "1",
            "--print-plan",
            "--log-file",
            &log_arg,
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 2, "a corrupt chunk range is metadata: {stderr}");

    let body = std::fs::read_to_string(&log).expect("the log file was written");
    let tail = body
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .expect("the log is not empty");
    assert!(
        tail.contains("fatal(exit=2)"),
        "the fatal line must be the log's tail:\n{body}"
    );

    mock.stop();
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
}

#[tokio::test]
async fn the_phase_completion_lines_carry_a_duration_field() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("phase_durations");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let game = dir.to_string_lossy().into_owned();

    let log = dir.join("run.log");
    let log_arg = log.to_string_lossy().into_owned();
    let (code, _, stderr) = run_binary(
        &[
            "--game-path",
            &game,
            "--biz",
            "hk4e_global",
            "--io-threads",
            "1",
            "--scan",
            "--log-file",
            &log_arg,
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");

    let body = std::fs::read_to_string(&log).expect("the log file was written");
    for phase in [
        "metadata", "manifest", "plan", "scan", "repair", "post", "run",
    ] {
        let marker = format!("phase={phase}");
        let line = body
            .lines()
            .find(|line| line.contains(&marker))
            .unwrap_or_else(|| panic!("no {marker} completion line:\n{body}"));
        assert!(line.contains("duration_ms="), "{line}");
    }

    // The per-file total is at debug and names the file.
    let rel = &fx2.files[0].rel;
    let line = body
        .lines()
        .find(|line| {
            line.contains("file repair finished")
                && line.contains(&format!("file={rel}"))
                && line.contains("duration_ms=")
        })
        .unwrap_or_else(|| panic!("no per-file duration for {rel}:\n{body}"));
    assert!(line.contains("duration_ms="), "{line}");

    // A phase that did not run must not log a duration.
    assert!(
        !body.contains("phase=check"),
        "check-only did not run:\n{body}"
    );

    // The check-only path reports its own phase and neither later phase.
    let check_log = dir.join("check.log");
    let check_log_arg = check_log.to_string_lossy().into_owned();
    let (code, _, stderr) = run_binary(
        &[
            "--game-path",
            &game,
            "--biz",
            "hk4e_global",
            "--io-threads",
            "1",
            "--check-only",
            "--log-file",
            &check_log_arg,
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");

    let body = std::fs::read_to_string(&check_log).expect("the check log file was written");
    let line = body
        .lines()
        .find(|line| line.contains("phase=check"))
        .unwrap_or_else(|| panic!("no check completion line:\n{body}"));
    assert!(line.contains("duration_ms="), "{line}");
    assert!(
        !body.contains("phase=repair"),
        "the repair loop did not run:\n{body}"
    );
    assert!(
        !body.contains("phase=post"),
        "the post-phase did not run:\n{body}"
    );

    mock.stop();
}

/// An unscanned run must not log a scan completion: the phase only runs when
/// `--scan` asked for it, and the repair phase still runs.
#[tokio::test]
async fn an_unscanned_run_does_not_log_a_scan_phase() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("unscanned_phase");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let game = dir.to_string_lossy().into_owned();

    let log = dir.join("run.log");
    let log_arg = log.to_string_lossy().into_owned();
    let (code, _, stderr) = run_binary(
        &[
            "--game-path",
            &game,
            "--biz",
            "hk4e_global",
            "--io-threads",
            "1",
            "--log-file",
            &log_arg,
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");

    let body = std::fs::read_to_string(&log).expect("the log file was written");
    assert!(
        !body.contains("phase=scan"),
        "an unscanned run must not log a scan phase:\n{body}"
    );
    assert!(
        body.contains("phase=repair"),
        "the repair phase still ran:\n{body}"
    );

    mock.stop();
}

/// The plan half's `publish` step and the execute half's `purge-before` step
/// each get their own completion line, so neither is hidden inside `plan` or
/// left untimed.
#[tokio::test]
async fn the_publish_and_pre_purge_steps_are_timed() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("phase_extra_steps");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let game = dir.to_string_lossy().into_owned();
    // The log lives outside the install: `--purge-before` treats the game tree
    // as its candidate set.
    let log_dir = temp_dir("phase_extra_steps_log");

    // Publishing only does work for `--print-plan` / `--plan-out`, so time it
    // there; the run stops before the execute half.
    let publish_log = log_dir.join("publish.log");
    let publish_log_arg = publish_log.to_string_lossy().into_owned();
    let (code, _, stderr) = run_binary(
        &[
            "--game-path",
            &game,
            "--biz",
            "hk4e_global",
            "--io-threads",
            "1",
            "--print-plan",
            "--log-file",
            &publish_log_arg,
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");

    let body = std::fs::read_to_string(&publish_log).expect("the publish log was written");
    let line = body
        .lines()
        .find(|line| line.contains("phase=publish"))
        .unwrap_or_else(|| panic!("no publish completion line:\n{body}"));
    assert!(line.contains("duration_ms="), "{line}");
    assert!(
        !body.contains("phase=repair"),
        "print-plan stops before the repair loop:\n{body}"
    );

    // The up-front cleanup is its own timed step in the execute half.
    let purge_log = log_dir.join("purge.log");
    let purge_log_arg = purge_log.to_string_lossy().into_owned();
    let (code, _, stderr) = run_binary(
        &[
            "--game-path",
            &game,
            "--biz",
            "hk4e_global",
            "--io-threads",
            "1",
            "--purge-before",
            "--log-file",
            &purge_log_arg,
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");

    let body = std::fs::read_to_string(&purge_log).expect("the purge log was written");
    let line = body
        .lines()
        .find(|line| line.contains("phase=purge-before"))
        .unwrap_or_else(|| panic!("no purge-before completion line:\n{body}"));
    assert!(line.contains("duration_ms="), "{line}");
    // The scan and publish steps did not run in this invocation.
    assert!(!body.contains("phase=scan"), "no --scan:\n{body}");
    assert!(!body.contains("phase=publish"), "no --print-plan:\n{body}");

    mock.stop();
}
