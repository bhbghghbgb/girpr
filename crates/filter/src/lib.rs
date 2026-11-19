//! The `--include` / `--exclude` glob-filtering rules shared by girpr and
//! girsync.
//!
//! Both programs filter `/`-separated paths with [`glob::Pattern`] under the
//! same matching rules: `*` crosses `/`, and a leading dot is not special.
//! They differ in only two places, both kept as arguments here rather than
//! guessed at:
//!
//! - the case mode — girpr hardcodes case-insensitive, girsync takes it from
//!   its run-wide `--case-sensitive` setting;
//! - what `\` means in a user-written glob — girpr treats it as a path
//!   separator (normalizing to `/`), girsync leaves it to `glob` as an escape.
//!
//! This crate owns those rules so the two filters cannot drift. Each filter
//! keeps only its own policy on top: girpr's `PathScope` and girsync's
//! subtree rule, `Filter` and `ExcludeMemo`.

pub use glob;

use anyhow::{Context, Result};
use glob::{MatchOptions, Pattern};
use std::borrow::Cow;

/// How a `\` in a user-written glob is interpreted.
#[derive(Clone, Copy, Debug)]
pub enum Backslash {
    /// A path separator: normalize `\` to `/` before compiling (girpr).
    Separator,
    /// A glob escape: leave the string for `glob` to consume (girsync).
    Escape,
}

/// The matching options the filters share.
///
/// `require_literal_separator: false` is what makes `*` (and `**`) cross `/`;
/// `require_literal_leading_dot: false` is what makes `*` match dotfiles.
/// Only the case mode varies, and it is passed in.
pub const fn match_options(case_sensitive: bool) -> MatchOptions {
    MatchOptions {
        case_sensitive,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    }
}

/// Compile glob strings, naming the offending pattern on failure.
///
/// `backslash` selects whether `\` is normalized to `/` first. The error
/// always quotes the string as the user wrote it.
pub fn compile(list: &[String], backslash: Backslash) -> Result<Vec<Pattern>> {
    list.iter()
        .map(|s| {
            let normalized: Cow<'_, str> = match backslash {
                Backslash::Separator => Cow::Owned(s.replace('\\', "/")),
                Backslash::Escape => Cow::Borrowed(s.as_str()),
            };
            Pattern::new(&normalized).with_context(|| format!("bad glob '{s}'"))
        })
        .collect()
}

/// True when any of `patterns` matches `rel` itself.
pub fn matches_any(patterns: &[Pattern], rel: &str, opts: MatchOptions) -> bool {
    patterns.iter().any(|p| p.matches_with(rel, opts))
}

/// `pattern` matches `rel`, or matches one of `rel`'s ancestor directories.
///
/// The ancestor test is what makes an excluded directory cover its subtree.
/// It walks prefixes on `/` boundaries, so `link` covers `link/x.txt` but not
/// `linkage/x.txt`.
pub fn matches_or_ancestor(pattern: &Pattern, rel: &str, opts: MatchOptions) -> bool {
    if pattern.matches_with(rel, opts) {
        return true;
    }
    let mut end = 0;
    while let Some(i) = rel[end..].find('/') {
        let prefix = &rel[..end + i];
        if pattern.matches_with(prefix, opts) {
            return true;
        }
        end += i + 1;
    }
    false
}

/// True when any of `patterns` matches `rel` or one of its ancestors.
pub fn any_matches_or_ancestor(patterns: &[Pattern], rel: &str, opts: MatchOptions) -> bool {
    patterns.iter().any(|p| matches_or_ancestor(p, rel, opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiled(list: &[&str], backslash: Backslash) -> Vec<Pattern> {
        let owned: Vec<String> = list.iter().map(|s| s.to_string()).collect();
        compile(&owned, backslash).unwrap()
    }

    #[test]
    fn separators_and_leading_dots_are_not_special() {
        let opts = match_options(false);
        assert!(matches_any(
            &compiled(&["*.blk"], Backslash::Escape),
            "a/b/00.dat.blk",
            opts
        ));
        assert!(matches_any(
            &compiled(&["*"], Backslash::Escape),
            ".hidden",
            opts
        ));
        assert!(matches_any(
            &compiled(&["**/*.blk"], Backslash::Escape),
            "09801749.blk",
            opts
        ));
    }

    #[test]
    fn backslash_mode_controls_normalization() {
        let as_separator = compiled(&["game\\foo.dat"], Backslash::Separator);
        let as_escape = compiled(&["game\\foo.dat"], Backslash::Escape);
        // The separator form is rewritten before compiling; the escape form is
        // handed to `glob` verbatim.
        assert_eq!(as_separator[0].as_str(), "game/foo.dat");
        assert_eq!(as_escape[0].as_str(), "game\\foo.dat");
        // The normalized form matches a `/`-separated path on every host.
        assert!(matches_any(
            &as_separator,
            "game/foo.dat",
            match_options(true)
        ));
    }

    #[test]
    fn ancestor_matching_respects_the_separator_boundary() {
        let patterns = compiled(&["link"], Backslash::Escape);
        let opts = match_options(true);
        assert!(any_matches_or_ancestor(&patterns, "link/x.txt", opts));
        assert!(any_matches_or_ancestor(&patterns, "link/sub/y.txt", opts));
        assert!(!any_matches_or_ancestor(&patterns, "linkage/x.txt", opts));
    }
}
