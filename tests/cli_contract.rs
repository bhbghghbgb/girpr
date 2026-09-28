//! Binary-level contract test for argument handling and exit codes.
//!
//! These run the real binary (`CARGO_BIN_EXE_girpr`) because the contract being
//! checked is process-level: which exit code a given kind of bad input
//! produces, and that clap's own `2` never leaks into the "2 = metadata/network"
//! slot (docs/02 §1).
//!
//! Every case here fails *during* argument handling, so no game directory is
//! ever touched. The post-parse cases (`--io-threads 0`, bad `--audio`) do reach
//! `logging::init_tracing`, so they leave a `logs/girpr_<ts>.log` beside the
//! built binary — inside the gitignored `target/` tree.

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
        "--json-summary",
        "--log-level",
    ] {
        assert!(stdout.contains(flag), "help is missing {flag}:\n{stdout}");
    }
}
