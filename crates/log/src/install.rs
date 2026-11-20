//! The format-agnostic mechanics of installing a logging subscriber.
//!
//! Each project still builds its own layers and owns the guard that keeps the
//! non-blocking file writer alive: the repair binary holds a process-global
//! slot and drains it from `flush_log()`, while the two dev tools keep the
//! guard in a struct that drops (or is finished) at the end of the run. What is
//! the same everywhere is the work *before* those layers: open the file, make
//! the non-blocking writer and the file's filter, say where the file went, and
//! report the warnings that do not stop a run. This module owns that part so
//! the three installations cannot drift in those mechanics.
//!
//! None of this decides a format. The text of a warning is chosen by the caller
//! (hence the `prefix` on [`emit_env_warnings`]) so a project's stderr bytes stay
//! exactly what they were; the file's timestamped name stays in [`crate::plan`].

use std::io::Write;
use std::path::Path;

use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_subscriber::EnvFilter;

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
/// `MakeWriter` over one — and it is handed straight to the non-blocking
/// appender so logging never blocks the run. The returned guard is *not*
/// installed or stored here: how long it lives is each project's policy.
/// The filter is the file's level spec with itself as the fallback, so a bad
/// spec is reported once and the file still receives records.
pub fn file_sink<W>(file: W, plan: &LogPlan) -> (NonBlocking, WorkerGuard, EnvFilter)
where
    W: std::io::Write + Send + 'static,
{
    let (writer, guard) = tracing_appender::non_blocking(file);
    let filter = crate::filter(&plan.file_filter(), &plan.file_filter());
    (writer, guard, filter)
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A scratch path under the system temp dir, unique per test.
    fn scratch_dir(label: &str) -> std::path::PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "girpr-log-install-{label}-{}-{n}",
            std::process::id()
        ))
    }

    #[test]
    fn open_log_file_creates_parents_and_appends() {
        let path = scratch_dir("append").join("nested").join("run.log");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());

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
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The announcement must travel under the caller's target: each project
    /// scopes its console filter to its own target, so an event stamped with
    /// this crate's target would be filtered out of stderr entirely.
    #[test]
    fn the_announcement_keeps_the_callers_target() {
        use std::sync::{Arc, Mutex};
        use tracing::{Event, Subscriber};
        use tracing_subscriber::Layer;
        use tracing_subscriber::layer::{Context, SubscriberExt as _};
        use tracing_subscriber::registry::LookupSpan;

        #[derive(Clone, Default)]
        struct Targets(Arc<Mutex<Vec<String>>>);

        impl<S> Layer<S> for Targets
        where
            S: Subscriber + for<'a> LookupSpan<'a>,
        {
            fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
                self.0
                    .lock()
                    .expect("not poisoned")
                    .push(event.metadata().target().to_string());
            }
        }

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
        assert_eq!(&*seen.0.lock().expect("not poisoned"), &["girpr::logging"]);
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
