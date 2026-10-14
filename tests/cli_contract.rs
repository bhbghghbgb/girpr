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
//! `target/` tree.
//!
//! NOTE: `--include` matching zero *planned* files is also exit `1`, and a
//! tripped ceiling is exit `5`, but both can only be observed after the API is
//! fetched, so they live in `tests/repair_offline.rs` rather than here — this
//! file never has a server.

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
        "--json-summary",
        "--log-level",
        "--include",
        "--exclude",
        "--max-files",
        "--max-delete-bytes",
        "--max-download-bytes",
    ] {
        assert!(stdout.contains(flag), "help is missing {flag}:\n{stdout}");
    }
}

#[test]
fn exit_five_is_a_refusal_not_an_argument_error() {
    // docs/04 §4.4: a tripped ceiling is exit `5`, deliberately *not* `1`.
    // Whether a limit trips is only knowable after the API is fetched, so a
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
        "--fail-fast",
        "--purge-only",
    ]);
    let ctx = args.into_ctx().expect("0 is a ceiling, not a usage error");
    assert_eq!(ctx.limits.files, Some(0));
    assert_eq!(ctx.limits.delete_bytes, Some(0));
    assert_eq!(ctx.limits.download_bytes, Some(0));
    assert_eq!(ctx.limits.write_bytes, Some(0));
    assert!(ctx.limits.is_active());
    assert!(ctx.fail_fast);
    assert!(ctx.purge_only);
}
