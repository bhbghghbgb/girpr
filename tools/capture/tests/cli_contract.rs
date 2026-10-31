//! The process-level contract: exit codes and the shape of the stdout data plane.
//!
//! Everything here spawns the built binary. The unit tests in `src/` can only
//! reach `clap_error_code`, because the remapping lives in `main.rs` — a shim
//! with no other logic, and still the only place the process's exit status is
//! decided. Asserting it here is what stops a future change from moving it
//! back into clap's default.
//!
//! **No case touches a network.** `capture` really does reach the live APIs, so
//! every `capture` case here points `--hyp-base`/`--sophon-base` at
//! `127.0.0.1:0`, where nothing can be listening: the connect is refused
//! immediately and the run takes its documented exit `2` without a packet
//! leaving the host. That is what makes the seam worth having — the contract is
//! asserted against a run that started, failed, and reported, rather than
//! against a stub.
//!
//! Port `0` rather than a closed high port like `1` is deliberate. On Windows a
//! connect to a closed port waits ~2 s for the SYN retry before failing; port
//! `0` is not a connectable port, so the failure is immediate. The contract is
//! identical either way (transport failure → exit `2`, `network` slug,
//! trustworthy artifact), so the tests use the fast refusal. `validate_base`
//! only requires loopback, which `127.0.0.1:0` satisfies.

use std::process::Command;

use girpr_tempdir::TempDir;

/// The binary under test, beside this test's own executable.
fn bin() -> std::path::PathBuf {
    // `target/debug/deps/<test>.exe` -> `target/debug/gircap.exe`
    let mut p = std::env::current_exe().expect("the test binary has a path");
    p.pop(); // deps/
    if p.ends_with("deps") {
        p.pop();
    }
    p.push(format!("gircap{}", std::env::consts::EXE_SUFFIX));
    p
}

/// Flags that point `capture` at a loopback port that cannot be connected to.
///
/// Port `0` is used on purpose, not as a placeholder to "fix" later: Windows
/// fails the connect immediately instead of waiting ~2 s for a closed port's SYN
/// retry. The run still produces the same contract — exit `2`, the
/// `network` slug, a trustworthy artifact — it just gets there without the dead
/// port's penalty.
///
/// `--retries 1` keeps the test to one attempt: a backoff would make a contract
/// test take seconds to prove something that is true immediately.
const OFFLINE: &[&str] = &[
    "--hyp-base",
    "http://127.0.0.1:0/hyp",
    "--sophon-base",
    "http://127.0.0.1:0/sophon",
    "--retries",
    "1",
    "--timeout",
    "2",
];

/// `capture` with the offline seam appended.
fn offline_capture(extra: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = ["capture"].iter().map(|s| s.to_string()).collect();
    v.extend(OFFLINE.iter().map(|s| s.to_string()));
    v.extend(extra.iter().map(|s| s.to_string()));
    v
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_owned(argv: &[String]) -> Run {
    let out = Command::new(bin())
        .args(argv)
        .output()
        .unwrap_or_else(|e| panic!("running {argv:?}: {e}"));
    Run {
        code: out.status.code().expect("the process was not killed"),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn run(args: &[&str]) -> Run {
    run_owned(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
}

fn assert_code(args: &[&str], want: i32) -> Run {
    let r = run(args);
    assert_eq!(
        r.code, want,
        "{args:?} exited {}\n--- stderr ---\n{}",
        r.code, r.stderr
    );
    r
}

#[test]
fn the_binary_exists_where_this_test_expects_it() {
    assert!(bin().exists(), "{} not built", bin().display());
}

#[test]
fn help_and_version_are_zero() {
    assert_code(&["--help"], 0);
    assert_code(&["--version"], 0);
    for cmd in ["capture", "verify", "materialize", "list"] {
        assert_code(&[cmd, "--help"], 0);
    }
}

/// The two `capture` help strings that contradicted the code and the spec.
///
/// Neither was pinned anywhere. `--require-class` said "Default: all" while an
/// empty list means nothing is required, and `--no-chunk-blobs` named three of
/// the eleven assertions it skips. `--help` is where a reader finds out what a
/// flag does, so it is the wrong place to be wrong.
#[test]
fn the_two_help_strings_that_were_stale_match_the_code() {
    let r = assert_code(&["capture", "--help"], 0);
    // clap wraps to the terminal width, so compare with the wrapping removed
    // rather than betting on where a line break lands.
    let help = r.stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        help.contains("Default: none (nothing is required unless named)"),
        "--require-class requires nothing unless named:\n{}",
        r.stdout
    );
    assert!(
        help.contains("Breaks assertions A6-A12, A19, A21-A23 by design"),
        "--no-chunk-blobs skips eleven assertions, not three:\n{}",
        r.stdout
    );
}

/// Every argument-level failure is `1`, never clap's default `2` — `2` means
/// "network or API failure" here, and a flag typo must not be indistinguishable
/// from a CDN outage.
///
/// Only the *code* is asserted here. Whether a failure prints a summary is a
/// separate contract — `a_usage_error_prints_a_summary_on_stdout_too` — and
/// holding this test to it too would make one assertion depend on another.
#[test]
fn every_argument_error_is_one() {
    let cases: &[&[&str]] = &[
        &[],                                      // no subcommand
        &["capture", "--nope"],                   // unknown flag
        &["frobnicate"],                          // unknown subcommand
        &["capture", "--biz", "hk4e_turkey"],     // bad enum
        &["capture", "--per-class", "many"],      // bad number
        &["capture", "--log-level", "loud"],      // bad level
        &["capture", "--log-file-level", "loud"], // bad level
        &["capture", "--output", "yaml"],
        &["verify"], // missing --artifact
        &["materialize"],
        &["list"],
        &["verify", "--artifact", "a", "--output"], // missing value
    ];
    for args in cases {
        assert_code(args, 1);
    }
}

/// A non-Unicode `--out` is recorded, not fatal.
///
/// The `PathBuf`-typed flags are the realistic carriers of an argument that is
/// not valid Unicode; `std::env::args()` panicked on it *after* clap had already
/// parsed the same value through `args_os`, which exited `101` and printed no
/// `SUMMARY`. The contract is the code and the line, whichever way the run ends.
#[cfg(windows)]
#[test]
fn a_non_unicode_output_path_reports_instead_of_panicking() {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt as _;

    let mut argv: Vec<OsString> = vec![
        OsString::from("capture"),
        OsString::from("--out"),
        OsString::from_wide(&[0xD800]),
    ];
    argv.extend(OFFLINE.iter().map(OsString::from));
    let out = Command::new(bin())
        .args(&argv)
        .output()
        .expect("the binary spawns with a non-Unicode argument");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        out.status.code(),
        Some(2),
        "an unreachable base is still exit 2:\n{stdout}"
    );
    assert!(
        stdout.starts_with("SUMMARY "),
        "the run must still report:\n{stdout:?}"
    );
}

#[test]
fn a_usage_error_prints_a_summary_on_stdout_too() {
    // A failure the *run* discovers — rather than the parser — still prints
    // exactly one summary on stdout, in text, so that a harness reading stdout
    // does not have to special-case a crash. `--materialize --artifact a --out
    // a` is such a failure: the request is well-formed and the plan refuses
    // it.
    let r = assert_code(&["materialize", "--artifact", "a", "--out", "a"], 1);
    assert!(r.stdout.starts_with("SUMMARY "), "{:?}", r.stdout);
    assert!(r.stdout.contains("exit_code=1"), "{:?}", r.stdout);
    assert!(r.stdout.contains("exit_code_slug=usage"), "{:?}", r.stdout);
    assert_eq!(r.stdout.lines().filter(|l| !l.is_empty()).count(), 1);
}

/// A bad `--log-level` is an *argument* failure, and it is refused by the
/// parser — like every other bad argument, and unlike the run-time failures
/// above, which print a summary because they happen after the parse.
#[test]
fn a_bad_log_level_is_refused_by_the_parser() {
    let r = assert_code(&["capture", "--log-level", "loud"], 1);
    assert!(
        r.stderr.contains("loud"),
        "the message names the value that was wrong:\n{:?}",
        r.stderr
    );
    assert!(
        r.stdout.is_empty(),
        "a parse failure owns no part of stdout:\n{:?}",
        r.stdout
    );
}

#[test]
fn the_summary_is_the_only_thing_on_stdout() {
    // clap owns the channel for its own help screens; it is not the data plane.
    let r = run(&["--help"]);
    assert!(r.stdout.contains("Usage:"), "{:?}", r.stdout);
    for cmd in ["capture", "verify", "materialize", "list"] {
        assert!(
            r.stdout.contains(cmd),
            "{cmd} missing from --help:\n{}",
            r.stdout
        );
    }

    let r = run_owned(&offline_capture(&["--output", "json"]));
    assert_eq!(r.code, 2, "an unreachable base is exit 2\n{}", r.stderr);
    let lines: Vec<&str> = r.stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1, "expected exactly one line: {lines:?}");
    let v: serde_json::Value = serde_json::from_str(lines[0]).expect("one JSON object");
    assert_eq!(v["event"], "capture");
    assert_eq!(v["exit_code"], 2);
    assert_eq!(v["exit_code_slug"], "network");
    assert_eq!(v["tool"], "gircap");
}

/// An api/base override is a seam, so a typo in it must not become a request.
#[test]
fn a_bad_base_override_is_a_usage_error_not_a_request() {
    for bad in [
        "not a url",
        "ftp://example.com/api",
        "http://example.com/api",
        "http://",
    ] {
        let argv: Vec<String> = ["capture", "--hyp-base", bad]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let r = run_owned(&argv);
        assert_eq!(r.code, 1, "{bad:?} exited {}\n{}", r.code, r.stderr);
        assert!(
            r.stdout.contains("exit_code_slug=usage"),
            "{bad:?}: {:?}",
            r.stdout
        );
        assert!(
            !r.stderr.contains("GET"),
            "{bad:?} issued a request anyway:\n{}",
            r.stderr
        );
    }
}

/// Plaintext is allowed to loopback and nowhere else, so a mistyped override
/// cannot quietly downgrade a production capture. The positive (loopback) half
/// is every other case in this file, which uses exactly that.
#[test]
fn a_plaintext_override_to_a_remote_host_is_refused_and_says_why() {
    let r = run(&["capture", "--hyp-base", "http://example.com/api"]);
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert!(
        r.stdout.contains("plaintext"),
        "the refusal must say why: {:?}",
        r.stdout
    );
}

/// `list` emits one `BUNDLE` record per bundle and then one `SUMMARY`, so a
/// machine can read it without parsing narration. This is the shape a consumer
/// depends on, so it is pinned line by line rather than by a substring.
///
/// No capture is needed: the point is the *record shape*, and an empty root
/// exercises the degenerate case (no `BUNDLE`, one `SUMMARY`) which is the one a
/// consumer is most likely to get wrong.
#[test]
fn list_emits_one_record_per_bundle_then_a_summary() {
    let scratch = TempDir::new("gircap-list", "one_record_per_bundle");

    let r = run(&[
        "list",
        "--root",
        &scratch.to_string_lossy(),
        "--output",
        "json",
    ]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let lines: Vec<&str> = r.stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines.len(),
        1,
        "an empty root is still one terminal record: {lines:?}"
    );
    let v: serde_json::Value = serde_json::from_str(lines[0]).expect("one JSON object");
    assert_eq!(v["event"], "list");
    assert_eq!(v["bundles"], 0);
    assert_eq!(v["exit_code"], 0);

    // Now a root holding something that is not a bundle: it must not become a
    // `BUNDLE` record. A directory that does not open is stderr-only, because
    // there is no bundle to describe.
    let junk = scratch.join("not-a-bundle");
    std::fs::create_dir_all(&junk).expect("junk dir");
    std::fs::write(junk.join("MANIFEST.json"), b"{ not json").expect("junk manifest");
    let r = run(&[
        "list",
        "--root",
        &scratch.to_string_lossy(),
        "--output",
        "json",
    ]);
    assert_eq!(
        r.code, 0,
        "a junk directory does not fail the walk: {}",
        r.stderr
    );
    let lines: Vec<&str> = r.stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines.len(),
        1,
        "still no bundle, so still one line: {lines:?}"
    );
    let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v["bundles"], 0);
}

/// stdout under `--output json` is **pure NDJSON**: one object per line, nothing
/// else, with `event` first so a consumer can dispatch mid-stream without having
/// parsed the whole line.
///
/// This runs against an empty root, so there is no `BUNDLE` record to inspect —
/// the per-bundle fields are asserted in `capture_layout.rs`, where a real
/// capture exists. What is pinned here is the part that holds whatever the root
/// contains.
#[test]
fn every_record_puts_event_first_and_the_stream_is_pure_ndjson() {
    let scratch = TempDir::new("gircap-ndjson", "event_first");

    for argv in [
        vec![
            "list",
            "--root",
            &scratch.to_string_lossy(),
            "--output",
            "json",
        ],
        vec![
            "verify",
            "--artifact",
            &scratch.to_string_lossy(),
            "--output",
            "json",
        ],
        vec![
            "materialize",
            "--artifact",
            &scratch.to_string_lossy(),
            "--output",
            "json",
        ],
    ] {
        let r = run_owned(&argv.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let lines: Vec<&str> = r.stdout.lines().filter(|l| !l.is_empty()).collect();
        assert!(!lines.is_empty(), "{argv:?} printed nothing to stdout");
        for line in lines {
            assert!(
                line.starts_with("{\"event\":"),
                "{argv:?}: `event` must lead every record: {line}"
            );
            serde_json::from_str::<serde_json::Value>(line).unwrap_or_else(|e| {
                panic!("{argv:?}: not one JSON object per line: {e:?}\n{line}")
            });
        }
    }
}

#[test]
fn the_text_summary_is_key_value_pairs() {
    let r = run_owned(&offline_capture(&[]));
    assert_eq!(r.code, 2);
    let line = r.stdout.trim();
    assert!(line.starts_with("SUMMARY "), "{line:?}");
    for kv in line.split_whitespace().skip(1) {
        assert!(kv.contains('='), "not a k=v pair: {kv:?} in {line:?}");
    }
}

#[test]
fn stderr_carries_the_narration_and_stdout_carries_none_of_it() {
    let r = run_owned(&offline_capture(&["--log-level", "debug"]));
    assert!(
        r.stderr.contains("\"GET\"") || r.stderr.contains("GET"),
        "expected the request on stderr, got {:?}",
        r.stderr
    );
    assert!(
        !r.stdout.contains("GET"),
        "narration leaked onto stdout: {:?}",
        r.stdout
    );
}

#[test]
fn a_log_file_is_written_and_flushed_before_the_process_ends() {
    // Absent on purpose: the run is what has to create the parent, and that is
    // half of what this case is about.
    let dir = TempDir::absent("gircap-contract", "log_file");
    let log = dir.join("nested").join("run.ndjson");

    let r = run_owned(&offline_capture(&["--log-file", log.to_str().unwrap()]));
    assert_eq!(r.code, 2);

    let text =
        std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("--log-file was not written: {e}"));
    assert!(
        text.lines().any(|l| l.contains("\"gircap start\"")),
        "the start event is missing from the log:\n{text}"
    );
}

/// `RUST_LOG` does not reach the log file.
///
/// The two sinks have two levels; the environment variable is a console lever
/// and the file's own level is `--log-file-level`.
#[test]
fn rust_log_does_not_reach_the_log_file() {
    let dir = TempDir::absent("gircap-rustlog", "rust_log_stays_off_the_file");
    let log = dir.join("run.ndjson");

    let argv = offline_capture(&["--log-level", "error", "--log-file", log.to_str().unwrap()]);
    let out = Command::new(bin())
        .args(&argv)
        .env("RUST_LOG", "warn")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2));

    let text = std::fs::read_to_string(&log).expect("log written");
    assert!(
        text.lines().any(|l| l.contains("\"gircap start\"")),
        "the file sink must not inherit the console's `error` filter:\n{text}"
    );
}

/// `RUST_LOG` adds to the console; it cannot silence the program.
///
/// A `RUST_LOG` naming an unrelated crate would replace the console's whole
/// spec, and the program would then say nothing at all. The program's own
/// level is `--log-level`, and `RUST_LOG` only adds to it.
#[test]
fn an_unrelated_rust_log_does_not_silence_the_capture() {
    let out = Command::new(bin())
        .args(offline_capture(&[]))
        .env("RUST_LOG", "reqwest=debug")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2));

    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains("gircap start"),
        "the capture still speaks at its own level:\n{stderr}"
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

    let r = run_owned(&offline_capture(&[]));
    assert_eq!(r.code, 2, "{}", r.stderr);

    if cfg!(debug_assertions) {
        assert!(
            r.stderr.contains("log file: none"),
            "a debug build says it wrote no file:\n{}",
            r.stderr
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
            r.stderr.contains("log file: "),
            "a release build says where the file went:\n{}",
            r.stderr
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
    let dir = TempDir::absent("gircap-contract", "no_log_file");
    let log = dir.join("from-env.ndjson");

    let out = Command::new(bin())
        .args(offline_capture(&["--no-log-file"]))
        .env("GIRCAP_LOG_FILE", &log)
        .output()
        .expect("spawn");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(2), "{stderr}");
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
    let dir = TempDir::absent("gircap-contract", "conflict");
    let log = dir.join("both.ndjson");

    let r = run_owned(&offline_capture(&[
        "--no-log-file",
        "--log-file",
        log.to_str().unwrap(),
    ]));
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert!(
        r.stderr.contains("cannot be used with"),
        "the conflict is reported:\n{}",
        r.stderr
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
    let dir = TempDir::absent("gircap-env", "log_file");
    let log = dir.join("from-env.ndjson");

    let out = Command::new(bin())
        .args(offline_capture(&[]))
        .env("GIRCAP_LOG_FILE", &log)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2));

    let text = std::fs::read_to_string(&log).expect("the environment named the file");
    assert!(
        text.lines().any(|l| l.contains("\"gircap start\"")),
        "and it holds the run's own records:\n{text}"
    );
}

/// The console is `info` and the log file is `debug`, by default.
///
/// The two levels are separate knobs, and the defaults are one step apart:
/// the console is for the person watching the run, the file for the person
/// reading it afterwards. The request narration is `debug`, so it is the
/// evidence — it is in the file and not on the console.
#[test]
fn the_file_is_louder_than_the_console_by_default() {
    let dir = TempDir::absent("gircap-levels", "file_vs_console");
    let log = dir.join("levels.ndjson");

    let r = run_owned(&offline_capture(&[
        "--log-level",
        "error",
        "--log-file",
        log.to_str().unwrap(),
    ]));
    assert_eq!(r.code, 2);

    let text = std::fs::read_to_string(&log).expect("the log file was written");
    assert!(
        text.lines().any(|l| l.contains("\"GET\"")),
        "the file is at `debug` by default, so it kept the request narration:\n{text}"
    );
    assert!(
        !r.stderr.contains("GET"),
        "the console is at `error`, so it did not:\n{}",
        r.stderr
    );
}

/// `--log-file-level` narrows the log file.
///
/// The flag is the file's own level, and it narrows *only* the file: the
/// console keeps its own. A file at `error` is the record of what failed and
/// nothing else.
#[test]
fn the_log_file_level_flag_narrows_the_file() {
    let dir = TempDir::absent("gircap-levels", "file_level_flag");
    let log = dir.join("levels.ndjson");

    let r = run_owned(&offline_capture(&[
        "--log-file",
        log.to_str().unwrap(),
        "--log-file-level",
        "error",
    ]));
    assert_eq!(r.code, 2);

    let text = std::fs::read_to_string(&log).expect("the log file was written");
    assert!(
        !text.lines().any(|l| l.contains("\"GET\"")),
        "the file is at `error`, so it kept no debug record:\n{text}"
    );
    assert!(
        !text.lines().any(|l| l.contains("\"gircap start\"")),
        "nor any info record:\n{text}"
    );
    assert!(
        text.lines().any(|l| l.contains("\"ERROR\"")),
        "but it kept the error, so the file is filtered rather than empty:\n{text}"
    );
}

/// The environment sets the log file's level, when the flag does not.
///
/// The one seam of the policy the flag table cannot show: the flag wins, the
/// environment answers for a flag that was never given, and `debug` — the
/// default — answers for both.
#[test]
fn the_environment_sets_the_log_files_level() {
    let dir = TempDir::absent("gircap-env", "log_file_level");
    let log = dir.join("levels.ndjson");

    let out = Command::new(bin())
        .args(offline_capture(&["--log-file", log.to_str().unwrap()]))
        .env("GIRCAP_LOG_FILE_LEVEL", "error")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2));

    let text = std::fs::read_to_string(&log).expect("the log file was written");
    assert!(
        !text.lines().any(|l| l.contains("\"GET\"")),
        "the file is at `error`, so it kept no debug record:\n{text}"
    );
}

/// The directory the executable is in — where the default log file lands.
fn exe_dir() -> std::path::PathBuf {
    let mut p = bin();
    p.pop();
    p
}

// ------------------------------------------ the offline three's exit codes
//
// `verify`, `materialize` and `list` are checked here for their exit codes
// against paths that are *wrong*, because a bundle that is right is a fixture's
// job (see `offline_bundle.rs`) and these tests must not grow one.

/// A scratch directory that does not exist yet, named per test.
///
/// Absent on purpose: these cases are about what the tool does with a path that
/// is *not* a bundle, so the fixture must not hand it one.
fn scratch(name: &str) -> TempDir {
    TempDir::absent("gircap-contract", name)
}

/// A path that is not a bundle is a usage error, not an assertion failure: the
/// operator named something that does not exist, and exit `3` would say the
/// artifact is broken when the artifact was never named.
#[test]
fn verify_on_a_directory_that_is_not_a_bundle_is_a_usage_error() {
    let d = scratch("verify-not-a-bundle");
    std::fs::create_dir_all(&d).unwrap();
    let r = assert_code(&["verify", "--artifact", d.to_str().unwrap()], 1);
    assert!(r.stdout.contains("event=verify"), "{}", r.stdout);
    assert!(r.stdout.contains("MANIFEST.json"), "{}", r.stdout);
    assert!(r.stdout.contains("exit_code=1"), "{}", r.stdout);
}

/// The same for `materialize`, and the same code - the two read the same bundle.
#[test]
fn materialize_on_a_directory_that_is_not_a_bundle_is_a_usage_error() {
    let d = scratch("materialize-not-a-bundle");
    std::fs::create_dir_all(&d).unwrap();
    let r = assert_code(&["materialize", "--artifact", d.to_str().unwrap()], 1);
    assert!(r.stdout.contains("event=materialize"), "{}", r.stdout);
}

/// `--out` equal to `--artifact` is refused before anything is written.
#[test]
fn materialize_refuses_to_write_into_the_bundle_it_is_proving() {
    let d = scratch("materialize-same-dir");
    std::fs::create_dir_all(&d).unwrap();
    let r = assert_code(
        &[
            "materialize",
            "--artifact",
            d.to_str().unwrap(),
            "--out",
            d.to_str().unwrap(),
        ],
        1,
    );
    assert!(r.stdout.contains("--out"), "{}", r.stdout);
    assert!(r.stdout.contains("--artifact"), "{}", r.stdout);
}

/// `list` over an empty tree is a success with zero bundles, not a failure.
#[test]
fn list_over_an_empty_tree_is_a_success_with_nothing_in_it() {
    let d = scratch("list-empty");
    std::fs::create_dir_all(&d).unwrap();
    let r = assert_code(&["list", "--root", d.to_str().unwrap()], 0);
    assert!(r.stdout.contains("bundles=0"), "{}", r.stdout);
    assert!(r.stdout.contains("result=ok"), "{}", r.stdout);
}

/// A root that is not there is a usage error - again, the operator's mistake.
#[test]
fn list_over_a_root_that_does_not_exist_is_a_usage_error() {
    let d = scratch("list-missing");
    let r = assert_code(&["list", "--root", d.to_str().unwrap()], 1);
    // The summary is whitespace-free, so the detail arrives percent-encoded.
    assert!(r.stdout.contains("not%20a%20directory"), "{}", r.stdout);
}

/// `verify` takes no base flags, so it has no `--hyp-base` to point somewhere
/// useless: a reader must not be left wondering whether it phoned home. Asserted
/// by the fact that a stray flag is *rejected* rather than silently ignored.
#[test]
fn verify_takes_no_base_flags_and_needs_none() {
    let d = scratch("verify-flags");
    std::fs::create_dir_all(&d).unwrap();
    let r = run(&[
        "verify",
        "--artifact",
        d.to_str().unwrap(),
        "--hyp-base",
        "http://127.0.0.1:1",
    ]);
    assert_eq!(
        r.code, 1,
        "an unknown flag is a usage error, as every argument error is: {}",
        r.stderr
    );
    assert!(
        r.stderr.contains("--hyp-base"),
        "and clap names the flag: {}",
        r.stderr
    );
}
