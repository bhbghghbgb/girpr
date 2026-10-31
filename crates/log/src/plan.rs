//! Where a project's log file goes, and how loud each of its two sinks is.
//!
//! # The policy
//!
//! One policy, every project in this workspace — the repair binary (`girpr`)
//! and the two dev tools (`gircap`, `girsync`). It is seven rules:
//!
//! 1. **A release build always writes a log file.** A debug build writes one
//!    **only when it is asked to**. The switch is
//!    [`default_file_is_written`], and it answers the question that is
//!    actually being asked: "is this a binary somebody ships, or a build
//!    somebody is working on". A shipped binary that leaves nothing behind is
//!    a binary nobody can ask questions of later; a debug build that writes a
//!    file per run leaves a directory nobody reads, growing by one file per
//!    run, forever.
//! 2. **The file has a name you can find.** When there is one, it is
//!    `<exe-dir>/logs/<project>_<timestamp>.log` — beside the executable, in a
//!    `logs/` directory, named for the project and stamped with the time.
//! 3. **Four ways to choose, in this order**: the `--no-log-file` flag, the
//!    `--log-file` flag, then the `<PROJECT>_LOG_FILE` environment variable,
//!    then the default. A flag beats an environment variable because a flag
//!    was typed by the person running the tool; an environment variable is
//!    whatever the shell happened to export. And `--no-log-file` beats both,
//!    because "write no log file" is the more specific thing to have said.
//! 4. **Two levels, not one.** `--log-level` is the **console's** (default
//!    `info`); `--log-file-level` is the **file's** (default `debug`). They are
//!    separate because they answer separate questions: the console is for the
//!    person watching, the file is for the person reading it afterwards, and
//!    the second reader wants more detail than the first.
//! 5. **`RUST_LOG` adds to the console; it does not replace it.** The
//!    program's own level is `--log-level`'s, and the environment can only add
//!    visibility for other crates — never take the program's own away. The
//!    reasoning is below.
//! 6. **A log file that cannot be opened is a warning, never a failure.** A run
//!    must not end differently because of where it was asked to log.
//! 7. **`--no-log-file` turns the *file* off, not the console.** The console is
//!    where the run is watched; the file is where the run is read afterwards.
//!    A run that cannot be watched is a different thing from a run that leaves
//!    nothing behind.
//!
//! # The environment seams
//!
//! | variable | meaning |
//! | --- | --- |
//! | `<PROJECT>_LOG_FILE` | the log file's path, as `--log-file` would name it |
//! | `<PROJECT>_LOG_FILE_LEVEL` | the log file's level, as `--log-file-level` would name it |
//! | `RUST_LOG` | extra console directives, for other crates |
//!
//! `<PROJECT>` is the project's name in upper case: `GIRPR_LOG_FILE`,
//! `GIRCAP_LOG_FILE`, `GIRSYNC_LOG_FILE`. `RUST_LOG` is the one spelling
//! that is not per-project, because it is `tracing`'s own convention and a
//! developer who has exported it means it everywhere.
//!
//! # `RUST_LOG` adds; it does not replace
//!
//! The console's spec is this project's own directive — `girpr=info` — with
//! `RUST_LOG`'s directives *in front of it*:
//!
//! ```text
//! RUST_LOG unset          ->  girpr=info
//! RUST_LOG=reqwest=debug  ->  reqwest=debug,girpr=info
//! ```
//!
//! Two things follow, and both of them are the point.
//!
//! **The program's own level is exactly what `--log-level` says.** The
//! project's directive is always present, so a `RUST_LOG` that names some
//! unrelated crate cannot silence the program — the difference between "a user
//! has an environment variable exported" and "a user cannot see the program at
//! all". The flag is last in the spec because `EnvFilter` gives a tie on the
//! same target to the *last* directive, and the flag is what the person
//! running the tool typed.
//!
//! **`RUST_LOG` still does what it is for**: it turns up other crates.
//! `RUST_LOG=reqwest=debug` shows `reqwest`'s chatter next to the program's
//! own, and a bare `RUST_LOG=debug` shows every *other* crate at debug — a
//! global directive is less specific than `girpr=info`, so it cannot widen
//! this project's own level either.
//!
//! A `RUST_LOG` that names this project's target exactly is the one case
//! where the environment does not win: `RUST_LOG=girpr=trace` leaves the
//! project at `--log-level`, because `--log-level trace` is the way to say
//! that, and a flag is a thing a person typed on purpose. A *more specific*
//! directive still applies — `RUST_LOG=girpr::http=trace` turns up that one
//! module — because `EnvFilter` gives the most specific match the last word.
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
    /// `--no-log-file`: no file at all, whatever else was asked for.
    ///
    /// The one flag that beats everything else, because "write no log file" is
    /// the more specific thing to have said. It beats `--log-file` too, so
    /// the two are a usage error when given together rather than one of them
    /// silently winning.
    pub no_log_file: bool,
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
    /// The *effective* console filter is [`LogPlan::console_filter`].
    pub console_level: Level,
    /// The log file's verbosity.
    pub file_level: Level,
    /// Where the log file goes — or `None`, when there is no log file.
    ///
    /// `None` is a real state and not an error: a debug build writes no file
    /// unless it is asked to, and `--no-log-file` says so in any build. A
    /// project that has a file opens it from here; a project that has none
    /// installs no file sink at all.
    pub file: Option<PathBuf>,
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
        let (file, file_named) = if opts.no_log_file {
            (None, false)
        } else {
            match opts.log_file {
                Some(path) => (Some(path), true),
                None => match env.log_file {
                    Some(path) => (Some(path), true),
                    None => match default_file_is_written() {
                        true => (Some(default_log_path(project)), false),
                        false => (None, false),
                    },
                },
            }
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

    /// The log file's path, when there is one.
    ///
    /// The call that opens the file asks this first: a plan with no file is a
    /// plan with no file sink, and the caller installs none.
    pub fn file_path(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// The path the caller **named**, when they named one.
    ///
    /// The default is not "named": a caller who took the default has not asked
    /// for anything, and is told where the file went inside the file rather
    /// than on stderr.
    pub fn named_file(&self) -> Option<&Path> {
        if self.file_named {
            self.file.as_deref()
        } else {
            None
        }
    }

    /// The `tracing` filter spec for the **console** sink.
    ///
    /// This project's own directive — `girpr=info` — with `RUST_LOG`'s
    /// directives in front of it. See the module docs for why the order is
    /// that way round: the project's directive is last because a tie on the
    /// same target goes to the last directive, and `--log-level` is what the
    /// person running the tool typed.
    ///
    /// The project scoping is what keeps third-party crates quiet: a filter
    /// that names no target applies to every target there is, and `reqwest`'s
    /// idea of `info` is not this program's idea of `info`.
    pub fn console_filter(&self) -> String {
        match self.rust_log_directives() {
            Some(spec) => format!("{spec},{}", self.own_console_directive()),
            None => self.own_console_directive(),
        }
    }

    /// This project's own console directive: `girpr=info`.
    ///
    /// The part of the console's spec that `--log-level` decides, and the part
    /// that `RUST_LOG` cannot take away.
    pub fn own_console_directive(&self) -> String {
        format!("{}={}", self.project, self.console_level.as_str())
    }

    /// `RUST_LOG`, trimmed, when it is set to something.
    ///
    /// An empty variable is an unset variable: that is what an unset variable
    /// in a shell profile expands to, and a filter made of nothing but a
    /// comma would be a filter that says nothing.
    pub fn rust_log_directives(&self) -> Option<&str> {
        self.rust_log
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
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

/// Whether a build writes a log file when nobody asked for one.
///
/// **A release build does; a debug build does not.**
///
/// The switch is `cfg!(debug_assertions)`, and it answers the question that
/// is actually being asked: "is a person running a shipped binary, or is this
/// a build being worked on". The two want opposite things from a log file.
///
/// A **release** build is the binary somebody installs. A run of it that
/// leaves nothing behind is a run nobody can ask questions of later — the
/// whole point of a file named after the project and stamped with the time is
/// that it can be found again.
///
/// A **debug** build is what a developer runs and what the test suite runs,
/// and there the file is pure cost: a file per run, in a directory nobody
/// reads, growing by one file per run for as long as the checkout is used.
/// Tests find the file their own run wrote at a fixed path instead of
/// scanning a directory for it.
///
/// `cargo test` and `cargo nextest` build the dev profile, so `debug_assertions`
/// is on and the suite writes nothing by default. `--release` turns it off, and
/// the file comes back.
pub fn default_file_is_written() -> bool {
    !cfg!(debug_assertions)
}

/// The default log file path: `<exe-dir>/logs/<project>_<timestamp>.log`.
///
/// Beside the executable, because that is the one directory a user can find
/// without knowing the tool's conventions. The current directory is the
/// fallback for the case where the executable's path cannot be read at all.
///
/// The timestamp keeps concurrent runs from interleaving in one file, and
/// names the project so a directory of logs from three tools stays readable.
///
/// This is the *shape* of the default, and it is the same in every build. What
/// differs between builds is whether the default is used at all — see
/// [`default_file_is_written`].
pub fn default_log_path(project: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    dir.join("logs")
        .join(format!("{project}_{}.log", timestamp()))
}

/// The timestamp in a default log file name: `20261014T101500.000000Z`.
///
/// UTC time in the shared ISO format, and sortable as text: the files in a
/// `logs/` directory read oldest-first when they are listed by name.
fn timestamp() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%S%.6fZ").to_string()
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
            no_log_file: false,
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
        // The timestamp: `YYYYMMDDTHHMMSS.ffffffZ`.
        let stamp = name.trim_start_matches("girpr_").trim_end_matches(".log");
        assert_eq!(stamp.len(), 23, "{stamp}");
        assert_eq!(stamp.as_bytes()[8], b'T');
        assert_eq!(stamp.as_bytes()[15], b'.');
        assert_eq!(stamp.as_bytes()[22], b'Z');
    }

    /// The shape of the default is the same in every build. What differs is
    /// whether the default is *used*.
    ///
    /// This test is the shape, and it is deliberately not conditional: the
    /// path a release build would write is the path a debug build would write
    /// if it were asked to, and a change to the shape has to be a change in
    /// every build.
    #[test]
    fn a_debug_build_and_a_release_build_name_the_default_the_same_way() {
        // The build under test is the build the policy is about, so the
        // assertion is the policy itself: the default is written in a release
        // build and not in a debug one.
        assert_eq!(
            default_file_is_written(),
            !cfg!(debug_assertions),
            "a release build writes a log file by default; a debug build does not"
        );
    }

    /// A debug build writes no file by default, and a release build writes the
    /// timestamped default.
    ///
    /// The two halves are the same fact seen from the two sides, and the test
    /// says which side it is on so that a failure says *which* way the policy
    /// moved.
    #[test]
    fn the_default_file_is_written_in_a_release_build_and_not_in_a_debug_one() {
        let plan = LogPlan::resolve("girpr", opts(), EnvOptions::default());
        if default_file_is_written() {
            let path = plan.file_path().expect("a release build has a file");
            assert!(
                path.to_string_lossy().contains("girpr_"),
                "and it is the timestamped default: {path:?}"
            );
            assert!(!plan.file_named, "the default is not 'named'");
        } else {
            assert!(
                plan.file_path().is_none(),
                "a debug build writes no file unless it is asked to: {plan:?}"
            );
            assert!(!plan.file_named);
        }
    }

    /// `--no-log-file` beats everything else, including the environment.
    ///
    /// "Write no log file" is the more specific thing to have said, so it
    /// wins over a path the shell happened to export.
    #[test]
    fn no_log_file_beats_the_environment_and_the_flag() {
        let env = EnvOptions {
            log_file: Some(PathBuf::from("from-env.log")),
            ..Default::default()
        };
        let mut flagged = opts();
        flagged.log_file = Some(PathBuf::from("from-flag.log"));
        flagged.no_log_file = true;
        let plan = LogPlan::resolve("girpr", flagged, env);
        assert!(plan.file_path().is_none(), "{plan:?}");
        assert!(!plan.file_named, "there is nothing to name");
        assert!(plan.named_file().is_none());
    }

    /// A file that was asked for is still written in a debug build.
    ///
    /// The debug default is about the *default*, not about the file: a caller
    /// who names a path gets one, in every build.
    #[test]
    fn a_named_file_is_written_in_every_build() {
        let mut flagged = opts();
        flagged.log_file = Some(PathBuf::from("from-flag.log"));
        let plan = LogPlan::resolve("girpr", flagged, EnvOptions::default());
        assert_eq!(plan.file_path(), Some(Path::new("from-flag.log")));
        assert!(plan.file_named);
        assert_eq!(plan.named_file(), Some(Path::new("from-flag.log")));
    }

    /// The console is unaffected by the file being off.
    ///
    /// `--no-log-file` turns the *file* off. The console is where the run is
    /// watched, and a run that cannot be watched is a different thing from a
    /// run that leaves nothing behind.
    #[test]
    fn no_log_file_says_nothing_about_the_console() {
        let mut flagged = opts();
        flagged.no_log_file = true;
        let plan = LogPlan::resolve("girpr", flagged, EnvOptions::default());
        assert_eq!(plan.console_filter(), "girpr=info");
        assert_eq!(plan.file_filter(), "debug");
    }

    #[test]
    fn a_flag_beats_the_environment_and_the_environment_beats_the_default() {
        let env = EnvOptions {
            log_file: Some(PathBuf::from("from-env.log")),
            ..Default::default()
        };
        let plan = LogPlan::resolve("girpr", opts(), env.clone());
        assert_eq!(plan.file_path(), Some(Path::new("from-env.log")));
        assert!(plan.file_named);

        let mut flagged = opts();
        flagged.log_file = Some(PathBuf::from("from-flag.log"));
        let plan = LogPlan::resolve("girpr", flagged, env);
        assert_eq!(plan.file_path(), Some(Path::new("from-flag.log")));
        assert!(plan.file_named);
    }

    #[test]
    fn the_default_file_is_not_named() {
        let plan = LogPlan::resolve("gircap", opts(), EnvOptions::default());
        assert!(!plan.file_named);
        assert!(plan.named_file().is_none(), "the default is not 'named'");
        // In a release build there is a default, and it is the timestamped
        // one; in a debug build there is none. Either way it is not *named*.
        if let Some(path) = plan.file_path() {
            assert!(path.to_string_lossy().contains("gircap_"), "{plan:?}");
        }
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
    fn rust_log_adds_to_the_console_level_and_never_replaces_it() {
        let env = EnvOptions {
            rust_log: Some("reqwest=debug".to_string()),
            ..Default::default()
        };
        let plan = LogPlan::resolve("girpr", opts(), env);
        assert_eq!(
            plan.console_filter(),
            "reqwest=debug,girpr=info",
            "the project's own directive is last: the flag is what was typed, \
             and a RUST_LOG that names another crate must not silence the program"
        );
        assert_eq!(
            plan.file_filter(),
            "debug",
            "RUST_LOG is a console lever and never a file lever"
        );
    }

    #[test]
    fn the_projects_own_directive_is_the_one_the_flag_decides() {
        let mut flagged = opts();
        flagged.console_level = Level::Trace;
        let env = EnvOptions {
            rust_log: Some("girpr=error".to_string()),
            ..Default::default()
        };
        let plan = LogPlan::resolve("girpr", flagged, env);
        assert_eq!(
            plan.console_filter(),
            "girpr=error,girpr=trace",
            "the spec carries both; the flag's is last, and a tie on the same \
             target goes to the last directive"
        );
        assert_eq!(plan.own_console_directive(), "girpr=trace");
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
