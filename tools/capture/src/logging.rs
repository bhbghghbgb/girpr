//! stderr narration, via `tracing`.
//!
//! This module owns stderr; [`crate::report`] owns stdout. Nothing here prints a
//! record, and nothing there logs one, so `--output json` is pipeable with no
//! filtering.
//!
//! Two sinks, one span tree:
//!
//! - a human-readable layer on stderr, filtered by `--log-level` or `RUST_LOG`;
//! - with `--log-file`, an additional JSON-lines sink carrying the same events.
//!
//! The console filter is deliberately *not* copied to the file sink. An artifact
//! run's narration is the evidence for why a version pair was chosen and what
//! each request cost, and it must survive a `--log-level warn` console.

use std::fs::OpenOptions;
use std::path::Path;
use std::sync::{Arc, Mutex};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer as _;
use tracing_subscriber::fmt::writer::MakeWriter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// Filter verbosities accepted by `--log-level`.
///
/// A closed set rather than free text: `EnvFilter` accepts a whole directive
/// language, and letting it reach the flag would make `--log-level` capable of
/// things the docs do not describe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl Level {
    /// The `tracing` filter string for this verbosity.
    pub const fn as_filter(self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warn => "warn",
            Level::Info => "info",
            Level::Debug => "debug",
            Level::Trace => "trace",
        }
    }

    /// Parse a `--log-level` value, case-insensitively.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "error" => Ok(Level::Error),
            "warn" => Ok(Level::Warn),
            "info" => Ok(Level::Info),
            "debug" => Ok(Level::Debug),
            "trace" => Ok(Level::Trace),
            other => Err(format!(
                "unknown log level {other:?}; expected one of \
                 error, warn, info, debug, trace"
            )),
        }
    }
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_filter())
    }
}

/// Keeps the non-blocking file sink's buffer alive for the length of the run.
///
/// Dropping this flushes the tail of the JSON-lines log. Without it the last few
/// hundred lines — the ones describing the run's outcome — would be lost, which is
/// the one part of the file a reader most wants.
pub struct LogGuard {
    _file: Option<WorkerGuard>,
}

impl LogGuard {
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

/// The filter spec for the console sink: `RUST_LOG` if set, else `--log-level`.
///
/// `RUST_LOG` wins over the flag, matching the repair tool's precedence, so a
/// developer who has exported it gets one consistent lever instead of two that
/// disagree. An empty or whitespace-only `RUST_LOG` is treated as unset — that
/// is what an unset variable in a shell profile expands to.
///
/// Takes the value as a parameter rather than reading the environment, so the
/// precedence is a pure function and can be tested without mutating global
/// state that other tests share.
pub fn console_filter_spec(level: Level, rust_log: Option<&str>) -> String {
    rust_log
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| level.as_filter().to_string())
}

/// The filter spec for the `--log-file` sink.
///
/// Everything, deliberately and unconditionally. A capture's narration is the
/// evidence for why a version pair was chosen and what each request cost, and it
/// must survive a `--log-level error` console.
pub const FILE_FILTER_SPEC: &str = "trace";

/// Install the subscriber. Call once, before any work.
///
/// Returns a [`LogGuard`] that must be held for the length of the run; dropping
/// it flushes the file sink. Errors on the file sink are reported to stderr and
/// then ignored — a log file that cannot be opened must not stop a capture,
/// because the capture itself is the thing that was asked for.
///
/// **The registry's own filter is `trace` and the two sinks filter themselves.**
/// That is not redundant: a `Layered` stack's max-level hint is the *minimum* of
/// its layers', so putting the console's `info` on the registry would cap the
/// whole stack at `info` and a per-sink `trace` on the file could never widen it
/// again. The global filter is therefore a ceiling that decides nothing, and the
/// real work happens on each sink.
pub fn init(level: Level, log_file: Option<&Path>) -> LogGuard {
    use tracing_subscriber::fmt::layer;

    let console_spec = console_filter_spec(level, std::env::var("RUST_LOG").ok().as_deref());

    let stderr_layer = layer()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_target(true);

    // The file sink, if `--log-file` named a writable path. A log file that
    // cannot be opened is reported and then skipped: the capture is the thing
    // that was asked for, and losing its narration is a lesser harm than losing
    // it. Everything before this point happens with no subscriber installed, so
    // the failure is a raw stderr write.
    let file = match log_file {
        None => None,
        Some(path) => match open_log_file(path) {
            Err(e) => {
                eprintln!("gircap: cannot open --log-file {}: {e}", path.display());
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
        let (non_blocking, guard) = tracing_appender::non_blocking(w);
        worker = Some(guard);
        layer()
            .json()
            .with_ansi(false)
            .with_target(true)
            .with_writer(non_blocking)
            .with_filter(EnvFilter::new(FILE_FILTER_SPEC))
    });

    let _ = tracing_subscriber::registry()
        .with(EnvFilter::new(FILE_FILTER_SPEC))
        .with(stderr_layer.with_filter(EnvFilter::new(&console_spec)))
        .with(file_layer)
        .try_init();

    let guard = LogGuard { _file: worker };

    tracing::info!(
        version = crate::VERSION,
        log_level = %level,
        console_filter = %console_spec,
        log_file = log_file.map(|p| p.display().to_string()),
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
pub const USER_AGENT: &str =
    "UnityPlayer/2019.4.40f1 (UnityWebRequest/1.0, libcurl/7.80.0-DEV)";

/// The [`USER_AGENT`] string, owned where a [`String`] is wanted.
pub fn user_agent() -> String {
    USER_AGENT.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn every_level_parses_in_any_case_and_rejects_the_rest() {
        for (s, want) in [
            ("error", Level::Error),
            ("WARN", Level::Warn),
            ("Info", Level::Info),
            ("debug", Level::Debug),
            ("TRACE", Level::Trace),
        ] {
            assert_eq!(Level::parse(s), Ok(want), "{s}");
        }
        let e = Level::parse("verbose").unwrap_err();
        assert!(e.contains("verbose"), "{e}");
        assert!(e.contains("error, warn, info, debug, trace"), "{e}");
    }

    #[test]
    fn the_default_level_is_info() {
        assert_eq!(Level::default(), Level::Info);
        assert_eq!(Level::default().as_filter(), "info");
    }

    #[test]
    fn the_user_agent_is_the_generic_unity_player_string() {
        assert_eq!(user_agent(), USER_AGENT);
        assert_eq!(
            USER_AGENT,
            "UnityPlayer/2019.4.40f1 (UnityWebRequest/1.0, libcurl/7.80.0-DEV)"
        );
    }

    #[test]
    fn rust_log_wins_over_the_flag_and_an_empty_one_is_treated_as_unset() {
        assert_eq!(console_filter_spec(Level::Info, Some("warn")), "warn");
        assert_eq!(
            console_filter_spec(Level::Debug, Some(" gircap::hyp=trace ")),
            "gircap::hyp=trace",
            "the value is trimmed but otherwise passed through verbatim"
        );
        for unset in [None, Some(""), Some("   ")] {
            assert_eq!(
                console_filter_spec(Level::Debug, unset),
                "debug",
                "{unset:?} must fall back to the flag"
            );
        }
    }

    #[test]
    fn the_file_sink_is_unfiltered() {
        assert_eq!(FILE_FILTER_SPEC, "trace");
    }

    #[test]
    fn a_log_file_writer_appends_and_flushes() {
        let dir = std::env::temp_dir().join(format!("gircap-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
