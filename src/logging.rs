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
//! - The file is filtered by `--log-file-level`, which defaults to `debug` in
//!   a debug build and `info` in a release build (see
//!   [`girpr_log::default_file_level`]). The two levels are separate because
//!   they answer separate questions: the console is for the person watching,
//!   the file is for the person reading it afterwards. A shipped binary keeps
//!   per-chunk lines off by default; `--log-file-level debug` (or `trace`)
//!   brings them back.
//! - The file is written when it is asked for: by `--log-file`, by
//!   `GIRPR_LOG_FILE`, or — in a **release** build — by the default, which is
//!   [`girpr_log::default_log_path`]'s timestamped path in a `logs/` directory beside
//!   the executable. A **debug** build writes no file unless it is asked to, and
//!   `--no-log-file` turns it off in any build.
//! - Data-plane output (`REPORT` / `PROGRESS` / `SUMMARY`) stays on stdout via
//!   [`mod@crate::report::emit`]; the file layer is plain text, not JSON, because
//!   it is meant to be read with `grep 'path=<file>'`.
//!
//! A log file that cannot be opened degrades to a warning on stderr rather than
//! failing the run, so [`init_tracing`] is infallible. [`flush_log`] is the
//! explicit last write: the binary calls it on every exit path after logging is
//! installed, so the queued tail is drained before the process ends.
//!
//! The file layer writes through a non-blocking appender: events are handed to
//! a worker thread, which writes them to the file. [`flush_log`] drops that
//! appender's worker guard on the way out, which drains the queue. That is the
//! durability promise: a normal exit keeps the whole log, and a failure that
//! reaches a flush path keeps it too; an exit that skips the flush, such as
//! `kill -9`, can lose the tail. The console layer is not buffered, because its
//! ordering and broken-pipe behavior are separate contracts.
//!
//! Because the worker writes off-thread, an external reader such as `tail -f`
//! sees a record only after the worker writes it. Log streaming is therefore
//! not real time: the file can lag by the queued records, and the file is the
//! durable record of the run rather than a live feed.

use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use girpr_log::install::{
    ConsoleSpec, FileFormat, SpanFields, emit_env_warnings, open_log_file, warn_subscriber_install,
};
use girpr_log::{Level, LogOptions, LogPlan};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::util::SubscriberInitExt as _;

/// This project's name: the default log file's stem, and the prefix of the
/// environment variables that answer for the flags.
pub const PROJECT: &str = "girpr";

/// The raw-stderr line that acknowledges a caller-named log file.
///
/// The definition lives in `girpr-cli` alongside the parse/error shim, shared
/// with the two tools; this re-export keeps `girpr::log_file_announcement` as
/// the path the binary and its tests use. [`init_tracing`] announces the path
/// into the file, while this line is emitted unfiltered on stderr for a caller
/// who named the path and may have silenced the console.
pub use girpr_cli::log_file_announcement;

/// The guard that keeps the non-blocking file writer alive for the run.
///
/// Dropping it drains the worker's queue and flushes the file. A static slot is
/// used rather than a local in `main` because the explicit flush seam
/// ([`flush_log`]) is what every exit path calls: a worker guard held in a local
/// would be dropped by `main`'s destructor on a normal return, but not on a
/// `process::exit`, and the seam has to cover both. `None` until logging is
/// installed, which is what makes [`flush_log`] safe on a path that never got
/// that far.
static WORKER_GUARD: Mutex<Option<WorkerGuard>> = Mutex::new(None);

/// Install the global tracing subscriber and return the resolved plan.
///
/// Scopes the console filter to the `girpr` target so third-party crates stay
/// quiet; the file layer is unscoped, because the file is the record of the
/// run and a record with a hole in it is not a record.
///
/// `console_level` is `--log-level`, `file_level` is `--log-file-level`
/// (`None` when the flag was not given, so the environment variable and then
/// the build's file default can answer for it), `log_file` is `--log-file`
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
    emit_env_warnings(warnings, "WARN", &env);
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

    // The shared stack builds the console and file layers; this binary keeps
    // its own text format, its warning destination and — crucially — its guard
    // policy. `open_or_warn` is passed in so the degradation path keeps both
    // its injectable sink and its exact bytes. A plan with no file never calls
    // it, and a file that cannot be opened leaves a console-only stack.
    let (subscriber, guard) = girpr_log::install::subscriber_for(
        &plan,
        ConsoleSpec {
            writer: std::io::stderr,
            filter: plan.console_filter(),
            // The old layer never chose: tracing-subscriber's default still
            // decides, so NO_COLOR keeps exactly the effect it had before.
            ansi: None,
        },
        FileFormat::Text,
        SpanFields::NONE,
        |path| open_or_warn(Some(path), warnings),
    );

    // A subscriber that is already installed would swallow the file layer
    // whole: the events would go to somebody else's subscriber and the log file
    // would stay empty, with nothing to say so. It cannot happen in this
    // binary's own `main`, and if it ever does, the run says so rather than
    // pretending to have logged. Only an accepted layer's guard is registered:
    // a rejected layer never receives an event, so retaining its guard would
    // keep a worker alive for a file that has nothing to write.
    match subscriber.try_init() {
        Ok(()) => install_guard(guard),
        Err(e) => {
            // `guard` is dropped here, draining the empty rejected file.
            warn_subscriber_install(warnings, e);
        }
    }

    girpr_log::announce_log_file!(plan, module_path!());
    plan
}

/// Open the plan's file, or warn and open nothing when it cannot be opened.
///
/// Split out of [`init_tracing_with`] so the degradation path is testable in
/// process without installing the one-per-process subscriber. The warning
/// bytes stay exactly as they were: a run must not end differently because of
/// where it was asked to log.
fn open_or_warn(path: Option<&Path>, warnings: &mut dyn Write) -> Option<std::fs::File> {
    let path = path?;
    match open_log_file(path) {
        Ok(f) => Some(f),
        Err(e) => {
            let _ = writeln!(
                warnings,
                "WARN disabling file logging ({}): {e}",
                path.display()
            );
            None
        }
    }
}

/// Register the guard the accepted subscriber's appender writes through.
///
/// Setting the slot replaces any previous registration: a replaced guard is
/// dropped, draining that appender's queue. Only the first subscriber install
/// can succeed, so a second accepted guard cannot exist in practice; replacing
/// is what keeps a stale guard from pinning a worker for the rest of the
/// process's life.
fn install_guard(guard: Option<WorkerGuard>) {
    *WORKER_GUARD.lock().unwrap_or_else(|e| e.into_inner()) = guard;
}

/// Flush the log file on the way out.
///
/// The file is the record of the run, so the caller drains it explicitly on
/// every path after [`init_tracing`] rather than trusting a `Drop` that an
/// abrupt exit would skip. Taking the guard out of the slot drops it, which
/// drains the worker's queue and flushes the file; a second call finds nothing
/// and is a no-op. Dropping the guard outside the lock keeps the drain (which
/// waits for the worker) from holding the slot. It is safe when no file was
/// opened and when logging was never installed.
pub fn flush_log() {
    let guard = WORKER_GUARD
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    drop(guard);
}

#[cfg(test)]
mod tests {
    use super::*;
    use girpr_log::EnvOptions;
    use girpr_tempdir::TempDir;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The stderr echo has one definition, and rendering it is pure.
    #[test]
    fn log_file_announcement_renders_the_named_path() {
        let scratch = TempDir::new("girpr_logging", "announcement");
        let named = scratch.join("nested").join("run.log");
        let plan = LogPlan::resolve(
            "girpr",
            LogOptions {
                log_file: Some(named.clone()),
                ..Default::default()
            },
            EnvOptions::default(),
        );
        assert_eq!(
            log_file_announcement(&plan).as_deref(),
            Some(format!("log file: {}", named.display()).as_str()),
            "the echo is the whole raw-stderr line"
        );

        // A bare name keeps its spelling: there is no parent to resolve.
        let bare = Path::new("girpr-bare.log");
        let bare_plan = LogPlan::resolve(
            "girpr",
            LogOptions {
                log_file: Some(bare.to_path_buf()),
                ..Default::default()
            },
            EnvOptions::default(),
        );
        assert_eq!(
            log_file_announcement(&bare_plan).as_deref(),
            Some("log file: girpr-bare.log")
        );

        // The default is not named; nobody asked for it, so nothing is echoed.
        let default = LogPlan::resolve("girpr", LogOptions::default(), EnvOptions::default());
        assert_eq!(log_file_announcement(&default), None);
    }

    /// A file that cannot be opened degrades to a warning, not a failed run.
    #[test]
    fn an_unopenable_log_file_warns_and_keeps_the_plan() {
        let scratch = TempDir::new("girpr_logging", "unopenable");
        let blocker = scratch.join("not_a_dir");
        std::fs::write(&blocker, b"x").expect("blocker file");
        let bad = blocker.join("run.log");

        let mut warnings = Vec::new();
        assert!(open_or_warn(Some(&bad), &mut warnings).is_none());
        let warnings = String::from_utf8(warnings).expect("the warnings are utf-8");
        assert!(
            warnings.contains("WARN disabling file logging"),
            "an unopenable log file degrades to a warning:\n{warnings}"
        );
        assert!(
            warnings.contains(bad.to_str().unwrap()),
            "the warning names the file it could not open:\n{warnings}"
        );

        // The decision did not move: an unopenable path is still the named
        // file, so the caller still gets the echo they asked for.
        let plan = LogPlan::resolve(
            "girpr",
            LogOptions {
                log_file: Some(bad.clone()),
                ..Default::default()
            },
            EnvOptions::default(),
        );
        assert_eq!(plan.file_path(), Some(bad.as_path()));
        assert_eq!(
            log_file_announcement(&plan).as_deref(),
            Some(format!("log file: {}", bad.display()).as_str())
        );
    }

    /// Serializes tests that touch the process-wide guard slot.
    ///
    /// The slot and the global subscriber are both process-wide, and the
    /// subscriber can only be installed once. The lock keeps the subscriber
    /// installs, guard installs, and flushes in these tests from interleaving.
    /// It is sufficient even when the test binary runs threaded, because the
    /// binary contains exactly one `try_init` (in the subscriber test), so no
    /// other test can install a subscriber ahead of it. Any future unit test
    /// that installs a subscriber must take this lock or install through a
    /// shared once-per-binary helper, or that guarantee is lost.
    static GUARD_SLOT_TEST_LOCK: Mutex<()> = Mutex::new(());

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

    /// Dropping the guard is what guarantees the queued tail reaches the file.
    ///
    /// The worker writes off-thread, so there is no deterministic point before
    /// the drop at which the file can be asserted empty. What is deterministic
    /// is that the drop drains the queue and flushes the writer.
    #[test]
    fn dropping_the_guard_flushes_the_queued_tail() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let flushes = Arc::new(AtomicUsize::new(0));
        let (mut writer, guard) = tracing_appender::non_blocking(RecordingWriter {
            bytes: Arc::clone(&bytes),
            flushes: Arc::clone(&flushes),
        });

        writer
            .write_all(b"the run's tail\n")
            .expect("queue the record");
        drop(guard);

        assert_eq!(
            &*bytes.lock().unwrap_or_else(|e| e.into_inner()),
            b"the run's tail\n",
            "the guard's drop writes the queued tail"
        );
        assert!(
            flushes.load(Ordering::Relaxed) >= 1,
            "the guard's drop flushes the writer"
        );
    }

    /// `flush_log` takes the guard out of the slot so a second call cannot
    /// flush anything, even if more bytes are queued in the meantime.
    #[test]
    fn flush_log_takes_the_guard_and_is_idempotent() {
        let _guard = GUARD_SLOT_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let flushes = Arc::new(AtomicUsize::new(0));
        let (mut writer, worker_guard) = tracing_appender::non_blocking(RecordingWriter {
            bytes: Arc::clone(&bytes),
            flushes: Arc::clone(&flushes),
        });

        install_guard(Some(worker_guard));
        writer
            .write_all(b"the run's tail\n")
            .expect("queue the record");

        flush_log();

        assert_eq!(
            &*bytes.lock().unwrap_or_else(|e| e.into_inner()),
            b"the run's tail\n",
            "the installed guard is drained by the exit flush"
        );

        // No guard is registered any more: further bytes stay queued and a
        // second flush has nothing to drain.
        writer
            .write_all(b"after the flush\n")
            .expect("the appender still accepts bytes");
        flush_log();
        assert_eq!(
            &*bytes.lock().unwrap_or_else(|e| e.into_inner()),
            b"the run's tail\n",
            "a second flush has no guard left to drain"
        );
    }

    /// Replacing the guard drops the old one (draining its appender) and leaves
    /// the new one as the target of the next flush.
    #[test]
    fn reinstalling_the_guard_flushes_the_old_and_keeps_the_new() {
        let _guard = GUARD_SLOT_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let first_bytes = Arc::new(Mutex::new(Vec::new()));
        let second_bytes = Arc::new(Mutex::new(Vec::new()));
        let (mut first_writer, first_guard) = tracing_appender::non_blocking(RecordingWriter {
            bytes: Arc::clone(&first_bytes),
            flushes: Arc::new(AtomicUsize::new(0)),
        });
        let (mut second_writer, second_guard) = tracing_appender::non_blocking(RecordingWriter {
            bytes: Arc::clone(&second_bytes),
            flushes: Arc::new(AtomicUsize::new(0)),
        });

        install_guard(Some(first_guard));
        first_writer.write_all(b"first\n").expect("queue the first");
        // Replacing drops the first guard, which drains its queue.
        install_guard(Some(second_guard));
        second_writer
            .write_all(b"second\n")
            .expect("queue the second");

        flush_log();

        assert_eq!(
            &*first_bytes.lock().unwrap_or_else(|e| e.into_inner()),
            b"first\n",
            "the replaced guard is drained when it is dropped"
        );
        assert_eq!(
            &*second_bytes.lock().unwrap_or_else(|e| e.into_inner()),
            b"second\n",
            "the new guard is the one the exit flush drains"
        );
    }

    /// A rejected install keeps the first install's guard, so the surviving
    /// subscriber keeps writing and its file keeps the run's tail.
    #[test]
    fn a_rejected_install_does_not_replace_the_registered_guard() {
        let _guard = GUARD_SLOT_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let first_dir = TempDir::new("girpr_logging", "first_install");
        let second_dir = TempDir::new("girpr_logging", "rejected_install");
        let first_path = first_dir.join("first.log");
        let second_path = second_dir.join("second.log");

        let first_plan = init_tracing(Level::Info, Some(Level::Debug), Some(&first_path), false);
        assert_eq!(first_plan.file_path(), Some(first_path.as_path()));
        assert!(
            WORKER_GUARD
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some(),
            "the accepted install registers its guard"
        );

        // The global subscriber is already installed, so this second call is
        // rejected: it still returns the plan it was asked for, but its guard is
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
        // The prefix is ours; the reason comes from the subscriber rejecting
        // the second install. `try_init` hides that rejection behind an opaque
        // error, so ask the public API for the same rejection as its concrete
        // `SetGlobalDefaultError` and expect its rendering: reworded upstream
        // text then still satisfies this assertion.
        let already_set: tracing::subscriber::SetGlobalDefaultError =
            tracing::subscriber::set_global_default(tracing::subscriber::NoSubscriber::default())
                .expect_err("the global default is already installed");
        assert!(
            warnings.contains(&format!(
                "WARN cannot install the tracing subscriber: {already_set}"
            )),
            "{warnings:?}"
        );

        assert!(
            WORKER_GUARD
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some(),
            "the rejected install leaves the first guard registered"
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
            "the first appender still receives the WARN record: {first:?}"
        );
        assert_eq!(
            std::fs::read(&second_path).expect("the rejected file was opened"),
            b"",
            "the rejected appender receives no events"
        );
    }
}
