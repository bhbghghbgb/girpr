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
//!
//! That rule is also what makes the decision worth remembering: a directory's
//! answer is the answer for everything under it, so a walk can ask a per
//! directory memo instead of testing every pattern against every ancestor
//! prefix. See [`ExcludeMemo`].
//!
//! The rule is asked two ways — one path at a time ([`Filter::is_excluded`])
//! or a walk at a time ([`ExcludeMemo`]) — and the halves it is built from stay
//! inside the module. One of them answers a weaker question than the rule (a
//! pattern that matches the path itself, without the ancestors that make a
//! directory a subtree), and a caller that reached for it would drop the
//! subtree rule without being told.

use anyhow::Result;
use girpr_filter::{Backslash, Compiled, any_matches_or_ancestor, compile, matches_any};
use std::collections::HashMap;

/// Compile glob strings, reporting the offending pattern on failure.
pub fn compile_patterns(list: &[String]) -> Result<Vec<Compiled>> {
    compile(list, Backslash::Escape)
}

/// The run's include and exclude rules, compiled once.
///
/// **One value, one home.** The patterns and the case mode they are matched
/// with travel together, so a caller cannot ask whether a path is excluded
/// without also saying which rules apply.
///
/// The case mode is not owned here. It is a run-wide setting that also selects
/// the cache's case flag, the planner's pairing, the rename pass and the mkdir
/// check, so it stays on girsync's `CommonOpts` config field; this value is
/// *built* from it, and the two have to agree. globset fixes the case mode when
/// a glob is compiled, so each [`Compiled`] keeps a matcher for each mode and
/// this filter only selects one.
#[derive(Debug, Clone)]
pub struct Filter {
    includes: Vec<Compiled>,
    excludes: Vec<Compiled>,
    case_sensitive: bool,
}

impl Filter {
    /// The rules of one run, matched under `case_sensitive`.
    ///
    /// Every path this filter is asked about is matched in the same case mode.
    pub fn new(includes: Vec<Compiled>, excludes: Vec<Compiled>, case_sensitive: bool) -> Self {
        Self {
            includes,
            excludes,
            case_sensitive,
        }
    }

    /// True when `rel` should be ignored. `--exclude` beats `--include`.
    ///
    /// The include patterns are a filter on the path itself, not on its
    /// ancestors: a pattern that matches only a descendant keeps that
    /// descendant. Phase A pulls the ancestor directories back into the
    /// effective map so a link above an included path stays visible to the
    /// planner; see `effective::phase_a`.
    ///
    /// An exclude pattern is a filter on the path *and* on the directories
    /// above it: a pattern that covers a directory covers everything under
    /// it, so an excluded directory is a whole subtree.
    ///
    /// This is the whole rule, asked about one path. A walk that asks about
    /// many paths wants [`ExcludeMemo`], which answers the same question
    /// with the same answers and less work.
    pub fn is_excluded(&self, rel: &str) -> bool {
        self.hidden_by_include(rel) || self.excluded_by_pattern_or_ancestor(rel)
    }

    /// True when the include patterns do not keep `rel`.
    ///
    /// An include pattern is a filter on the path itself, so this is a
    /// question about `rel` alone: nothing above it is consulted.
    fn hidden_by_include(&self, rel: &str) -> bool {
        !self.includes.is_empty() && !matches_any(&self.includes, rel, self.case_sensitive)
    }

    /// True when an exclude pattern matches `rel` itself.
    ///
    /// The directories above `rel` are not consulted: a pattern that matches
    /// a directory covers the whole subtree below it, and that is the other
    /// half of the rule — see [`Filter::excluded_by_pattern_or_ancestor`].
    fn excluded_by_pattern(&self, rel: &str) -> bool {
        matches_any(&self.excludes, rel, self.case_sensitive)
    }

    /// True when an exclude pattern matches `rel` or one of its ancestor
    /// directories.
    ///
    /// The ancestor test is what makes an excluded directory cover its
    /// subtree.
    fn excluded_by_pattern_or_ancestor(&self, rel: &str) -> bool {
        any_matches_or_ancestor(&self.excludes, rel, self.case_sensitive)
    }

    /// The `--include` patterns, as compiled.
    pub fn includes(&self) -> &[Compiled] {
        &self.includes
    }

    /// The `--exclude` patterns, as compiled.
    pub fn excludes(&self) -> &[Compiled] {
        &self.excludes
    }
}

/// The directory `rel` sits in, or `None` when `rel` is at the root.
fn parent_of(rel: &str) -> Option<&str> {
    rel.rsplit_once('/').map(|(parent, _)| parent)
}

/// The exclude decision for a walk, remembered per directory.
///
/// **The subtree rule is what makes this worth remembering.** An exclude
/// pattern that covers a directory covers everything under it, so a
/// directory's answer is the answer for its whole subtree. A walk that
/// reaches a directory before its contents can therefore ask this instead of
/// the patterns: a child of a covered directory is covered without a test,
/// and a child of a directory no pattern covered only needs its own path
/// tested — the directories above it have already been answered.
///
/// **The answer does not depend on the order the entries arrive in.** A
/// parent the walk has not decided yet — it may never reach it — falls back
/// to testing the whole chain, which is the answer the plain rule gives, so
/// the order can cost time but cannot change the answer.
pub struct ExcludeMemo<'a> {
    filter: &'a Filter,
    /// The decision for each directory reached so far: `true` when a pattern
    /// covers it, `false` when none does.
    decided: HashMap<&'a str, bool>,
}

impl<'a> ExcludeMemo<'a> {
    /// A memo over `filter`, with nothing decided yet.
    pub fn new(filter: &'a Filter) -> Self {
        Self {
            filter,
            decided: HashMap::new(),
        }
    }

    /// True when the walk hides `rel` — the same answer
    /// [`Filter::is_excluded`] gives.
    ///
    /// `is_dir` says whether the entry is a directory. A directory's answer is
    /// remembered, because it is the answer for everything under it; a file's
    /// is not, because nothing is below it.
    pub fn is_excluded(&mut self, rel: &'a str, is_dir: bool) -> bool {
        if self.filter.excludes().is_empty() {
            // No pattern can cover a subtree, so there is nothing to remember.
            return self.filter.hidden_by_include(rel);
        }
        let hidden = self.filter.hidden_by_include(rel);
        if hidden && !is_dir {
            // The include patterns already hide it, and a file's answer is its
            // own: nothing below it needs the exclude decision.
            return true;
        }
        let covered = match parent_of(rel).and_then(|p| self.decided.get(p)) {
            // The parent is covered, so everything under it is.
            Some(&true) => true,
            // No pattern covered the parent or anything above it, so only this
            // path is left to test.
            Some(&false) => self.filter.excluded_by_pattern(rel),
            // The parent has not been decided: test the whole chain.
            None => self.filter.excluded_by_pattern_or_ancestor(rel),
        };
        if is_dir {
            self.decided.insert(rel, covered);
        }
        hidden || covered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use girpr_filter::matches_or_ancestor;

    /// Compiled patterns, from `&str`s.
    fn patterns(list: &[&str]) -> Vec<Compiled> {
        let owned: Vec<String> = list.iter().map(|s| s.to_string()).collect();
        compile_patterns(&owned).unwrap()
    }

    /// The rule the bundle replaced: ask the same question per call rather
    /// than through the filter's cached case mode.
    ///
    /// Kept as the reference the bundled filter has to agree with, so a change
    /// to the cached mode cannot quietly change which paths a run hides.
    fn per_call_is_excluded(
        rel: &str,
        includes: &[Compiled],
        excludes: &[Compiled],
        case_sensitive: bool,
    ) -> bool {
        if !includes.is_empty() && !matches_any(includes, rel, case_sensitive) {
            return true;
        }
        excludes
            .iter()
            .any(|p| matches_or_ancestor(p, rel, case_sensitive))
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

    /// The paths a case names, and whether each of them is a directory.
    type Paths = &'static [(&'static str, bool)];

    /// One filter shape: the include patterns, the exclude patterns, and the
    /// paths the case asks about.
    type Case = (&'static [&'static str], &'static [&'static str], Paths);

    /// The remembered decision is the plain decision.
    ///
    /// The memo answers the same question as [`Filter::is_excluded`] and has
    /// to: a directory's answer is reused for everything under it, so a
    /// mistake in the reuse would hide paths the plain rule keeps. The paths
    /// are fed in the order a walk reaches them — a directory before its
    /// contents — and in the reverse order as well, because the answer must
    /// not depend on the order.
    #[test]
    fn the_remembered_decision_is_the_plain_one() {
        let table: &[Case] = &[
            // No pattern at all.
            (
                &[],
                &[],
                &[("a", true), ("a/b.txt", false), ("a/deep/c.txt", false)],
            ),
            // A covered directory covers its subtree.
            (
                &[],
                &["link"],
                &[
                    ("link", true),
                    ("link/x.txt", false),
                    ("link/sub", true),
                    ("link/sub/y.txt", false),
                    ("linkage", true),
                    ("linkage/x.txt", false),
                ],
            ),
            // A pattern that matches a file in the middle of a tree.
            (
                &[],
                &["*.bin"],
                &[
                    ("a", true),
                    ("a/x.bin", false),
                    ("a/y.txt", false),
                    ("a/deep", true),
                    ("a/deep/z.bin", false),
                ],
            ),
            // A covered directory named deep in the tree.
            (
                &[],
                &["a/b"],
                &[
                    ("a", true),
                    ("a/b", true),
                    ("a/b/c.txt", false),
                    ("a/x.txt", false),
                ],
            ),
            // An include that keeps only some paths, and an exclude on top.
            (
                &["*.txt"],
                &["secret"],
                &[
                    ("a", true),
                    ("a/x.txt", false),
                    ("a/y.bin", false),
                    ("secret", true),
                    ("secret/s.txt", false),
                ],
            ),
            // The case mode changes which paths match.
            (
                &["*.TXT"],
                &["*.BIN"],
                &[
                    ("d", true),
                    ("d/x.TXT", false),
                    ("d/x.txt", false),
                    ("d/y.BIN", false),
                    ("d/y.bin", false),
                ],
            ),
        ];
        for (includes, excludes, paths) in table {
            for &case_sensitive in &[true, false] {
                let filter = Filter::new(patterns(includes), patterns(excludes), case_sensitive);
                // The order a walk reaches them in: a directory before its
                // contents.
                let mut memo = ExcludeMemo::new(&filter);
                for &(path, is_dir) in *paths {
                    assert_eq!(
                        memo.is_excluded(path, is_dir),
                        filter.is_excluded(path),
                        "includes={includes:?} excludes={excludes:?} \
                         case_sensitive={case_sensitive} path={path}"
                    );
                }
                // And the reverse, so the answer cannot depend on the order.
                let mut memo = ExcludeMemo::new(&filter);
                for &(path, is_dir) in paths.iter().rev() {
                    assert_eq!(
                        memo.is_excluded(path, is_dir),
                        filter.is_excluded(path),
                        "includes={includes:?} excludes={excludes:?} \
                         case_sensitive={case_sensitive} path={path} (reversed)"
                    );
                }
            }
        }
    }
}
