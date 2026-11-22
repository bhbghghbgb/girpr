//! Process-level cases: the shipped binary's endpoints, output, logging and
//! closed-stdout behaviour.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common::{
    ChunkSpec, FileSpec, FixtureSpec, GameFixture, LATEST_TAG, Lies, MockOpts, MockServer,
    build_fixture, build_fixture_spec, read_game_file, run_binary, temp_dir, write_game_file,
};
use girpr_tempdir::TempDir;

/// One base-precedence row: its label, flags and environment overrides.
type PrecedenceRow<'a> = (&'a str, Vec<&'a str>, Vec<(&'a str, &'a str)>);

/// One phase row: its label, flags, log name and assertion.
type PhaseRow<'a> = (&'a str, &'a [&'a str], &'a str, fn(&str, &GameFixture));

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
    crate::common::remove_run_lock_for_args(args);
    (
        out.status.code().expect("process must exit normally"),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The standard fixture as a running mock plus a fresh install holding that
/// fixture's files, ready for a run.
///
/// One built fixture backs both: the mock gets a clone and the original lays
/// the bytes out on disk, so the install starts from the game the mock serves.
async fn start_standard_install(name: &str) -> (MockServer, TempDir, GameFixture, String) {
    let fx = build_fixture();
    let mock = MockServer::start(fx.clone(), MockOpts::default()).await;
    let dir = temp_dir(name);
    for f in &fx.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let game = dir.to_string_lossy().into_owned();
    (mock, dir, fx, game)
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
            "--jobs",
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
    assert!(cfg.contains(&format!("game_version={LATEST_TAG}")), "{cfg}");

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
    let (mock, dir, _, game) = start_standard_install("success_log_tail").await;
    let log = dir.join("run.log");
    let log_arg = log.to_string_lossy().into_owned();

    let (code, stdout, stderr) = run_binary(
        &[
            "--game-path",
            &game,
            "--biz",
            "hk4e_global",
            "--jobs",
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
            "--jobs",
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
async fn a_scoped_cleanup_of_a_removed_path_works_through_the_binary() {
    // Reduced to a smoke: the one process-only fact worth pinning is that the
    // shipped binary's `main` renders `SUMMARY … total=0` to stdout for the
    // `--purge-only --include <removed path>` flag combination. The old comment
    // claimed a process-only "exit 1 / usage guard" distinction, but the run
    // asserts `code == 0`; the disk state, empty plan, zero chunk hits and
    // `scoped=1`/`config_bumped=0` are all asserted at library level in
    // `repair_purge.rs::purge_only_deletes_a_scoped_path_that_is_absent_from_the_new_manifest`,
    // and CLAP acceptance of the flags is pinned in `cli_contract.rs`.
    let (mock, dir, _fx, game) = start_standard_install("binary_removed").await;
    write_game_file(&dir, "game/removed.dat", &[0x7Eu8; 512]);
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
        stdout.contains("SUMMARY ")
            && stdout.contains("total=0")
            && stdout.contains("scoped=1")
            && stdout.contains("config_bumped=0"),
        "{stdout}"
    );

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
    let (mock, _dir, fx, game) = start_standard_install("output_json").await;
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
    assert_eq!(last["total"], fx.files.len() as u64);
    assert_eq!(last["exit"], 0);
    let plan: serde_json::Value = serde_json::from_str(stdout.lines().nth(1).unwrap()).unwrap();
    assert_eq!(plan["files"], fx.files.len() as u64);
    assert_eq!(plan["scoped"], 0);

    // The default is the text plane, and it is unchanged by any of this.
    let mut text_args = base.clone();
    text_args.push("--print-plan");
    let (code, stdout, stderr) = run_binary(&text_args, &[]).await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.starts_with("REPORT local_version=")
            && stdout.contains(&format!("PLAN files={}", fx.files.len())),
        "{stdout}"
    );
    assert!(stdout.trim_end().ends_with("exit=0"), "{stdout}");

    mock.stop();
}

#[tokio::test]
async fn base_precedence_and_env_fallback() {
    // Two rows over one mock and install. The first points the env seams at a
    // **dead** port, so an env win would fail with exit 2 instead of 0; the
    // second drops the flags so the seam every offline e2e run uses is still
    // exercised. Each row keeps its own assertion message as the label.
    let (mock, _dir, _, game) = start_standard_install("base_precedence").await;
    let rows: &[PrecedenceRow<'_>] = &[
        (
            "the flag must win over the env var",
            vec![
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
            vec![
                ("GIRPR_HYP_BASE", "http://127.0.0.1:1"),
                ("GIRPR_SOPHON_BASE", "http://127.0.0.1:1"),
            ],
        ),
        (
            "env overrides must still work",
            vec!["--game-path", &game, "--biz", "hk4e_global", "--check-only"],
            vec![
                ("GIRPR_HYP_BASE", &mock.hyp_base),
                ("GIRPR_SOPHON_BASE", &mock.sophon_base),
            ],
        ),
    ];
    for (message, args, env) in rows {
        let (code, _, stderr) = run_binary(args, env).await;
        assert_eq!(code, 0, "{message}: {stderr}");
        // The endpoint override reached `HypClient::new_with_bases`: a miss would
        // have gone to production (flag row) or the dead port (env row) and
        // exited 2. The REPORT/SUMMARY, repair and scope facts of a full narrowed
        // run are pinned in cli_contract.rs and repair_scope.rs; this seam check
        // is the part that needs the binary.
        assert!(
            mock.first_target_with("getBuild").is_some(),
            "{message}: the endpoint override reached the client"
        );
    }
    mock.stop();
}

/// Every phase that runs reports one `duration_ms` line, and a phase that did
/// not run reports nothing. One table over flag sets shares a single mock and
/// install; each row keeps its own log file and its own labelled assertions.
#[tokio::test]
async fn phase_completion_timings_follow_the_flags() {
    let (mock, _dir, fx, game) = start_standard_install("phase_durations").await;
    // Logs live outside the install: `--purge-before` treats the game tree as
    // its candidate set.
    let log_dir = temp_dir("phase_durations_log");

    fn full_phases(body: &str, fx: &GameFixture) {
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
        let rel = &fx.files[0].rel;
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
    }

    fn check_only_phase(body: &str, _fx: &GameFixture) {
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
    }

    fn unscanned_phase(body: &str, _fx: &GameFixture) {
        assert!(
            !body.contains("phase=scan"),
            "an unscanned run must not log a scan phase:\n{body}"
        );
        assert!(
            body.contains("phase=repair"),
            "the repair phase still ran:\n{body}"
        );
    }

    fn publish_phase(body: &str, _fx: &GameFixture) {
        let line = body
            .lines()
            .find(|line| line.contains("phase=publish"))
            .unwrap_or_else(|| panic!("no publish completion line:\n{body}"));
        assert!(line.contains("duration_ms="), "{line}");
        assert!(
            !body.contains("phase=repair"),
            "print-plan stops before the repair loop:\n{body}"
        );
    }

    fn purge_before_phase(body: &str, _fx: &GameFixture) {
        let line = body
            .lines()
            .find(|line| line.contains("phase=purge-before"))
            .unwrap_or_else(|| panic!("no purge-before completion line:\n{body}"));
        assert!(line.contains("duration_ms="), "{line}");
        // The scan and publish steps did not run in this invocation.
        assert!(!body.contains("phase=scan"), "no --scan:\n{body}");
        assert!(!body.contains("phase=publish"), "no --print-plan:\n{body}");
    }

    let rows: &[PhaseRow<'_>] = &[
        ("--scan full run", &["--scan"], "full.log", full_phases),
        (
            "--check-only",
            &["--check-only"],
            "check.log",
            check_only_phase,
        ),
        ("no --scan", &[], "unscanned.log", unscanned_phase),
        (
            "--print-plan",
            &["--print-plan"],
            "publish.log",
            publish_phase,
        ),
        (
            "--purge-before",
            &["--purge-before"],
            "purge.log",
            purge_before_phase,
        ),
    ];
    for (label, flags, log_name, check) in rows {
        let log = log_dir.join(log_name);
        let log_arg = log.to_string_lossy().into_owned();
        let mut args: Vec<&str> = vec![
            "--game-path",
            &game,
            "--biz",
            "hk4e_global",
            "--jobs",
            "1",
            "--log-file",
            &log_arg,
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
        ];
        args.extend_from_slice(flags);
        let (code, _, stderr) = run_binary(&args, &[]).await;
        assert_eq!(code, 0, "{label}: stderr: {stderr}");

        let body = std::fs::read_to_string(&log)
            .unwrap_or_else(|e| panic!("the {label} log file was written: {e}"));
        check(&body, &fx);
    }

    mock.stop();
}
