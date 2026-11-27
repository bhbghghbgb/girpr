//! The parse/error and log-announcement shims shared by the three programs.
//!
//! These are the parts of a `main` that are pure mechanism: turn a clap parse
//! failure into the caller's usage status after printing clap's own message,
//! and render the "log file: …" line when a caller named a file. The process
//! boundary is injected by the caller, so a tool that must flush stdout before
//! `process::exit` and one that returns `ExitCode` to let destructors run keep
//! their own exit paths while sharing the decision and the bytes.

use std::path::Path;

use clap::error::ErrorKind;

/// The process code a clap parse error maps to.
///
/// `usage_code` is the caller's own usage status, passed in rather than looked
/// up here, so no exit code or exit table crosses between programs. Help and
/// version requests are `0` whatever the caller uses for a usage error.
pub fn clap_error_code(kind: ErrorKind, usage_code: u8) -> u8 {
    match kind {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
        _ => usage_code,
    }
}

/// Parse, or print clap's failure and hand the mapped code to `exit`.
///
/// Printing first and exiting through the injected closure is byte-for-byte
/// clap's own `Error::exit`, with the caller's usage status substituted for
/// clap's fixed `2`. `Ok` values pass through untouched.
pub fn parse_or_report_error<T>(
    result: Result<T, clap::Error>,
    usage_code: u8,
    exit: impl FnOnce(u8) -> !,
) -> T {
    match result {
        Ok(value) => value,
        Err(e) => {
            let _ = e.print();
            exit(clap_error_code(e.kind(), usage_code))
        }
    }
}

/// The raw-stderr line that acknowledges a caller-named log file.
///
/// The default path is announced inside the file already; a caller who *named*
/// the path never saw that default and may have silenced the console. This is
/// the whole line as a value so the process boundary (a plain `eprintln!`) and
/// the assertion on its bytes stay at each program's call site. `None` when the
/// file was not named. Pure: the plan is resolved and nothing here touches a
/// process, a sink or the environment.
pub fn log_file_announcement(plan: &girpr_log::LogPlan) -> Option<String> {
    plan.named_file()
        .map(|path: &Path| format!("log file: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::error::ErrorKind;

    use super::{clap_error_code, log_file_announcement, parse_or_report_error};
    use girpr_log::LogPlan;

    #[test]
    fn help_and_version_are_zero_for_any_usage_code() {
        for usage_code in [1u8, 3, 42] {
            assert_eq!(clap_error_code(ErrorKind::DisplayHelp, usage_code), 0);
            assert_eq!(clap_error_code(ErrorKind::DisplayVersion, usage_code), 0);
        }
    }

    #[test]
    fn every_other_error_kind_is_the_callers_usage_code() {
        for kind in [
            ErrorKind::UnknownArgument,
            ErrorKind::InvalidSubcommand,
            ErrorKind::InvalidValue,
            ErrorKind::MissingRequiredArgument,
            ErrorKind::ValueValidation,
        ] {
            assert_eq!(clap_error_code(kind, 7), 7, "{kind:?}");
        }
    }

    #[test]
    fn parse_or_report_error_passes_ok_through() {
        let value = parse_or_report_error(Ok(41u8), 7, |_| unreachable!());
        assert_eq!(value, 41);
    }

    #[test]
    fn the_announcement_renders_only_a_named_file() {
        let plan = LogPlan {
            project: "probe",
            console_level: girpr_log::Level::Info,
            file_level: girpr_log::Level::Debug,
            file: Some(PathBuf::from("run.log")),
            file_named: true,
            rust_log: None,
        };
        assert_eq!(
            log_file_announcement(&plan).as_deref(),
            Some("log file: run.log")
        );
        let defaulted = LogPlan {
            file_named: false,
            ..plan
        };
        assert_eq!(log_file_announcement(&defaulted), None);
    }
}
