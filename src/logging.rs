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
//! - The file is **always** written. Without `--log-file` or
//!   `GIRPR_LOG_FILE` it is [`default_log_path`]'s timestamped default, in a
//!   `logs/` directory beside the executable.
//! - Data-plane output (`REPORT` / `PROGRESS` / `SUMMARY`) stays on stdout via
//!   [`crate::report::emit`]; the file layer is plain text, not JSON, because
//!   it is meant to be read with `grep 'path=<file>'`.
//!
//! A log file that cannot be opened degrades to a warning on stderr rather than
//! failing the run, so [`init_tracing`] is infallible.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use girpr_log::{Level, LogOptions, LogPlan};
use tracing_subscriber::prelude::*;

/// This project's name: the default log file's stem, and the prefix of the
/// environment variables that answer for the flags.
pub const PROJECT: &str = "girpr";

/// Where the log file lives when neither `--log-file` nor `GIRPR_LOG_FILE`
/// named one: `<exe-dir>/logs/girpr_<timestamp>.log`, falling back to the
/// current directory when the exe path is unavailable.
///
/// The timestamp keeps concurrent runs from interleaving but also makes the
/// name unguessable to the caller, which is why the flag and the environment
/// variable exist.
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
/// the `debug` default can answer for it), and `log_file` is `--log-file`
/// (`None` for the timestamped default).
///
/// The returned plan is what was *asked for*, not what was opened: a file
/// that could not be opened is a stderr warning, and this stays infallible so
/// an unwritable log never costs a run its exit code.
pub fn init_tracing(
    console_level: Level,
    file_level: Option<Level>,
    log_file: Option<&Path>,
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
        },
        env,
    );

    let console_fallback = format!("{PROJECT}={}", console_level.as_str());
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(girpr_log::filter(&plan.console_filter(), &console_fallback));

    let file_writer: Mutex<Box<dyn Write + Send + Sync>> = match open_log_file(&plan.file) {
        Ok(f) => Mutex::new(Box::new(f)),
        Err(e) => {
            eprintln!("WARN disabling file logging ({}): {e}", plan.file.display());
            Mutex::new(Box::new(std::io::sink()))
        }
    };
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_filter(girpr_log::filter(&plan.file_filter(), &plan.file_filter()));

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .init();

    tracing::info!("log file: {}", plan.file.display());
    plan
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
