//! The verbosity of one sink.
//!
//! A closed set of five names, in one place, because every project in this
//! workspace spells them the same way: `error`, `warn`, `info`, `debug`,
//! `trace`.
//!
//! It is a *closed* set rather than free text on purpose. `tracing`'s
//! `EnvFilter` accepts a whole directive language (`target=level`, spans,
//! field filters), and letting that language reach a flag would make
//! `--log-level` capable of things no flag documentation describes. A level is
//! a level.

use std::fmt;
use std::str::FromStr;

/// How verbose one sink is.
///
/// Ordered from quietest to loudest, so `Level::Error <= Level::Trace` is the
/// comparison a filter is asking about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Level {
    /// Only failures.
    Error,
    /// Failures and things that look like them.
    Warn,
    /// The run's own milestones. The **console** default.
    #[default]
    Info,
    /// The detail behind the milestones. The **log file** default.
    Debug,
    /// Everything, including the per-entry chatter.
    Trace,
}

impl Level {
    /// The `tracing` filter spelling of this verbosity.
    ///
    /// The same word in every filter spec this workspace builds, so a reader
    /// who knows one knows all three.
    pub const fn as_str(self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warn => "warn",
            Level::Info => "info",
            Level::Debug => "debug",
            Level::Trace => "trace",
        }
    }

    /// Parse a level name, case-insensitively.
    ///
    /// `warning` is accepted as a spelling of `warn`, because it is the word
    /// people type. The error names the whole set, so a typo is fixable from
    /// the message alone.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "error" => Ok(Level::Error),
            "warn" | "warning" => Ok(Level::Warn),
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

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Level {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Level::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_level_parses_in_any_case_and_rejects_the_rest() {
        for (s, want) in [
            ("error", Level::Error),
            ("WARN", Level::Warn),
            ("Warning", Level::Warn),
            ("Info", Level::Info),
            ("debug", Level::Debug),
            ("TRACE", Level::Trace),
            ("  trace  ", Level::Trace),
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
        assert_eq!(Level::default().as_str(), "info");
    }

    #[test]
    fn the_levels_are_ordered_from_quiet_to_loud() {
        assert!(Level::Error < Level::Warn);
        assert!(Level::Warn < Level::Info);
        assert!(Level::Info < Level::Debug);
        assert!(Level::Debug < Level::Trace);
    }

    #[test]
    fn the_display_is_the_filter_spelling() {
        for level in [
            Level::Error,
            Level::Warn,
            Level::Info,
            Level::Debug,
            Level::Trace,
        ] {
            assert_eq!(level.to_string(), level.as_str());
        }
    }
}
