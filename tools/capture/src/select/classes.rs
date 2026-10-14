//! The matrix classes: one predicate per distinct decision the tool under test makes.
//!
//! **M5.** A class is a *branch*, not a file. Predicates overlap in general — a
//! file can satisfy several — which is why admission is greedy with an exclusion
//! set rather than nine independent picks, and why every captured file is
//! tagged with all the classes it satisfies.
//!
//! ## Why the predicates take a [`Shape`] and not a path
//!
//! A shape is the five numbers a decision turns on and nothing else. That keeps
//! the predicates pure and total — no manifest, no filesystem, no network — so
//! every one of the nine can be tested against a table of shapes in the same
//! breath, and a future tenth class is a new arm rather than a new fixture.

use serde::{Deserialize, Serialize};

/// What a class predicate is allowed to look at.
///
/// Deliberately lossy: a path's *name* never decides a class, so it is not here.
/// A class is a statement about how the content changed, and a predicate that
/// could look at the name would eventually start doing so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// A path in both manifests, with the numbers the decision turns on.
    Scored {
        /// The two whole-file MD5s agree.
        same_md5: bool,
        /// The two declared sizes agree.
        size_equal: bool,
        /// How many chunks the new entry has.
        nchunks: u64,
        /// How many of them exist in this same old file.
        reused: u64,
        /// Of those, how many sit at a different offset.
        moved: u64,
    },
    /// The path is only in the new manifest.
    Added,
    /// The path is only in the old manifest.
    Removed,
}

impl Shape {
    /// Build a shape from a score, treating a one-sided row as itself.
    pub fn from_row(state: crate::manifest::diff::State, s: &crate::manifest::diff::PathScore) -> Shape {
        match state {
            crate::manifest::diff::State::Added => Shape::Added,
            crate::manifest::diff::State::Removed => Shape::Removed,
            _ => Shape::Scored {
                same_md5: s.same_md5,
                size_equal: s.old_size == s.new_size,
                nchunks: s.nchunks,
                reused: s.reused,
                moved: s.moved,
            },
        }
    }
}

/// The nine variant classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// `old.md5 == new.md5`: the skip branch. Size and MD5 match, 0 bytes.
    Identical,
    /// `reused == 0`, `nchunks == 1`: one download into an empty `_tmp`.
    ZeroReuseSingle,
    /// `reused == 0`, `nchunks > 1`: N sequential downloads, no reuse.
    ZeroReuseMulti,
    /// `0 < reused < nchunks`, `moved == 0`, size equal: the reuse happy path.
    PartialSameOffset,
    /// `0 < reused < nchunks`, `moved > 0`: reuse where the source offset differs.
    PartialMovedOffset,
    /// `0 < reused < nchunks`, `moved == 0`, size differs: oversize-`_tmp` truncate.
    PartialSizeChanged,
    /// Only in the new manifest: the missing-local-file branch.
    Added,
    /// Only in the old manifest: not planned; a `--purge-after` target.
    Removed,
    /// `reused == nchunks` and the MD5 differs: every chunk reused, layout changed.
    FullReuseReordered,
}

impl Class {
    /// Every class, in no particular order.
    pub const ALL: [Class; 9] = [
        Class::Identical,
        Class::ZeroReuseSingle,
        Class::ZeroReuseMulti,
        Class::PartialSameOffset,
        Class::PartialMovedOffset,
        Class::PartialSizeChanged,
        Class::Added,
        Class::Removed,
        Class::FullReuseReordered,
    ];

    /// The order admission walks them in.
    ///
    /// The spec's own table order, which runs cheapest-class-first and leaves
    /// `full_reuse_reordered` last. That is the right shape for a greedy loop:
    /// the classes that are cheap and reliably satisfiable pick first, and the
    /// one that is usually unsatisfiable never costs a class that could use a
    /// cheap candidate.
    pub const REQUIRED_ORDER: [Class; 9] = [
        Class::Identical,
        Class::ZeroReuseSingle,
        Class::ZeroReuseMulti,
        Class::PartialSameOffset,
        Class::PartialMovedOffset,
        Class::PartialSizeChanged,
        Class::Added,
        Class::Removed,
        Class::FullReuseReordered,
    ];

    /// The wire spelling, and the directory name a capture is filed under.
    pub const fn id(self) -> &'static str {
        match self {
            Class::Identical => "identical",
            Class::ZeroReuseSingle => "zero_reuse_single",
            Class::ZeroReuseMulti => "zero_reuse_multi",
            Class::PartialSameOffset => "partial_same_offset",
            Class::PartialMovedOffset => "partial_moved_offset",
            Class::PartialSizeChanged => "partial_size_changed",
            Class::Added => "added",
            Class::Removed => "removed",
            Class::FullReuseReordered => "full_reuse_reordered",
        }
    }

    /// What this class exercises, for `--help` and the generated README.
    pub const fn exercises(self) -> &'static str {
        match self {
            Class::Identical => "skip: size and MD5 both match, 0 bytes fetched",
            Class::ZeroReuseSingle => "a single download into an empty _tmp",
            Class::ZeroReuseMulti => "N sequential downloads with nothing reusable",
            Class::PartialSameOffset => "verified reuse, source offset == target offset",
            Class::PartialMovedOffset => "verified reuse, source offset != target offset",
            Class::PartialSizeChanged => "reuse into an oversize _tmp, truncated to length",
            Class::Added => "the missing-local-file branch",
            Class::Removed => "not planned; a --purge-after target",
            Class::FullReuseReordered => "every chunk reused, whole-file layout changed",
        }
    }

    /// Parse a `--require-class` value, naming the valid set on failure.
    pub fn parse(s: &str) -> Result<Class, String> {
        Class::ALL
            .into_iter()
            .find(|c| c.id() == s)
            .ok_or_else(|| {
                format!(
                    "unknown class {s:?}; expected one of: {}",
                    Class::ALL.map(|c| c.id()).join(", ")
                )
            })
    }

    /// Whether this shape satisfies the class.
    pub fn matches(self, s: Shape) -> bool {
        use Shape::{Added, Removed, Scored};
        match self {
            Class::Identical => matches!(s, Scored { same_md5: true, .. }),
            Class::ZeroReuseSingle => {
                matches!(s, Scored { same_md5: false, nchunks: 1, reused: 0, .. })
            }
            Class::ZeroReuseMulti => {
                matches!(s, Scored { same_md5: false, reused: 0, nchunks, .. } if nchunks > 1)
            }
            Class::PartialSameOffset => matches!(
                s,
                Scored { size_equal: true, moved: 0, reused, nchunks, .. }
                    if reused > 0 && reused < nchunks
            ),
            Class::PartialMovedOffset => matches!(
                s,
                Scored { moved, reused, nchunks, .. } if moved > 0 && reused > 0 && reused < nchunks
            ),
            Class::PartialSizeChanged => matches!(
                s,
                Scored { size_equal: false, moved: 0, reused, nchunks, .. }
                    if reused > 0 && reused < nchunks
            ),
            Class::Added => matches!(s, Added),
            Class::Removed => matches!(s, Removed),
            Class::FullReuseReordered => matches!(
                s,
                Scored { same_md5: false, reused, nchunks, .. } if nchunks > 0 && reused == nchunks
            ),
        }
    }

    /// Every class a shape satisfies.
    ///
    /// Usually one. More than one means the predicates genuinely overlap, and a
    /// capture of that file is tagged with every class it answers — which is
    /// what lets `matrix[].covered_by` exist at all.
    pub fn all_satisfied_by(s: Shape) -> Vec<Class> {
        Class::ALL.into_iter().filter(|c| c.matches(s)).collect()
    }

    /// Every class a shape satisfies *besides* `primary`.
    ///
    /// What an admission records as `also_satisfies`. The point of that field is
    /// that one capture can answer several classes, and a list that echoes the
    /// primary back at you makes every admission look like it covers something
    /// extra.
    pub fn all_others_satisfied_by(s: Shape, primary: Class) -> Vec<Class> {
        Class::ALL
            .into_iter()
            .filter(|c| *c != primary && c.matches(s))
            .collect()
    }
}

impl std::fmt::Display for Class {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

/// Every class, in admission order.
///
/// Not a *requirement* — the matrix reports on all nine whether or not any of
/// them is required, because "this pair has no `full_reuse_reordered` file" is
/// worth knowing even to a caller who asked for nothing in particular. What
/// `--require-class` does is promote a subset to fatal.
pub fn all_in_admission_order() -> Vec<Class> {
    Class::REQUIRED_ORDER.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scored(same_md5: bool, size_equal: bool, nchunks: u64, reused: u64, moved: u64) -> Shape {
        Shape::Scored {
            same_md5,
            size_equal,
            nchunks,
            reused,
            moved,
        }
    }

    /// The nine classes against the shapes the spec's table describes. Each
    /// shape here *is* one row of that table, so a predicate that drifts fails
    /// against the sentence it claims to implement.
    #[test]
    fn each_predicate_matches_exactly_its_documented_shape() {
        let cases: Vec<(Class, Shape)> = vec![
            // old.md5 == new.md5
            (Class::Identical, scored(true, true, 1, 1, 0)),
            // reused == 0, nchunks == 1
            (Class::ZeroReuseSingle, scored(false, true, 1, 0, 0)),
            // reused == 0, nchunks > 1
            (Class::ZeroReuseMulti, scored(false, false, 3, 0, 0)),
            // 0 < reused < nchunks, moved == 0, size equal
            (Class::PartialSameOffset, scored(false, true, 3, 1, 0)),
            // 0 < reused < nchunks, moved > 0
            (Class::PartialMovedOffset, scored(false, true, 3, 1, 1)),
            // 0 < reused < nchunks, moved == 0, size differs
            (Class::PartialSizeChanged, scored(false, false, 2, 1, 0)),
            // only in the new manifest
            (Class::Added, Shape::Added),
            // only in the old manifest
            (Class::Removed, Shape::Removed),
            // reused == nchunks, md5 differs
            (Class::FullReuseReordered, scored(false, true, 4, 4, 0)),
        ];
        for (class, shape) in cases {
            assert!(class.matches(shape), "{class} should match {shape:?}");
        }
    }

    /// The predicates are *not* nine copies of the same test, and the boundaries
    /// are where a bug would live. Each shape below sits exactly on a boundary.
    #[test]
    fn the_boundaries_between_the_three_partial_shapes_are_exact() {
        let same_offset = scored(false, true, 3, 1, 0);
        assert!(Class::PartialSameOffset.matches(same_offset));
        assert!(!Class::PartialSizeChanged.matches(same_offset), "size is equal");
        assert!(!Class::PartialMovedOffset.matches(same_offset), "moved == 0");

        let size_changed = scored(false, false, 3, 1, 0);
        assert!(Class::PartialSizeChanged.matches(size_changed));
        assert!(!Class::PartialSameOffset.matches(size_changed), "size differs");
        assert!(!Class::PartialMovedOffset.matches(size_changed), "moved == 0");

        let moved = scored(false, true, 3, 1, 1);
        assert!(Class::PartialMovedOffset.matches(moved));
        assert!(
            !Class::PartialSameOffset.matches(moved) && !Class::PartialSizeChanged.matches(moved),
            "a moved chunk is neither offset-stable nor size-stable"
        );
    }

    /// `reused` must be *strictly* between 0 and `nchunks`; the ends belong to
    /// the zero-reuse and full-reuse classes.
    #[test]
    fn partial_means_strictly_between_and_the_ends_belong_elsewhere() {
        let none_reused = scored(false, true, 4, 0, 0);
        for c in [
            Class::PartialSameOffset,
            Class::PartialMovedOffset,
            Class::PartialSizeChanged,
        ] {
            assert!(!c.matches(none_reused), "{c} must not take reused == 0");
        }
        assert!(Class::ZeroReuseMulti.matches(none_reused));

        let all_reused = scored(false, true, 4, 4, 0);
        for c in [
            Class::PartialSameOffset,
            Class::PartialMovedOffset,
            Class::PartialSizeChanged,
        ] {
            assert!(!c.matches(all_reused), "{c} must not take reused == nchunks");
        }
        assert!(Class::FullReuseReordered.matches(all_reused));
    }

    /// `identical` and `full_reuse_reordered` are both "every chunk reused", and
    /// the whole-file MD5 is the only thing that separates them.
    #[test]
    fn identical_and_full_reuse_differ_only_by_the_file_md5() {
        let identical = scored(true, true, 4, 4, 0);
        let reordered = scored(false, true, 4, 4, 0);
        assert!(Class::Identical.matches(identical));
        assert!(!Class::FullReuseReordered.matches(identical));
        assert!(Class::FullReuseReordered.matches(reordered));
        assert!(!Class::Identical.matches(reordered));
    }

    /// A one-sided path is exactly one class and no other. A predicate that
    /// accidentally matched a common path would make `added` satisfiable by
    /// every file in the game.
    #[test]
    fn a_one_sided_path_is_exactly_one_class() {
        assert_eq!(
            Class::all_satisfied_by(Shape::Added),
            vec![Class::Added]
        );
        assert_eq!(
            Class::all_satisfied_by(Shape::Removed),
            vec![Class::Removed]
        );
    }

    /// An empty changed file matches no class, which is a real state and is
    /// recorded as one rather than being forced into a bucket.
    #[test]
    fn an_empty_changed_file_matches_nothing_and_that_is_fine() {
        let empty = scored(false, true, 0, 0, 0);
        assert!(Class::all_satisfied_by(empty).is_empty(), "{:?}", Class::all_satisfied_by(empty));
        // An empty *identical* file is still the skip branch.
        assert_eq!(
            Class::all_satisfied_by(scored(true, true, 0, 0, 0)),
            vec![Class::Identical]
        );
    }

    /// Most shapes satisfy exactly one class. Where they do not, the tag list
    /// has to say so, because `covered_by` is built from it.
    #[test]
    fn the_common_case_is_exactly_one_class() {
        for shape in [
            scored(true, true, 1, 1, 0),
            scored(false, true, 1, 0, 0),
            scored(false, true, 3, 1, 0),
            scored(false, true, 3, 1, 1),
            scored(false, false, 2, 1, 0),
            scored(false, true, 4, 4, 0),
        ] {
            let all = Class::all_satisfied_by(shape);
            assert_eq!(all.len(), 1, "{shape:?} -> {all:?}");
        }
    }

    #[test]
    fn ids_round_trip_and_a_typo_names_the_valid_set() {
        for c in Class::ALL {
            assert_eq!(Class::parse(c.id()).unwrap(), c);
        }
        let e = Class::parse("partially_same_offset").unwrap_err();
        assert!(e.contains("partially_same_offset"), "{e}");
        for c in Class::ALL {
            assert!(e.contains(c.id()), "{e} is missing {c}");
        }
    }

    #[test]
    fn every_required_order_entry_is_a_real_class_and_uniquely_once() {
        assert_eq!(all_in_admission_order().len(), 9);
        let mut sorted = all_in_admission_order();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 9, "a class appears twice in REQUIRED_ORDER");
        assert_eq!(sorted, Class::ALL);
    }

    /// The ordering is a decision, not an accident: the usually-unsatisfiable
    /// class goes last so it never costs a cheap one.
    #[test]
    fn full_reuse_reordered_is_admitted_last() {
        assert_eq!(
            Class::REQUIRED_ORDER.last().copied(),
            Some(Class::FullReuseReordered)
        );
    }

    #[test]
    fn every_class_has_a_distinct_id_and_a_description() {
        let mut ids = Vec::new();
        for c in Class::ALL {
            assert!(!c.id().is_empty());
            assert!(c.exercises().len() > 10, "{} has no description", c.id());
            ids.push(c.id());
        }
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 9);
    }
}