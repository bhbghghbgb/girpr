//! The log-file and log-level policy every project in this workspace follows.
//!
//! One crate, three projects: the repair binary (`girpr`), the capture tool
//! (`gircap`) and the mirror (`girsync`). They are separate programs with
//! separate purposes, but they answer the same three questions the same way:
//!
//! - **Where does the log file go?** `<exe-dir>/logs/<project>_<timestamp>.log`,
//!   unless `--log-file` or `<PROJECT>_LOG_FILE` says otherwise.
//! - **How loud is each sink?** The console is `info` by default
//!   (`--log-level`); the file is `debug` by default (`--log-file-level`).
//! - **What happens when the file cannot be opened?** A warning on stderr,
//!   never a failed run.
//!
//! The rules, and the reasoning behind them, are in [`plan`]; the level names
//! are in [`level`]; the one place a broken filter spec is turned into a
//! warning is in [`filter`].
//!
//! # What this crate is not
//!
//! It is not a logging *implementation*. Each project still installs its own
//! `tracing` subscriber, because each one writes a different file format and
//! each one owns the writer that flushes it. What is shared is the policy:
//! the path, the levels, the precedence, and the names of the environment
//! variables. Two projects that each re-decided those would drift, and the
//! drift would show up as "why does this tool's log file live somewhere else".
//!
//! # Using it
//!
//! ```no_run
//! use girpr_log::{Level, LogOptions, LogPlan};
//!
//! let env = girpr_log::env_options("girpr");
//! let plan = LogPlan::resolve(
//!     "girpr",
//!     LogOptions {
//!         console_level: Level::Info,
//!         file_level: None,
//!         log_file: None,
//!     },
//!     env,
//! );
//! println!("log file: {}", plan.file.display());
//! ```

pub mod filter;
pub mod level;
pub mod plan;

pub use filter::filter;
pub use level::Level;
pub use plan::{EnvOptions, LogOptions, LogPlan, default_log_path, env_options};

/// Parse a level name, for use as a `clap` value parser.
///
/// A function rather than a `ValueEnum`, because the enum would have to live
/// next to `clap` — and this crate is deliberately the one dependency the
/// three projects share, so it is kept free of it.
pub fn parse_level(s: &str) -> Result<Level, String> {
    Level::parse(s)
}
