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

use std::str::FromStr;

use strum::IntoEnumIterator;

/// How verbose one sink is.
///
/// Ordered from quietest to loudest, so `Level::Error <= Level::Trace` is the
/// comparison a filter is asking about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
// strum owns the filter spellings; `as_str` is the thin const seam over them.
// No `EnumString` derive: `FromStr` must keep accepting a trimmed, mixed-case
// name and report the same normalized spelling in its error, which is this
// wrapper's policy rather than a property of strum's generated parser.
#[derive(strum::Display, strum::EnumIter, strum::IntoStaticStr)]
#[strum(const_into_str)]
pub enum Level {
    /// Only failures.
    #[strum(serialize = "error")]
    Error,
    /// Failures and things that look like them.
    #[strum(serialize = "warn")]
    Warn,
    /// The run's own milestones. The **console** default.
    #[strum(serialize = "info")]
    #[default]
    Info,
    /// The detail behind the milestones. The **log file** default.
    #[strum(serialize = "debug")]
    Debug,
    /// Everything, including the per-entry chatter.
    #[strum(serialize = "trace")]
    Trace,
}

impl Level {
    /// The `tracing` filter spelling of this verbosity.
    ///
    /// The same word in every filter spec this workspace builds, so a reader
    /// who knows one knows all three.
    pub const fn as_str(self) -> &'static str {
        self.into_str()
    }

    /// Parse a level name, case-insensitively.
    ///
    /// `warning` is accepted as a spelling of `warn`, because it is the word
    /// people type. The error names the whole set, so a typo is fixable from
    /// the message alone.
    pub fn parse(s: &str) -> Result<Self, String> {
        let normalized = s.trim().to_ascii_lowercase();
        // The alias is this wrapper's policy, not a second spelling of `warn`
        // in the vocabulary: strum's `Warn` stays `warn` for Display and logs.
        if normalized == "warning" {
            return Ok(Level::Warn);
        }
        // The canonical names come from strum's iteration over the enum, so
        // there is no second list of spellings here to fall out of step.
        Level::iter()
            .find(|level| level.as_str() == normalized)
            .ok_or_else(|| {
                format!(
                    "unknown log level {normalized:?}; expected one of {}",
                    Level::iter()
                        .map(|level| level.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
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
