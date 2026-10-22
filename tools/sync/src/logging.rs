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
use tracing_subscriber::{fmt, prelude::*};

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
    for warning in &env.warnings {
        eprintln!("WARN {warning}");
    }
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

    let console_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(true)
        .with_target(true)
        .with_filter(girpr_log::filter(
            &console_filter(&plan),
            &console_fallback(&plan),
        ));

    // The file is opened append-only, so two runs against one path do not
    // erase each other. A file that cannot be opened costs the run its
    // narration and nothing else. A plan with no file — a debug build that was
    // not asked for one, or any build told `--no-log-file` — has no sink at
    // all, so the run leaves nothing behind.
    let file = match plan.file_path() {
        None => None,
        Some(path) => match open_log_file(path) {
            Ok(f) => Some(f),
            Err(e) => {
                eprintln!("WARN disabling file logging ({}): {e}", path.display());
                None
            }
        },
    };

    let mut worker = None;
    let file_layer = file.map(|f| {
        let (nb, guard) = tracing_appender::non_blocking(f);
        worker = Some(guard);
        fmt::layer()
            .json()
            .with_writer(nb)
            .with_ansi(false)
            .with_target(true)
            .with_current_span(true)
            .with_span_list(true)
            .with_filter(girpr_log::filter(&plan.file_filter(), &plan.file_filter()))
    });

    // A subscriber that is already installed would swallow the file layer
    // whole: the events would go to somebody else's subscriber and the log file
    // would stay empty, with nothing to say so. It cannot happen in this
    // binary's own `main`, and if it ever does, the run says so rather than
    // pretending to have logged.
    if let Err(e) = tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .try_init()
    {
        eprintln!("WARN cannot install the tracing subscriber: {e}");
    }

    // The file's fate, said out loud and said the same way in all three
    // projects: `log file: <path>`, or `log file: none` when there is no file.
    // A run that leaves nothing behind should say so, rather than saying
    // nothing and leaving the reader to guess.
    match plan.file_path() {
        Some(path) => info!("log file: {}", path.display()),
        None => info!("log file: none"),
    }

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

/// The console's filter spec: `redb=warn`, then `RUST_LOG`, then this
/// project's own directive.
///
/// The shared policy keeps third-party crates off the console, and `redb` is
/// the one exception: it is this tool's cache, and a cache that complains is
/// worth saying out loud.
///
/// The order matters, because `EnvFilter` gives the most specific matching
/// directive the last word, and a tie on the same target to the *last* one:
/// this project's own directive is last, so `--log-level` decides it; `RUST_LOG`
/// sits in front of it, so it adds visibility for other crates without being
/// able to take the project's own away.
fn console_filter(plan: &girpr_log::LogPlan) -> String {
    let own = plan.own_console_directive();
    match plan.rust_log_directives() {
        Some(spec) => format!("redb=warn,{spec},{own}"),
        None => format!("redb=warn,{own}"),
    }
}

/// The filter to fall back to when the console's spec does not parse.
///
/// The same spec, without the environment's contribution: a broken `RUST_LOG`
/// must not be able to widen the console past what the flag asked for.
fn console_fallback(plan: &girpr_log::LogPlan) -> String {
    format!("redb=warn,{}", plan.own_console_directive())
}

/// Open the log file for appending, creating the parent directory if needed.
fn open_log_file(path: &Path) -> std::io::Result<std::fs::File> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

/// The default log file path: `<exe-dir>/logs/girsync_<timestamp>.log`.
pub fn default_log_path() -> std::path::PathBuf {
    girpr_log::default_log_path(PROJECT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_file_is_named_for_the_project() {
        let path = default_log_path();
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("girsync_"), "{name}");
        assert!(name.ends_with(".log"), "{name}");
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
