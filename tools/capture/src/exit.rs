//! The exit-code contract, in one place.
//!
//! `gircap` has its own table and shares none of it with the repair binary. The
//! numbers are load-bearing — they are what a harness reads — so they live in one
//! enum rather than being scattered as literals through the commands.
//!
//! | code | meaning |
//! |---|---|
//! | `0` | artifact complete and every asserted invariant green |
//! | `1` | usage / configuration error |
//! | `2` | network or API failure (incl. an unservable `--old-tag`) |
//! | `3` | an assertion failed — the artifact is **not** trustworthy |
//! | `4` | a `--require-class` could not be satisfied |
//!
//! Note the deliberate overlap with the repair binary's table: `1`/`2` mean the
//! same thing in both tools. That is a coincidence of two independently designed
//! contracts, not a shared one — nothing in this crate reads the other tool's
//! codes, and either may be renumbered without consulting the other.

use std::fmt;
use std::process;

/// Every way a run of this tool can end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExitCode {
    /// The artifact is complete and every asserted invariant is green.
    Ok,
    /// The command line or the resolved configuration is wrong.
    Usage,
    /// A request failed, or the API answered something unusable.
    Network,
    /// An asserted invariant did not hold; the artifact cannot be trusted.
    AssertionFailed,
    /// A class named by `--require-class` was not satisfied.
    RequirementUnmet,
}

impl ExitCode {
    /// Every code, in table order. Used by tests to pin the table.
    pub const ALL: [ExitCode; 5] = [
        ExitCode::Ok,
        ExitCode::Usage,
        ExitCode::Network,
        ExitCode::AssertionFailed,
        ExitCode::RequirementUnmet,
    ];

    /// The process exit status this code maps to.
    pub const fn code(self) -> i32 {
        match self {
            ExitCode::Ok => 0,
            ExitCode::Usage => 1,
            ExitCode::Network => 2,
            ExitCode::AssertionFailed => 3,
            ExitCode::RequirementUnmet => 4,
        }
    }

    /// The short slug used in logs and in `SUMMARY exit_code=…`.
    pub const fn slug(self) -> &'static str {
        match self {
            ExitCode::Ok => "ok",
            ExitCode::Usage => "usage",
            ExitCode::Network => "network",
            ExitCode::AssertionFailed => "assertion-failed",
            ExitCode::RequirementUnmet => "requirement-unmet",
        }
    }

    /// The one-line meaning, as it appears in the docs table.
    pub const fn describe(self) -> &'static str {
        match self {
            ExitCode::Ok => "artifact complete and every asserted invariant green",
            ExitCode::Usage => "usage / configuration error",
            ExitCode::Network => "network or API failure",
            ExitCode::AssertionFailed => "an assertion failed",
            ExitCode::RequirementUnmet => "a required class could not be satisfied",
        }
    }

    /// `true` when the artifact this code describes is worth keeping.
    ///
    /// A usage or network failure is the caller's problem and leaves nothing
    /// trustworthy behind; an assertion failure is the *artifact's* problem and
    /// leaves a tree that must be read, not deleted.
    pub const fn artifact_is_untrustworthy(self) -> bool {
        matches!(self, ExitCode::AssertionFailed | ExitCode::RequirementUnmet)
    }
}

impl fmt::Display for ExitCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.code(), self.slug())
    }
}

impl From<ExitCode> for i32 {
    fn from(c: ExitCode) -> i32 {
        c.code()
    }
}

/// Exit the process with `code`, flushing stdout first.
///
/// The flush matters: stdout is the data plane, and a `SUMMARY` written into a
/// line-buffered handle can otherwise be lost when the process ends on a path
/// that never reaches a normal flush.
pub fn exit_with(code: ExitCode) -> ! {
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    process::exit(code.code())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is the contract. A renumbering that passes every other test
    /// suite would still break every harness reading these codes, so it is
    /// pinned here in full rather than one case at a time.
    #[test]
    fn the_exit_table_is_the_documented_one() {
        let table: Vec<(i32, &'static str)> =
            ExitCode::ALL.iter().map(|c| (c.code(), c.slug())).collect();
        assert_eq!(
            table,
            vec![
                (0, "ok"),
                (1, "usage"),
                (2, "network"),
                (3, "assertion-failed"),
                (4, "requirement-unmet"),
            ]
        );
    }

    #[test]
    fn codes_are_distinct_and_every_variant_is_in_all() {
        let mut seen: Vec<i32> = ExitCode::ALL.iter().map(|c| c.code()).collect();
        let total = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), total, "two codes collide: {seen:?}");
    }

    #[test]
    fn only_assertion_failures_mark_an_artifact_untrustworthy() {
        for c in ExitCode::ALL {
            let want = matches!(
                c,
                ExitCode::AssertionFailed | ExitCode::RequirementUnmet
            );
            assert_eq!(c.artifact_is_untrustworthy(), want, "{c}");
        }
    }

    #[test]
    fn display_carries_both_the_number_and_the_slug() {
        assert_eq!(ExitCode::Network.to_string(), "2 (network)");
        assert_eq!(i32::from(ExitCode::AssertionFailed), 3);
    }

    #[test]
    fn every_code_describes_itself() {
        for c in ExitCode::ALL {
            assert!(!c.describe().is_empty(), "{c} has no description");
        }
    }
}