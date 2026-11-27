//! Shared CLI mechanics: the clap-error/exit decision, the five global flag
//! declarations, and the log-file announcement.
//!
//! A clap parse error has two kinds of outcome: `--help` and `--version` are
//! answers and map to `0`; every other error is a usage error and maps to the
//! caller's own usage code. This crate decides only that number and prints
//! clap's message — it holds no exit table, no program's code and no process
//! boundary. The caller injects the exit, so a tool that flushes stdout before
//! `process::exit` and one that returns `ExitCode` to let destructors run keep
//! their own boundary while sharing the mechanism.
//!
//! The global flags are declared once as a macro with the caller's published
//! help text passed in as data. See [`global_args!`] for why an item-level
//! macro rather than a flattened struct.

mod argv;
mod global;
mod shims;

pub use argv::argv_with_program;
pub use girpr_log::{Level, parse_level};
pub use girpr_report_core::OutputFormat;
pub use global::output_format_parser;
pub use shims::{clap_error_code, log_file_announcement, parse_or_report_error};
