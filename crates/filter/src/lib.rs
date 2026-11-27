//! The `--include` / `--exclude` glob-filtering rules shared by girpr and
//! girsync.
//!
//! Both programs filter `/`-separated paths with [`globset`] under the same
//! matching rules: `*` crosses `/`, and a leading dot is not special. They
//! differ in only two places, both kept as arguments here rather than guessed
//! at:
//!
//! - the case mode — girpr hardcodes case-insensitive, girsync takes it from
//!   its run-wide `--case-sensitive` setting;
//! - what `\` means in a user-written glob — girpr treats it as a path
//!   separator (normalizing to `/`), girsync leaves it to globset as an escape.
//!
//! This crate owns those rules so the two filters cannot drift. Each filter
//! keeps only its own policy on top: girpr's `PathScope` and girsync's
//! subtree rule, `Filter` and `ExcludeMemo`. Compile-time case mode is a
//! globset constraint, so a [`Compiled`] holds a matcher for each mode and the
//! caller selects one when it asks a question.

pub use globset;

use anyhow::{Context, Result};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use std::borrow::Cow;
use std::fmt;

/// How a `\` in a user-written glob is interpreted.
#[derive(Clone, Copy, Debug)]
pub enum Backslash {
    /// A path separator: normalize `\` to `/` before compiling (girpr).
    Separator,
    /// A glob escape: leave the string for globset to consume (girsync).
    Escape,
}

/// One user-written glob, compiled into globset matchers.
///
/// globset fixes the case mode when it builds a matcher, while the case mode
/// here is a run-wide setting that girsync can change after compilation. A
/// `Compiled` therefore keeps both modes, and two questions per mode:
///
/// - `direct` matches the path itself (girpr's includes, girsync's includes);
/// - `subtree` matches the path itself or one of its ancestor directories,
///   implemented by adding `glob/**` alongside `glob` — no prefix walker.
#[derive(Clone)]
pub struct Compiled {
    written: String,
    direct: [GlobSet; 2],
    subtree: [GlobSet; 2],
}

impl fmt::Debug for Compiled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Compiled").field(&self.written).finish()
    }
}

impl Compiled {
    /// The glob exactly as the user wrote it, for diagnostics.
    pub fn written(&self) -> &str {
        &self.written
    }

    /// True when the glob matches `rel` itself.
    pub fn matches(&self, rel: &str, case_sensitive: bool) -> bool {
        self.direct[usize::from(case_sensitive)].is_match(rel)
    }

    /// True when the glob matches `rel` itself or an ancestor directory.
    pub fn matches_or_ancestor(&self, rel: &str, case_sensitive: bool) -> bool {
        self.subtree[usize::from(case_sensitive)].is_match(rel)
    }
}

/// Compile glob strings, naming the offending pattern on failure.
///
/// `backslash` selects whether `\` is normalized to `/` first. The error
/// always quotes the string as the user wrote it.
pub fn compile(list: &[String], backslash: Backslash) -> Result<Vec<Compiled>> {
    let escape = matches!(backslash, Backslash::Escape);
    list.iter()
        .map(|s| {
            let normalized: Cow<'_, str> = match backslash {
                Backslash::Separator => Cow::Owned(girpr_fs_names::forward_slash(s, true)),
                Backslash::Escape => Cow::Borrowed(s.as_str()),
            };
            let build = |case_sensitive: bool, subtree: bool| -> Result<GlobSet> {
                let mut builder = GlobSetBuilder::new();
                let globs = if subtree {
                    vec![normalized.to_string(), format!("{normalized}/**")]
                } else {
                    vec![normalized.to_string()]
                };
                for glob in globs {
                    let built = GlobBuilder::new(&glob)
                        .case_insensitive(!case_sensitive)
                        .literal_separator(false)
                        .backslash_escape(escape)
                        .build()
                        .with_context(|| format!("bad glob '{s}'"))?;
                    builder.add(built);
                }
                builder.build().with_context(|| format!("bad glob '{s}'"))
            };
            Ok(Compiled {
                written: s.clone(),
                direct: [build(false, false)?, build(true, false)?],
                subtree: [build(false, true)?, build(true, true)?],
            })
        })
        .collect()
}

/// True when any of `patterns` matches `rel` itself.
pub fn matches_any(patterns: &[Compiled], rel: &str, case_sensitive: bool) -> bool {
    patterns.iter().any(|p| p.matches(rel, case_sensitive))
}

/// True when `pattern` matches `rel` itself or one of its ancestors.
///
/// The subtree rule is what makes an excluded directory cover its contents.
pub fn matches_or_ancestor(pattern: &Compiled, rel: &str, case_sensitive: bool) -> bool {
    pattern.matches_or_ancestor(rel, case_sensitive)
}

/// True when any of `patterns` matches `rel` or one of its ancestors.
pub fn any_matches_or_ancestor(patterns: &[Compiled], rel: &str, case_sensitive: bool) -> bool {
    patterns
        .iter()
        .any(|p| p.matches_or_ancestor(rel, case_sensitive))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiled(list: &[&str], backslash: Backslash) -> Vec<Compiled> {
        let owned: Vec<String> = list.iter().map(|s| s.to_string()).collect();
        compile(&owned, backslash).unwrap()
    }

    #[test]
    fn separators_and_leading_dots_are_not_special() {
        let patterns = compiled(&["*.blk", "*", "**/*.blk"], Backslash::Escape);
        assert!(matches_any(&patterns[..1], "a/b/00.dat.blk", false));
        assert!(matches_any(&patterns[1..2], ".hidden", false));
        assert!(matches_any(&patterns[2..], "09801749.blk", false));
    }

    #[test]
    fn backslash_mode_controls_normalization() {
        let as_separator = compiled(&["game\\foo.dat"], Backslash::Separator);
        let as_escape = compiled(&["game\\foo.dat"], Backslash::Escape);
        // The normalized form matches a `/`-separated path on every host.
        assert!(matches_any(&as_separator, "game/foo.dat", true));
        // The escape form is a different glob: `\` is consumed by globset.
        assert!(matches_any(&as_escape, "gamefoo.dat", true));
        assert!(!matches_any(&as_escape, "game/foo.dat", true));
    }

    #[test]
    fn ancestor_matching_respects_the_separator_boundary() {
        let patterns = compiled(&["link"], Backslash::Escape);
        assert!(any_matches_or_ancestor(&patterns, "link/x.txt", true));
        assert!(any_matches_or_ancestor(&patterns, "link/sub/y.txt", true));
        assert!(!any_matches_or_ancestor(&patterns, "linkage/x.txt", true));
    }

    #[test]
    fn a_double_star_glob_is_a_valid_subtree_rule() {
        let patterns = compiled(&["a/**", "b"], Backslash::Escape);
        assert!(any_matches_or_ancestor(&patterns, "a/x/y.txt", true));
        assert!(any_matches_or_ancestor(&patterns, "b/c/d.txt", true));
        assert!(!any_matches_or_ancestor(&patterns, "c/d.txt", true));
    }

    #[test]
    fn a_bad_glob_is_named_as_written() {
        let owned = vec!["[".to_string()];
        let err = compile(&owned, Backslash::Escape).unwrap_err();
        assert!(format!("{err:#}").contains("bad glob '['"));
    }
}
