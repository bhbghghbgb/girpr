//! `--include` / `--exclude` glob filtering.
//!
//! Filtered-out paths are treated as nonexistent on *both* sides: skipped in
//! walks, dropped from records, pruned from caches, and left untouched in dst.

use anyhow::{Context, Result};
use glob::{MatchOptions, Pattern};

/// Compile glob strings, reporting the offending pattern on failure.
pub fn compile_patterns(list: &[String]) -> Result<Vec<Pattern>> {
    list.iter()
        .map(|s| Pattern::new(s).with_context(|| format!("bad glob '{}'", s)))
        .collect()
}

/// True when `rel` should be ignored. `--exclude` beats `--include`.
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
    if excludes.iter().any(|p| p.matches_with(rel, opts)) {
        return true;
    }
    false
}
