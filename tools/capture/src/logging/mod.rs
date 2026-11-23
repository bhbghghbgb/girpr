//! stderr narration, via `tracing`.
//!
//! This module owns stderr; [`crate::report`] owns stdout. Nothing here prints a
//! record, and nothing there logs one, so `--output json` is pipeable with no
//! filtering.
//!
//! Two sinks, one span tree:
//!
//! - a human-readable layer on stderr, filtered by `--log-level` (or
//!   `RUST_LOG`, which replaces it);
//! - a JSON-lines file layer, filtered by `--log-file-level`.
//!
//! The file is written when it is asked for: by `--log-file`, by
//! `GIRCAP_LOG_FILE`, or — in a **release** build — by the default, which is
//! `<exe-dir>/logs/gircap_<timestamp>.log`. A **debug** build writes no file
//! unless it is asked to, and `--no-log-file` turns it off in any build. The
//! console filter is deliberately *not* copied to the file sink — the two are
//! separate questions. In a debug build the file's default (`debug`) is one
//! step louder than the console's (`info`); in a release build it starts at
//! `info` and `--log-file-level debug` (or `trace`) opts back into the louder
//! per-event lines. An artifact run's narration is the evidence for why a
//! version pair was chosen and what each request cost, and it must survive a
//! `--log-level warn` console.
//!
//! The *policy* — where the file goes, which level each sink gets, which
//! environment variable answers for which flag — is the workspace's and lives
//! in the `girpr-log` crate, so that `girpr` and `girsync` answer the same
//! questions the same way.

use std::path::Path;
use std::sync::{Arc, Mutex};

use girpr_log::install::{
    emit_env_warnings, file_sink, open_log_file as open_shared_log_file, warn_subscriber_install,
};
use girpr_log::{LogOptions, LogPlan};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::Layer as _;
use tracing_subscriber::fmt::writer::MakeWriter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// The verbosity names, re-exported so that `crate::logging::Level` is the one
/// type the CLI and the log layer share.
pub use girpr_log::Level;

/// This project's name: the default log file's stem, and the prefix of the
/// environment variables that answer for the flags.
pub const PROJECT: &str = "gircap";

/// Keeps the non-blocking file sink's buffer alive for the length of the run.
///
/// Dropping this flushes the tail of the JSON-lines log. Without it the last few
/// hundred lines — the ones describing the run's outcome — would be lost, which is
/// the one part of the file a reader most wants.
pub struct LogGuard {
    plan: LogPlan,
    _file: Option<WorkerGuard>,
}

impl LogGuard {
    /// The resolved policy: where the log file went, and at what level each
    /// sink is running.
    pub fn plan(&self) -> &LogPlan {
        &self.plan
    }

    /// Flushes the file sink and forgets it. Idempotent.
    pub fn finish(&mut self) {
        self._file.take();
    }
}

/// A [`MakeWriter`] over one append-opened file, serialised across threads.
#[derive(Clone)]
struct AppendFile(Arc<Mutex<std::fs::File>>);

impl std::io::Write for AppendFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.locked().and_then(|mut f| f.write(buf))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.locked().and_then(|mut f| f.flush())
    }
}

impl AppendFile {
    /// The locked handle, naming the one way this lock can fail.
    fn locked(&self) -> std::io::Result<std::sync::MutexGuard<'_, std::fs::File>> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("log file mutex poisoned"))
    }
}

impl<'a> MakeWriter<'a> for AppendFile {
    type Writer = AppendFile;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Install the subscriber. Call once, before any work.
///
/// Returns a [`LogGuard`] that must be held for the length of the run; dropping
/// it flushes the file sink. Errors on the file sink are reported to stderr and
/// then ignored — a log file that cannot be opened must not stop a capture,
/// because the capture itself is the thing that was asked for.
///
/// **Each sink filters itself.** There is no filter on the registry: a filter
/// there would be a *ceiling* over both sinks, and the whole point of the two
/// levels is that the file is louder than the console. The stack's level hint
/// is the louder of the two, so a `debug` event is still worth building when
/// only the file wants it.
pub fn init(
    console_level: Level,
    file_level: Option<Level>,
    log_file: Option<&Path>,
    no_log_file: bool,
) -> LogGuard {
    use tracing_subscriber::fmt::layer;

    let env = girpr_log::env_options(PROJECT);
    emit_env_warnings(&mut std::io::stderr(), "gircap:", &env);
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

    let console_filter = plan.console_filter();
    // Kept as text for the start-up event below; the filter itself is moved
    // into the subscriber layer.
    let console_filter_text = console_filter.to_string();
    let stderr_layer = layer()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_target(true);

    // The file sink. A plan with no file has no sink at all: a debug build
    // that was not asked for a file, or any build that was told
    // `--no-log-file`. A log file that cannot be opened is reported and then
    // skipped: the capture is the thing that was asked for, and losing its
    // narration is a lesser harm than losing it. Everything before this point
    // happens with no subscriber installed, so the failure is a raw stderr
    // write.
    let file = match plan.file_path() {
        None => None,
        Some(path) => match open_log_file(path) {
            Err(e) => {
                eprintln!("gircap: cannot open the log file {}: {e}", path.display());
                None
            }
            Ok(writer) => Some(writer),
        },
    };

    let mut worker = None;
    let file_layer = file.map(|w| {
        // JSON lines, not the human format: this file is the evidence for why a
        // version pair was chosen and what each request cost, and a reader parsing
        // it should not have to strip a timestamp and a target off the front of
        // every line.
        let (non_blocking, guard, filter) = file_sink(w, &plan);
        worker = Some(guard);
        layer()
            .json()
            .with_ansi(false)
            .with_target(true)
            .with_writer(non_blocking)
            .with_filter(filter)
    });

    // A subscriber that is already installed would swallow the file layer
    // whole: the events would go to somebody else's subscriber and the log file
    // would stay empty, with nothing to say so. It cannot happen in this
    // binary's own `main`, and if it ever does, the run says so rather than
    // pretending to have logged.
    if let Err(e) = tracing_subscriber::registry()
        .with(stderr_layer.with_filter(console_filter))
        .with(file_layer)
        .try_init()
    {
        warn_subscriber_install(&mut std::io::stderr(), e);
    }

    let guard = LogGuard {
        plan: plan.clone(),
        _file: worker,
    };

    // The file's fate, said out loud and said the same way in all three
    // projects: `log file: <path>`, or `log file: none` when there is no
    // file. A run that leaves nothing behind should say so, rather than
    // saying nothing and leaving the reader to guess.
    girpr_log::announce_log_file!(plan, module_path!());

    tracing::info!(
        version = crate::VERSION,
        log_level = %console_level,
        console_filter = %console_filter_text,
        file_level = %plan.file_level,
        log_file = %plan.file_path().map(|p| p.display().to_string()).unwrap_or_else(|| "none".into()),
        "gircap start"
    );
    guard
}

/// Opening the file is shared; this only adds capture's thread-safe
/// `MakeWriter` wrapper around the handle the shared opener returns.
fn open_log_file(path: &Path) -> std::io::Result<AppendFile> {
    let file = open_shared_log_file(path)?;
    Ok(AppendFile(Arc::new(Mutex::new(file))))
}

/// The `User-Agent` every request carries.
///
/// Owned by `gircap-http` (the fetcher sends it) and re-exported here so this
/// module's own tests and callers keep the same path. The string is unchanged:
/// a captured `api/requests.ndjson` records exactly what the CDN saw.
pub use gircap_http::{USER_AGENT, user_agent};

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn the_user_agent_is_the_generic_unity_player_string() {
        assert_eq!(user_agent(), USER_AGENT);
        assert_eq!(
            USER_AGENT,
            "UnityPlayer/2019.4.40f1 (UnityWebRequest/1.0, libcurl/7.80.0-DEV)"
        );
    }

    /// The project's name becomes the prefix of the log environment variables:
    /// `GIRCAP_LOG_FILE` and `GIRCAP_LOG_FILE_LEVEL`. `girpr_log::env_options`
    /// derives them by upper-casing `PROJECT`; this pins the one spelling that
    /// is this binary's responsibility. The shared crate covers the file-name
    /// shape and the precedence of those variables.
    #[test]
    fn the_project_prefix_names_the_log_environment_variables() {
        assert_eq!(PROJECT.to_ascii_uppercase(), "GIRCAP");
    }

    #[test]
    fn a_log_file_writer_appends_and_flushes() {
        let dir = girpr_tempdir::TempDir::new("gircap-log-test", "appends_and_flushes");
        let path = dir.join("nested").join("run.log");

        let w = open_log_file(&path).expect("creates the parent directory");
        let mut w = w;
        w.write_all(b"one\n").unwrap();
        // A second handle appends rather than truncating: two runs against one
        // log path must not erase each other.
        drop(w);
        let mut w2 = open_log_file(&path).unwrap();
        w2.write_all(b"two\n").unwrap();
        w2.flush().unwrap();
        drop(w2);

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "one\ntwo\n",
            "--log-file must append"
        );
    }
}
