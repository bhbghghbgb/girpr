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

/// True when `rel` should be ignored. `--exclude` beats `--include`.
///
/// `includes` is a filter on the path itself, not on its ancestors: a pattern
/// that matches only a descendant keeps that descendant. Phase A pulls the
/// ancestor directories back into the effective map so a link above an included
/// path stays visible to the planner; see `effective::phase_a`.
pub fn is_excluded(
    rel: &str,
    includes: &[Pattern],
    excludes: &[Pattern],
    case_sensitive: bool,
) -> bool {
    let opts = MatchOptions {
        case_sensitive,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    };
    if !includes.is_empty() && !includes.iter().any(|p| p.matches_with(rel, opts)) {
        return true;
    }
    if excludes.iter().any(|p| matches_or_ancestor(p, rel, opts)) {
        return true;
    }
    false
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
