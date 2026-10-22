//! Where a project's log file goes, and how loud each of its two sinks is.
//!
//! # The policy
//!
//! One policy, every project in this workspace — the repair binary (`girpr`)
//! and the two dev tools (`gircap`, `girsync`). It is five rules:
//!
//! 1. **A log file is always written.** There is no "no log file" mode: a run
//!    that leaves nothing behind is a run nobody can ask questions of later.
//! 2. **The file has a name you can find.** By default it is
//!    `<exe-dir>/logs/<project>_<timestamp>.log` — beside the executable, in a
//!    `logs/` directory, named for the project and stamped with the time.
//! 3. **Three ways to choose the path, in this order**: the `--log-file` flag,
//!    then the `<PROJECT>_LOG_FILE` environment variable, then the default.
//!    A flag beats an environment variable because a flag was typed by the
//!    person running the tool; an environment variable is whatever the shell
//!    happened to export.
//! 4. **Two levels, not one.** `--log-level` is the **console's** (default
//!    `info`); `--log-file-level` is the **file's** (default `debug`). They are
//!    separate because they answer separate questions: the console is for the
//!    person watching, the file is for the person reading it afterwards, and
//!    the second reader wants more detail than the first.
//! 5. **A log file that cannot be opened is a warning, never a failure.** A run
//!    must not end differently because of where it was asked to log.
//!
//! # The environment seams
//!
//! | variable | meaning |
//! | --- | --- |
//! | `<PROJECT>_LOG_FILE` | the log file's path, as `--log-file` would name it |
//! | `<PROJECT>_LOG_FILE_LEVEL` | the log file's level, as `--log-file-level` would name it |
//! | `RUST_LOG` | the console's filter spec, replacing `--log-level` |
//!
//! `<PROJECT>` is the project's name in upper case: `GIRPR_LOG_FILE`,
//! `GIRCAP_LOG_FILE`, `GIRSYNC_LOG_FILE`. `RUST_LOG` is the one spelling
//! that is not per-project, because it is `tracing`'s own convention and a
//! developer who has exported it means it everywhere.
//!
//! `RUST_LOG` replaces `--log-level` rather than adding to it. Two filters
//! that disagree are worse than one: the flag would be a lie about what the
//! console is doing.
//!
//! # What is *not* here
//!
//! The file's **format** is not policy. `girpr` writes a human-readable text
//! log, because it is meant to be read with `grep`; the two dev tools write
//! JSON lines, because they are meant to be parsed. The path, the levels and
//! the failure mode are the same everywhere, and that is what this crate
//! owns.

use std::path::{Path, PathBuf};

use crate::level::Level;

/// What the command line asked for, before the environment is consulted.
#[derive(Debug, Clone, Default)]
pub struct LogOptions {
    /// `--log-level`: the console's verbosity.
    pub console_level: Level,
    /// `--log-file-level`: the file's verbosity, when the caller named one.
    ///
    /// `None` rather than a defaulted value, because a defaulted value cannot
    /// be told apart from one the user typed — and the environment variable
    /// has to be able to answer for a flag that was never given.
    pub file_level: Option<Level>,
    /// `--log-file`: the file's path, when the caller named one.
    pub log_file: Option<PathBuf>,
}

/// The environment seams for one project.
///
/// Read once, by [`env_options`], and passed in — so that resolving a plan is
/// a pure function of its arguments and can be tested without mutating
/// global state that other tests share.
#[derive(Debug, Clone, Default)]
pub struct EnvOptions {
    /// `<PROJECT>_LOG_FILE`.
    pub log_file: Option<PathBuf>,
    /// `<PROJECT>_LOG_FILE_LEVEL`.
    pub file_level: Option<Level>,
    /// `RUST_LOG`.
    pub rust_log: Option<String>,
    /// Problems found while reading the above, to be reported as warnings.
    ///
    /// A bad environment variable is a warning and not an error: the shell is
    /// not the command line, and a stale export must not be able to stop a
    /// run. The value is dropped and the default takes its place.
    pub warnings: Vec<String>,
}

/// The resolved policy: what a project is going to log, and where.
#[derive(Debug, Clone)]
pub struct LogPlan {
    /// The project's name: `girpr`, `gircap`, `girsync`.
    ///
    /// It is the default file name's stem and the environment variables'
    /// prefix, so it is the one string that ties the three together.
    pub project: &'static str,
    /// The console's verbosity, as `--log-level` asked for it.
    ///
    /// The *effective* console filter is [`LogPlan::console_filter`], which
    /// `RUST_LOG` may have replaced.
    pub console_level: Level,
    /// The log file's verbosity.
    pub file_level: Level,
    /// Where the log file goes.
    pub file: PathBuf,
    /// Whether the caller named the file, rather than taking the default.
    ///
    /// This is what decides whether the path is echoed on stderr: a caller who
    /// asked for a specific file has to be told where it went even if the
    /// console is quiet, and a caller who took the default has not asked
    /// anything and is told inside the file instead.
    pub file_named: bool,
    /// `RUST_LOG`, when it is set.
    pub rust_log: Option<String>,
}

impl LogPlan {
    /// Resolve the policy from the command line and the environment.
    ///
    /// Precedence, everywhere: **flag > environment > default**.
    pub fn resolve(project: &'static str, opts: LogOptions, env: EnvOptions) -> Self {
        let (file, file_named) = match opts.log_file {
            Some(path) => (path, true),
            None => match env.log_file {
                Some(path) => (path, true),
                None => (default_log_path(project), false),
            },
        };
        let file_level = opts.file_level.or(env.file_level).unwrap_or(Level::Debug);
        Self {
            project,
            console_level: opts.console_level,
            file_level,
            file,
            file_named,
            rust_log: env.rust_log,
        }
    }

    /// The `tracing` filter spec for the **console** sink.
    ///
    /// `RUST_LOG` when it is set, verbatim; otherwise the project's own target
    /// at `--log-level`. The project scoping is what keeps third-party crates
    /// quiet: a filter that names no target applies to every target there is,
    /// and `reqwest`'s idea of `info` is not this program's idea of `info`.
    pub fn console_filter(&self) -> String {
        match self
            .rust_log
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(spec) => spec.to_string(),
            None => format!("{}={}", self.project, self.console_level.as_str()),
        }
    }

    /// The `tracing` filter spec for the **log file** sink.
    ///
    /// Every target, at the file's level. The file is the record of the run,
    /// and a record that dropped a third-party crate's line would be a record
    /// with a hole in it — the file is where a post-mortem looks.
    pub fn file_filter(&self) -> String {
        self.file_level.as_str().to_string()
    }
}

/// The default log file path: `<exe-dir>/logs/<project>_<timestamp>.log`.
///
/// Beside the executable, because that is the one directory a user can find
/// without knowing the tool's conventions. The current directory is the
/// fallback for the case where the executable's path cannot be read at all.
///
/// The timestamp keeps concurrent runs from interleaving in one file, and
/// names the project so a directory of logs from three tools stays readable.
pub fn default_log_path(project: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    dir.join("logs")
        .join(format!("{project}_{}.log", timestamp()))
}

/// The timestamp in a default log file name: `2026-10-22_13-45-01.123`.
///
/// Local time, and sortable as text: the files in a `logs/` directory read
/// oldest-first when they are listed by name.
fn timestamp() -> String {
    chrono::Local::now()
        .format("%Y-%m-%d_%H-%M-%S%.3f")
        .to_string()
}

/// Read the environment seams for one project.
///
/// The one place in the workspace that reads the environment for logging, so
/// that the set of variables a project answers to is written down once.
pub fn env_options(project: &str) -> EnvOptions {
    let prefix = project.to_ascii_uppercase();
    let mut warnings = Vec::new();

    // An empty variable is an unset variable. That is what an unset variable
    // in a shell profile expands to, and treating it as a path would try to
    // open a file whose name is the empty string.
    let log_file = std::env::var_os(format!("{prefix}_LOG_FILE"))
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);

    let file_level = match std::env::var(format!("{prefix}_LOG_FILE_LEVEL")) {
        Ok(raw) if !raw.trim().is_empty() => {
            let (level, warning) = parse_file_level(&raw, &prefix);
            if let Some(warning) = warning {
                warnings.push(warning);
            }
            level
        }
        _ => None,
    };

    let rust_log = std::env::var("RUST_LOG")
        .ok()
        .filter(|v| !v.trim().is_empty());

    EnvOptions {
        log_file,
        file_level,
        rust_log,
        warnings,
    }
}

/// Parse the value of `<PROJECT>_LOG_FILE_LEVEL`.
///
/// A bad value is a warning and not an error: the shell is not the command
/// line, and a stale export must not be able to stop a run. The value is
/// dropped and the default takes its place.
fn parse_file_level(raw: &str, prefix: &str) -> (Option<Level>, Option<String>) {
    match Level::parse(raw) {
        Ok(level) => (Some(level), None),
        Err(e) => (
            None,
            Some(format!(
                "{prefix}_LOG_FILE_LEVEL: {e}; using the default ({})",
                Level::Debug
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> LogOptions {
        LogOptions {
            console_level: Level::Info,
            file_level: None,
            log_file: None,
        }
    }

    #[test]
    fn the_default_file_is_beside_the_executable_in_logs() {
        let path = default_log_path("girpr");
        assert_eq!(
            path.parent()
                .and_then(Path::file_name)
                .and_then(|s| s.to_str()),
            Some("logs"),
            "the file lives in a `logs/` directory: {path:?}"
        );
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with("girpr_") && name.ends_with(".log"),
            "the name carries the project and a timestamp: {name}"
        );
        // The timestamp: `YYYY-MM-DD_HH-MM-SS.mmm`.
        let stamp = name.trim_start_matches("girpr_").trim_end_matches(".log");
        assert_eq!(stamp.len(), 23, "{stamp}");
        assert_eq!(stamp.as_bytes()[4], b'-');
        assert_eq!(stamp.as_bytes()[10], b'_');
    }

    #[test]
    fn a_flag_beats_the_environment_and_the_environment_beats_the_default() {
        let env = EnvOptions {
            log_file: Some(PathBuf::from("from-env.log")),
            ..Default::default()
        };
        let plan = LogPlan::resolve("girpr", opts(), env.clone());
        assert_eq!(plan.file, PathBuf::from("from-env.log"));
        assert!(plan.file_named);

        let mut flagged = opts();
        flagged.log_file = Some(PathBuf::from("from-flag.log"));
        let plan = LogPlan::resolve("girpr", flagged, env);
        assert_eq!(plan.file, PathBuf::from("from-flag.log"));
        assert!(plan.file_named);
    }

    #[test]
    fn the_default_file_is_not_named() {
        let plan = LogPlan::resolve("gircap", opts(), EnvOptions::default());
        assert!(!plan.file_named);
        assert!(plan.file.to_string_lossy().contains("gircap_"), "{plan:?}");
    }

    #[test]
    fn the_file_level_is_debug_unless_something_says_otherwise() {
        let plan = LogPlan::resolve("girpr", opts(), EnvOptions::default());
        assert_eq!(plan.file_level, Level::Debug);

        let env = EnvOptions {
            file_level: Some(Level::Trace),
            ..Default::default()
        };
        let plan = LogPlan::resolve("girpr", opts(), env);
        assert_eq!(plan.file_level, Level::Trace);

        let mut flagged = opts();
        flagged.file_level = Some(Level::Error);
        let env = EnvOptions {
            file_level: Some(Level::Trace),
            ..Default::default()
        };
        let plan = LogPlan::resolve("girpr", flagged, env);
        assert_eq!(plan.file_level, Level::Error);
    }

    #[test]
    fn the_console_is_scoped_to_the_project_and_the_file_is_not() {
        let plan = LogPlan::resolve("girsync", opts(), EnvOptions::default());
        assert_eq!(plan.console_filter(), "girsync=info");
        assert_eq!(plan.file_filter(), "debug");
    }

    #[test]
    fn rust_log_replaces_the_console_level() {
        let env = EnvOptions {
            rust_log: Some("warn".to_string()),
            ..Default::default()
        };
        let plan = LogPlan::resolve("girpr", opts(), env);
        assert_eq!(plan.console_filter(), "warn");
        assert_eq!(
            plan.file_filter(),
            "debug",
            "RUST_LOG is a console lever and never a file lever"
        );
    }

    #[test]
    fn an_empty_rust_log_is_treated_as_unset() {
        for raw in ["", "   "] {
            let env = EnvOptions {
                rust_log: Some(raw.to_string()),
                ..Default::default()
            };
            let plan = LogPlan::resolve("girpr", opts(), env);
            assert_eq!(plan.console_filter(), "girpr=info", "{raw:?}");
        }
    }

    #[test]
    fn a_bad_file_level_is_a_warning_and_not_a_failure() {
        let (level, warning) = parse_file_level("loud", "GIRPR");
        assert_eq!(level, None, "the value is dropped");
        let warning = warning.expect("the user is told");
        assert!(warning.contains("GIRPR_LOG_FILE_LEVEL"), "{warning}");
        assert!(warning.contains("loud"), "{warning}");
        assert!(warning.contains("debug"), "{warning}");

        let (level, warning) = parse_file_level("trace", "GIRPR");
        assert_eq!(level, Some(Level::Trace));
        assert_eq!(warning, None);
    }
}
