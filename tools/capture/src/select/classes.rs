//! The matrix classes: one predicate per distinct **patch shape**.
//!
//! **M5.** A class is a *branch*, not a file. Predicates overlap in general —
//! a file can satisfy several — which is why admission is greedy with an
//! exclusion set rather than six independent picks, and why every captured
//! file is tagged with all the classes it satisfies.
//!
//! ## What a class is about
//!
//! A class is a statement about the **patch**: what a consumer has to
//! download to produce the new file, and what the file becomes. It is not a
//! statement about how a repair tool gets there.
//!
//! The tier the repair picks, the order it writes in, whether it releases the
//! dead tail before the first write — those are the tool's own decisions.
//! They are not shared with any other repair tool, they are not what a
//! bundle is for, and they are tested where they belong: on crafted
//! fixtures, against the tool itself.
//!
//! ## The six shapes are the patch space's boundary conditions
//!
//! | class | the patch it covers |
//! |---|---|
//! | `identical` | nothing to fetch: the file is already right |
//! | `no_reuse` | everything fetched: the whole file is overwritten |
//! | `reuse_in_place` | a subset fetched: the reused chunks are already correct where they belong |
//! | `reuse_moved` | a subset fetched: the reused chunks live elsewhere in the file and have to be copied |
//! | `added` | the file is created whole |
//! | `removed` | the file is purged |
//!
//! Six shapes, six boundaries: **nothing** fetched, **everything** fetched, a
//! **subset** fetched with the reused part already in place, a **subset**
//! fetched with the reused part moved, a **new** file and a **deleted** file.
//!
//! ## Why the predicates take a [`Shape`] and not a path
//!
//! A shape is the numbers a decision turns on and nothing else. That keeps
//! the predicates pure and total — no manifest, no filesystem, no network — so
//! every one of the six can be tested against a table of shapes in the same
//! breath, and a future seventh class is a new arm rather than a new fixture.

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
        /// How many chunks the new entry has.
        nchunks: u64,
        /// How many of them exist in this same old file.
        reused: u64,
        /// Of those, how many sit at a different offset.
        moved: u64,
        /// The old file's declared size.
        old_size: u64,
        /// The new file's declared size.
        new_size: u64,
    },
    /// The path is only in the new manifest.
    Added,
    /// The path is only in the old manifest.
    Removed,
}

impl Shape {
    /// Build a shape from a score, treating a one-sided row as itself.
    pub fn from_row(
        state: crate::manifest::diff::State,
        s: &crate::manifest::diff::PathScore,
    ) -> Shape {
        match state {
            crate::manifest::diff::State::Added => Shape::Added,
            crate::manifest::diff::State::Removed => Shape::Removed,
            _ => Shape::Scored {
                same_md5: s.same_md5,
                nchunks: s.nchunks,
                reused: s.reused,
                moved: s.moved,
                old_size: s.old_size.max(0) as u64,
                new_size: s.new_size.max(0) as u64,
            },
        }
    }
}

/// The six variant classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// `old.md5 == new.md5`: the skip branch. Size and MD5 match, 0 bytes.
    Identical,
    /// `reused == 0`, `nchunks > 1`: every chunk is fetched, so the whole
    /// file is written from the network.
    ///
    /// The `nchunks > 1` is deliberate: a one-chunk file is a degenerate
    /// patch, and `added` already covers the "create a file whole" case.
    NoReuse,
    /// `0 < reused < nchunks`, `moved == 0`: the surviving chunks are already
    /// correct where they belong — **kept**, not copied.
    ReuseInPlace,
    /// `moved > 0`: the surviving chunks live **elsewhere** in the file, so
    /// they have to be copied.
    ReuseMoved,
    /// Only in the new manifest: the missing-local-file branch.
    Added,
    /// Only in the old manifest: not planned; a `--purge-after` target.
    Removed,
}

/// Whether a shape is *partially* reusable: some chunks survive and some do not.
///
/// Spelled out here rather than at every call site: it was spelled out
/// several times inside [`Class::matches`], and each spelling is a place
/// where the definition of "partial" can drift without anything noticing.
const fn is_partial(reused: u64, nchunks: u64) -> bool {
    reused > 0 && reused < nchunks
}

impl Class {
    /// Whether this class is the skip branch.
    ///
    /// Named rather than spelled at the call site because A19 turns on it: it is
    /// the *only* class whose two sides are expected to be byte-identical, and
    /// every other class means the file must change. Getting that backwards
    /// would make the assertion pass for exactly the wrong bundles.
    pub const fn is_identical(self) -> bool {
        matches!(self, Class::Identical)
    }

    /// Every class, **in admission order**.
    ///
    /// The order is a decision, not an accident: the two reuse classes are
    /// adjacent because a file that reuses *and* moves is the `reuse_moved`
    /// one, and the cheap shapes come first so a small budget buys the
    /// boundaries before the expensive ones.
    pub const ALL: [Class; 6] = [
        Class::Identical,
        Class::NoReuse,
        Class::ReuseInPlace,
        Class::ReuseMoved,
        Class::Added,
        Class::Removed,
    ];

    /// The order admission walks them in.
    ///
    /// **`= ALL`, not a second list.** The two were spelled out separately and
    /// had drifted into the same contents; the day they stop agreeing, a class
    /// added to one and not the other becomes either invisible to `parse` or
    /// invisible to admission, and neither failure names the other list.
    pub const REQUIRED_ORDER: [Class; 6] = Class::ALL;

    /// The wire spelling, and the directory name a capture is filed under.
    pub const fn id(self) -> &'static str {
        match self {
            Class::Identical => "identical",
            Class::NoReuse => "no_reuse",
            Class::ReuseInPlace => "reuse_in_place",
            Class::ReuseMoved => "reuse_moved",
            Class::Added => "added",
            Class::Removed => "removed",
        }
    }

    /// What this class exercises, for `--help` and the generated README.
    pub const fn exercises(self) -> &'static str {
        match self {
            Class::Identical => {
                "the skip branch: the scan's whole-file MD5 matches, 0 bytes fetched"
            }
            Class::NoReuse => {
                "no local source: every chunk is fetched and the whole file is written"
            }
            Class::ReuseInPlace => {
                "some of the chunks are already correct where they belong: kept, not copied; \
                 the rest are fetched"
            }
            Class::ReuseMoved => {
                "some of the chunks live elsewhere in the file: they are copied from their \
                 source, and the rest are fetched"
            }
            Class::Added => "the file is created: absent locally, fetched whole",
            Class::Removed => "not planned; a --purge-after target",
        }
    }

    /// Parse a `--require-class` value, naming the valid set on failure.
    pub fn parse(s: &str) -> Result<Class, String> {
        Class::ALL.into_iter().find(|c| c.id() == s).ok_or_else(|| {
            format!(
                "unknown class {s:?}; expected one of: {}",
                Class::ALL.map(|c| c.id()).join(", ")
            )
        })
    }

    /// Whether this shape satisfies the class.
    ///
    /// **Every content class requires `same_md5: false`.** A file whose
    /// whole-file MD5s agree is the **skip** branch — the repair makes no
    /// content decision about it at all, so it cannot be a statement about how
    /// the content changed. The live pool holds one such path
    /// (`StreamingAssets/AudioAssets/Music4.pck`): the chunking was re-tiled,
    /// so four chunks "move", yet the file's content is byte-identical. It
    /// used to satisfy a direction class as well as `identical`, which was
    /// three defects in one: the published population was one too high, the
    /// instrument could not reproduce it, and A19's "is unchanged but its
    /// class says it should differ" was a trip-wire waiting for a cheap
    /// candidate.
    ///
    /// The class set is a **partition** on the content, and `identical` is the
    /// class that owns an unchanged file.
    pub fn matches(self, s: Shape) -> bool {
        use Shape::{Added, Removed, Scored};
        match self {
            Class::Identical => matches!(s, Scored { same_md5: true, .. }),
            Class::NoReuse => {
                matches!(
                    s,
                    Scored {
                        same_md5: false,
                        nchunks,
                        reused: 0,
                        ..
                    } if nchunks > 1
                )
            }
            Class::ReuseInPlace => matches!(
                s,
                Scored {
                    same_md5: false,
                    moved: 0,
                    reused,
                    nchunks,
                    ..
                } if is_partial(reused, nchunks)
            ),
            // The reused chunks live elsewhere in the file, so they have to be
            // copied rather than kept.
            Class::ReuseMoved => {
                matches!(
                    s,
                    Scored {
                        same_md5: false,
                        moved,
                        ..
                    } if moved > 0
                )
            }
            Class::Added => matches!(s, Added),
            Class::Removed => matches!(s, Removed),
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
/// Not a *requirement* — the matrix reports on all six whether or not any of
/// them is required, because "this pair has no `reuse_moved` file" is worth
/// knowing even to a caller who asked for nothing in particular. What
/// `--require-class` does is promote a subset to fatal.
pub fn all_in_admission_order() -> Vec<Class> {
    Class::REQUIRED_ORDER.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scored shape, spelled the way the predicates read it.
    fn scored(same_md5: bool, nchunks: u64, reused: u64, moved: u64) -> Shape {
        Shape::Scored {
            same_md5,
            nchunks,
            reused,
            moved,
            old_size: 100,
            new_size: 100,
        }
    }

    /// The full shape, every field spoken for.
    fn shape(
        same_md5: bool,
        nchunks: u64,
        reused: u64,
        moved: u64,
        old_size: u64,
        new_size: u64,
    ) -> Shape {
        Shape::Scored {
            same_md5,
            nchunks,
            reused,
            moved,
            old_size,
            new_size,
        }
    }

    /// The six classes against the shapes the spec's table describes. Each
    /// shape here *is* one row of that table, so a predicate that drifts fails
    /// against the sentence it claims to implement.
    #[test]
    fn each_predicate_matches_exactly_its_documented_shape() {
        let cases: Vec<(Class, Shape)> = vec![
            // old.md5 == new.md5
            (Class::Identical, scored(true, 1, 1, 0)),
            // reused == 0, nchunks > 1
            (Class::NoReuse, scored(false, 3, 0, 0)),
            // 0 < reused < nchunks, moved == 0
            (Class::ReuseInPlace, shape(false, 3, 1, 0, 100, 100)),
            // moved > 0
            (Class::ReuseMoved, shape(false, 4, 3, 2, 100, 100)),
            // only in the new manifest
            (Class::Added, Shape::Added),
            // only in the old manifest
            (Class::Removed, Shape::Removed),
        ];
        for (class, shape) in cases {
            assert!(class.matches(shape), "{class} should match {shape:?}");
        }
    }

    /// A class that does not match its own row would be worse than no class at
    /// all, and a predicate that matches *two* rows would make the matrix lie
    /// about which patch shape a capture covers.
    #[test]
    fn each_documented_shape_matches_exactly_its_documented_classes() {
        let cases: Vec<(Vec<Class>, Shape)> = vec![
            (vec![Class::Identical], scored(true, 1, 1, 0)),
            (vec![Class::NoReuse], scored(false, 3, 0, 0)),
            (vec![Class::ReuseInPlace], shape(false, 3, 1, 0, 100, 100)),
            (vec![Class::ReuseMoved], shape(false, 4, 3, 2, 100, 100)),
            // A file that reuses *and* moves is the moved shape, and only that
            // one: the two reuse classes are exclusive.
            (vec![Class::ReuseMoved], shape(false, 10, 8, 4, 200, 100)),
            // A file that shrinks and reuses without moving is still the
            // in-place shape: the shrink is not a class of its own.
            (vec![Class::ReuseInPlace], shape(false, 3, 2, 0, 200, 100)),
        ];
        for (want, shape) in cases {
            assert_eq!(Class::all_satisfied_by(shape), want, "{shape:?}");
        }
    }

    /// The two reuse classes are exclusive, and the moved one is exactly "the
    /// chunks did not all stay where they belong".
    #[test]
    fn the_two_reuse_classes_are_exclusive() {
        let still = shape(false, 4, 3, 0, 100, 100);
        assert!(Class::ReuseInPlace.matches(still));
        assert!(!Class::ReuseMoved.matches(still));

        let moved = shape(false, 4, 3, 2, 100, 100);
        assert!(Class::ReuseMoved.matches(moved));
        assert!(!Class::ReuseInPlace.matches(moved));
    }

    /// `reused` must be *strictly* between 0 and `nchunks` for the in-place
    /// class; the ends belong to the other classes.
    #[test]
    fn partial_means_strictly_between_and_the_ends_belong_elsewhere() {
        let none_reused = scored(false, 4, 0, 0);
        assert!(!Class::ReuseInPlace.matches(none_reused));
        assert!(Class::NoReuse.matches(none_reused));

        let all_reused = scored(false, 4, 4, 0);
        assert!(
            !Class::ReuseInPlace.matches(all_reused),
            "a file whose every chunk is reused and none of them moved is not a \
             class of its own: it is the shape the removed `full_reuse_reordered` \
             used to claim, and there is no decision behind it"
        );
    }

    /// A one-chunk file with no reusable chunk is a **degenerate** patch: the
    /// `added` class already covers "the file is created whole", so it is not
    /// a shape of its own.
    #[test]
    fn a_one_chunk_file_with_no_reuse_is_not_a_class() {
        let one = scored(false, 1, 0, 0);
        assert!(!Class::NoReuse.matches(one));
        assert!(Class::all_satisfied_by(one).is_empty());
    }

    /// `identical` is separated from everything else by the whole-file MD5: a
    /// file whose every chunk is reused is the skip branch when the MD5s
    /// agree and nothing else when they do not.
    #[test]
    fn identical_is_separated_from_the_rest_by_the_file_md5() {
        let identical = scored(true, 4, 4, 0);
        assert_eq!(Class::all_satisfied_by(identical), vec![Class::Identical]);
        let reordered = scored(false, 4, 4, 0);
        assert!(
            Class::all_satisfied_by(reordered).is_empty(),
            "a file whose every chunk is reused and none of them moved is not a \
             class of its own: it is the shape the removed `full_reuse_reordered` \
             used to claim, and there is no decision behind it"
        );
    }

    /// A file whose content is **unchanged** is the skip branch, whatever its
    /// chunking did.
    #[test]
    fn an_unchanged_file_is_the_skip_branch_even_when_its_chunking_moved() {
        let retiled = shape(true, 336, 336, 4, 398_535_566, 398_535_566);
        assert_eq!(
            Class::all_satisfied_by(retiled),
            vec![Class::Identical],
            "the class set is a partition on the content, and `identical` owns an \
             unchanged file"
        );
    }

    /// A one-sided path is exactly one class and no other. A predicate that
    /// accidentally matched a common path would make `added` satisfiable by
    /// every file in the game.
    #[test]
    fn a_one_sided_path_is_exactly_one_class() {
        assert_eq!(Class::all_satisfied_by(Shape::Added), vec![Class::Added]);
        assert_eq!(
            Class::all_satisfied_by(Shape::Removed),
            vec![Class::Removed]
        );
    }

    /// An empty changed file matches no class, which is a real state and is
    /// recorded as one rather than being forced into a bucket.
    #[test]
    fn an_empty_changed_file_matches_nothing_and_that_is_fine() {
        let empty = scored(false, 0, 0, 0);
        assert!(
            Class::all_satisfied_by(empty).is_empty(),
            "{:?}",
            Class::all_satisfied_by(empty)
        );
        // An empty *identical* file is still the skip branch.
        assert_eq!(
            Class::all_satisfied_by(scored(true, 0, 0, 0)),
            vec![Class::Identical]
        );
    }

    /// Most shapes satisfy exactly one class, and the class set is a
    /// **partition** on the content: no two of them claim the same file.
    #[test]
    fn the_common_case_is_exactly_one_class() {
        for shape in [
            scored(true, 1, 1, 0),
            scored(false, 3, 0, 0),
            shape(false, 3, 1, 0, 100, 100),
            shape(false, 4, 3, 2, 100, 100),
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
        let n = Class::ALL.len();
        assert_eq!(all_in_admission_order().len(), n);
        let mut sorted = all_in_admission_order();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), n, "a class appears twice in REQUIRED_ORDER");
        assert_eq!(sorted, Class::ALL);
    }

    /// The two reuse classes are adjacent in the admission order, because a
    /// file that reuses *and* moves is the moved one and the two are
    /// exclusive: the cheap shapes come first, so a small budget buys the
    /// boundaries before the expensive ones.
    #[test]
    fn the_reuse_classes_are_admitted_next_to_each_other() {
        let order = all_in_admission_order();
        let at = |c: Class| order.iter().position(|x| *x == c).expect("in the order");
        assert!(
            at(Class::ReuseInPlace) < at(Class::ReuseMoved),
            "the in-place shape is the cheaper of the two"
        );
        assert!(at(Class::ReuseMoved) < at(Class::Added));
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
        assert_eq!(ids.len(), Class::ALL.len());
    }
}
