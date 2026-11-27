//! Tracing setup: human console layer plus a JSON file layer.
//!
//! This module owns **stderr** only. The data plane — `MISSING`/`COPY`/`SUMMARY`
//! and friends — is stdout, and it goes through [`crate::report::Report`], which
//! `--output` chooses between a human line and a JSON object. Keeping the two
//! apart is what lets `--output json` hand a caller a clean stream of JSON
//! records with no log chatter mixed into it, while stderr stays the narration.
//!
//! Design:
//! - Console (stderr, human-readable) is filtered by `--log-level`, or by
//!   `RUST_LOG` when it is set.
//! - The file (JSON) is filtered by `--log-file-level`. The two levels are
//!   separate: the console is for the person watching, the file is for the
//!   person reading it afterwards.
//! - The file is written when it is asked for: by `--log-file`, by
//!   `GIRSYNC_LOG_FILE`, or — in a **release** build — by the default, which is
//!   `<exe-dir>/logs/girsync_<timestamp>.log`. A **debug** build writes no
//!   file unless it is asked to, and `--no-log-file` turns it off in any
//!   build.
//! - Nothing is printed raw: every diagnostic is a `tracing` event with named
//!   fields, so the JSON log and the console are the same records rendered twice.
//! - Per-command spans carry config so every event inside is correlated.
//! - Data-plane records reach this channel too: `Report::emit` logs each one as a
//!   `debug` event with the same `event` name and fields it printed, so a log file
//!   carries the answer without stdout having been captured.
//!
//! The *policy* — where the file goes, which level each sink gets, which
//! environment variable answers for which flag — is the workspace's and lives
//! in the `girpr-log` crate, so that `girpr` and `gircap` answer the same
//! questions the same way.

use std::path::Path;
use tracing::info;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, util::SubscriberInitExt as _};

use girpr_log::install::{
    ConsoleSpec, FileFormat, SpanFields, emit_env_warnings, open_log_file, warn_subscriber_install,
};

pub use girpr_log::Level;

/// This project's name: the default log file's stem, and the prefix of the
/// environment variables that answer for the flags.
pub const PROJECT: &str = "girsync";

/// Keeps the non-blocking file sink's buffer alive for the length of the run.
///
/// Dropping this flushes the tail of the JSON log. Without it the last few
/// hundred lines — the ones describing the run's outcome — would be lost, which
/// is the one part of the file a reader most wants.
pub struct LogGuard {
    plan: girpr_log::LogPlan,
    _file: Option<tracing_appender::non_blocking::WorkerGuard>,
}

impl LogGuard {
    /// The resolved policy: where the log file went, and at what level each
    /// sink is running.
    pub fn plan(&self) -> &girpr_log::LogPlan {
        &self.plan
    }
}

/// Resolve the log policy for a run, without installing anything.
///
/// The path, the levels and the precedence live in the shared `girpr-log`
/// crate; this is the one call a caller makes to get them.
pub fn plan(
    console_level: Level,
    file_level: Option<Level>,
    log_file: Option<&Path>,
    no_log_file: bool,
) -> girpr_log::LogPlan {
    let env = girpr_log::env_options(PROJECT);
    emit_env_warnings(&mut std::io::stderr(), "WARN", &env);
    girpr_log::LogPlan::resolve(
        PROJECT,
        girpr_log::LogOptions {
            console_level,
            file_level,
            log_file: log_file.map(Path::to_path_buf),
            no_log_file,
        },
        env,
    )
}

/// Install the global tracing subscriber.
///
/// Console (stderr, human-readable) is filtered by `--log-level` for our
/// `girsync` target; third-party targets stay quiet. The file (JSON) is
/// filtered by `--log-file-level`, and is written when it is asked for.
///
/// A log file that cannot be opened is a warning on stderr and not a failure:
/// a run must not end differently because of where it was asked to log. The
/// guard must be kept alive for the whole run — otherwise the buffered file log
/// is dropped.
pub fn init_tracing(
    console_level: Level,
    file_level: Option<Level>,
    log_file: Option<&Path>,
    no_log_file: bool,
) -> LogGuard {
    let plan = plan(console_level, file_level, log_file, no_log_file);

    // The console is stderr in the binary; `subscriber_for` is the same stack
    // with the writer injected, so an in-process caller can install it as a
    // scoped default and read the rendered events back from its own buffer.
    let (subscriber, worker) = subscriber_for(&plan, std::io::stderr);

    // A subscriber that is already installed would swallow the file layer
    // whole: the events would go to somebody else's subscriber and the log file
    // would stay empty, with nothing to say so. It cannot happen in this
    // binary's own `main`, and if it ever does, the run says so rather than
    // pretending to have logged.
    if let Err(e) = subscriber.try_init() {
        warn_subscriber_install(&mut std::io::stderr(), e);
    }

    // The file's fate, said out loud and said the same way in all three
    // projects: `log file: <path>`, or `log file: none` when there is no file.
    // A run that leaves nothing behind should say so, rather than saying
    // nothing and leaving the reader to guess.
    girpr_log::announce_log_file!(plan, module_path!());

    info!(
        path = %plan
            .file_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "none".into()),
        console_level = %plan.console_level,
        file_level = %plan.file_level,
        pid = std::process::id(),
        "log start"
    );

    LogGuard {
        plan,
        _file: worker,
    }
}

/// Build the tracing stack a resolved plan describes, with the console sink
/// writing through `console_writer` instead of a hardwired `std::io::stderr`.
///
/// This is the seam behind both installation paths: [`init_tracing`] passes
/// `std::io::stderr` and installs the subscriber as the process-wide default,
/// while a test passes a shared buffer and installs it thread-scoped with
/// `tracing::subscriber::with_default`. The layer stack, the two filters, the
/// JSON format and the file path are identical either way — only the console
/// writer and where the subscriber is installed differ. The stack itself is
/// shared with the other two programs; this wrapper supplies only this tool's
/// console filter and its file-open warning.
///
/// The returned [`WorkerGuard`] keeps the non-blocking file writer's queue
/// alive. It is returned separately from the subscriber so the caller can make
/// it outlive every event: `init_tracing` folds it into the [`LogGuard`] that
/// `main` drops at exit, and a scoped caller keeps it in a local binding for the
/// whole closure. Dropping it early loses the tail of the log.
pub fn subscriber_for<W>(
    plan: &girpr_log::LogPlan,
    console_writer: W,
) -> (
    impl tracing::Subscriber + Send + Sync + 'static,
    Option<WorkerGuard>,
)
where
    W: for<'a> fmt::MakeWriter<'a> + Send + Sync + 'static,
{
    // redb's own chatter is an implementation detail; keeping it at warn is
    // this tool's console policy, not a log-file policy. The plan's own
    // directive is still appended last so `--log-level` wins a tie.
    let console_filter = girpr_log::console_filter(
        plan.rust_log_directives(),
        &["redb=warn", &plan.own_console_directive()],
    );

    girpr_log::install::subscriber_for(
        plan,
        ConsoleSpec {
            writer: console_writer,
            filter: console_filter,
            ansi: true,
        },
        FileFormat::Json,
        SpanFields::ALL,
        |path| match open_log_file(path) {
            Ok(f) => Some(f),
            Err(e) => {
                eprintln!("WARN disabling file logging ({}): {e}", path.display());
                None
            }
        },
    )
}

/// The default log file path: `<exe-dir>/logs/girsync_<timestamp>.log`.
pub fn default_log_path() -> std::path::PathBuf {
    girpr_log::default_log_path(PROJECT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The project's name becomes the prefix of the log environment variables:
    /// `GIRSYNC_LOG_FILE` and `GIRSYNC_LOG_FILE_LEVEL`. `girpr_log::env_options`
    /// derives them by upper-casing `PROJECT`; this pins the one spelling that
    /// is this binary's responsibility. The shared crate covers the file-name
    /// shape and the precedence of those variables.
    #[test]
    fn the_project_prefix_names_the_log_environment_variables() {
        assert_eq!(PROJECT.to_ascii_uppercase(), "GIRSYNC");
    }

    #[test]
    fn a_level_name_is_normalized_or_refused() {
        // The flag is typed by `clap`, so this is the level *type* doing the
        // refusing: one spelling, one message, in every project.
        assert_eq!(Level::parse("DEBUG").unwrap(), girpr_log::Level::Debug);
        assert_eq!(Level::parse("warning").unwrap(), girpr_log::Level::Warn);
        let e = Level::parse("loud").unwrap_err();
        assert!(e.contains("loud"), "{e}");
    }
}
