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
//! The file is **always** written: by default to
//! `<exe-dir>/logs/gircap_<timestamp>.log`, or wherever `--log-file` or
//! `GIRCAP_LOG_FILE` says. The console filter is deliberately *not* copied to
//! the file sink — the two are separate questions, and the file's default
//! (`debug`) is one step louder than the console's (`info`) for exactly that
//! reason. An artifact run's narration is the evidence for why a version pair
//! was chosen and what each request cost, and it must survive a
//! `--log-level warn` console.
//!
//! The *policy* — where the file goes, which level each sink gets, which
//! environment variable answers for which flag — is the workspace's and lives
//! in the `girpr-log` crate, so that `girpr` and `girsync` answer the same
//! questions the same way.

use std::fs::OpenOptions;
use std::path::Path;
use std::sync::{Arc, Mutex};

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
pub fn init(console_level: Level, file_level: Option<Level>, log_file: Option<&Path>) -> LogGuard {
    use tracing_subscriber::fmt::layer;

    let env = girpr_log::env_options(PROJECT);
    for warning in &env.warnings {
        eprintln!("gircap: {warning}");
    }
    let plan = LogPlan::resolve(
        PROJECT,
        LogOptions {
            console_level,
            file_level,
            log_file: log_file.map(Path::to_path_buf),
        },
        env,
    );

    let console_spec = plan.console_filter();
    let console_fallback = format!("{PROJECT}={}", console_level.as_str());
    let stderr_layer = layer()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_target(true);

    // The file sink. A log file that cannot be opened is reported and then
    // skipped: the capture is the thing that was asked for, and losing its
    // narration is a lesser harm than losing it. Everything before this point
    // happens with no subscriber installed, so the failure is a raw stderr
    // write.
    let file = match open_log_file(&plan.file) {
        Err(e) => {
            eprintln!(
                "gircap: cannot open the log file {}: {e}",
                plan.file.display()
            );
            None
        }
        Ok(writer) => Some(writer),
    };

    let mut worker = None;
    let file_layer = file.map(|w| {
        // JSON lines, not the human format: this file is the evidence for why a
        // version pair was chosen and what each request cost, and a reader parsing
        // it should not have to strip a timestamp and a target off the front of
        // every line.
        let (non_blocking, guard) = tracing_appender::non_blocking(w);
        worker = Some(guard);
        layer()
            .json()
            .with_ansi(false)
            .with_target(true)
            .with_writer(non_blocking)
            .with_filter(girpr_log::filter(&plan.file_filter(), &plan.file_filter()))
    });

    let _ = tracing_subscriber::registry()
        .with(stderr_layer.with_filter(girpr_log::filter(&console_spec, &console_fallback)))
        .with(file_layer)
        .try_init();

    let guard = LogGuard {
        plan: plan.clone(),
        _file: worker,
    };

    tracing::info!(
        version = crate::VERSION,
        log_level = %console_level,
        console_filter = %console_spec,
        file_level = %plan.file_level,
        log_file = %plan.file.display(),
        "gircap start"
    );
    guard
}

fn open_log_file(path: &Path) -> std::io::Result<AppendFile> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    Ok(AppendFile(Arc::new(Mutex::new(file))))
}

/// The `User-Agent` every request carries.
///
/// A generic Unity player string rather than one naming this tool: the API is a
/// third party, and a request should not identify the client that made it. It is
/// still sent as a real header rather than left to the client default, so a
/// captured `api/requests.ndjson` records exactly what the CDN saw.
pub const USER_AGENT: &str = "UnityPlayer/2019.4.40f1 (UnityWebRequest/1.0, libcurl/7.80.0-DEV)";

/// The [`USER_AGENT`] string, owned where a [`String`] is wanted.
pub fn user_agent() -> String {
    USER_AGENT.to_string()
}

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

    #[test]
    fn a_log_file_writer_appends_and_flushes() {
        let dir = crate::testtemp::TempDir::new("gircap-log-test", "appends_and_flushes");
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
