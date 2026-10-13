//! The process-level contract: exit codes and the shape of the stdout data plane.
//!
//! Everything here spawns the built binary. The unit tests in `src/` can only
//! reach `clap_error_code`, because the remapping lives in `main.rs` — a shim
//! with no other logic, and still the only place the process's exit status is
//! decided. Asserting it here is what stops a future refactor from moving it
//! back into clap's default.
//!
//! **No case touches a network.** `capture` really does reach the live APIs, so
//! every `capture` case here points `--hyp-base`/`--sophon-base` at
//! `127.0.0.1:1`, where nothing is listening: the request is refused immediately
//! and the run takes its documented exit `2` without a packet leaving the host.
//! That is what makes the seam worth having — the contract is asserted against a
//! run that started, failed, and reported, rather than against a stub.

use std::process::Command;

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

/// Flags that point `capture` at a dead loopback port.
///
/// `--retries 1` keeps the test to one attempt: a backoff would make a contract
/// test take seconds to prove something that is true immediately.
const OFFLINE: &[&str] = &[
    "--hyp-base",
    "http://127.0.0.1:1/hyp",
    "--sophon-base",
    "http://127.0.0.1:1/sophon",
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
        &[],                                 // no subcommand
        &["capture", "--nope"],              // unknown flag
        &["frobnicate"],                     // unknown subcommand
        &["capture", "--biz", "hk4e_turkey"], // bad enum
        &["capture", "--per-class", "many"],  // bad number
        &["capture", "--log-level", "loud"],  // unvalidated string flag
        &["capture", "--output", "yaml"],
        &["verify"],                         // missing --artifact
        &["materialize"],
        &["list"],
        &["verify", "--artifact", "a", "--output"], // missing value
    ];
    for args in cases {
        assert_code(args, 1);
    }
}

#[test]
fn a_usage_error_prints_a_summary_on_stdout_too() {
    // Resolved *after* the parse: `--log-level` is a string flag on the clap
    // surface and is typed later, so it is the one usage error that has to be
    // discovered by the run rather than by the parser. It is also the only
    // failure that cannot honour `--output`, since `--output` is what failed
    // alongside it — so this pins the one case that always prints text.
    let r = assert_code(&["capture", "--log-level", "loud", "--output", "json"], 1);
    assert!(r.stdout.starts_with("SUMMARY "), "{:?}", r.stdout);
    assert!(r.stdout.contains("exit_code=1"), "{:?}", r.stdout);
    assert!(r.stdout.contains("exit_code_slug=usage"), "{:?}", r.stdout);
    assert_eq!(r.stdout.lines().filter(|l| !l.is_empty()).count(), 1);
}

#[test]
fn the_summary_is_the_only_thing_on_stdout() {
    // clap owns the channel for its own help screens; it is not the data plane.
    let r = run(&["--help"]);
    assert!(r.stdout.contains("Usage:"), "{:?}", r.stdout);
    for cmd in ["capture", "verify", "materialize", "list"] {
        assert!(r.stdout.contains(cmd), "{cmd} missing from --help:\n{}", r.stdout);
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
    let dir = std::env::temp_dir().join(format!("gircap-contract-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let log = dir.join("nested").join("run.ndjson");

    let r = run_owned(&offline_capture(&["--log-file", log.to_str().unwrap()]));
    assert_eq!(r.code, 2);

    let text = std::fs::read_to_string(&log)
        .unwrap_or_else(|e| panic!("--log-file was not written: {e}"));
    assert!(
        text.lines().any(|l| l.contains("\"gircap start\"")),
        "the start event is missing from the log:\n{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rust_log_overrides_the_flag() {
    let dir = std::env::temp_dir().join(format!("gircap-rustlog-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
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
    let _ = std::fs::remove_dir_all(&dir);
}