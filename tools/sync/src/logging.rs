//! Tracing setup: human console layer plus an optional always-trace JSON file.
//!
//! Design:
//! - Console (stderr, human-readable) is filtered by `--log-level`.
//! - File (if `--log-file`, JSON) always captures TRACE and above, no matter
//!   `--log-level`.
//! - Data-plane output (MISSING/COPY/SUMMARY/...) stays on stdout via println!.
//! - All operational chatter uses tracing events with structured fields.
//! - Per-command spans carry config so every event inside is correlated.

use anyhow::{bail, Context, Result};
use std::path::Path;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*};

/// Normalize a `--log-level` string, rejecting anything unsupported.
pub fn parse_level_name(s: &str) -> Result<String> {
    match s.to_ascii_lowercase().as_str() {
        "trace" | "debug" | "info" | "warn" | "warning" | "error" => {
            Ok(if s.eq_ignore_ascii_case("warning") {
                "warn".to_string()
            } else {
                s.to_ascii_lowercase()
            })
        }
        other => bail!(
            "invalid --log-level '{}' (expected trace|debug|info|warn|error)",
            other
        ),
    }
}

/// Install the global tracing subscriber.
///
/// Console (stderr, human-readable) is filtered by --log-level for our
/// `girsync` target; third-party targets (e.g. redb) stay at warn to avoid
/// noise. File (if --log-file, JSON) always captures TRACE for `girsync`
/// regardless of --log-level.
///
/// Returns the file guard which must be kept alive for the whole run
/// (otherwise buffered file logs are dropped).
pub fn init_tracing(
    level_str: &str,
    log_file: Option<&Path>,
) -> Result<Option<tracing_appender::non_blocking::WorkerGuard>> {
    let console_level = parse_level_name(level_str)?;
    // Scope filters to our target so `redb` etc. don't flood stderr/file.
    let console_filter =
        tracing_subscriber::EnvFilter::new(format!("girsync={},redb=warn", console_level));
    let console_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(true)
        .with_target(true)
        .with_filter(console_filter);

    if let Some(p) = log_file {
        if let Some(parent) = p.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create log dir {}", parent.display()))?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .with_context(|| format!("open log file {}", p.display()))?;
        let (nb, guard) = tracing_appender::non_blocking(file);
        let file_filter = tracing_subscriber::EnvFilter::new("girsync=trace,redb=warn");
        let file_layer = fmt::layer()
            .json()
            .with_writer(nb)
            .with_ansi(false)
            .with_target(true)
            .with_current_span(true)
            .with_span_list(true)
            .with_filter(file_filter);
        tracing_subscriber::registry()
            .with(console_layer)
            .with(file_layer)
            .init();
        info!(
            path = %p.display(),
            console_level = %console_level,
            file_level = "trace",
            pid = std::process::id(),
            "log start"
        );
        Ok(Some(guard))
    } else {
        tracing_subscriber::registry().with(console_layer).init();
        Ok(None)
    }
}
