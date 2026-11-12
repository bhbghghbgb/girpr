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

#[cfg(test)]
mod tests {
    use super::*;

    /// Compiled patterns, from `&str`s.
    fn patterns(list: &[&str]) -> Vec<Pattern> {
        let owned: Vec<String> = list.iter().map(|s| s.to_string()).collect();
        compile_patterns(&owned).unwrap()
    }

    /// The rule the bundle replaced: build the match options per call and
    /// apply them.
    ///
    /// Kept as the reference the bundled filter has to agree with, so a change
    /// to the cached options cannot quietly change which paths a run hides.
    fn per_call_is_excluded(
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
        excludes.iter().any(|p| matches_or_ancestor(p, rel, opts))
    }

    /// An excluded directory is a subtree, and the separator is the boundary:
    /// `link` covers `link/x.txt` and not `linkage/x.txt`.
    #[test]
    fn an_excluded_directory_excludes_its_descendants() {
        let filter = Filter::new(vec![], patterns(&["link"]), true);
        assert!(filter.is_excluded("link"));
        assert!(filter.is_excluded("link/x.txt"));
        assert!(filter.is_excluded("link/sub/y.txt"));
        assert!(
            !filter.is_excluded("linkage/x.txt"),
            "a prefix is not a subtree: the separator is the boundary"
        );
        assert!(!filter.is_excluded("other/x.txt"));
    }

    /// An include is a filter on the path itself, not on its ancestors: a
    /// pattern that matches only a descendant keeps that descendant and hides
    /// its sibling.
    #[test]
    fn an_include_that_matches_only_a_descendant_keeps_that_descendant() {
        let filter = Filter::new(patterns(&["sub/*.txt"]), vec![], true);
        assert!(
            !filter.is_excluded("sub/keep.txt"),
            "the descendant the pattern names is kept"
        );
        assert!(
            filter.is_excluded("sub/skip.bin"),
            "and its sibling, which the pattern does not name, is not"
        );
        assert!(filter.is_excluded("top.txt"));
    }

    /// The case mode is the filter's own, and it changes which paths match:
    /// the same include keeps a differently-cased name in one mode and hides
    /// it in the other.
    #[test]
    fn the_case_mode_changes_matching() {
        let sensitive = Filter::new(patterns(&["*.TXT"]), vec![], true);
        let insensitive = Filter::new(patterns(&["*.TXT"]), vec![], false);
        assert!(
            sensitive.is_excluded("a.txt"),
            "the sensitive mode does not match the differently-cased name, \
             so the include hides it"
        );
        assert!(
            !insensitive.is_excluded("a.txt"),
            "the insensitive mode matches it, so the include keeps it"
        );
    }

    /// The cached options have to be the ones the per-call rule used, for
    /// every pattern and path the table names and for both case modes.
    #[test]
    fn the_cached_options_match_the_per_call_ones() {
        let table: &[(&[&str], &[&str], &[&str])] = &[
            (&[], &[], &["a.txt", "sub/a.txt", "sub/deep/a.txt"]),
            (
                &["*.txt"],
                &[],
                &["a.txt", "a.bin", "sub/a.txt", "sub/deep/a.txt", "sub"],
            ),
            (
                &["sub/*.txt"],
                &["*.bin"],
                &["sub/a.txt", "sub/a.bin", "top.txt", "sub/deep/a.txt"],
            ),
            (
                &["link"],
                &[],
                &["link", "link/x.txt", "linkage/x.txt", "other/x.txt"],
            ),
            (
                &["*.TXT"],
                &["secret*"],
                &["a.txt", "A.TXT", "secret.txt", "secret/a.txt"],
            ),
        ];
        for (includes, excludes, paths) in table {
            for &case_sensitive in &[true, false] {
                let inc = patterns(includes);
                let exc = patterns(excludes);
                let filter = Filter::new(inc.clone(), exc.clone(), case_sensitive);
                for path in *paths {
                    assert_eq!(
                        filter.is_excluded(path),
                        per_call_is_excluded(path, &inc, &exc, case_sensitive),
                        "includes={includes:?} excludes={excludes:?} \
                         case_sensitive={case_sensitive} path={path}"
                    );
                }
            }
        }
    }
}
