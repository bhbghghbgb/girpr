//! Shared clap-error to process-code mechanics, parameterised by the caller.
//!
//! A clap parse error has two kinds of outcome: `--help` and `--version` are
//! answers and map to `0`; every other error is a usage error and maps to the
//! caller's own usage code. This crate decides only that number. It holds no
//! exit table, no code of any program and no process boundary — printing clap's
//! message and turning the number into an exit status stay with the caller.

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
