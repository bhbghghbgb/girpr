//! Tracing setup: a human console layer plus a file layer.
//!
//! The *policy* — where the file goes, what each layer's default level is,
//! which environment variable answers for which flag — is the workspace's, and
//! lives in the `girpr-log` crate. It is shared with the two dev tools so
//! that `--log-file`, `<PROJECT>_LOG_FILE` and the default levels mean the
//! same thing in all three. What is here is what is *this* binary's: the two
//! layers, their formats, and the writer behind the file.
//!
//! Design (automation contract, see README):
//! - Console (stderr) is filtered by `--log-level`, optionally replaced by
//!   `RUST_LOG`.
//! - The file is filtered by `--log-file-level`, which defaults to `debug`.
//!   The two levels are separate because they answer separate questions: the
//!   console is for the person watching, the file is for the person reading
//!   it afterwards.
//! - The file is written when it is asked for: by `--log-file`, by
//!   `GIRPR_LOG_FILE`, or — in a **release** build — by the default, which is
//!   [`default_log_path`]'s timestamped path in a `logs/` directory beside the
//!   executable. A **debug** build writes no file unless it is asked to, and
//!   `--no-log-file` turns it off in any build.
//! - Data-plane output (`REPORT` / `PROGRESS` / `SUMMARY`) stays on stdout via
//!   [`mod@crate::report::emit`]; the file layer is plain text, not JSON, because
//!   it is meant to be read with `grep 'path=<file>'`.
//!
//! A log file that cannot be opened degrades to a warning on stderr rather than
//! failing the run, so [`init_tracing`] is infallible. [`flush_log`] is the
//! explicit last write: the binary calls it on every exit path after logging is
//! installed, so a buffered sink cannot lose the tail of the run.

use std::io::Write;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use girpr_log::{Level, LogOptions, LogPlan};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;

/// This project's name: the default log file's stem, and the prefix of the
/// environment variables that answer for the flags.
pub const PROJECT: &str = "girpr";

/// The file layer's writer, shared between the subscriber and [`flush_log`].
type FileSink = Arc<Mutex<Box<dyn Write + Send + Sync>>>;

/// The sink [`flush_log`] reaches, set once by [`init_tracing`].
///
/// `None` until logging is installed, which is what makes [`flush_log`] safe on
/// a path that never got that far.
static FILE_SINK: OnceLock<FileSink> = OnceLock::new();

/// A `MakeWriter` that hands `tracing` the shared file sink.
struct SharedWriter(FileSink);

/// One event's exclusive use of the sink.
struct SharedWriterGuard<'a>(MutexGuard<'a, Box<dyn Write + Send + Sync>>);

impl Write for SharedWriterGuard<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl<'a> MakeWriter<'a> for SharedWriter {
    type Writer = SharedWriterGuard<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        // A panic while holding the lock must not silence the log's tail: the
        // writer is an append-only file, whose state is still usable.
        SharedWriterGuard(self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// Where the log file lives when neither `--log-file` nor `GIRPR_LOG_FILE`
/// named one: `<exe-dir>/logs/girpr_<timestamp>.log`, falling back to the
/// current directory when the exe path is unavailable.
///
/// The timestamp keeps concurrent runs from interleaving but also makes the
/// name unguessable to the caller, which is why the flag and the environment
/// variable exist.
#[cfg(test)]
pub fn default_log_path() -> PathBuf {
    girpr_log::default_log_path(PROJECT)
}

/// Install the global tracing subscriber and return the resolved plan.
///
/// Scopes the console filter to the `girpr` target so third-party crates stay
/// quiet; the file layer is unscoped, because the file is the record of the
/// run and a record with a hole in it is not a record.
///
/// `console_level` is `--log-level`, `file_level` is `--log-file-level`
/// (`None` when the flag was not given, so the environment variable and then
/// the `debug` default can answer for it), `log_file` is `--log-file`
/// (`None` for the timestamped default), and `no_log_file` is `--no-log-file`.
///
/// The returned plan is what was *asked for*, not what was opened: a file
/// that could not be opened is a stderr warning, and this stays infallible so
/// an unwritable log never costs a run its exit code.
pub fn init_tracing(
    console_level: Level,
    file_level: Option<Level>,
    log_file: Option<&Path>,
    no_log_file: bool,
) -> LogPlan {
    let env = girpr_log::env_options(PROJECT);
    for warning in &env.warnings {
        eprintln!("WARN {warning}");
    }
    let plan = LogPlan::resolve(
        PROJECT,
        LogOptions {
            console_level,
            file_level,
            log_file: log_file.map(Path::to_path_buf),
            no_log_file,
        },
        env,
    );

    let console_fallback = format!("{PROJECT}={}", console_level.as_str());
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(girpr_log::filter(&plan.console_filter(), &console_fallback));

    // A plan with no file has no file sink: a debug build that was not asked
    // for a file, or any build that was told `--no-log-file`. The sink then
    // writes nowhere, so the run leaves nothing behind and nothing has to be
    // cleaned up afterwards.
    let file_writer: FileSink = Arc::new(Mutex::new(match plan.file_path() {
        None => Box::new(std::io::sink()) as Box<dyn Write + Send + Sync>,
        Some(path) => match open_log_file(path) {
            Ok(f) => Box::new(f),
            Err(e) => {
                eprintln!("WARN disabling file logging ({}): {e}", path.display());
                Box::new(std::io::sink())
            }
        },
    }));
    // The subscriber owns one handle; `flush_log` owns the other, so the tail
    // can still be written after the subscriber has no more events to render.
    let _ = FILE_SINK.set(Arc::clone(&file_writer));
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(SharedWriter(file_writer))
        .with_ansi(false)
        .with_filter(girpr_log::filter(&plan.file_filter(), &plan.file_filter()));

    // A subscriber that is already installed would swallow the file layer
    // whole: the events would go to somebody else's subscriber and the log file
    // would stay empty, with nothing to say so. It cannot happen in this
    // binary's own `main`, and if it ever does, the run says so rather than
    // pretending to have logged.
    if let Err(e) = tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .try_init()
    {
        eprintln!("WARN cannot install the tracing subscriber: {e}");
    }

    match plan.file_path() {
        Some(path) => tracing::info!("log file: {}", path.display()),
        None => tracing::info!("log file: none"),
    }
    plan
}

/// Flush the log file on the way out.
///
/// The file is the record of the run, so the caller flushes it explicitly on
/// every path after [`init_tracing`] rather than trusting a `Drop` that an
/// abrupt exit would skip. It is safe when no file was opened — the sink writes
/// nowhere — and when logging was never installed, in which case there is
/// nothing to flush.
///
/// A flush failure is reported once on stderr and never changes the exit code:
/// the run already earned its code before the log was written down.
pub fn flush_log() {
    let Some(sink) = FILE_SINK.get() else {
        return;
    };
    if let Err(e) = flush_sink(sink) {
        static REPORTED: AtomicBool = AtomicBool::new(false);
        if !REPORTED.swap(true, Ordering::Relaxed) {
            eprintln!("WARN cannot flush the log file: {e}");
        }
    }
}

fn flush_sink(sink: &FileSink) -> std::io::Result<()> {
    sink.lock().unwrap_or_else(|e| e.into_inner()).flush()
}

/// Open the log file for appending, creating the parent directory if needed.
fn open_log_file(path: &Path) -> std::io::Result<std::fs::File> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
    std::fs::create_dir_all(dir)?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_file_is_named_for_the_project() {
        let path = default_log_path();
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("girpr_"), "{name}");
        assert!(name.ends_with(".log"), "{name}");
        assert_eq!(
            path.parent()
                .and_then(Path::file_name)
                .and_then(|s| s.to_str()),
            Some("logs")
        );
    }
}
