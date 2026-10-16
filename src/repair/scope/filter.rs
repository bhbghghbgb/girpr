//! `--include` / `--exclude` glob filtering.
//!
//! **Filtered-out paths are treated as nonexistent on both sides**: skipped in
//! walks, dropped from the plan, left untouched by the cleanup. Ported from
//! `tools/sync/src/filter.rs` (girsync's semantics) — deliberately, because
//! this repo already made that decision once and `girpr` has no business
//! growing a second spelling of it. No code is shared: the crates are
//! independent by design (`tools/sync` is a sibling tool, not a dependency).
//!
//! Matched against the `/`-separated, game-dir-rooted rel path — the same
//! string `plan::build_plan` stores and `util::normalize_rel` converts at
//! the filesystem boundary.
//!
//! Three semantics are inherited deliberately and must not be "fixed" here:
//!
//! - **`*` crosses `/`.** With `require_literal_separator: false`,
//!   `--include "*.blk"` matches
//!   `GenshinImpact_Data/StreamingAssets/.../00/09801749.blk`. A user expecting
//!   `*.blk` to mean "in the current directory" is surprised; the flag help says
//!   so. (`**/` also works and additionally matches at the root.)
//! - **Case-insensitive, with no flag.** Game paths are Windows and the live
//!   manifests are lowercase anyway; `girsync` defaults `--case-sensitive` to
//!   `false` and this port hardcodes that.
//! - **`--exclude` beats `--include`.** Last-writer-wins would be the
//!   alternative; exclusion wins so a narrow `--include` can always be widened
//!   down without reordering anything.

use anyhow::{Context, Result};
use glob::{MatchOptions, Pattern};

/// girsync's matching options, hardcoded (see the module docs).
fn match_options() -> MatchOptions {
    MatchOptions {
        case_sensitive: false,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    }
}

/// The set of globs narrowing one run, compiled once at startup.
///
/// An empty `includes` admits **everything** — that is what makes
/// `--exclude`-only and unflagged runs identical.
#[derive(Debug, Default)]
pub struct PathScope {
    includes: Vec<Pattern>,
    excludes: Vec<Pattern>,
}

impl PathScope {
    /// Compile the raw `--include` / `--exclude` lists. Both may be empty.
    pub fn new(includes: &[String], excludes: &[String]) -> Result<Self> {
        Ok(Self {
            includes: compile_patterns(includes)?,
            excludes: compile_patterns(excludes)?,
        })
    }

    /// `true` when this scope admits everything — no `--include` was given.
    /// An `--exclude`-only scope is *not* open: it still hides paths, so it
    /// still counts as an active scope for the `config.ini` decision.
    pub fn is_open(&self) -> bool {
        self.includes.is_empty() && self.excludes.is_empty()
    }

    /// `true` when at least one path is hidden — i.e. the run is scoped.
    pub fn is_active(&self) -> bool {
        !self.is_open()
    }

    /// `true` when `--include` was given. A non-empty `--include` that matches
    /// nothing is a usage error (a typo), while a non-matching `--exclude` is
    /// unremarkable — see [`Self::has_includes`].
    pub fn has_includes(&self) -> bool {
        !self.includes.is_empty()
    }

    /// `true` when `rel` is inside the scope. `--exclude` beats `--include`.
    pub fn is_visible(&self, rel: &str) -> bool {
        let opts = match_options();
        if !self.includes.is_empty() && !self.includes.iter().any(|p| p.matches_with(rel, opts)) {
            return false;
        }
        !self.excludes.iter().any(|p| p.matches_with(rel, opts))
    }

    /// The `--include` globs as written, for the "matched nothing" usage
    /// error — an error that names its own typo is the point.
    pub fn describe_includes(&self) -> String {
        join_patterns(&self.includes)
    }

    /// The `--exclude` globs as written, for log/diagnostic lines.
    pub fn describe_excludes(&self) -> String {
        join_patterns(&self.excludes)
    }
}

fn join_patterns(patterns: &[Pattern]) -> String {
    patterns
        .iter()
        .map(|p| p.as_str().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Compile glob strings, reporting the offending pattern on failure (exit 1).
///
/// `\` is normalized to `/` first: the paths being matched are `/`-separated
/// game-dir-rooted rel paths, and a Windows user writing
/// `--include "game\*.dat"` means a path separator, not a glob escape. The cost
/// is that `\` cannot escape a metacharacter, which is not a thing anyone types
/// at a game install.
pub fn compile_patterns(list: &[String]) -> Result<Vec<Pattern>> {
    list.iter()
        .map(|s| Pattern::new(&s.replace('\\', "/")).with_context(|| format!("bad glob '{s}'")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn scope(includes: &[&str], excludes: &[&str]) -> PathScope {
        PathScope::new(&owned(includes), &owned(excludes)).unwrap()
    }

    #[test]
    fn empty_scope_admits_everything() {
        let s = PathScope::default();
        assert!(s.is_open());
        assert!(!s.is_active());
        assert!(!s.has_includes());
        assert!(s.is_visible("a/b.dat"));
    }

    #[test]
    fn include_only_admits_exactly_what_it_matches() {
        let s = scope(&["game/foo.dat"], &[]);
        assert!(s.is_active());
        assert!(s.has_includes());
        assert!(s.is_visible("game/foo.dat"));
        assert!(!s.is_visible("game/bar.dat"));
    }

    #[test]
    fn exclude_only_hides_what_it_matches() {
        let s = scope(&[], &["**/*.blk"]);
        assert!(s.is_active());
        assert!(!s.has_includes());
        assert!(!s.is_visible("a/b/x.blk"));
        assert!(s.is_visible("game/foo.dat"));
    }

    #[test]
    fn exclude_beats_include() {
        let s = scope(&["game/*.dat"], &["game/bar.dat"]);
        assert!(s.is_visible("game/foo.dat"));
        assert!(!s.is_visible("game/bar.dat"));
    }

    #[test]
    fn star_crosses_slashes_and_double_star_is_recursive() {
        // Inherited from girsync: `*` is not "in the current directory", so the
        // two spellings below agree — `**/` adds nothing this use case needs,
        // but is the habit people reach for, so it must keep working.
        for glob in ["*.blk", "**/*.blk"] {
            let s = scope(&[glob], &[]);
            assert!(
                s.is_visible("GenshinImpact_Data/StreamingAssets/xx/00/09801749.blk"),
                "{glob} must match at any depth"
            );
            assert!(
                s.is_visible("09801749.blk"),
                "{glob} must match at the root"
            );
            assert!(!s.is_visible("09801749.blk.tmp"), "{glob}");
        }
    }

    #[test]
    fn matching_is_case_insensitive() {
        let s = scope(&["GAMESHARED/X.dat"], &["**/*.TMP"]);
        assert!(s.is_visible("GameShared/x.DAT"));
        assert!(!s.is_visible("GameShared/x.tmp"));
    }

    #[test]
    fn several_patterns_of_each_kind_are_ored() {
        let s = scope(&["a/*.dat", "b/*.dat"], &["b/skip.dat"]);
        assert!(s.is_visible("a/one.dat"));
        assert!(s.is_visible("b/two.dat"));
        assert!(!s.is_visible("b/skip.dat"));
        assert!(!s.is_visible("c/three.dat"));
    }

    #[test]
    fn backslashes_are_normalized_before_matching() {
        // The rel path is `/`-separated everywhere in girpr; a Windows-style
        // glob from a user must not silently match nothing.
        let s = scope(&["game\\foo.dat"], &[]);
        assert!(s.is_visible("game/foo.dat"));
    }

    #[test]
    fn a_bad_pattern_is_an_error_naming_the_glob() {
        let err = PathScope::new(&owned(&["game/*.dat", "["]), &[]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("bad glob '['"), "{msg}");
        // ...and the exclude side too.
        let err = PathScope::new(&[], &owned(&["a["])).unwrap_err();
        assert!(format!("{err:#}").contains("bad glob"), "{err}");
    }
}
