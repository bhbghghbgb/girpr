//! Test fixtures shared with `sled2redb`, now owned by `girpr-tempdir`.
//!
//! This used to be a 702-line verbatim copy of the same file in `sled2redb`.
//! The implementations now live once, in `girpr-tempdir::test_support`; this
//! module only re-exports them so every call site in this suite keeps its
//! existing `mod common;`, `common::helper(..)`, `use common::*`, and unqualified
//! `skip_if_*!` spellings.

pub use girpr_tempdir::test_support::*;
// `#[macro_export]` macros live at the defining crate's root, not inside the
// `test_support` module, so re-export them explicitly as well. Most test
// binaries in this suite use neither, hence the allow.
#[allow(unused_imports)]
pub use girpr_tempdir::{skip_if_case_insensitive, skip_if_no_link};

use clap::Parser;
use girsync::cli::Cli;
use girsync::logging::Level;
use girsync::report::OutputFormat;
use girsync::{Report, RunOutcome, plan};
use serde_json::Value;

/// The quiet log policy an in-process run is dispatched under: no console
/// events, no log file. The report's records are captured regardless of level.
fn capture_plan() -> girsync::girpr_log::LogPlan {
    plan(Level::Error, None, None, true)
}

/// Run a parsed CLI in-process, capturing the records, exit code and error
/// string a process test used to read off the binary's stdout/stderr.
///
/// `GIRSYNC_TEST_CACHE=file` is pinned here: a spawned child got a fresh
/// in-memory cache per process, but the memory registry is shared across
/// in-process runs on the same paths. Forcing the file backend keeps each run's
/// cache local to its fixture and makes the result independent of an ambient
/// `GIRSYNC_TEST_CACHE`. Nextest isolates each test in its own process, so the
/// `set_var` cannot leak to another test.
#[allow(unsafe_code, dead_code)] // `set_var` is unsafe in edition 2024; nextest isolates each test process
pub fn run_cli(cli: Cli) -> RunOutcome {
    unsafe { std::env::set_var("GIRSYNC_TEST_CACHE", "file") };
    girsync::run_captured(cli, &capture_plan()).expect("run_captured")
}

/// Parse arguments (as passed to `Command::args`, without a program name) and
/// run the CLI in-process. The binary name is supplied as argv[0] for clap.
#[allow(dead_code)]
pub fn run_args<I, S>(args: I) -> RunOutcome
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let argv = std::iter::once(std::ffi::OsString::from("girsync"))
        .chain(args.into_iter().map(Into::into));
    run_cli(Cli::try_parse_from(argv).expect("valid CLI args"))
}

/// Parse argv, run in-process, and return the exit code plus the error message
/// a caller would read off stderr: clap's own text for a parse error (exit 2),
/// or the `anyhow` string `main` logs as `fatal` (exit 3). Empty on success.
#[allow(dead_code)]
pub fn run_error_text<I, S>(args: I) -> (i32, String)
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let argv = std::iter::once(std::ffi::OsString::from("girsync"))
        .chain(args.into_iter().map(Into::into));
    match Cli::try_parse_from(argv) {
        Err(e) => (2, e.to_string()),
        Ok(cli) => {
            let out = run_cli(cli);
            (out.code, out.error.unwrap_or_default())
        }
    }
}

/// The captured records as the JSON values a `--output json` parser would read.
#[allow(dead_code)]
pub fn values(out: &RunOutcome) -> Vec<Value> {
    out.records.iter().map(|r| r.json()).collect()
}

/// The captured records rendered as the text a stdout consumer would read, in
/// the run's `--output` format, one line per record with a trailing newline.
#[allow(dead_code)]
pub fn rendered(out: &RunOutcome, format: OutputFormat) -> String {
    let report = Report::new(format);
    let mut text = String::new();
    for rec in &out.records {
        text.push_str(&report.render(rec));
        text.push('\n');
    }
    text
}

/// Parse argv, run in-process, and return the exit code plus rendered stdout —
/// the drop-in for a binary helper that returned `(code, text)`.
#[allow(dead_code)]
pub fn run_text<I, S>(args: I) -> (i32, String)
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let argv = std::iter::once(std::ffi::OsString::from("girsync"))
        .chain(args.into_iter().map(Into::into));
    let cli = Cli::try_parse_from(argv).expect("valid CLI args");
    let format = cli.output;
    let out = run_cli(cli);
    (out.code, rendered(&out, format))
}
