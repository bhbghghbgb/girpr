//! Binary-level contract tests: argument handling, exit codes, and the stdout
//! data plane.
//!
//! These run the real binary (`CARGO_BIN_EXE_girpr`) because the contract being
//! checked is process-level: which exit code a given kind of bad input
//! produces, and that clap's own `2` never leaks into the "2 = metadata/network"
//! slot (docs/02 §1).
//!
//! The argument cases fail *during* argument handling, so no game directory is
//! ever touched. The post-parse cases (`--jobs 0`, bad `--audio`, a bad
//! `--include` glob) do reach `logging::init_tracing`, so they leave a
//! `logs/girpr_<ts>.log` beside the built binary — inside the gitignored
//! `target/` tree. The two `--log-file` cases are the exception: they name
//! their own destination (in a scratch dir) instead, which is what they are for.
//!
//! The data-plane goldens at the end are the other kind of case: they need the
//! mock HoYoPlay/Sophon server in `tests/common`, because `REPORT`/`PLAN` are
//! only emitted once the metadata has been fetched. They pin the *sequence* of
//! records and each record's field keys, never a value, so a wording change
//! that keeps the contract passes and a reordered or duplicated record fails.
//!
//! NOTE: `--include` matching zero *planned* files is also exit `1`, and a
//! tripped ceiling is exit `5`, but the ceiling cases are exercised in
//! `tests/repair_limits.rs`; this file's argument cases never have a server.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use clap::Parser;
use girpr::Args;
use girpr_tempdir::TempDir;
use std::process::Command;

use common::{
    MockOpts, MockServer, build_fixture, read_game_file, remove_run_lock_for_args, temp_dir,
    write_game_file,
};

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

/// The console's records, with the colour taken out.
///
/// `tracing`'s human format colours the level and the target separately, and
/// the escape sequences land *between* them: the text is `INFO`, then an
/// escape, then `girpr::logging`. A substring that spans one is not a
/// substring at all — and an assertion that a coloured string does *not*
/// contain one passes for the wrong reason, which is worse than no assertion.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // Skip the escape sequence: ESC, '[', then up to its final byte.
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() || c == '~' {
                break;
            }
        }
    }
    out
}

/// The indent clap's help gives each option line.
///
/// The command's `--help` lays options out flat under `Options:` with six
/// leading spaces. [`help_entry`] depends on that layout: if a clap upgrade
/// changes it, the entry will not be found and the helper panics with the
/// expected indent and the help text, rather than returning a slice of the
/// wrong part of the output.
const HELP_OPTION_INDENT: &str = "      ";

/// The slice of `--help` output that belongs to one option: from the option's
/// own line to the next option's line.
///
/// Help is a flat text plane, so a substring search over all of it can be
/// satisfied by another option's annotation. The block keeps an assertion about
/// one flag about that flag.
fn help_entry<'a>(help: &'a str, flag: &str) -> &'a str {
    let entry_line = format!("{HELP_OPTION_INDENT}{flag}");
    // A flag is a whole token: `--jobs` must not match a `--jobs-<something>`
    // option line. clap follows the flag with a space when it takes a value and
    // with a newline when it does not; the end of the text is also a boundary.
    let at_entry_line = |line: usize| {
        help[line..].starts_with(&entry_line)
            && matches!(
                help.as_bytes().get(line + entry_line.len()),
                Some(b' ' | b'\n') | None
            )
    };
    let start = std::iter::once(0)
        .chain(help.match_indices('\n').map(|(i, _)| i + 1))
        .find(|&line| at_entry_line(line))
        .unwrap_or_else(|| {
            panic!(
                "help has no {flag} entry at the expected {}-space option indent:\n{help}",
                HELP_OPTION_INDENT.len()
            )
        });
    let entry = &help[start..];
    let next_option = format!("\n{HELP_OPTION_INDENT}--");
    let end = entry
        .match_indices(&next_option)
        .next()
        .map_or(entry.len(), |(i, _)| i);
    &entry[..end]
}

#[test]
fn help_entry_does_not_borrow_a_neighbours_annotations() {
    let help =
        "      --jobs <JOBS>\n          jobs text\n\n      --other\n          [default: 4]\n";
    assert!(
        !help_entry(help, "--jobs").contains("[default: 4]"),
        "the block ended before the next option"
    );
    assert!(help_entry(help, "--other").contains("[default: 4]"));
}

#[test]
fn help_entry_requires_a_boundary_after_the_flag() {
    // `--jobs` is a prefix of `--jobs-extra`, but they are different options.
    // `--jobs` is the first entry, so the first line that carries a newline
    // before it is `--jobs-extra`: a search for the newline-marked form of
    // `--jobs` lands there, and only the space after the flag tells them apart.
    let help = "      --jobs <JOBS>\n          jobs text\n\n      --jobs-extra <N>\n          different option\n";
    let jobs = help_entry(help, "--jobs");
    assert_eq!(
        jobs, "      --jobs <JOBS>\n          jobs text\n",
        "the block belongs to --jobs, not the option whose name it prefixes"
    );
    assert!(!jobs.contains("different option"));
    // The boundary is also a newline (a flag with no value) or the end of the
    // text, not only the space before a value.
    let boolean = "      --dry-run\n          no value\n";
    assert_eq!(
        help_entry(boolean, "--dry-run"),
        "      --dry-run\n          no value\n"
    );
    assert_eq!(
        help_entry("      --dry-run", "--dry-run"),
        "      --dry-run"
    );
}

#[test]
fn help_entry_finds_an_entry_after_a_preceding_option() {
    // The desired entry is not the first line of the help, so its start comes
    // from the newline before it rather than from the start of the text.
    let help = "      --first\n          first text\n\n      --jobs <JOBS>\n          jobs text\n\n      --last\n          last text\n";
    assert!(!help.starts_with("      --jobs"));
    assert_eq!(
        help_entry(help, "--jobs"),
        "      --jobs <JOBS>\n          jobs text\n",
        "the block runs from --jobs to the next option"
    );
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
        vec!["--game-path", ".", "--biz", "hk4e_global", "--jobs", "0"],
        vec!["--game-path", ".", "--biz", "hk4e_global", "--jobs", "abc"],
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
        "--jobs",
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
        "--no-log-file",
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
fn help_names_jobs_and_keeps_the_compatibility_alias_discoverable() {
    let (_, stdout, _) = run(&["--help"]);
    let jobs = help_entry(&stdout, "--jobs");
    assert!(jobs.contains("--jobs"), "{jobs}");
    assert!(
        jobs.contains("--io-threads"),
        "the compatibility alias stays discoverable:\n{jobs}"
    );
    // The entry must show the default the run actually applies, and only the
    // `--jobs` entry is searched: another option's `[default: …]` cannot make
    // this pass.
    let applied = Args::parse_from(["girpr", "--game-path", ".", "--biz", "hk4e_global"])
        .into_ctx()
        .expect("the defaults are a valid configuration")
        .jobs;
    assert!(
        jobs.contains(&format!("[default: {applied}]")),
        "the --jobs entry does not show the applied default:\n{jobs}"
    );
}

#[test]
fn zero_jobs_is_exit_one_and_the_message_names_the_flag() {
    let (code, _, stderr) = run(&["--game-path", ".", "--biz", "hk4e_global", "--jobs", "0"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("--jobs must be >= 1"), "{stderr}");
}

#[test]
fn an_oversized_jobs_is_exit_one_instead_of_a_runtime_failure() {
    // A value this large would ask the runtime for that many threads. It must
    // be refused while the flag is being checked — named as a usage error, not
    // surfaced later as "failed to start runtime" (exit 1 either way, but the
    // latter reads like an environment problem and does not name the flag).
    let (code, _, stderr) = run(&[
        "--game-path",
        ".",
        "--biz",
        "hk4e_global",
        "--jobs",
        "1000000",
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains("usage error: --jobs must be <= 1024"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("failed to start runtime"),
        "the flag must be refused before the runtime is built:\n{stderr}"
    );
}

#[test]
fn the_io_threads_alias_validates_at_the_same_jobs_ceiling() {
    // The ceiling is checked before `--audio`, so a bad audio value after the
    // jobs flag shows which check the run reached. At 1024 the alias passes
    // jobs validation and fails on the audio; at 1025 it fails on the jobs
    // ceiling first, with the same usage error `--jobs` produces. Neither case
    // builds the runtime or touches a game directory.
    let (code, _, stderr) = run(&[
        "--game-path",
        ".",
        "--biz",
        "hk4e_global",
        "--io-threads",
        "1024",
        "--audio",
        "fr-fr",
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains("usage error: unknown audio lang"),
        "the alias accepted the ceiling and reached the next check:\n{stderr}"
    );
    assert!(
        !stderr.contains("--jobs must be <="),
        "the ceiling itself must not be rejected:\n{stderr}"
    );

    let (code, _, stderr) = run(&[
        "--game-path",
        ".",
        "--biz",
        "hk4e_global",
        "--io-threads",
        "1025",
        "--audio",
        "fr-fr",
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains("usage error: --jobs must be <= 1024"),
        "one past the ceiling is the same usage error through the alias:\n{stderr}"
    );
}

#[test]
fn the_jobs_flag_is_validated_at_the_same_process_level_ceiling_as_the_alias() {
    // The same boundary pair as the alias case, through the primary spelling:
    // 1024 passes jobs validation and fails on the next check (a bad
    // `--audio`), while 1025 fails the jobs ceiling first. Each value runs
    // through both spellings so the pair is compared rather than only pinned.
    let run_boundary = |flag: &str, value: &str| {
        run(&[
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            flag,
            value,
            "--audio",
            "fr-fr",
        ])
    };
    let cases = [
        ("1024", "usage error: unknown audio lang", false),
        ("1025", "usage error: --jobs must be <= 1024", true),
    ];
    for (value, expected, ceiling_reported) in cases {
        let (jobs_code, jobs_stdout, jobs_stderr) = run_boundary("--jobs", value);
        let (alias_code, alias_stdout, alias_stderr) = run_boundary("--io-threads", value);
        assert_eq!(jobs_code, 1, "--jobs {value}: {jobs_stderr}");
        assert_eq!(alias_code, 1, "--io-threads {value}: {alias_stderr}");
        assert_eq!(
            jobs_code, alias_code,
            "--jobs and --io-threads disagree at {value}"
        );
        assert_eq!(jobs_stdout, alias_stdout, "--jobs {value} changed stdout");
        assert!(
            jobs_stderr.contains(expected),
            "--jobs {value} stopped at the wrong check:\n{jobs_stderr}"
        );
        assert!(
            alias_stderr.contains(expected),
            "--io-threads {value} stopped at the wrong check:\n{alias_stderr}"
        );
        assert_eq!(
            jobs_stderr.contains("--jobs must be <="),
            ceiling_reported,
            "--jobs {value} reported the ceiling at the wrong step:\n{jobs_stderr}"
        );
    }
}

#[test]
fn plan_out_help_documents_every_serialized_field() {
    // R-9: `RepairEntry` serializes `mode` and `extra_bytes` too. The help is
    // the operator-facing description of the document, so it must name the
    // whole shape the binary writes. `help_text_lists_every_documented_flag`
    // only checks flag names and cannot catch a stale JSON shape.
    let (_, stdout, _) = run(&["--help"]);
    assert!(
        stdout.contains("mode,extra_bytes"),
        "the --plan-out help omits the serialized fields:\n{stdout}"
    );
}

#[test]
fn an_unopenable_log_file_warns_and_changes_nothing_about_the_exit_code() {
    // `logging::init_tracing` is infallible by contract: an unwritable log is a
    // stderr warning, because a run must not be able to end differently just
    // because of where it was asked to log. If that ever stops holding, this
    // fails — which is the point of pinning it here.
    //
    // `--jobs 0` is the cheapest way to reach a *decided* exit code after
    // the file layer is installed: validation rejects it (exit 1) before any
    // game directory or network is touched, so the expected code is exact and
    // the case needs no server.
    let scratch = TempDir::new("girpr_cli_logfile", "unopenable");
    // A regular file where the log's parent directory should be: `create_dir_all`
    // cannot succeed, and cannot be confused with "the directory was absent".
    let blocker = scratch.join("not_a_dir");
    std::fs::write(&blocker, b"x").expect("blocker file");
    let bad_log = blocker.join("run.log");

    let base: &[&str] = &["--game-path", ".", "--biz", "hk4e_global", "--jobs", "0"];
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
    let scratch = TempDir::new("girpr_cli_logfile", "explicit");
    let log = scratch.join("nested").join("run.log");

    let (code, _, stderr) = run(&[
        "--game-path",
        ".",
        "--biz",
        "hk4e_global",
        "--jobs",
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

/// A **debug** build writes no log file; a **release** build writes one.
///
/// The policy is the shared crate's — `girpr-log` decides whether there is a
/// default at all, and the shape of that default is asserted there — so what
/// is pinned here is that the *program* does what the policy says: it says
/// on stderr which of the two it is doing, and a debug build does not bring
/// the `logs/` directory beside the executable into existence.
///
/// It is not pinned by *finding* the file. The default's name carries a
/// timestamp, so the only way to find it is to list the directory and read
/// every file in it — and that cost is the history of the machine rather
/// than the code.
#[test]
fn the_default_log_file_follows_the_build_it_was_built_from() {
    let logs = exe_dir().join("logs");
    let existed = logs.exists();

    let (code, _, stderr) = run(&["--game-path", ".", "--biz", "hk4e_global", "--jobs", "0"]);
    assert_eq!(code, 1, "{stderr}");

    if cfg!(debug_assertions) {
        assert!(
            stderr.contains("log file: none"),
            "a debug build says it wrote no file:\n{stderr}"
        );
        if !existed {
            assert!(
                !logs.exists(),
                "and it did not bring {} into existence",
                logs.display()
            );
        }
    } else {
        assert!(
            stderr.contains("log file: "),
            "a release build says where the file went:\n{stderr}"
        );
    }
}

/// `--no-log-file` turns the file off, and it beats the environment.
///
/// "Write no log file" is the more specific thing to have said, so it wins
/// over a path the shell happened to export. The file is not written at
/// all — which is the whole point, and which is why the assertion is
/// `exists()` on a known path rather than a search of a directory.
#[test]
fn no_log_file_turns_the_file_off_even_when_the_environment_names_one() {
    let scratch = TempDir::new("girpr_cli_logfile", "no_log_file");
    let log = scratch.join("from-env.log");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girpr"))
        .args([
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--jobs",
            "0",
            "--no-log-file",
        ])
        .env("GIRPR_LOG_FILE", &log)
        .output()
        .expect("run girpr");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(!log.exists(), "no log file was written: {}", log.display());
    assert!(
        !stderr.contains(&format!("log file: {}", log.display())),
        "and the path was not announced:\n{stderr}"
    );
}

/// `--no-log-file` and `--log-file` say opposite things, so they are a usage
/// error rather than one of them silently winning.
#[test]
fn no_log_file_and_log_file_together_are_a_usage_error() {
    let scratch = TempDir::new("girpr_cli_logfile", "conflict");
    let log = scratch.join("both.log");

    let (code, _, stderr) = run(&[
        "--game-path",
        ".",
        "--biz",
        "hk4e_global",
        "--jobs",
        "0",
        "--no-log-file",
        "--log-file",
        log.to_str().unwrap(),
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains("cannot be used with"),
        "the conflict is reported:\n{stderr}"
    );
    assert!(!log.exists(), "and no file was written");
}

/// The environment variable names the log file when the flag does not.
///
/// Same precedence as everywhere else in the workspace: the flag wins, the
/// environment answers for a flag that was never given, and the timestamped
/// default answers for both.
#[test]
fn the_environment_names_the_log_file_when_the_flag_does_not() {
    let scratch = TempDir::new("girpr_cli_logfile", "env");
    let log = scratch.join("from-env.log");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girpr"))
        .args(["--game-path", ".", "--biz", "hk4e_global", "--jobs", "0"])
        .env("GIRPR_LOG_FILE", &log)
        .output()
        .expect("run girpr");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains(&format!("log file: {}", log.display())),
        "the path is echoed on stderr, because the caller named it:\n{stderr}"
    );
    let body = std::fs::read_to_string(&log).expect("the environment named the file");
    assert!(
        body.contains("usage error: --jobs must be >= 1"),
        "and it holds the run's own records:\n{body}"
    );
}

/// The flag wins over the environment.
///
/// The flag was typed by the person running the tool; the environment is
/// whatever the shell happened to export. The one that was typed wins.
#[test]
fn the_flag_wins_over_the_environment() {
    let scratch = TempDir::new("girpr_cli_logfile", "precedence");
    let from_flag = scratch.join("from-flag.log");
    let from_env = scratch.join("from-env.log");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girpr"))
        .args([
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--jobs",
            "0",
            "--log-file",
            from_flag.to_str().unwrap(),
        ])
        .env("GIRPR_LOG_FILE", &from_env)
        .output()
        .expect("run girpr");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(from_flag.exists(), "the flag's file was written");
    assert!(!from_env.exists(), "the environment's file was not");
    assert!(
        stderr.contains(&format!("log file: {}", from_flag.display())),
        "{stderr}"
    );
}

/// The console is `info` and the log file is `debug`, by default.
///
/// `--log-level error` takes the console below the file, and the file still
/// holds the run's own record of itself: the two levels answer separate
/// questions, and the file's is the one asked afterwards.
#[test]
fn the_file_keeps_what_a_quiet_console_drops() {
    let scratch = TempDir::new("girpr_cli_logfile", "levels");
    let log = scratch.join("levels.log");

    let (code, _, stderr) = run(&[
        "--game-path",
        ".",
        "--biz",
        "hk4e_global",
        "--jobs",
        "0",
        "--log-level",
        "error",
        "--log-file",
        log.to_str().unwrap(),
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        !strip_ansi(&stderr).contains("INFO girpr::logging"),
        "the console is at `error`, so it kept no info record:\n{stderr}"
    );
    let body = std::fs::read_to_string(&log).expect("the log file was written");
    assert!(
        body.contains("INFO girpr::logging: log file:"),
        "the file is at `debug` by default, so it kept it:\n{body}"
    );
}

/// The directory the executable is in — where the default log file lands.
fn exe_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_BIN_EXE_girpr"))
        .parent()
        .expect("the executable has a directory")
        .to_path_buf()
}

/// The environment sets the log file's level, when the flag does not.
///
/// The one seam of the policy that the flag table cannot show: the flag wins,
/// the environment answers for a flag that was never given, and `debug` — the
/// default — answers for both.
#[test]
fn the_environment_sets_the_log_files_level() {
    let scratch = TempDir::new("girpr_cli_logfile", "file_level");
    let log = scratch.join("levels.log");

    let out = Command::new(env!("CARGO_BIN_EXE_girpr"))
        .args([
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--jobs",
            "0",
            "--log-file",
            log.to_str().unwrap(),
        ])
        .env("GIRPR_LOG_FILE_LEVEL", "error")
        .output()
        .expect("run girpr");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{stderr}");

    let body = std::fs::read_to_string(&log).expect("the log file was written");
    assert!(
        !body.contains("INFO girpr::logging"),
        "the file is at `error`, so it kept no info record:\n{body}"
    );
    assert!(
        body.contains("ERROR girpr: usage error"),
        "but it kept the error:\n{body}"
    );
}

/// The flag beats the environment for the log file's level too.
#[test]
fn the_log_file_level_flag_beats_the_environment() {
    let scratch = TempDir::new("girpr_cli_logfile", "file_level_flag");
    let log = scratch.join("levels.log");

    let out = Command::new(env!("CARGO_BIN_EXE_girpr"))
        .args([
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--jobs",
            "0",
            "--log-file",
            log.to_str().unwrap(),
            "--log-file-level",
            "debug",
        ])
        .env("GIRPR_LOG_FILE_LEVEL", "error")
        .output()
        .expect("run girpr");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{stderr}");

    let body = std::fs::read_to_string(&log).expect("the log file was written");
    assert!(
        body.contains("INFO girpr::logging: log file:"),
        "the flag's `debug` beat the environment's `error`:\n{body}"
    );
}

/// `RUST_LOG` adds to the console; it cannot silence the program. The
/// program's own level is `--log-level`, and `RUST_LOG` only adds to it.
#[test]
fn an_unrelated_rust_log_does_not_silence_the_program() {
    let out = Command::new(env!("CARGO_BIN_EXE_girpr"))
        .args(["--game-path", ".", "--biz", "hk4e_global", "--jobs", "0"])
        .env("RUST_LOG", "reqwest=debug")
        .output()
        .expect("run girpr");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        strip_ansi(&stderr).contains("INFO girpr::logging"),
        "the program still speaks at its own level:\n{stderr}"
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

// ---------------------------------------------------------------------------
// the stdout data plane: event order, field keys, one record per event
// ---------------------------------------------------------------------------
//
// `--output json` is the plane a script parses: one object per line, each with
// an `event` name. The contract is the *sequence* of events and the *keys* of
// each object — a parser reads fields by name, and the fixture owns the values.
//
// The goldens are deliberately compact: an event name and its field keys, in
// report order. They cannot fail on a value or a human wording change, and they
// do fail when a record moves, repeats, or loses/renames a key.
//
// The key lists below are the test-side copy of the output vocabulary: there is
// no production key manifest to derive them from, so a change to a key name or
// its order is a deliberate edit here as well.

/// The keys `REPORT` carries, in order.
///
/// The shape does not depend on whether the install has a version: an absent
/// one is reported as `local_version=none`, not by omitting the key.
const REPORT_KEYS: &[&str] = &[
    "local_version",
    "latest_version",
    "biz",
    "exe",
    "download_mode",
    "branch",
    "package_id",
    "build_id",
    "audio_langs",
    "diff_tags",
];

/// The keys `PLAN` always carries, in order. [`PLAN_SCAN_BYTES_KEY`] sits
/// between `est_extra_bytes` and `purge_files` only when `--scan` asked for it.
const PLAN_KEYS: &[&str] = &[
    "files",
    "est_download_bytes",
    "est_write_bytes",
    "est_extra_bytes",
    "purge_files",
    "purge_bytes",
    "max_chunk_bytes",
    "scoped",
];

/// The conditional `PLAN` key: present only on a `--scan` run, between
/// `est_extra_bytes` and `purge_files`.
const PLAN_SCAN_BYTES_KEY: &str = "scan_bytes";

/// The keys `PROGRESS` carries, in order. A short run may emit none.
const PROGRESS_KEYS: &[&str] = &[
    "done",
    "total",
    "skipped",
    "repaired",
    "failed",
    "download_bytes",
];

/// The keys an unflagged `SUMMARY` carries, in order. A scoped or limited run
/// appends its conditional keys after these.
const SUMMARY_KEYS: &[&str] = &[
    "total",
    "skipped",
    "repaired",
    "failed",
    "download_bytes",
    "deleted_extra_bytes",
    "exit",
];

/// The conditional `SUMMARY` keys, appended after [`SUMMARY_KEYS`].
const SUMMARY_CONDITIONAL_KEYS: &[&str] = &[
    "limited",
    "scoped",
    "config_bumped",
    "plan_files",
    "plan_download_bytes",
    "download_bytes_spent",
    "write_bytes",
    "write_bytes_spent",
    "repair_mode",
    "inplace",
    "temp",
    "kept_bytes",
    "ram_bytes",
    "damaged_chunks",
    "stopped",
    "local_manifest",
];

/// One run's data plane, already reduced to what the contract is about.
struct DataPlaneRun {
    code: i32,
    /// `(event, field keys in report order)`, one entry per stdout line.
    records: Vec<(String, Vec<String>)>,
    stdout: String,
    stderr: String,
}

/// Run the binary in JSON mode against the mock server, and index its records.
async fn run_data_plane(game: &std::path::Path, mock: &MockServer, extra: &[&str]) -> DataPlaneRun {
    run_data_plane_as(game, mock, extra, "json").await
}

/// Run the binary in text mode against the mock server, and index its records.
///
/// The text and JSON planes render the same [`girpr::Record`], so the caller
/// can check both against the same golden. This reports just the event and the
/// field keys; a text line's values are not part of the contract.
async fn run_text_plane(game: &std::path::Path, mock: &MockServer, extra: &[&str]) -> DataPlaneRun {
    run_data_plane_as(game, mock, extra, "text").await
}

async fn run_data_plane_as(
    game: &std::path::Path,
    mock: &MockServer,
    extra: &[&str],
    output: &str,
) -> DataPlaneRun {
    let mut args: Vec<String> = vec![
        "--game-path".into(),
        game.display().to_string(),
        "--biz".into(),
        "hk4e_global".into(),
        "--hyp-base".into(),
        mock.hyp_base.clone(),
        "--sophon-base".into(),
        mock.sophon_base.clone(),
        "--jobs".into(),
        "1".into(),
        "--output".into(),
        output.into(),
        // Keep the trace log out of the build tree; the log is not the plane
        // under test.
        "--no-log-file".into(),
    ];
    args.extend(extra.iter().map(|arg| (*arg).to_string()));

    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_girpr"))
        .args(&args)
        .output()
        .await
        .expect("run girpr");
    remove_run_lock_for_args(&args);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let records = match output {
        "json" => json_records(&stdout),
        "text" => text_records(&stdout),
        other => panic!("unknown output plane {other}"),
    };
    DataPlaneRun {
        code: out.status.code().expect("process must exit normally"),
        records,
        stdout,
        stderr,
    }
}

fn json_records(stdout: &str) -> Vec<(String, Vec<String>)> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("data-plane line is not JSON ({e}): {line}"));
            let event = value["event"]
                .as_str()
                .unwrap_or_else(|| panic!("no event name in: {line}"))
                .to_string();
            let mut keys = json_object_keys(line);
            assert_eq!(keys.first().map(String::as_str), Some("event"), "{line}");
            keys.remove(0);
            (event, keys)
        })
        .collect()
}

/// Index `EVENT key=value …` lines, keeping the keys in the line's order.
///
/// Parsing the text plane rather than matching substrings is what pins its
/// shape: every field must carry its key, in report order, and a line must
/// still begin with its event label. The values are parsed too, so a field's
/// end is found from the grammar rather than from token shape; this projection
/// keeps only the keys the golden compares.
fn text_records(stdout: &str) -> Vec<(String, Vec<String>)> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (event, fields) = text_record(line);
            let keys = fields.into_iter().map(|(key, _)| key).collect();
            (event, keys)
        })
        .collect()
}

/// Parse one `EVENT key=value …` line into its event label and fields.
///
/// The grammar is the one [`girpr::Record::text`] writes: an uppercase label,
/// then zero or more `key=value` fields separated by one space. A value is bare
/// — the renderer never quotes or escapes it — so it may contain spaces and
/// `=`. The only reliable boundary is therefore a key the event actually
/// carries: `exe=Some word=.exe` keeps `word=.` in `exe` because `word` is not
/// a `REPORT` field, and a value's spaces stay part of the value. Recognising a
/// key by its identifier shape alone would split that value.
///
/// **Known grammar limit.** When a value itself contains a space followed by
/// one of its event's own keys and `=`, it is indistinguishable from the next
/// field and is read as one: `REPORT exe=Some branch=main.exe` yields
/// `exe=Some` and `branch=main.exe`, not a single `exe` value. The values are
/// API-supplied strings and the text plane has no quoting or escaping to
/// resolve that, so the limit is pinned by a test rather than assumed away;
/// the JSON plane is the unambiguous alternative.
///
/// The first field must follow the label directly. A token there is not a
/// continuation (there is no previous value to continue), so it is rejected.
fn text_record(line: &str) -> (String, Vec<(String, String)>) {
    let (label, rest) = line.split_once(' ').unwrap_or((line, ""));
    assert!(
        label.chars().all(|c| c.is_ascii_uppercase() || c == '_'),
        "text record must start with its event label: {line}"
    );
    let event = label.to_ascii_lowercase();
    assert!(
        is_record_event(&event),
        "text record has an unknown event label: {line}"
    );
    let mut fields = Vec::new();
    if rest.is_empty() {
        return (event, fields);
    }
    let mut separator = label.len();
    loop {
        let (key, value_start) = text_field_at(line, separator, &event)
            .unwrap_or_else(|| panic!("text record has a token before its first field: {line}"));
        match next_text_field(line, value_start, &event) {
            Some(next) => {
                fields.push((key.to_string(), line[value_start..next].to_string()));
                separator = next;
            }
            None => {
                fields.push((key.to_string(), line[value_start..].to_string()));
                break;
            }
        }
    }
    (event, fields)
}

/// The key and the index just past its `=` for a field whose separating space
/// is at `separator`, when the key is one the event carries.
fn text_field_at<'a>(line: &'a str, separator: usize, event: &str) -> Option<(&'a str, usize)> {
    if line.as_bytes().get(separator) != Some(&b' ') {
        return None;
    }
    let key_start = separator + 1;
    let rest = line.get(key_start..)?;
    let key_len = rest.find(['=', ' '])?;
    let key = &rest[..key_len];
    if !is_record_key(event, key) {
        return None;
    }
    let equals = key_start + key_len;
    (line.as_bytes().get(equals) == Some(&b'=')).then_some((key, equals + 1))
}

/// The index of the next known-field separator at or after `from`.
fn next_text_field(line: &str, from: usize, event: &str) -> Option<usize> {
    line.match_indices(' ')
        .map(|(index, _)| index)
        .find(|&index| index >= from && text_field_at(line, index, event).is_some())
}

/// Whether `event` is one of the labels the data plane emits.
fn is_record_event(event: &str) -> bool {
    matches!(event, "report" | "plan" | "progress" | "summary")
}

/// Whether `key` is a field of `event`, including its conditional tail.
fn is_record_key(event: &str, key: &str) -> bool {
    match event {
        "report" => REPORT_KEYS.contains(&key),
        "plan" => PLAN_KEYS.contains(&key) || key == PLAN_SCAN_BYTES_KEY,
        "progress" => PROGRESS_KEYS.contains(&key),
        "summary" => SUMMARY_KEYS.contains(&key) || SUMMARY_CONDITIONAL_KEYS.contains(&key),
        _ => false,
    }
}

/// A value with a space is one field, not two: `exe=Genshin Impact.exe` must
/// contribute `exe` and rejoin to exactly that value, not just that key.
#[test]
fn text_records_keeps_a_key_whose_value_contains_a_space() {
    let (event, fields) =
        text_record("REPORT local_version=none exe=Genshin Impact.exe branch=main");
    assert_eq!(event, "report");
    let fields: Vec<(&str, &str)> = fields
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        fields,
        vec![
            ("local_version", "none"),
            ("exe", "Genshin Impact.exe"),
            ("branch", "main"),
        ]
    );
}

/// A key-shaped word inside a value is value text, not a new field: `word=.`
/// is not a `REPORT` key, so `exe` keeps it and its spaces.
#[test]
fn text_records_keeps_a_key_shaped_word_inside_a_value() {
    let (event, fields) = text_record("REPORT local_version=none exe=Some word=.exe branch=main");
    assert_eq!(event, "report");
    let fields: Vec<(&str, &str)> = fields
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        fields,
        vec![
            ("local_version", "none"),
            ("exe", "Some word=.exe"),
            ("branch", "main"),
        ]
    );
}

/// A value carrying one of its event's own key tokens is the known limit of the
/// bare text grammar: it is read as the next field. The renderer neither quotes
/// nor escapes values, so the text plane cannot tell the two apart; the JSON
/// plane is the unambiguous alternative. Pinning this keeps a parser change
/// from silently reinterpreting the same line.
#[test]
fn text_records_reads_an_embedded_key_token_as_the_next_field() {
    let (event, fields) = text_record("REPORT local_version=none exe=Some branch=main.exe");
    assert_eq!(event, "report");
    let fields: Vec<(&str, &str)> = fields
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        fields,
        vec![
            ("local_version", "none"),
            ("exe", "Some"),
            ("branch", "main.exe"),
        ],
        "a value containing `branch=` is split at that token, not kept whole"
    );
}

/// A token cannot precede the first field: there is no previous value for it to
/// continue, so the record is rejected instead of silently gaining a field.
#[test]
#[should_panic(expected = "text record has a token before its first field")]
fn text_records_rejects_a_token_before_the_first_field() {
    let _ = text_records("REPORT junk total=0\n");
}

/// Check a run against a golden sequence of `(event, field keys)`.
///
/// `PROGRESS` is the only repeatable record. It may appear zero or more times,
/// but only in the documented window before the terminal `SUMMARY`: after
/// every fixed event except the last one. A run with `--print-plan` therefore
/// has it between `PLAN` and `SUMMARY`, and a run without one has it between
/// `REPORT` and `SUMMARY`. Every fixed event appears once, in the golden's
/// order, with the golden's key order. A second `PLAN`, a second `SUMMARY`, a
/// `SUMMARY` before `PLAN`, or a `PROGRESS` before `PLAN` is a failure.
fn check_golden(run: &DataPlaneRun, golden: &[(&str, &[&str])]) -> Result<(), String> {
    let mut at = 0usize;
    for (event, keys) in &run.records {
        let keys = key_list(keys);
        if event == "progress" {
            // The only window is the one immediately before the final
            // `SUMMARY`; every earlier fixed record must already be behind us.
            let in_window = at + 1 == golden.len()
                && golden.get(at).is_some_and(|(event, _)| *event == "summary");
            if !in_window {
                return Err(format!(
                    "PROGRESS outside the window before the final SUMMARY:\n{}",
                    run.stdout
                ));
            }
            if keys != PROGRESS_KEYS {
                return Err(format!("PROGRESS field keys changed:\n{}", run.stdout));
            }
            continue;
        }
        if at == golden.len() {
            return Err(format!(
                "more than one {event} record, or a record after the final one:\n{}",
                run.stdout
            ));
        }
        let (want_event, want_keys) = golden[at];
        if event != want_event {
            return Err(format!(
                "event order: expected {want_event} at position {at}, got {event}:\n{}",
                run.stdout
            ));
        }
        if keys != want_keys {
            return Err(format!(
                "{event} field keys or their order changed:\n{}",
                run.stdout
            ));
        }
        at += 1;
    }
    if at != golden.len() {
        return Err(format!(
            "missing records: the run ended after {at} of {}:\n{}",
            golden.len(),
            run.stdout
        ));
    }
    Ok(())
}

fn assert_golden(run: &DataPlaneRun, golden: &[(&str, &[&str])]) {
    if let Err(message) = check_golden(run, golden) {
        panic!("{message}");
    }
}

fn key_list(keys: &[String]) -> Vec<&str> {
    keys.iter().map(String::as_str).collect()
}

/// The top-level keys of a flat JSON object, in the line's own order.
///
/// `serde_json::Map` is a `BTreeMap` unless the workspace opts into
/// `preserve_order`, and a parsed `Value` therefore cannot answer "in what
/// order were these keys written". The records are flat objects of primitives
/// (`Record` has no nested field), so a small scanner is enough — and the line
/// has already been validated as JSON by the caller.
fn json_object_keys(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut i = skip_whitespace(bytes, 0);
    assert_eq!(bytes.get(i), Some(&b'{'), "not a JSON object: {line}");
    i += 1;
    let mut keys = Vec::new();
    loop {
        i = skip_whitespace(bytes, i);
        match bytes.get(i) {
            Some(b'}') => break,
            Some(b',') => i += 1,
            Some(b'"') => {
                let (key, next) = json_string(bytes, i);
                keys.push(key);
                i = skip_whitespace(bytes, next);
                assert_eq!(bytes.get(i), Some(&b':'), "a key without a value: {line}");
                i = skip_json_value(bytes, i + 1);
            }
            _ => panic!("expected a key or the object's end in: {line}"),
        }
    }
    keys
}

/// Read one JSON string starting at its opening quote, returning it and the
/// index just past its closing quote. Keys and language tags are static ASCII,
/// so no `\u` escape can arise here; the two that can are handled.
fn json_string(bytes: &[u8], start: usize) -> (String, usize) {
    let mut out = Vec::new();
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return (String::from_utf8(out).expect("a UTF-8 key"), i + 1),
            b'\\' => {
                i += 1;
                if let Some(escaped) = bytes.get(i) {
                    out.push(*escaped);
                }
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    panic!("unterminated JSON string")
}

/// Skip one JSON value and return the index of the comma or closing brace that
/// follows it. Values are primitives; nested containers are skipped for safety.
fn skip_json_value(bytes: &[u8], start: usize) -> usize {
    let mut i = skip_whitespace(bytes, start);
    let mut depth = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i = json_string(bytes, i).1;
                continue;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' if depth > 0 => depth -= 1,
            b'}' | b']' | b',' if depth == 0 => return i,
            _ => {}
        }
        i += 1;
    }
    i
}

fn skip_whitespace(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// A damaged copy of the two-file fixture: one clean file, one corrupt file.
fn seed_one_damaged_one_clean(dir: &std::path::Path) {
    let fx = build_fixture();
    // `bar.dat` is intact, so the plan is a skip; `foo.dat` is the same length
    // with a flipped tail, so its digest fails and the repair rebuilds it.
    write_game_file(dir, &fx.files[1].rel, &fx.files[1].data);
    let mut damaged = fx.files[0].data.clone();
    for byte in damaged.iter_mut().skip(64) {
        *byte ^= 0xFF;
    }
    write_game_file(dir, &fx.files[0].rel, &damaged);
}

/// A tiny repair pins the order and the one-record-per-event rule: `REPORT`
/// once, then (if the run is long enough) any `PROGRESS` ticks, then exactly
/// one `SUMMARY` last.
#[tokio::test]
async fn a_tiny_repair_emits_report_then_exactly_one_summary() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("golden_repair");
    // One clean file and one damaged file: the repair skips one and rebuilds
    // the other, so the run is short and its `SUMMARY` is the base shape.
    seed_one_damaged_one_clean(&dir);

    let run = run_data_plane(&dir, &mock, &[]).await;
    assert_eq!(run.code, 0, "a tiny repair exits 0:\n{}", run.stderr);
    assert_golden(&run, &[("report", REPORT_KEYS), ("summary", SUMMARY_KEYS)]);
    mock.stop();
}

/// `--print-plan` adds exactly one `PLAN` between `REPORT` and the run's single
/// `SUMMARY`, and writes nothing.
#[tokio::test]
async fn print_plan_emits_one_plan_between_report_and_summary() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("golden_print_plan");
    seed_one_damaged_one_clean(&dir);

    let run = run_data_plane(&dir, &mock, &["--print-plan"]).await;
    assert_eq!(run.code, 0, "--print-plan exits 0:\n{}", run.stderr);
    assert_golden(
        &run,
        &[
            ("report", REPORT_KEYS),
            ("plan", PLAN_KEYS),
            ("summary", SUMMARY_KEYS),
        ],
    );
    mock.stop();
}

/// `--check-only` is `REPORT` and the single terminal `SUMMARY`, with no
/// `PLAN`; a damaged install is exit `4`.
#[tokio::test]
async fn check_only_emits_report_then_exactly_one_summary() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("golden_check_only");
    seed_one_damaged_one_clean(&dir);

    let run = run_data_plane(&dir, &mock, &["--check-only"]).await;
    assert_eq!(run.code, 4, "a damaged install is exit 4:\n{}", run.stderr);
    assert_golden(&run, &[("report", REPORT_KEYS), ("summary", SUMMARY_KEYS)]);
    mock.stop();
}

/// Exactly one `SUMMARY` record is emitted by a real binary run, and it is
/// the terminal record.
fn assert_one_terminal_summary(run: &DataPlaneRun) {
    let summaries = run
        .records
        .iter()
        .filter(|(event, _)| event == "summary")
        .count();
    assert_eq!(
        summaries, 1,
        "the run must emit exactly one SUMMARY:\n{}",
        run.stdout
    );
    assert_eq!(
        run.records.last().map(|(event, _)| event.as_str()),
        Some("summary"),
        "SUMMARY must be the final record:\n{}",
        run.stdout
    );
}

/// The text plane is the same records as the JSON plane, rendered as
/// `EVENT key=value …`: same events, same field order, exactly one terminal
/// `SUMMARY`. The keys come from the shared constants above rather than a
/// second copy of the expectation.
#[tokio::test]
async fn a_tiny_repair_pins_the_text_plane_with_one_terminal_summary() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("text_plane_repair");
    seed_one_damaged_one_clean(&dir);

    let run = run_text_plane(&dir, &mock, &[]).await;
    assert_eq!(run.code, 0, "a tiny repair exits 0:\n{}", run.stderr);
    assert_one_terminal_summary(&run);
    assert_golden(&run, &[("report", REPORT_KEYS), ("summary", SUMMARY_KEYS)]);
    mock.stop();
}

/// The text plane's `PLAN` sits between `REPORT` and the single terminal
/// `SUMMARY`, with the same field order the JSON plane uses.
#[tokio::test]
async fn the_text_plane_pins_plan_between_report_and_summary() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("text_plane_print_plan");
    seed_one_damaged_one_clean(&dir);

    let run = run_text_plane(&dir, &mock, &["--print-plan"]).await;
    assert_eq!(run.code, 0, "--print-plan exits 0:\n{}", run.stderr);
    assert_one_terminal_summary(&run);
    assert_golden(
        &run,
        &[
            ("report", REPORT_KEYS),
            ("plan", PLAN_KEYS),
            ("summary", SUMMARY_KEYS),
        ],
    );
    mock.stop();
}

/// `--check-only` on the text plane is the same `REPORT` and single terminal
/// `SUMMARY` as the JSON plane, with no `PLAN`; a damaged install is exit `4`.
#[tokio::test]
async fn the_text_plane_check_only_emits_report_then_exactly_one_summary() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("text_plane_check_only");
    seed_one_damaged_one_clean(&dir);

    let run = run_text_plane(&dir, &mock, &["--check-only"]).await;
    assert_eq!(run.code, 4, "a damaged install is exit 4:\n{}", run.stderr);
    assert_one_terminal_summary(&run);
    assert_golden(&run, &[("report", REPORT_KEYS), ("summary", SUMMARY_KEYS)]);
    mock.stop();
}

/// `--plan-out` alone writes its JSON document and adds nothing to the text
/// plane: no `PLAN` record, because the flag implies nothing and only
/// `--print-plan` asks for one. The run still executes, so the plane is the
/// ordinary `REPORT … SUMMARY` with the same terminal shape.
#[tokio::test]
async fn the_text_plane_plan_out_alone_writes_the_document_without_a_plan_record() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("text_plane_plan_out");
    seed_one_damaged_one_clean(&dir);
    let reports = temp_dir("text_plane_plan_out_doc");
    let out = reports.join("plan.json");
    let out_arg = out.to_string_lossy().into_owned();

    let run = run_text_plane(&dir, &mock, &["--plan-out", &out_arg]).await;
    assert_eq!(run.code, 0, "--plan-out alone exits 0:\n{}", run.stderr);
    assert!(
        !run.stdout.lines().any(|line| line.starts_with("PLAN ")),
        "--plan-out alone must not add a PLAN record:\n{}",
        run.stdout
    );
    assert_one_terminal_summary(&run);
    assert_golden(&run, &[("report", REPORT_KEYS), ("summary", SUMMARY_KEYS)]);

    let written = std::fs::read_to_string(&out).expect("--plan-out writes its document");
    let document: serde_json::Value = serde_json::from_str(&written).expect("valid plan JSON");
    let fixture = build_fixture();
    // The plan is the sorted, deduped work list, so the document's entries are
    // exactly the fixture's files in that order.
    let mut expected_rels: Vec<&str> = fixture.files.iter().map(|file| file.rel.as_str()).collect();
    expected_rels.sort_unstable();
    let rels: Vec<&str> = document["repair"]
        .as_array()
        .expect("the document has a repair list")
        .iter()
        .map(|entry| entry["rel"].as_str().expect("every entry names its rel"))
        .collect();
    assert_eq!(
        rels, expected_rels,
        "the document describes exactly the fixture's files, in plan order:\n{written}"
    );
    assert_eq!(
        read_game_file(&dir, &fixture.files[0].rel),
        Some(fixture.files[0].data.clone()),
        "the run still did the work it planned"
    );
    mock.stop();
}

/// A synthetic sequence, so the golden's own failure modes are testable
/// without a run.
fn synthetic(records: &[(&str, &[&str])]) -> DataPlaneRun {
    DataPlaneRun {
        code: 0,
        records: records
            .iter()
            .map(|(event, keys)| {
                (
                    (*event).to_string(),
                    keys.iter().map(|key| (*key).to_string()).collect(),
                )
            })
            .collect(),
        stdout: "synthetic records".to_string(),
        stderr: String::new(),
    }
}

/// The golden is only worth keeping if it rejects the breaking changes:
/// `SUMMARY` moving before `PLAN`, a second `SUMMARY`, a `PROGRESS` before
/// `PLAN` or after the run is over, and a renamed field key.
#[test]
fn the_golden_rejects_a_reorder_a_duplicate_a_misplaced_progress_and_a_rename() {
    let golden: &[(&str, &[&str])] = &[
        ("report", REPORT_KEYS),
        ("plan", PLAN_KEYS),
        ("summary", SUMMARY_KEYS),
    ];
    const RENAMED_EXIT: &[&str] = &[
        "total",
        "skipped",
        "repaired",
        "failed",
        "download_bytes",
        "deleted_extra_bytes",
        "exit_code",
    ];
    let cases = [
        (
            "SUMMARY before PLAN",
            synthetic(&[
                ("report", REPORT_KEYS),
                ("summary", SUMMARY_KEYS),
                ("plan", PLAN_KEYS),
            ]),
        ),
        (
            "a second SUMMARY",
            synthetic(&[
                ("report", REPORT_KEYS),
                ("plan", PLAN_KEYS),
                ("summary", SUMMARY_KEYS),
                ("summary", SUMMARY_KEYS),
            ]),
        ),
        (
            "PROGRESS before PLAN",
            synthetic(&[
                ("report", REPORT_KEYS),
                ("progress", PROGRESS_KEYS),
                ("plan", PLAN_KEYS),
                ("summary", SUMMARY_KEYS),
            ]),
        ),
        (
            "PROGRESS after SUMMARY",
            synthetic(&[
                ("report", REPORT_KEYS),
                ("plan", PLAN_KEYS),
                ("summary", SUMMARY_KEYS),
                ("progress", PROGRESS_KEYS),
            ]),
        ),
        (
            "a renamed SUMMARY key",
            synthetic(&[
                ("report", REPORT_KEYS),
                ("plan", PLAN_KEYS),
                ("summary", RENAMED_EXIT),
            ]),
        ),
    ];
    for (what, run) in cases {
        assert!(
            check_golden(&run, golden).is_err(),
            "the golden must reject {what}"
        );
    }
    assert!(
        check_golden(&synthetic(golden), golden).is_ok(),
        "the matching sequence must be accepted"
    );
    // `PROGRESS` is legitimate in the window before the terminal `SUMMARY`,
    // with or without a `PLAN` ahead of it.
    let with_plan = synthetic(&[
        ("report", REPORT_KEYS),
        ("plan", PLAN_KEYS),
        ("progress", PROGRESS_KEYS),
        ("progress", PROGRESS_KEYS),
        ("summary", SUMMARY_KEYS),
    ]);
    assert!(
        check_golden(&with_plan, golden).is_ok(),
        "PROGRESS between PLAN and SUMMARY is the documented window"
    );
    let without_plan: &[(&str, &[&str])] = &[("report", REPORT_KEYS), ("summary", SUMMARY_KEYS)];
    let no_plan = synthetic(&[
        ("report", REPORT_KEYS),
        ("progress", PROGRESS_KEYS),
        ("summary", SUMMARY_KEYS),
    ]);
    assert!(
        check_golden(&no_plan, without_plan).is_ok(),
        "a run without PLAN has its PROGRESS window between REPORT and SUMMARY"
    );
}
