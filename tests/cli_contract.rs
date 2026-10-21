//! Binary-level contract test for argument handling and exit codes.
//!
//! These run the real binary (`CARGO_BIN_EXE_girpr`) because the contract being
//! checked is process-level: which exit code a given kind of bad input
//! produces, and that clap's own `2` never leaks into the "2 = metadata/network"
//! slot (docs/02 §1).
//!
//! Every case here fails *during* argument handling, so no game directory is
//! ever touched. The post-parse cases (`--io-threads 0`, bad `--audio`, a bad
//! `--include` glob) do reach `logging::init_tracing`, so they leave a
//! `logs/girpr_<ts>.log` beside the built binary — inside the gitignored
//! `target/` tree. The two `--log-file` cases are the exception: they name
//! their own destination (in a scratch dir) instead, which is what they are for.
//!
//! NOTE: `--include` matching zero *planned* files is also exit `1`, and a
//! tripped ceiling is exit `5`, but both can only be observed after the API is
//! fetched, so they live in `tests/repair_offline.rs` rather than here — this
//! file never has a server.

#[path = "common/tempdir.rs"]
mod tempdir;

use clap::Parser;
use girpr::cli::Args;
use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_girpr"))
        .args(args)
        .output()
        .expect("run girpr");
    (
        out.status.code().expect("process must exit normally"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn help_exits_zero() {
    // `Args` declares no `version`, so only `--help` gets the conventional 0.
    let (code, stdout, _) = run(&["--help"]);
    assert_eq!(code, 0, "--help should exit 0");
    assert!(stdout.contains("Usage"), "{stdout}");
}

#[test]
fn argument_errors_exit_one_not_two() {
    // clap's default is 2, which is the documented "metadata/network" code.
    let cases: Vec<Vec<&str>> = vec![
        vec!["--biz", "hk4e_global"],               // missing --game-path
        vec!["--game-path", ".", "--biz", "bogus"], // unknown enum value
        vec![
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--output",
            "xml",
        ], // unknown --output variant
        vec!["--game-path", ".", "--biz", "hk4e_global", "-x"], // unknown flag
        vec!["--game-path", ".", "--biz"],          // missing enum value
        vec![
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--io-threads",
            "0",
        ],
        vec![
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--io-threads",
            "abc",
        ],
        vec![
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--audio",
            "fr-fr",
        ],
        vec![
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--audio",
            "en-us",
            "--audio",
            "none",
        ],
        // A malformed glob is a usage error like any other bad argument.
        vec![
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--include",
            "game/[",
        ],
        vec![
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--exclude",
            "game/[",
        ],
        // A ceiling value that will not parse is exit 1. (`--max-files 0`
        // parses — it is a meaningful ceiling — and that case lives in
        // `src/cli.rs`'s unit tests, which do not need a server.)
        vec![
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--max-files",
            "abc",
        ],
        vec![
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--max-delete-bytes",
            "1.5",
        ],
        vec![
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--max-download-bytes",
            "abc",
        ],
        vec![
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--max-write-bytes",
            "abc",
        ],
        vec![
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--max-verified-download-bytes",
            "abc",
        ],
        vec![
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--max-verified-write-bytes",
            "abc",
        ],
        vec![
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--max-ram-bytes",
            "abc",
        ],
        // An unknown repair mode is an argument-level failure like any other.
        vec![
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--repair-mode",
            "sideways",
        ],
    ];
    for args in cases {
        let (code, _, stderr) = run(&args);
        assert_eq!(code, 1, "{args:?} should exit 1, got {code} ({stderr})");
        assert!(
            !stderr.is_empty(),
            "{args:?} should explain itself on stderr"
        );
    }
}

#[test]
fn help_text_lists_every_documented_flag() {
    let (_, stdout, _) = run(&["--help"]);
    for flag in [
        "--game-path",
        "--biz",
        "--audio",
        "--io-threads",
        "--purge-after",
        "--purge-before",
        "--check-only",
        "--dry-run",
        "--purge-only",
        "--fail-fast",
        "--print-plan",
        "--plan-out",
        "--output",
        "--log-level",
        "--log-file",
        "--include",
        "--exclude",
        "--hyp-base",
        "--sophon-base",
        "--max-files",
        "--max-delete-bytes",
        "--max-download-bytes",
        "--max-write-bytes",
        "--max-verified-download-bytes",
        "--max-verified-write-bytes",
        "--max-ram-bytes",
        "--repair-mode",
        "--scan",
    ] {
        assert!(stdout.contains(flag), "help is missing {flag}:\n{stdout}");
    }
}

#[test]
fn an_unopenable_log_file_warns_and_changes_nothing_about_the_exit_code() {
    // `logging::init_tracing` is infallible by contract: an unwritable log is a
    // stderr warning, because a run must not be able to end differently just
    // because of where it was asked to log. If that ever stops holding, this
    // fails — which is the point of pinning it here.
    //
    // `--io-threads 0` is the cheapest way to reach a *decided* exit code after
    // the file layer is installed: validation rejects it (exit 1) before any
    // game directory or network is touched, so the expected code is exact and
    // the case needs no server.
    let scratch = tempdir::TempDir::new("girpr_cli_logfile", "unopenable");
    // A regular file where the log's parent directory should be: `create_dir_all`
    // cannot succeed, and cannot be confused with "the directory was absent".
    let blocker = scratch.join("not_a_dir");
    std::fs::write(&blocker, b"x").expect("blocker file");
    let bad_log = blocker.join("run.log");

    let base: &[&str] = &[
        "--game-path",
        ".",
        "--biz",
        "hk4e_global",
        "--io-threads",
        "0",
    ];
    let mut flagged: Vec<&str> = base.to_vec();
    flagged.extend_from_slice(&["--log-file", bad_log.to_str().unwrap()]);

    let (code, _, stderr) = run(&flagged);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains("WARN disabling file logging"),
        "an unopenable log file must degrade to a warning:\n{stderr}"
    );
    assert!(
        stderr.contains(bad_log.to_str().unwrap()),
        "the warning must name the file it could not open:\n{stderr}"
    );
    // The default location is announced by a `tracing::info!`, which stderr
    // only carries at `info` or lower — that is fine, because nobody asked for
    // it. What must hold is that the failure moved nothing at all.
    let (base_code, _, _) = run(base);
    assert_eq!(code, base_code, "the log file must not move the exit code");
}

#[test]
fn an_explicit_log_file_is_echoed_to_stderr_and_created() {
    // The other half of the contract: a caller that asked for a specific path
    // is told where it went on stderr, not just inside the file, so a wrapper
    // that captured only stdout can still find it.
    //
    // `--log-level warn` is what makes this sharp. The `tracing::info!` that
    // reports the default path is filtered out at `warn`, so anything still
    // saying `log file:` on stderr at that level is the acknowledgement the
    // caller asked for.
    let scratch = tempdir::TempDir::new("girpr_cli_logfile", "explicit");
    let log = scratch.join("nested").join("run.log");

    let (code, _, stderr) = run(&[
        "--game-path",
        ".",
        "--biz",
        "hk4e_global",
        "--io-threads",
        "0", // stops before any game directory or network is touched
        "--log-level",
        "warn",
        "--log-file",
        log.to_str().unwrap(),
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains(&format!("log file: {}", log.display())),
        "the requested path must be echoed on stderr even at --log-level warn:\n{stderr}"
    );
    // Missing parents are created, so `--log-file a/b/c.log` works in one go.
    let body = std::fs::read_to_string(&log).expect("the log file is created");
    assert!(
        body.contains("log file:"),
        "the file carries the same announcement:\n{body}"
    );
}

#[test]
fn exit_five_is_a_refusal_not_an_argument_error() {
    // A tripped ceiling is exit `5`, deliberately *not* `1`. Whether a limit
    // trips is only knowable after the API is fetched, so a
    // "the run was misconfigured" code cannot mean both — which is also why
    // `--max-files 0` must parse here rather than be rejected: it is a
    // meaningful ceiling, and only an unparseable value is exit `1`.
    //
    // Reaching the refusal needs a server, so the behavioural case lives in
    // `tests/repair_offline.rs`; what is pinned here is that the flag surface
    // accepts 0 and that no argument-level failure has become something else.
    let args = Args::parse_from([
        "girpr",
        "--game-path",
        ".",
        "--biz",
        "hk4e_global",
        "--max-files",
        "0",
        "--max-delete-bytes",
        "0",
        "--max-download-bytes",
        "0",
        "--max-write-bytes",
        "0",
        "--max-verified-download-bytes",
        "0",
        "--max-verified-write-bytes",
        "0",
        "--fail-fast",
        "--purge-only",
    ]);
    let ctx = args.into_ctx().expect("0 is a ceiling, not a usage error");
    assert_eq!(ctx.limits.files, Some(0));
    assert_eq!(ctx.limits.delete_bytes, Some(0));
    assert_eq!(ctx.limits.download_bytes, Some(0));
    assert_eq!(ctx.limits.write_bytes, Some(0));
    assert_eq!(ctx.limits.verified_download_bytes, Some(0));
    assert_eq!(ctx.limits.verified_write_bytes, Some(0));
    assert!(ctx.limits.is_active());
    assert!(ctx.fail_fast);
    assert!(ctx.purge_only);
}

#[test]
fn a_zero_ram_budget_is_a_budget_not_a_usage_error() {
    // `--max-ram-bytes 0` is meaningful: it says "stage nothing", which
    // selects the other write order. It is not a malformed number.
    let args = Args::parse_from([
        "girpr",
        "--game-path",
        ".",
        "--biz",
        "hk4e_global",
        "--max-ram-bytes",
        "0",
    ]);
    let ctx = args.into_ctx().expect("0 is a budget, not a usage error");
    assert_eq!(ctx.ram_budget(), Some(0));
    assert!(ctx.limits.is_active());
}
