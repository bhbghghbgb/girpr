//! Tracing setup: a human console layer plus an always-`TRACE` file layer.
//!
//! Design (automation contract, see README):
//! - Console (stderr) is filtered by `--log-level`, optionally widened by
//!   `RUST_LOG`.
//! - File always captures `TRACE` and above regardless of `--log-level`, so a
//!   post-mortem never loses chunk-level detail. `--log-file <FILE>` chooses
//!   where it goes; without the flag it is [`log_file_path`]'s timestamped
//!   default, which only the binary that wrote it can name.
//! - Data-plane output (`REPORT` / `PROGRESS` / `SUMMARY`) stays on stdout via
//!   [`crate::report::emit`]; the file layer is plain text, not JSON, because
//!   it is meant to be read with `grep 'path=<file>'`.
//!
//! A log file that cannot be opened degrades to a warning on stderr rather than
//! failing the run, so [`init_tracing`] is infallible.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing_subscriber::prelude::*;

/// Where the log file lives when `--log-file` was not given:
/// `<exe-dir>/logs/girpr_<timestamp>.log`, falling back to the current
/// directory when the exe path is unavailable.
///
/// The timestamp keeps concurrent runs from interleaving but also makes the
/// name unguessable to the caller, which is why `--log-file` exists.
pub fn log_file_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        })
        .join("logs")
        .join(format!(
            "girpr_{}.log",
            chrono::Local::now().format("%Y-%m-%d_%H-%M-%S%.3f")
        ))
}

/// Install the global tracing subscriber and return the log file path.
///
/// Scopes the console filter to the `girpr` target so third-party crates stay
/// quiet; the file layer is `TRACE` for the same target.
///
/// `log_file` is the `--log-file` value, or `None` for the timestamped default.
/// The returned path is what was *asked for*, not what was opened: a file that
/// could not be opened is a stderr warning, and this stays infallible so a
/// unwritable log never costs a run its exit code.
pub fn init_tracing(level: &str, log_file: Option<&Path>) -> PathBuf {
    let rust_log = std::env::var("RUST_LOG").unwrap_or_default();
    let rust_log = rust_log.trim();
    let filter = if rust_log.is_empty() {
        format!("girpr={level}")
    } else {
        format!("girpr={level},{rust_log}")
    };
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(
            filter
                .parse::<tracing_subscriber::EnvFilter>()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        );

    let log_path = log_file.map_or_else(log_file_path, Path::to_path_buf);
    let file_writer: Mutex<Box<dyn Write + Send + Sync>> = match open_log_file(&log_path) {
        Ok(f) => Mutex::new(Box::new(f)),
        Err(e) => {
            eprintln!("WARN disabling file logging ({}): {e}", log_path.display());
            Mutex::new(Box::new(std::io::sink()))
        }
    };
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_filter(tracing_subscriber::filter::LevelFilter::TRACE);

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .init();

    tracing::info!("log file: {}", log_path.display());
    log_path
}

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
