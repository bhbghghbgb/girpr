//! The format-agnostic mechanics of installing a logging subscriber.
//!
//! Each project still owns the guard that keeps the non-blocking file writer
//! alive: the repair binary holds a process-global slot and drains it from
//! `flush_log()`, while the two dev tools keep the guard in a struct that drops
//! (or is finished) at the end of the run. Everything up to that guard is now
//! assembled here by [`subscriber_for`]: the console and file layers, opening
//! the file, the non-blocking writer and the file's filter, and joining the two
//! layers into a registry. Saying where the file went and reporting warnings
//! that do not stop a run stay here too, so the three installations cannot
//! drift in those mechanics.
//!
//! None of this decides a program's policy. The text of a warning is chosen by
//! the caller (hence the `prefix` on [`emit_env_warnings`]), as are the console
//! filter and colouring, the file's format and how a failed open is worded; a
//! project's stderr and file bytes stay exactly what they were. The file's
//! timestamped name stays in [`crate::plan`].

use std::io::Write;
use std::path::Path;

use tracing_appender::non_blocking::{NonBlocking, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::{EnvFilter, Layer as _, fmt};

use crate::plan::{EnvOptions, LogPlan};

/// Open `path` for appending, creating missing parent directories.
///
/// The file is append-only so two runs against one path do not erase each
/// other. A plan with no file never calls this; a file that cannot be opened is
/// a warning in the caller, not a failure of the run.
///
/// A bare file name has an empty parent (`Path::parent` returns `Some("")`), so
/// there is no directory to create and `create_dir_all` is skipped rather than
/// being asked to make `""`.
pub fn open_log_file(path: &Path) -> std::io::Result<std::fs::File> {
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

/// The three parts of a file sink that every project builds the same way.
///
/// `file` is whatever the caller opened — a plain `File`, or a project-specific
/// `MakeWriter` over one — and it is handed to the non-blocking appender. The
/// appender is **lossless**: its bounded channel exerts backpressure on the
/// logger when full instead of silently dropping lines, so the file stays the
/// complete record of the run. The trade-off is deliberate — a very verbose
/// run can block a logging caller until the worker drains — and is the reason
/// per-chunk lines are off by default in a release build; an operator who opts
/// into `--log-file-level debug`/`trace` is choosing that backpressure.
/// The returned guard is *not* installed or stored here: how long it lives is
/// each project's policy. The filter is the file's level spec with itself as
/// the fallback, so a bad spec is reported once and the file still receives
/// records.
pub fn file_sink<W>(file: W, plan: &LogPlan) -> (NonBlocking, WorkerGuard, EnvFilter)
where
    W: std::io::Write + Send + 'static,
{
    let (writer, guard) = NonBlockingBuilder::default().lossy(false).finish(file);
    let filter = crate::filter(&plan.file_filter(), &plan.file_filter());
    (writer, guard, filter)
}

/// The file layer's rendering format.
///
/// The repair binary writes text so a run can be read with `grep`; the two dev
/// tools write JSON lines so a run can be parsed. The choice is the caller's —
/// this crate owns the stack around the layer, not the shape of its records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileFormat {
    /// Human-readable lines, with a level, target and message.
    Text,
    /// One JSON object per event, with the event's fields at the top level.
    #[cfg(feature = "json")]
    Json,
}

/// Which span context a JSON file layer renders alongside each event.
///
/// The text formatter's `with_current_span`/`with_span_list` are no-ops, so
/// only the JSON arm consumes these. Callers pass the values their current
/// stack uses rather than relying on an upstream default that could move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpanFields {
    /// Render the event's innermost span.
    pub current_span: bool,
    /// Render the event's enclosing span list.
    pub span_list: bool,
}

impl SpanFields {
    /// Neither span field: the text file layer's current output.
    pub const NONE: Self = Self {
        current_span: false,
        span_list: false,
    };
    /// Both span fields: what the JSON formatter already defaults to.
    pub const ALL: Self = Self {
        current_span: true,
        span_list: true,
    };
}

/// The console sink's three program-specific choices.
///
/// `writer` is stderr in a binary and a shared buffer in an in-process test;
/// `filter` is built by the caller because girsync adds its own fixed
/// directives; `ansi` follows the program's console policy. None of the three
/// is a log-file policy, so none of them is decided here.
pub struct ConsoleSpec<W> {
    /// Where the human-readable layer writes.
    pub writer: W,
    /// What the human-readable layer shows.
    pub filter: EnvFilter,
    /// Whether the human-readable layer uses ANSI escapes.
    ///
    /// `None` leaves the formatter's own default in place — including its
    /// `NO_COLOR` handling — for callers that never chose either way.
    pub ansi: Option<bool>,
}

/// Assemble the tracing stack a resolved plan describes, without installing it.
///
/// This is the layer *mechanics* the three programs used to repeat: make the
/// console layer, open the plan's file (when it has one), make the non-blocking
/// sink and its filter, choose the file layer's format, and join the two layers
/// into a registry. What stays at each call site is everything that is policy
/// or ownership: how to open the file and word a failed open, the console's
/// filter and colouring, the project's target on the announcement, and — most
/// importantly — how long the returned [`WorkerGuard`] lives. girpr keeps it in
/// a process-global slot that every exit path drains; the tools keep it in a
/// guard that drops or is finished at the end of the run.
///
/// `open_file` is called at most once, only when the plan has a file. It owns
/// both the writer type and the degradation wording, because a program's
/// warning bytes are its own contract. Returning `None` leaves a console-only
/// stack, so a file that cannot be opened never turns into a failed run.
pub fn subscriber_for<W, FW, Open>(
    plan: &LogPlan,
    console: ConsoleSpec<W>,
    format: FileFormat,
    span_fields: SpanFields,
    mut open_file: Open,
) -> (
    impl tracing::Subscriber + Send + Sync + 'static,
    Option<WorkerGuard>,
)
where
    W: for<'a> fmt::MakeWriter<'a> + Send + Sync + 'static,
    FW: std::io::Write + Send + 'static,
    Open: FnMut(&Path) -> Option<FW>,
{
    let console_layer = fmt::layer().with_writer(console.writer).with_target(true);
    let console_layer = match console.ansi {
        Some(ansi) => console_layer.with_ansi(ansi),
        None => console_layer,
    }
    .with_filter(console.filter);

    let file = plan.file_path().and_then(&mut open_file);
    let mut worker = None;
    let file_layer = file.map(|f| {
        let (nb, guard, filter) = file_sink(f, plan);
        worker = Some(guard);
        // The text formatter has no span toggles; only the JSON arm reads
        // `span_fields`, and it does so explicitly so the bytes are pinned.
        let _ = span_fields;
        match format {
            FileFormat::Text => fmt::layer()
                .with_writer(nb)
                .with_ansi(false)
                .with_target(true)
                .with_filter(filter)
                .boxed(),
            #[cfg(feature = "json")]
            FileFormat::Json => fmt::layer()
                .json()
                .with_writer(nb)
                .with_ansi(false)
                .with_target(true)
                .with_current_span(span_fields.current_span)
                .with_span_list(span_fields.span_list)
                .with_filter(filter)
                .boxed(),
        }
    });

    (
        tracing_subscriber::registry()
            .with(console_layer)
            .with(file_layer),
        worker,
    )
}

/// Say where the log file went, or that there is none.
///
/// A macro, not a function, because `tracing` stamps an event's callsite with a
/// *constant* target. Each project scopes its console filter to its own target,
/// so the event has to be stamped with the caller's `module_path!()`; emitting
/// from a plain function here would leave it under `girpr_log`'s target, where
/// the project's filter drops it and the rendered target changes. Expanding at
/// the caller also keeps the callsite's file and line the project's own. Only
/// the wording lives here, so all three projects announce the same thing.
///
/// Invoke after installing the subscriber: `girpr_log::announce_log_file!(plan,
/// module_path!());`.
#[macro_export]
macro_rules! announce_log_file {
    ($plan:expr, $target:expr) => {
        match $plan.file_path() {
            Some(path) => ::tracing::info!(target: $target, "log file: {}", path.display()),
            None => ::tracing::info!(target: $target, "log file: none"),
        }
    };
}

/// Report the warnings found while reading the logging environment.
///
/// `prefix` is the project's own spelling (`WARN` in the product and the
/// mirror, `gircap:` in the capture tool). Passing it in rather than fixing it
/// here is what lets the mechanic be shared while each program's stderr bytes
/// stay unchanged. Like every install warning, this is written straight to the
/// sink rather than emitted as an event: there may be no subscriber yet.
pub fn emit_env_warnings(out: &mut dyn Write, prefix: &str, env: &EnvOptions) {
    for warning in &env.warnings {
        let _ = writeln!(out, "{prefix} {warning}");
    }
}

/// Report a subscriber that could not be installed.
///
/// A second install would silently swallow the file layer, so the run says so
/// rather than pretending to have logged. The text is identical in all three
/// projects; the sink is passed in because the product routes warnings through
/// a buffer in its tests while the dev tools write to stderr.
pub fn warn_subscriber_install(out: &mut dyn Write, error: impl std::fmt::Display) {
    let _ = writeln!(out, "WARN cannot install the tracing subscriber: {error}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{SharedBytes, Targets};
    use std::io::Write;

    /// A scratch directory under the system temp dir, unique per test.
    /// tempfile supplies the random name and its guard removes the tree when
    /// the test ends, however it ends.
    fn scratch_dir(label: &str) -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(&format!("girpr-log-install-{label}-"))
            .tempdir()
            .expect("create scratch directory")
    }

    /// The console layer obeys the filter the caller chose, and a plan with no
    /// file never asks the caller to open one or returns a guard to own.
    #[test]
    fn subscriber_for_scopes_the_console_and_skips_a_fileless_plan() {
        let console = SharedBytes::default();
        let plan = crate::LogPlan::resolve(
            "girpr",
            crate::LogOptions {
                no_log_file: true,
                ..Default::default()
            },
            crate::EnvOptions::default(),
        );
        let (subscriber, worker) = subscriber_for(
            &plan,
            ConsoleSpec {
                writer: console.clone(),
                filter: plan.console_filter(),
                ansi: Some(false),
            },
            FileFormat::Text,
            SpanFields::NONE,
            |_| -> Option<std::fs::File> { panic!("a plan with no file opens nothing") },
        );
        assert!(worker.is_none(), "no file means no guard");

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "girpr::check", "own line");
            tracing::info!(target: "reqwest::x", "other line");
        });
        let text = String::from_utf8(console.snapshot()).expect("the console writes utf-8");
        assert!(text.contains("own line"), "{text}");
        assert!(!text.contains("other line"), "{text}");
    }

    /// A failed open is the caller's call: it writes its own warning and the
    /// stack stays a working console-only one, with nothing to flush.
    #[test]
    fn subscriber_for_degrades_to_console_only() {
        let console = SharedBytes::default();
        let named = std::path::PathBuf::from("unopenable.log");
        let plan = crate::LogPlan::resolve(
            "girpr",
            crate::LogOptions {
                log_file: Some(named.clone()),
                ..Default::default()
            },
            crate::EnvOptions::default(),
        );
        let mut warnings = Vec::new();
        let mut opens = 0;
        let (subscriber, worker) = subscriber_for(
            &plan,
            ConsoleSpec {
                writer: console.clone(),
                filter: plan.console_filter(),
                ansi: Some(false),
            },
            FileFormat::Text,
            SpanFields::NONE,
            |path| {
                opens += 1;
                let _ = writeln!(
                    warnings,
                    "WARN disabling file logging ({}): denied",
                    path.display()
                );
                None::<std::fs::File>
            },
        );
        assert_eq!(opens, 1, "the caller opens the file at most once");
        assert!(worker.is_none(), "a failed open leaves no guard to own");
        assert_eq!(
            String::from_utf8(warnings).expect("the warning is utf-8"),
            format!(
                "WARN disabling file logging ({}): denied\n",
                named.display()
            ),
            "the degradation wording belongs to the caller"
        );

        tracing::subscriber::with_default(subscriber, || tracing::info!("still narrated"));
        assert!(
            String::from_utf8(console.snapshot())
                .expect("the console writes utf-8")
                .contains("still narrated")
        );
    }

    /// The text arm keeps the record girpr greps for: level, target, message.
    #[test]
    fn subscriber_for_writes_the_text_file_layer() {
        let dir = scratch_dir("text-stack");
        let path = dir.path().join("run.log");
        let plan = crate::LogPlan::resolve(
            "girpr",
            crate::LogOptions {
                log_file: Some(path.clone()),
                ..Default::default()
            },
            crate::EnvOptions::default(),
        );
        let (subscriber, worker) = subscriber_for(
            &plan,
            ConsoleSpec {
                writer: SharedBytes::default(),
                filter: plan.console_filter(),
                ansi: Some(false),
            },
            FileFormat::Text,
            SpanFields::NONE,
            |p| open_log_file(p).ok(),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "girpr::logging", "log file: {}", path.display());
        });
        drop(worker.expect("the opened file owns a guard"));

        let body = std::fs::read_to_string(&path).expect("the file was written");
        assert!(
            body.contains("INFO girpr::logging: log file:"),
            "the text layer keeps the target: {body}"
        );
    }

    /// The JSON arm keeps one object per event, with a level field. Enabled by
    /// the tools that supply the formatter, so it is tested where it compiles.
    #[cfg(feature = "json")]
    #[test]
    fn subscriber_for_writes_the_json_file_layer() {
        let dir = scratch_dir("json-stack");
        let path = dir.path().join("run.log");
        let plan = crate::LogPlan::resolve(
            "girsync",
            crate::LogOptions {
                log_file: Some(path.clone()),
                ..Default::default()
            },
            crate::EnvOptions::default(),
        );
        let (subscriber, worker) = subscriber_for(
            &plan,
            ConsoleSpec {
                writer: SharedBytes::default(),
                filter: plan.console_filter(),
                ansi: Some(false),
            },
            FileFormat::Json,
            SpanFields::ALL,
            |p| open_log_file(p).ok(),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "girsync::logging", "a json line");
        });
        drop(worker.expect("the opened file owns a guard"));

        let body = std::fs::read_to_string(&path).expect("the file was written");
        assert!(body.contains("\"level\":\"INFO\""), "{body}");
        assert!(body.contains("\"message\":\"a json line\""), "{body}");
    }

    /// The shared sink is lossless: everything queued before the guard drops
    /// reaches the writer, with no silent drop-counter to go with it.
    #[test]
    fn file_sink_preserves_every_queued_line_on_flush() {
        let bytes = SharedBytes::default();
        let plan = crate::LogPlan::resolve(
            "girpr",
            crate::LogOptions::default(),
            crate::EnvOptions::default(),
        );
        let (mut writer, guard, _filter) = file_sink(bytes.clone(), &plan);
        for i in 0..32 {
            writeln!(writer, "line {i}").expect("queue the line");
        }
        drop(guard);

        let written = String::from_utf8(bytes.snapshot()).expect("the lines are utf-8");
        assert_eq!(written.lines().count(), 32, "no line is dropped: {written}");
    }

    #[test]
    fn open_log_file_creates_parents_and_appends() {
        let dir = scratch_dir("append");
        let path = dir.path().join("nested").join("run.log");

        let mut f = open_log_file(&path).expect("creates the parent directory");
        f.write_all(b"one\n").expect("writes the first line");
        drop(f);

        let mut f = open_log_file(&path).expect("reopens an existing file");
        f.write_all(b"two\n").expect("appends the second line");
        drop(f);

        assert_eq!(
            std::fs::read_to_string(&path).expect("readable"),
            "one\ntwo\n"
        );
    }

    /// The announcement must travel under the caller's target: each project
    /// scopes its console filter to its own target, so an event stamped with
    /// this crate's target would be filtered out of stderr entirely.
    #[test]
    fn the_announcement_keeps_the_callers_target() {
        use tracing_subscriber::layer::SubscriberExt as _;

        let plan = crate::LogPlan::resolve(
            "girpr",
            crate::LogOptions::default(),
            crate::EnvOptions::default(),
        );
        let seen = Targets::default();
        let subscriber = tracing_subscriber::registry().with(seen.clone());
        tracing::subscriber::with_default(subscriber, || {
            crate::announce_log_file!(plan, "girpr::logging");
        });
        assert_eq!(seen.seen(), ["girpr::logging"]);
    }

    #[test]
    fn env_warning_wording_is_chosen_by_the_caller() {
        let env = EnvOptions {
            warnings: vec!["first".into(), "second".into()],
            ..EnvOptions::default()
        };
        let mut out = Vec::new();
        emit_env_warnings(&mut out, "WARN", &env);
        assert_eq!(
            String::from_utf8(out).expect("utf-8"),
            "WARN first\nWARN second\n"
        );

        let mut out = Vec::new();
        emit_env_warnings(&mut out, "gircap:", &env);
        assert_eq!(
            String::from_utf8(out).expect("utf-8"),
            "gircap: first\ngircap: second\n"
        );
    }

    #[test]
    fn the_install_warning_keeps_its_text() {
        let mut out = Vec::new();
        warn_subscriber_install(&mut out, "already set");
        assert_eq!(
            String::from_utf8(out).expect("utf-8"),
            "WARN cannot install the tracing subscriber: already set\n"
        );
    }
}
