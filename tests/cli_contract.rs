//! Binary-level contract tests: argument handling, exit codes, and the stdout
//! data plane.
//!
//! These run the real binary (`CARGO_BIN_EXE_girpr`) because the contract being
//! checked is process-level: which exit code a given kind of bad input
//! produces, and that clap's own `2` never leaks into the "2 = metadata/network"
//! slot (docs/02 §1).
//!
//! The rendered `--help` is a trycmd snapshot under `tests/cmd`; flag presence
//! and descriptions are checked through clap's own introspection, never by
//! parsing its layout by hand. Bad arguments are classified in process through
//! [`usage_rejection`]. A small set of real binary cases remains to pin the
//! process-level contract: a bad flag exits 1, `--help` exits 0, and post-parse
//! usage errors carry their exact stderr text.
//!
//! The log-announcement cases resolve a plan and render the announcement in
//! process; only one binary smoke reads the same bytes back out of a real log
//! file. Every binary case that reaches `logging::init_tracing` names a scratch
//! `--log-file`, so no default log can land beside the binary in `target/`.
//!
//! The data-plane goldens at the end are the other kind of case: they need the
//! mock HYP/Sophon server in `tests/common`, because `REPORT`/`PLAN` are
//! only emitted once the metadata has been fetched. They pin the *sequence* of
//! records and each record's field keys, never a value, so a wording change
//! that keeps the contract passes and a reordered or duplicated record fails.
//!
//! NOTE: `--include` matching zero *planned* files is also exit `1`, and a
//! tripped ceiling is exit `5`, but the ceiling cases are exercised in
//! `tests/repair_limits.rs`; this file's argument cases never have a server.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use assert_cmd::assert::Assert;
use clap::{CommandFactory, Parser};
use girpr::girpr_log::{self, EnvOptions, LogOptions, LogPlan};
use girpr::{Args, Level, log_file_announcement};
use girpr_tempdir::TempDir;
use predicates::prelude::*;

use crate::common::{
    MockOpts, MockServer, PLAN_SCAN_BYTES_KEY, StdoutPolicy, build_fixture, read_game_file,
    run_binary_with, strip_ansi, temp_dir, write_game_file,
};

/// Spawn the binary and hand its captured output to the caller's assertions.
///
/// Every post-parse run names a scratch log: a run that reaches
/// `init_tracing` must never leave a default log beside the binary. The
/// captured `Assert` carries stdout, stderr and the exit code, so each caller
/// chains the predicates its own contract needs.
fn run(args: &[&str]) -> Assert {
    let scratch = TempDir::new("girpr_cli", "post_parse");
    let log = scratch.join("run.log");
    assert_cmd::Command::cargo_bin("girpr")
        .expect("the girpr binary is built for integration tests")
        .args(args)
        .args([
            "--log-file",
            log.to_str().expect("the scratch path is utf-8"),
        ])
        .assert()
}

/// The message from a panic a parser raises for input it cannot read.
///
/// Catching the panic keeps the message assertable inside a table-driven test —
/// the same cases used to be separate `#[should_panic]` functions. The payload
/// downcast is the shared leaf's; this helper keeps its own strict rule that a
/// non-string payload is a test bug, not a fallback string.
fn panic_message(f: impl FnOnce() + std::panic::UnwindSafe) -> String {
    let panic = std::panic::catch_unwind(f).expect_err("the helper must panic");
    girpr_async_join::panic_text_opt(panic.as_ref()).expect("the panic carries a message")
}

/// The binary's file name, passed as argv[0] to in-process parses so their
/// diagnostics name the same program the real binary does.
fn bin_name() -> String {
    std::path::Path::new(env!("CARGO_BIN_EXE_girpr"))
        .file_name()
        .expect("the binary path has a file name")
        .to_str()
        .expect("the binary name is UTF-8")
        .to_string()
}

/// Why `args` is rejected as a usage error, computed in process.
///
/// A bad argument is refused either by clap while parsing (which the binary
/// maps to exit 1 before tracing starts) or by `Args::into_ctx`'s post-parse
/// validation (also exit 1). Both are "usage" failures; this returns the
/// explanation for either, or `None` when the arguments are actually accepted.
/// It lets the argument-classification cases run without spawning a process.
fn usage_rejection(args: &[&str]) -> Option<String> {
    let name = bin_name();
    let argv = std::iter::once(name.as_str()).chain(args.iter().copied());
    match Args::try_parse_from(argv) {
        Err(e) => Some(e.to_string()),
        Ok(a) => a.into_ctx().err().map(|e| e.to_string()),
    }
}

/// The whole rendered `--help` is a trycmd snapshot under `tests/cmd`. It pins
/// every flag's presence and description without reconstructing clap's layout:
/// a rendering change fails here and the snapshot is regenerated deliberately.
#[test]
fn help_output_matches_the_snapshot() {
    trycmd::TestCases::new()
        .case("tests/cmd/*.trycmd")
        .default_bin_path(env!("CARGO_BIN_EXE_girpr"));
}

#[test]
fn help_exits_zero() {
    // `Args` declares no `version`, so only `--help` gets the conventional 0.
    run(&["--help"])
        .code(0)
        .stdout(predicate::str::contains("Usage"));
}

/// The argument carrying `flag` (given without its leading `--`).
///
/// clap's own command model is the source of truth for flag names and their
/// descriptions. Reading it avoids pinning rendering details such as indentation
/// or the position of a short alias.
fn argument<'a>(command: &'a clap::Command, flag: &str) -> &'a clap::Arg {
    command
        .get_arguments()
        .find(|arg| arg.get_long() == Some(flag))
        .unwrap_or_else(|| panic!("no argument registered for --{flag}"))
}

/// The help text clap stores for `flag`, including the long form when present.
fn argument_help(command: &clap::Command, flag: &str) -> String {
    let arg = argument(command, flag);
    arg.get_long_help()
        .or_else(|| arg.get_help())
        .expect("the argument has help text")
        .to_string()
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
        let explanation =
            usage_rejection(&args).unwrap_or_else(|| panic!("{args:?} should be a usage error"));
        assert!(!explanation.is_empty(), "{args:?} should explain itself");
    }

    // One real spawn pins the process-level contract this test is named for:
    // a bad flag exits 1 (never clap's default 2, the metadata/network slot)
    // and explains itself on stderr. The other cases are classified in process
    // above — the same rejection, without paying for a process each.
    run(&["--game-path", ".", "--biz", "hk4e_global", "-x"])
        .code(1)
        .stderr(predicate::str::is_empty().not());
}

#[test]
fn every_documented_flag_is_registered_with_clap() {
    let command = Args::command();
    let registered: Vec<&str> = command
        .get_arguments()
        .filter_map(|arg| arg.get_long())
        .collect();
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
        "--with-persistent",
        "--scan",
    ] {
        let long = flag.strip_prefix("--").expect("a documented long flag");
        assert!(
            registered.contains(&long),
            "clap has no --{long} argument: {registered:?}"
        );
    }
}

#[test]
fn jobs_keeps_the_compatibility_alias_and_documents_the_applied_default() {
    let command = Args::command();
    let jobs = argument(&command, "jobs");
    assert!(
        jobs.get_visible_aliases()
            .is_some_and(|aliases| aliases.contains(&"io-threads")),
        "the compatibility alias stays discoverable"
    );
    // The help stored on this argument must show the default the run actually
    // applies — not a neighbour's `[default: …]` in a flat help plane.
    let applied = Args::parse_from(["girpr", "--game-path", ".", "--biz", "hk4e_global"])
        .into_ctx()
        .expect("the defaults are a valid configuration")
        .jobs;
    let help = argument_help(&command, "jobs");
    assert!(
        help.contains(&format!("[default: {applied}]")),
        "the --jobs help does not show the applied default:\n{help}"
    );
}

#[test]
fn zero_jobs_is_exit_one_and_the_message_names_the_flag() {
    run(&["--game-path", ".", "--biz", "hk4e_global", "--jobs", "0"])
        .code(1)
        .stderr(predicate::str::contains("--jobs must be >= 1"));
}

#[test]
fn an_oversized_jobs_is_exit_one_instead_of_a_runtime_failure() {
    // A value this large would ask the runtime for that many threads. It must
    // be refused while the flag is being checked — named as a usage error, not
    // surfaced later as "failed to start runtime" (exit 1 either way, but the
    // latter reads like an environment problem and does not name the flag).
    //
    // This is the one jobs-boundary case kept as a real spawn: it pins the exact
    // stderr text, including the `usage error: ` prefix that main.rs adds and
    // `into_ctx` (hence `usage_rejection`) does not, and that the runtime is
    // never built. The other boundary cases classify in process via
    // `usage_rejection`, since the ceiling text itself is unit-tested in cli.rs.
    run(&[
        "--game-path",
        ".",
        "--biz",
        "hk4e_global",
        "--jobs",
        "1000000",
    ])
    .code(1)
    // The flag must be refused before the runtime is built.
    .stderr(
        predicate::str::contains("usage error: --jobs must be <= 1024")
            .and(predicate::str::contains("failed to start runtime").not()),
    );
}

/// `--with-persistent` is a boolean, defaults off, and its own help says
/// read-only modes only list the tier.
#[test]
fn with_persistent_is_an_opt_in_flag_with_its_own_help() {
    let off = Args::parse_from(["girpr", "--game-path", ".", "--biz", "hk4e_global"])
        .into_ctx()
        .expect("the default is valid");
    assert!(!off.with_persistent);
    assert!(off.persistent_dispatch_override.is_none());
    let on = Args::parse_from([
        "girpr",
        "--game-path",
        ".",
        "--biz",
        "hk4e_global",
        "--with-persistent",
    ])
    .into_ctx()
    .expect("the flag is valid");
    assert!(on.with_persistent);
    assert!(on.persistent_dispatch_override.is_none());

    let command = Args::command();
    let help = argument_help(&command, "with-persistent");
    for needle in ["Opt-in", "Read-only modes", "--purge-only"] {
        assert!(help.contains(needle), "{needle} missing from: {help}");
    }
}

/// The flagged key vectors extend, never replace, the unflagged goldens.
#[test]
fn persistent_key_vectors_only_append_to_the_unflagged_tails() {
    assert_eq!(
        &SUMMARY_CONDITIONAL_KEYS[..16],
        &[
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
        ]
    );
    assert_eq!(
        &SUMMARY_CONDITIONAL_KEYS[16..],
        &[
            "persistent",
            "persistent_files",
            "persistent_repaired",
            "persistent_failed",
            "persistent_base_hash",
        ]
    );
    assert_eq!(
        PLAN_PERSISTENT_KEYS,
        &[
            "persistent_files",
            "persistent_bytes",
            "scan_excludes_persistent"
        ]
    );
}

#[test]
fn the_io_threads_alias_validates_at_the_same_jobs_ceiling() {
    // The ceiling is checked before `--audio`, so a bad audio value after the
    // jobs flag shows which check the run reached. At 1024 the alias passes
    // jobs validation and fails on the audio; at 1025 it fails on the jobs
    // ceiling first, with the same usage error `--jobs` produces. Both are
    // post-parse usage errors (exit 1); the ceiling text is unit-tested in
    // cli.rs and the exact stderr is pinned by the retained spawn in
    // `an_oversized_jobs_is_exit_one_instead_of_a_runtime_failure`, so classify
    // them in process here rather than spawning for each boundary value.
    let rejection = |jobs: &str| {
        usage_rejection(&[
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--io-threads",
            jobs,
            "--audio",
            "fr-fr",
        ])
        .unwrap_or_else(|| panic!("--io-threads {jobs} should be a usage error"))
    };

    let at_ceiling = rejection("1024");
    assert!(
        at_ceiling.contains("unknown audio lang"),
        "the alias accepted the ceiling and reached the next check:\n{at_ceiling}"
    );
    assert!(
        !at_ceiling.contains("--jobs must be <="),
        "the ceiling itself must not be rejected:\n{at_ceiling}"
    );

    let past_ceiling = rejection("1025");
    assert!(
        past_ceiling.contains("--jobs must be <= 1024"),
        "one past the ceiling is the same usage error through the alias:\n{past_ceiling}"
    );
}

#[test]
fn the_jobs_flag_is_validated_at_the_same_process_level_ceiling_as_the_alias() {
    // The same boundary pair as the alias case, through the primary spelling:
    // 1024 passes jobs validation and fails on the next check (a bad
    // `--audio`), while 1025 fails the jobs ceiling first. Each value runs
    // through both spellings so the pair is compared rather than only pinned.
    // These are post-parse usage errors (exit 1); the ceiling text is
    // unit-tested in cli.rs and the exact stderr is pinned by the retained
    // spawn in `an_oversized_jobs_is_exit_one_instead_of_a_runtime_failure`, so
    // classify them in process instead of spawning per value per spelling.
    let rejection = |flag: &str, jobs: &str| {
        usage_rejection(&[
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            flag,
            jobs,
            "--audio",
            "fr-fr",
        ])
        .unwrap_or_else(|| panic!("{flag} {jobs} should be a usage error"))
    };
    let cases = [
        ("1024", "unknown audio lang", false),
        ("1025", "--jobs must be <= 1024", true),
    ];
    for (value, expected, ceiling_reported) in cases {
        let jobs_rejection = rejection("--jobs", value);
        let alias_rejection = rejection("--io-threads", value);
        assert_eq!(
            jobs_rejection, alias_rejection,
            "--jobs and --io-threads disagree at {value}"
        );
        assert!(
            jobs_rejection.contains(expected),
            "--jobs {value} stopped at the wrong check:\n{jobs_rejection}"
        );
        assert!(
            alias_rejection.contains(expected),
            "--io-threads {value} stopped at the wrong check:\n{alias_rejection}"
        );
        assert_eq!(
            jobs_rejection.contains("--jobs must be <="),
            ceiling_reported,
            "--jobs {value} reported the ceiling at the wrong step:\n{jobs_rejection}"
        );
    }
}

#[test]
fn plan_out_help_documents_every_serialized_field() {
    // The help is the operator-facing description of the document, so the
    // argument's own help must name the whole shape the binary writes. A flag
    // list alone cannot catch a stale JSON shape.
    let command = Args::command();
    let help = argument_help(&command, "plan-out");
    assert!(
        help.contains("mode,extra_bytes"),
        "the --plan-out help omits the serialized fields:\n{help}"
    );
}

/// The base arguments that reach a decided exit code without a server.
const LOG_BASE_ARGS: &[&str] = &["--game-path", ".", "--biz", "hk4e_global", "--jobs", "0"];

/// Resolve a plan from explicit inputs and return the echo line the binary
/// would print. Both halves are pure: no subscriber, environment read or
/// process spawn is involved.
fn announced(opts: LogOptions, env: EnvOptions) -> (LogPlan, Option<String>) {
    let plan = LogPlan::resolve("girpr", opts, env);
    let echo = log_file_announcement(&plan);
    (plan, echo)
}

/// The announcement the plan puts into the log, rendered in process.
///
/// The macro is the log contract's single source; capturing its event here
/// pins the bytes without needing a process. The process smoke reads the same
/// line back out of a real log file.
fn announced_event(plan: &LogPlan) -> String {
    girpr_log::test_support::logs_of(|| girpr_log::announce_log_file!(plan, "girpr::logging"))
}

/// An unopenable log file is a warning and does not move the run's decision.
///
/// The warning bytes are pinned by logging's own in-process test. What is
/// pinned here is that naming a file that cannot be opened leaves the usage
/// decision exactly where it was; no process spawn is needed to show that.
#[test]
fn an_unopenable_log_file_warns_and_changes_nothing_about_the_exit_code() {
    let base = usage_rejection(LOG_BASE_ARGS).expect("the base run is rejected");
    for path in ["not_a_dir/run.log", "another_dir/run.log"] {
        let mut args = LOG_BASE_ARGS.to_vec();
        args.extend_from_slice(&["--log-file", path]);
        assert_eq!(
            usage_rejection(&args).as_deref(),
            Some(base.as_str()),
            "{path}: the log file must not move the exit code"
        );
    }
}

/// A **debug** build writes no log file; a **release** build writes one.
///
/// The policy is the shared crate's, so what is pinned here is that the plan
/// follows it and the event says which of the two it is doing. A debug build
/// brings no `logs/` directory into existence because no file is opened at all.
#[test]
fn the_default_log_file_follows_the_build_it_was_built_from() {
    let (plan, echo) = announced(LogOptions::default(), EnvOptions::default());
    let emitted = announced_event(&plan);
    assert_eq!(echo, None, "the default is never echoed on raw stderr");

    if girpr_log::default_file_is_written() {
        let path = plan.file_path().expect("a release build has a file");
        assert!(
            path.to_string_lossy().contains("girpr_"),
            "and it is the timestamped default: {path:?}"
        );
        assert!(!plan.file_named, "the default is not named");
        assert!(
            emitted.contains("log file: "),
            "a release build says where the file went:\n{emitted}"
        );
    } else {
        assert!(
            plan.file_path().is_none(),
            "a debug build writes no file unless it is asked to: {plan:?}"
        );
        assert!(!plan.file_named);
        assert!(
            emitted.contains("log file: none"),
            "a debug build says it wrote no file:\n{emitted}"
        );
    }
}

/// An explicitly named log file — by flag or by environment — is announced
/// and resolved, with the flag winning when both name one, and the
/// announcement survives a quiet console. Creation and the file's own records
/// are pinned by the process smoke below.
#[test]
fn an_explicit_log_file_is_echoed_and_created() {
    let scratch = TempDir::new("girpr_cli_logfile", "explicit");
    let cases = [
        // (name, flag names it, environment names it, `--log-level warn`)
        ("the flag names the file", true, false, false),
        ("the environment names the file", false, true, false),
        ("the flag beats the environment", true, true, false),
        (
            "the path is echoed even at --log-level warn",
            true,
            false,
            true,
        ),
    ];
    for (i, (name, use_flag, use_env, warn_console)) in cases.iter().enumerate() {
        // Nested paths so every case still pins that missing parents are part
        // of the path, not a separate decision.
        let flag_path = scratch.join(format!("f{i}")).join("nested").join("run.log");
        let env_path = scratch.join(format!("e{i}")).join("nested").join("run.log");
        let expected = if *use_flag { &flag_path } else { &env_path };

        let (plan, echo) = announced(
            LogOptions {
                console_level: if *warn_console {
                    Level::Warn
                } else {
                    Level::Info
                },
                log_file: if *use_flag {
                    Some(flag_path.clone())
                } else {
                    None
                },
                ..LogOptions::default()
            },
            EnvOptions {
                log_file: if *use_env {
                    Some(env_path.clone())
                } else {
                    None
                },
                ..EnvOptions::default()
            },
        );
        assert_eq!(plan.file_path(), Some(expected.as_path()), "{name}");
        assert_eq!(
            echo.as_deref(),
            Some(format!("log file: {}", expected.display()).as_str()),
            "{name}: the path must be echoed on stderr"
        );
        let emitted = announced_event(&plan);
        assert!(
            emitted.contains(&format!("log file: {}", expected.display())),
            "{name}: the file carries the announcement:\n{emitted}"
        );
        if *use_flag && *use_env {
            assert_ne!(
                plan.file_path(),
                Some(env_path.as_path()),
                "the flag's file wins over the environment's"
            );
        }
    }
}

/// A bare `--log-file <name>` is accepted: the file lands in the working
/// directory and its spelling is echoed unchanged. `Path::parent` of a bare
/// name is `Some("")`, so there is no directory to create.
#[test]
fn a_bare_log_file_name_is_created_in_the_working_directory() {
    let name = "girpr-bare-log-file.log";
    let (plan, echo) = announced(
        LogOptions {
            log_file: Some(std::path::PathBuf::from(name)),
            ..LogOptions::default()
        },
        EnvOptions::default(),
    );
    assert_eq!(plan.file_path(), Some(std::path::Path::new(name)));
    assert_eq!(
        echo.as_deref(),
        Some(format!("log file: {name}").as_str()),
        "the bare name is echoed on stderr"
    );
}

/// `--no-log-file` is both an off-switch that beats the environment and, with
/// `--log-file`, a conflict clap refuses. The shared crate pins the
/// precedence; these are the process-level refusals, both in process.
#[test]
fn no_log_file_is_honoured_and_conflicts_with_log_file() {
    let scratch = TempDir::new("girpr_cli_logfile", "no_log_file");
    let log = scratch.join("case.log");
    let cases = [
        // (name, `--no-log-file`, `--log-file` too)
        ("--no-log-file beats the environment", true, false),
        ("--no-log-file with --log-file is a usage error", true, true),
    ];
    for (name, no_flag, log_flag) in &cases {
        if *log_flag {
            let mut args = LOG_BASE_ARGS.to_vec();
            args.extend_from_slice(&["--no-log-file", "--log-file", log.to_str().unwrap()]);
            let rejection = usage_rejection(&args).expect("the conflict is a usage error");
            assert!(
                rejection.contains("cannot be used with"),
                "{name}: the conflict is reported:\n{rejection}"
            );
        } else {
            let (plan, echo) = announced(
                LogOptions {
                    no_log_file: *no_flag,
                    ..LogOptions::default()
                },
                EnvOptions {
                    log_file: Some(log.clone()),
                    ..EnvOptions::default()
                },
            );
            assert!(
                plan.file_path().is_none(),
                "{name}: no log file was written"
            );
            assert_eq!(echo, None, "{name}: the path was not announced");
        }
    }
}

/// One case per way the file and console levels are chosen: the shared crate
/// owns the precedence and the defaults; these assert the plan applies them
/// to the two sinks' filters. The process smoke checks the rendered sinks.
#[test]
fn log_levels_and_rust_log_are_applied_to_the_two_sinks() {
    let scratch = TempDir::new("girpr_cli_logfile", "levels");
    let log = scratch.join("level.log");
    struct Case {
        name: &'static str,
        console_level: Option<Level>,
        file_level_flag: Option<Level>,
        file_level_env: Option<Level>,
        rust_log: Option<&'static str>,
        use_log_file: bool,
        console_forbid: &'static [&'static str],
        console_require: &'static [&'static str],
        file_require: &'static [&'static str],
    }
    let cases = [
        Case {
            name: "a quiet console keeps the file's info record",
            console_level: Some(Level::Error),
            file_level_flag: None,
            file_level_env: None,
            rust_log: None,
            use_log_file: true,
            console_forbid: &["girpr=info"],
            console_require: &["girpr=error"],
            file_require: &["debug"],
        },
        Case {
            name: "the environment sets the file level",
            console_level: None,
            file_level_flag: None,
            file_level_env: Some(Level::Error),
            rust_log: None,
            use_log_file: true,
            console_forbid: &[],
            console_require: &[],
            file_require: &["error"],
        },
        Case {
            name: "the file-level flag beats the environment",
            console_level: None,
            file_level_flag: Some(Level::Debug),
            file_level_env: Some(Level::Error),
            rust_log: None,
            use_log_file: true,
            console_forbid: &[],
            console_require: &[],
            file_require: &["debug"],
        },
        Case {
            name: "RUST_LOG adds to the console but cannot silence it",
            console_level: None,
            file_level_flag: None,
            file_level_env: None,
            rust_log: Some("reqwest=debug"),
            use_log_file: false,
            console_forbid: &[],
            console_require: &["reqwest=debug,girpr=info"],
            file_require: &[],
        },
    ];
    for case in cases {
        let (plan, _) = announced(
            LogOptions {
                console_level: case.console_level.unwrap_or_default(),
                file_level: case.file_level_flag,
                log_file: if case.use_log_file {
                    Some(log.clone())
                } else {
                    None
                },
                ..LogOptions::default()
            },
            EnvOptions {
                file_level: case.file_level_env,
                rust_log: case.rust_log.map(str::to_string),
                ..EnvOptions::default()
            },
        );
        let console = plan.console_filter().to_string();
        for needle in case.console_forbid {
            assert!(
                !console.contains(needle),
                "{}: console kept {needle}:\n{console}",
                case.name
            );
        }
        for needle in case.console_require {
            assert!(
                console.contains(needle),
                "{}: console dropped {needle}:\n{console}",
                case.name
            );
        }
        if case.use_log_file {
            let file = plan.file_filter();
            for needle in case.file_require {
                assert!(
                    file.contains(needle),
                    "{}: file dropped {needle}:\n{file}",
                    case.name
                );
            }
        }
    }
}

/// The one process-level log case: the announcement is a log artifact, so the
/// bytes on stderr and in the file must come from the real binary. A named
/// file in a scratch directory keeps the run out of the build tree.
#[test]
fn the_log_file_announcement_reaches_the_process() {
    let scratch = TempDir::new("girpr_cli_logfile", "process_smoke");
    let name = "girpr-process-smoke.log";
    let assert = assert_cmd::Command::cargo_bin("girpr")
        .expect("the girpr binary is built for integration tests")
        .args(LOG_BASE_ARGS)
        .args(["--log-file", name, "--log-level", "error"])
        .current_dir(&scratch)
        .assert()
        .code(1)
        .stderr(predicate::str::contains(format!("log file: {name}")));
    // The level and target are coloured separately, with the escape between
    // them, so the substring check needs the colour taken out first.
    let console = strip_ansi(&String::from_utf8_lossy(&assert.get_output().stderr));
    assert!(
        !console.contains("INFO girpr::logging"),
        "a quiet console keeps the info record in the file only:\n{console}"
    );
    let body = std::fs::read_to_string(scratch.join(name)).expect("the log file is created");
    assert!(
        body.contains(&format!("INFO girpr::logging: log file: {name}")),
        "the file carries the announcement:\n{body}"
    );
    assert!(
        body.contains("usage error: --jobs must be >= 1"),
        "the file holds the run's own records:\n{body}"
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

/// The flag-only `PLAN` keys, appended after [`PLAN_KEYS`]. `scan_bytes` keeps
/// its own conditional slot before `purge_files`, so the persistent tail is
/// still the last three keys of a flagged line.
const PLAN_PERSISTENT_KEYS: &[&str] = &[
    "persistent_files",
    "persistent_bytes",
    "scan_excludes_persistent",
];

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
    // `--with-persistent` appends these next to the last pre-existing key.
    "persistent",
    "persistent_files",
    "persistent_repaired",
    "persistent_failed",
    "persistent_base_hash",
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

    let (code, stdout, stderr) = run_binary_with(&args, &[], StdoutPolicy::Captured).await;
    let records = match output {
        "json" => json_records(&stdout),
        "text" => text_records(&stdout),
        other => panic!("unknown output plane {other}"),
    };
    DataPlaneRun {
        code,
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
            // `serde_json::Map` is a `BTreeMap` unless the workspace opts into
            // `preserve_order`, so write order must be read off the bytes.
            let mut keys = girpr_report_core::test_support::json_keys_in_order(line);
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

/// One text-parser case: the line, its expected `(key, value)` pairs, and why.
type TextRecordCase<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a str);

/// The text-plane parser's edge cases, in one table.
#[test]
fn text_record_parser_cases() {
    let cases: &[TextRecordCase<'_>] = &[
        (
            // A value with a space is one field, not two: `Genshin Impact.exe`
            // must stay whole instead of contributing only its key.
            "REPORT local_version=none exe=Genshin Impact.exe branch=main",
            &[
                ("local_version", "none"),
                ("exe", "Genshin Impact.exe"),
                ("branch", "main"),
            ],
            "a value with a space stays one field",
        ),
        (
            // `word=.` is not a REPORT key, so a key-shaped word inside a
            // value is value text, not a new field.
            "REPORT local_version=none exe=Some word=.exe branch=main",
            &[
                ("local_version", "none"),
                ("exe", "Some word=.exe"),
                ("branch", "main"),
            ],
            "a key-shaped word inside a value is value text",
        ),
        (
            // `branch=` *is* one of the event's own keys, so this is the known
            // limit of the bare grammar: the value is split at that token.
            // Pinning it keeps a parser change from silently reinterpreting
            // the same line; the JSON plane is the unambiguous alternative.
            "REPORT local_version=none exe=Some branch=main.exe",
            &[
                ("local_version", "none"),
                ("exe", "Some"),
                ("branch", "main.exe"),
            ],
            "a value containing `branch=` is split at that token, not kept whole",
        ),
    ];
    for (line, expected, note) in cases {
        let (event, fields) = text_record(line);
        assert_eq!(event, "report");
        let fields: Vec<(&str, &str)> = fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        assert_eq!(fields.as_slice(), *expected, "{note}");
    }

    // A token cannot precede the first field: there is no previous value for
    // it to continue, so the record is rejected instead of silently gaining a
    // field. Catch the panic so this case lives with its non-panicking peers.
    let message = panic_message(|| {
        text_records("REPORT junk total=0\n");
    });
    assert!(
        message.contains("text record has a token before its first field"),
        "a token before the first field is rejected: {message}"
    );
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

/// A synthetic sequence, so the golden's own failure modes are testable
/// without a run. `check_golden` is test-only and the real goldens exercise
/// only its happy path; these cases pin its rejection branches directly.
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

/// Restore a fresh one-clean-one-damaged install before each plane of a merged
/// test. The first plane's run bumps `config.ini` and repairs the damaged file;
/// removing the version file and re-seeding keeps the second plane's report
/// identical to what a standalone test saw.
fn reseed_install(dir: &std::path::Path) {
    let _ = std::fs::remove_file(dir.join("config.ini"));
    seed_one_damaged_one_clean(dir);
}

/// A tiny repair pins the order and the one-record-per-event rule on both
/// output planes: `REPORT` once, then (if the run is long enough) any
/// `PROGRESS` ticks, then exactly one `SUMMARY` last. The two planes share one
/// mock and install; the install is re-seeded between runs so each plane sees
/// the same one-clean-one-damaged state.
#[tokio::test]
async fn the_json_and_text_planes_agree_on_a_tiny_repair() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("golden_repair");

    // JSON plane: the script-parsed form. One clean file and one damaged file
    // make the run short, so its `SUMMARY` is the base shape.
    reseed_install(&dir);
    let run = run_data_plane(&dir, &mock, &[]).await;
    assert_eq!(run.code, 0, "a tiny repair exits 0:\n{}", run.stderr);
    assert_golden(&run, &[("report", REPORT_KEYS), ("summary", SUMMARY_KEYS)]);

    // Text plane: the same records rendered as `EVENT key=value …`, with the
    // extra one-terminal-`SUMMARY` check the text plane carries.
    reseed_install(&dir);
    let run = run_text_plane(&dir, &mock, &[]).await;
    assert_eq!(run.code, 0, "a tiny repair exits 0:\n{}", run.stderr);
    assert_one_terminal_summary(&run);
    assert_golden(&run, &[("report", REPORT_KEYS), ("summary", SUMMARY_KEYS)]);
    mock.stop();
}

/// `--print-plan` adds exactly one `PLAN` between `REPORT` and the run's single
/// `SUMMARY` on both output planes, and writes nothing.
#[tokio::test]
async fn the_json_and_text_planes_agree_on_print_plan() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("golden_print_plan");

    // JSON plane.
    reseed_install(&dir);
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

    // Text plane: the `PLAN` sits in the same place, with the same field order.
    reseed_install(&dir);
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

/// `--check-only` is `REPORT` and the single terminal `SUMMARY`, with no
/// `PLAN`, on both output planes; a damaged install is exit `4`.
#[tokio::test]
async fn the_json_and_text_planes_agree_on_check_only() {
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;
    let dir = temp_dir("golden_check_only");

    // JSON plane.
    reseed_install(&dir);
    let run = run_data_plane(&dir, &mock, &["--check-only"]).await;
    assert_eq!(run.code, 4, "a damaged install is exit 4:\n{}", run.stderr);
    assert_golden(&run, &[("report", REPORT_KEYS), ("summary", SUMMARY_KEYS)]);

    // Text plane.
    reseed_install(&dir);
    let run = run_text_plane(&dir, &mock, &["--check-only"]).await;
    assert_eq!(run.code, 4, "a damaged install is exit 4:\n{}", run.stderr);
    assert_one_terminal_summary(&run);
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
