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
//!
//! The file sink is buffered for the run and explicitly flushed at exit and on
//! the fatal paths. That is the durability promise: a normal exit keeps the
//! whole log, and a failure that reaches a flush path keeps it too; an exit
//! that skips the flush, such as `kill -9`, can lose the tail. The console
//! layer is not buffered, because its ordering and broken-pipe behavior are
//! separate contracts.
//!
//! Because the file sink holds up to [`FILE_BUFFER_CAPACITY`] bytes before a
//! write, an external reader such as `tail -f` sees a record only when that
//! buffer fills or when [`flush_log`] runs. Log streaming is therefore not
//! real time: the file can lag by up to the buffer size, and the file is the
//! durable record of the run rather than a live feed.

use std::io::{BufWriter, Write};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use girpr_log::{Level, LogOptions, LogPlan};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;

/// This project's name: the default log file's stem, and the prefix of the
/// environment variables that answer for the flags.
pub const PROJECT: &str = "girpr";

/// How much of the file log is held in memory between writes.
///
/// One buffer per run, not per event: the file layer renders many small
/// records, and batching them cuts both the write syscalls and the time the
/// shared sink's mutex is held under load.
const FILE_BUFFER_CAPACITY: usize = 64 * 1024;

/// The file layer's writer, shared between the subscriber and [`flush_log`].
///
/// The box holds a `BufWriter<File>` when a file was opened, and
/// [`std::io::sink`] when the plan has no file; the sink stays unbuffered
/// because buffering nothing buys nothing.
type FileSink = Arc<Mutex<Box<dyn Write + Send + Sync>>>;

/// The sink [`flush_log`] reaches, registered by a successful
/// [`init_tracing`].
///
/// `None` until logging is installed, which is what makes [`flush_log`] safe
/// on a path that never got that far. The slot holds the one sink the
/// installed subscriber writes through: the global subscriber can be installed
/// only once, so a later [`init_tracing`] is rejected and its file is dropped
/// rather than retained as a handle that could never be written to.
static FILE_SINK: Mutex<Option<FileSink>> = Mutex::new(None);

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
    init_tracing_with(
        &mut std::io::stderr(),
        console_level,
        file_level,
        log_file,
        no_log_file,
    )
}

/// The body of [`init_tracing`], with the warning destination injectable.
///
/// The install warning goes straight to stderr rather than through an event:
/// the subscriber that rejected the install must not be able to filter the
/// report of its own rejection. A test reads that warning through its own
/// buffer instead of capturing the process's stderr.
fn init_tracing_with(
    warnings: &mut dyn Write,
    console_level: Level,
    file_level: Option<Level>,
    log_file: Option<&Path>,
    no_log_file: bool,
) -> LogPlan {
    let env = girpr_log::env_options(PROJECT);
    for warning in &env.warnings {
        let _ = writeln!(warnings, "WARN {warning}");
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
                let _ = writeln!(
                    warnings,
                    "WARN disabling file logging ({}): {e}",
                    path.display()
                );
                Box::new(std::io::sink())
            }
        },
    }));
    // The subscriber owns one handle; `flush_log` gets the other once the
    // subscriber accepts it, so the tail can still be written after the
    // subscriber has no more events to render.
    let flush_sink = Arc::clone(&file_writer);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(SharedWriter(file_writer))
        .with_ansi(false)
        .with_filter(girpr_log::filter(&plan.file_filter(), &plan.file_filter()));

    // A subscriber that is already installed would swallow the file layer
    // whole: the events would go to somebody else's subscriber and the log file
    // would stay empty, with nothing to say so. It cannot happen in this
    // binary's own `main`, and if it ever does, the run says so rather than
    // pretending to have logged. Only an accepted sink is registered: a
    // rejected layer never receives an event, so retaining its file would keep
    // a handle that has nothing to flush.
    match tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .try_init()
    {
        Ok(()) => install_sink(flush_sink),
        Err(e) => {
            let _ = writeln!(warnings, "WARN cannot install the tracing subscriber: {e}");
        }
    }

    match plan.file_path() {
        Some(path) => tracing::info!("log file: {}", path.display()),
        None => tracing::info!("log file: none"),
    }
    plan
}

/// Register the sink the accepted subscriber writes through.
///
/// Setting the slot replaces any previous registration. Only the first
/// subscriber install can succeed, so a second accepted sink cannot exist;
/// replacing is what keeps a later registration from pinning an unreachable
/// file handle for the rest of the process's life.
fn install_sink(sink: FileSink) {
    *FILE_SINK.lock().unwrap_or_else(|e| e.into_inner()) = Some(sink);
}

/// Flush the log file on the way out.
///
/// The file is the record of the run, so the caller flushes it explicitly on
/// every path after [`init_tracing`] rather than trusting a `Drop` that an
/// abrupt exit would skip. For a buffered file this is what pushes the last
/// records out of the buffer; for the sink it is a no-op. It is safe when no
/// file was opened and when logging was never installed, in which case there is
/// nothing to flush.
///
/// A flush failure is reported once on stderr and never changes the exit code:
/// the run already earned its code before the log was written down. The
/// registry holds at most one sink, so the report is that sink's failure.
pub fn flush_log() {
    let sink = FILE_SINK.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(sink) = sink
        && let Err(e) = flush_sink(&sink)
    {
        static REPORTED: AtomicBool = AtomicBool::new(false);
        report_flush_failure(&e, &REPORTED, &mut std::io::stderr());
    }
}

fn flush_sink(sink: &FileSink) -> std::io::Result<()> {
    sink.lock().unwrap_or_else(|e| e.into_inner()).flush()
}

/// Report a flush failure at most once, without letting the report itself fail
/// the run.
fn report_flush_failure(error: &std::io::Error, reported: &AtomicBool, stderr: &mut dyn Write) {
    if !reported.swap(true, Ordering::Relaxed) {
        let _ = writeln!(stderr, "WARN cannot flush the log file: {error}");
    }
}

/// Open the log file for appending, creating the parent directory if needed.
///
/// The file is buffered so the file layer's many small records become few
/// writes. [`flush_log`] is what empties the buffer at exit; a dropped
/// [`BufWriter`] would flush on its own, but the process exits through paths
/// that skip destructors, which is why the explicit flush exists.
fn open_log_file(path: &Path) -> std::io::Result<BufWriter<std::fs::File>> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
    std::fs::create_dir_all(dir)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    Ok(BufWriter::with_capacity(FILE_BUFFER_CAPACITY, file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use girpr_tempdir::TempDir;
    use std::sync::atomic::AtomicUsize;

    /// Serializes tests that touch the process-wide sink registry.
    ///
    /// The registry and the global subscriber are both process-wide, and the
    /// subscriber can only be installed once. The lock keeps the subscriber
    /// installs, sink installs, and flushes in these tests from interleaving.
    /// It is sufficient even when the test binary runs threaded, because the
    /// binary contains exactly one `try_init` (in the subscriber test), so no
    /// other test can install a subscriber ahead of it. Any future unit test
    /// that installs a subscriber must take this lock or install through a
    /// shared once-per-binary helper, or that guarantee is lost.
    static SINK_REGISTRY_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// A writer that records everything it is given and counts its flushes.
    struct RecordingWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
        flushes: Arc<AtomicUsize>,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.flushes.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    /// A writer whose only observable act is the flush.
    struct FlushRecorder {
        flushes: Arc<AtomicUsize>,
    }

    impl Write for FlushRecorder {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.flushes.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    fn recording_sink(flushes: &Arc<AtomicUsize>) -> FileSink {
        Arc::new(Mutex::new(Box::new(FlushRecorder {
            flushes: Arc::clone(flushes),
        }) as Box<dyn Write + Send + Sync>))
    }

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

    #[test]
    fn the_file_sink_keeps_records_in_the_buffer_until_the_flush() {
        let dir = TempDir::new("girpr_logging", "buffered_file");
        let path = dir.join("run.log");
        let sink: FileSink = Arc::new(Mutex::new(Box::new(
            open_log_file(&path).expect("open the log file"),
        ) as Box<dyn Write + Send + Sync>));

        sink.lock()
            .unwrap_or_else(|e| e.into_inner())
            .write_all(b"the run's tail\n")
            .expect("write into the buffer");

        assert_eq!(
            std::fs::read(&path).expect("the file exists"),
            b"",
            "a buffered record has not reached the file yet"
        );

        // `flush_log` is the exit seam and reaches exactly this call.
        flush_sink(&sink).expect("flush the sink");

        assert_eq!(
            std::fs::read(&path).expect("read the flushed file"),
            b"the run's tail\n",
            "the flush writes the buffered tail"
        );
    }

    #[test]
    fn flush_log_writes_the_buffered_output_of_an_installed_sink() {
        let _guard = SINK_REGISTRY_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let flushes = Arc::new(AtomicUsize::new(0));
        let writer = RecordingWriter {
            bytes: Arc::clone(&bytes),
            flushes: Arc::clone(&flushes),
        };
        let sink: FileSink = Arc::new(Mutex::new(
            Box::new(BufWriter::with_capacity(64, writer)) as Box<dyn Write + Send + Sync>
        ));

        install_sink(Arc::clone(&sink));
        sink.lock()
            .unwrap_or_else(|e| e.into_inner())
            .write_all(b"the run's tail\n")
            .expect("write into the buffer");
        assert_eq!(
            &*bytes.lock().unwrap_or_else(|e| e.into_inner()),
            b"",
            "the record is still in the buffer before the exit flush"
        );

        flush_log();

        assert_eq!(
            &*bytes.lock().unwrap_or_else(|e| e.into_inner()),
            b"the run's tail\n",
            "the installed sink receives the buffered tail"
        );
        assert_eq!(flushes.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn reinstalling_replaces_the_sink_instead_of_retaining_it() {
        let _guard = SINK_REGISTRY_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let first = Arc::new(AtomicUsize::new(0));
        let second = Arc::new(AtomicUsize::new(0));
        let first_sink = recording_sink(&first);

        install_sink(Arc::clone(&first_sink));
        install_sink(recording_sink(&second));
        flush_log();

        assert_eq!(
            first.load(Ordering::Relaxed),
            0,
            "the replaced sink is no longer registered"
        );
        assert_eq!(second.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_rejected_install_does_not_replace_the_registered_sink() {
        let _guard = SINK_REGISTRY_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let first_dir = TempDir::new("girpr_logging", "first_install");
        let second_dir = TempDir::new("girpr_logging", "rejected_install");
        let first_path = first_dir.join("first.log");
        let second_path = second_dir.join("second.log");

        let first_plan = init_tracing(Level::Info, Some(Level::Debug), Some(&first_path), false);
        assert_eq!(first_plan.file_path(), Some(first_path.as_path()));
        let registered = FILE_SINK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .expect("the accepted install registers its sink");

        // The global subscriber is already installed, so this second call is
        // rejected: it still returns the plan it was asked for, but its sink is
        // never registered.
        let mut warnings = Vec::new();
        let second_plan = init_tracing_with(
            &mut warnings,
            Level::Info,
            Some(Level::Debug),
            Some(&second_path),
            false,
        );
        assert_eq!(second_plan.file_path(), Some(second_path.as_path()));
        let warnings = String::from_utf8(warnings).expect("the warnings are utf-8");
        assert!(
            warnings.contains("WARN cannot install the tracing subscriber"),
            "{warnings:?}"
        );
        // The prefix is ours; the reason is `tracing`'s. Pin only the stable
        // claim that the global default is already set, not its exact wording.
        assert!(warnings.contains("already been set"), "{warnings:?}");

        let after = FILE_SINK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .expect("the rejected install leaves the first sink registered");
        assert!(
            Arc::ptr_eq(&registered, &after),
            "the rejected install does not replace the first sink"
        );

        // The surviving subscriber is still the one that renders records into
        // the first file, so a WARN emitted now must reach it.
        tracing::warn!("after the rejected install");
        flush_log();

        let first = std::fs::read_to_string(&first_path).expect("the first log is readable");
        assert!(
            first
                .lines()
                .any(|line| line.contains("WARN") && line.contains("after the rejected install")),
            "the first sink still receives the WARN record: {first:?}"
        );
        assert_eq!(
            std::fs::read(&second_path).expect("the rejected file was opened"),
            b"",
            "the rejected sink receives no events and is not flushed"
        );
    }

    #[test]
    fn a_flush_failure_is_reported_once() {
        let reported = AtomicBool::new(false);
        let error = std::io::Error::other("disk full");
        let mut stderr = Vec::new();

        report_flush_failure(&error, &reported, &mut stderr);
        report_flush_failure(&error, &reported, &mut stderr);

        let message = String::from_utf8(stderr).expect("the report is utf-8");
        assert_eq!(
            message.matches("WARN cannot flush the log file").count(),
            1,
            "{message}"
        );
        assert!(message.contains("disk full"), "{message}");
    }
}
