//! Test fixtures shared with `sled2redb`, now owned by `girpr-sync-test-support`.
//!
//! This used to be a 702-line verbatim copy of the same file in `sled2redb`.
//! The implementations now live once, in `girpr_sync_test_support`; this module
//! only re-exports them so every call site in this suite keeps its existing
//! `mod common;`, `common::helper(..)`, `use common::*`, and unqualified
//! `skip_if_*!` spellings.

pub use girpr_sync_test_support::*;
// `#[macro_export]` macros live at the defining crate's root, so re-export them
// explicitly as well. Most test binaries in this suite use neither, hence the
// allow. The fixtures crate re-exports them from `girpr-tempdir`, where they are
// still defined.
#[allow(unused_imports)]
pub use girpr_sync_test_support::{skip_if_case_insensitive, skip_if_no_link};

use std::sync::{Arc, Mutex};

use clap::Parser;
use girsync::cli::Cli;
use girsync::logging::Level;
use girsync::report::OutputFormat;
use girsync::{Report, RunOutcome, TestHooks, plan};
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
    run_cli_with_hooks(cli, TestHooks::from_env())
}

/// Run a parsed CLI in-process with the given hooks injected, capturing records,
/// exit code and error string exactly as [`run_cli`] does.
///
/// This is the seam the failure-path cases use instead of spawning a child just
/// to set a `GIRSYNC_TEST_*` variable: the hooks are data, so two runs in one
/// process can ask for different failures without a `OnceLock` or a subprocess.
#[allow(unsafe_code, dead_code)] // `set_var` is unsafe in edition 2024; nextest isolates each test process
pub fn run_cli_with_hooks(cli: Cli, hooks: TestHooks) -> RunOutcome {
    unsafe { std::env::set_var("GIRSYNC_TEST_CACHE", "file") };
    girsync::run_captured_with_hooks(cli, &capture_plan(), hooks).expect("run_captured")
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

/// [`run_args`] with hooks injected through [`run_cli_with_hooks`].
#[allow(dead_code)]
pub fn run_args_with_hooks<I, S>(args: I, hooks: TestHooks) -> RunOutcome
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let argv = std::iter::once(std::ffi::OsString::from("girsync"))
        .chain(args.into_iter().map(Into::into));
    run_cli_with_hooks(Cli::try_parse_from(argv).expect("valid CLI args"), hooks)
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

/// A console sink a test can read back: each event's formatted bytes are
/// appended to one shared buffer, so a scoped subscriber's stderr is available
/// even though no child process was spawned.
#[derive(Clone, Default)]
pub struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

#[allow(dead_code)] // each integration target gets its own copy of `common`
impl SharedBuffer {
    /// Everything the console layer has rendered so far, as text.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(|e| e.into_inner())).into_owned()
    }
}

pub struct BufferWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for BufferWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedBuffer {
    type Writer = BufferWriter;

    fn make_writer(&'a self) -> Self::Writer {
        BufferWriter(Arc::clone(&self.0))
    }
}

/// Parse argv and run the CLI in-process under a scoped copy of the binary's
/// tracing stack, with the console rendered into a returned buffer.
///
/// This is the seam for the tests that used to spawn the binary to read its
/// log: the plan is resolved from the same CLI as `main` resolves it, the layer
/// stack is the one [`girsync::subscriber_for`] builds, and the non-blocking
/// file guard is held in a binding that outlives the `with_default` closure so
/// the log file's tail is flushed before this returns. `env` lists any variable
/// the plan must see (`RUST_LOG`, `<PROJECT>_LOG_FILE_LEVEL`); each is removed
/// afterwards so a second run in the same test starts clean.
#[allow(unsafe_code, dead_code)] // `set_var`/`remove_var` are unsafe in edition 2024; nextest isolates each test process
pub fn run_args_logged<I, S>(args: I, env: &[(&str, &str)]) -> (RunOutcome, SharedBuffer)
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let argv = std::iter::once(std::ffi::OsString::from("girsync"))
        .chain(args.into_iter().map(Into::into));
    let cli = Cli::try_parse_from(argv).expect("valid CLI args");
    for (key, value) in env {
        unsafe { std::env::set_var(key, value) };
    }
    let plan = plan(
        cli.log_level,
        cli.log_file_level,
        cli.log_file.as_deref(),
        cli.no_log_file,
    );
    unsafe { std::env::set_var("GIRSYNC_TEST_CACHE", "file") };
    let console = SharedBuffer::default();
    let (subscriber, _guard) = girsync::subscriber_for(&plan, console.clone());
    let out = tracing::subscriber::with_default(subscriber, || {
        girsync::run_captured(cli, &plan).expect("run_captured")
    });
    for (key, _) in env {
        unsafe { std::env::remove_var(key) };
    }
    (out, console)
}
