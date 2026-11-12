//! `--include` / `--exclude` glob filtering.
//!
//! Filtered-out paths are treated as nonexistent on *both* sides: skipped in
//! walks, dropped from records, pruned from caches, and left untouched in dst.
//!
//! **An excluded directory excludes its subtree.** A glob names paths, and a
//! directory that matches is a whole subtree, not one entry: `--exclude link`
//! has to cover `link/x.txt` too, or the walk still reports the link's target
//! contents and a later pass can act on them. That was the shape that let
//! `--exclude link` delete the excluded link instead of leaving it alone. The
//! same prefix rule applies to any exclude glob, not just a literal one.

use anyhow::{Context, Result};
use glob::{MatchOptions, Pattern};

/// Compile glob strings, reporting the offending pattern on failure.
pub fn compile_patterns(list: &[String]) -> Result<Vec<Pattern>> {
    list.iter()
        .map(|s| Pattern::new(s).with_context(|| format!("bad glob '{}'", s)))
        .collect()
}

/// The run's include and exclude rules, compiled once.
///
/// **One value, one home.** The patterns and the options they are matched with
/// travel together, so a caller cannot ask whether a path is excluded without
/// also saying which rules apply — and the options are built once per run
/// rather than once per path.
///
/// The case mode is not owned here. It is a run-wide setting that also selects
/// the cache's case flag, the planner's pairing, the rename pass and the mkdir
/// check, so it stays on [`crate::config::CommonOpts`]; this value is *built*
/// from it, and the two have to agree.
#[derive(Debug, Clone)]
pub struct Filter {
    includes: Vec<Pattern>,
    excludes: Vec<Pattern>,
    opts: MatchOptions,
}

impl Filter {
    /// The rules of one run, matched under `case_sensitive`.
    ///
    /// The matching options are built here and kept, so every path this filter
    /// is asked about is matched the same way.
    pub fn new(includes: Vec<Pattern>, excludes: Vec<Pattern>, case_sensitive: bool) -> Self {
        Self {
            includes,
            excludes,
            opts: MatchOptions {
                case_sensitive,
                require_literal_separator: false,
                require_literal_leading_dot: false,
            },
        }
    }

    /// True when `rel` should be ignored. `--exclude` beats `--include`.
    ///
    /// The include patterns are a filter on the path itself, not on its
    /// ancestors: a pattern that matches only a descendant keeps that
    /// descendant. Phase A pulls the ancestor directories back into the
    /// effective map so a link above an included path stays visible to the
    /// planner; see `effective::phase_a`.
    pub fn is_excluded(&self, rel: &str) -> bool {
        if !self.includes.is_empty()
            && !self.includes.iter().any(|p| p.matches_with(rel, self.opts))
        {
            return true;
        }
        self.excludes
            .iter()
            .any(|p| matches_or_ancestor(p, rel, self.opts))
    }

    /// The `--include` patterns, as compiled.
    pub fn includes(&self) -> &[Pattern] {
        &self.includes
    }

    /// The `--exclude` patterns, as compiled.
    pub fn excludes(&self) -> &[Pattern] {
        &self.excludes
    }
}

/// `pattern` matches `rel`, or matches one of `rel`'s ancestor directories.
///
/// The ancestor test is what makes an excluded directory cover its subtree. It
/// walks prefixes on `/` boundaries, so `link` covers `link/x.txt` but not
/// `linkage/x.txt`.
fn matches_or_ancestor(pattern: &Pattern, rel: &str, opts: MatchOptions) -> bool {
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
