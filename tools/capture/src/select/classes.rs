//! The matrix classes: one predicate per distinct decision the tool under test makes.
//!
//! **M5.** A class is a *branch*, not a file. Predicates overlap in general — a
//! file can satisfy several — which is why admission is greedy with an exclusion
//! set rather than eleven independent picks, and why every captured file is
//! tagged with all the classes it satisfies.
//!
//! ## Why the predicates take a [`Shape`] and not a path
//!
//! A shape is the numbers a decision turns on and nothing else. That keeps
//! the predicates pure and total — no manifest, no filesystem, no network — so
//! every one of the eleven can be tested against a table of shapes in the same
//! breath, and a future twelfth class is a new arm rather than a new fixture.
//!
//! ## The six content decisions, and the classes that force them
//!
//! The repair spec makes six *content* decisions per file, and the class set
//! is built so that every one of them is forced by at least one case:
//!
//! | decision | the classes that force it |
//! |---|---|
//! | skip | `identical` |
//! | what happens to each chunk (kept / copied / fetched) | `zero_reuse_*`, `kept_in_place`, `moved_*` |
//! | the tier (RAM-staged or ordered) | `moved_*` (the payload) |
//! | the direction (ascending or descending) | `moved_ascending`, `moved_descending`, `moved_mixed` |
//! | the trim (eager or deferred) | `trim_eager`, `trim_deferred` |
//! | the growth (the file on disk is shorter than the target) | any capture whose new side is longer |
//!
//! The **flag axes** — the mode, the RAM budget, the resume, the zero-fill —
//! are decisions the *flags* make, not the content, and they are not classes.
//! `docs/00-spec.md` §4.1 says where each of them goes instead.

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
        /// Of the moved ones, how many move **later**: the source is *before*
        /// the destination. The descending write order keeps these alive.
        moved_up: u64,
        /// Of the moved ones, how many move **earlier**: the source is *after*
        /// the destination. The ascending write order keeps these alive.
        moved_down: u64,
        /// The old file's declared size.
        old_size: u64,
        /// The new file's declared size.
        new_size: u64,
        /// Whether the dead tail can be released **before** the first write: no
        /// reuse source reaches past the new size.
        ///
        /// The trim's other half — a source that reaches past the target's end
        /// is a source the eager trim would delete, so the trim has to wait for
        /// the end and the file grows during the repair.
        trim_free: bool,
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
            _ => {
                let new_size = s.new_size.max(0) as u64;
                Shape::Scored {
                    same_md5: s.same_md5,
                    nchunks: s.nchunks,
                    reused: s.reused,
                    moved: s.moved,
                    moved_up: s.moved_up,
                    moved_down: s.moved_down,
                    old_size: s.old_size.max(0) as u64,
                    new_size,
                    // The trim is free when no source reaches past the target's
                    // end. `old_max_end` is the largest source end the diff
                    // measured, and the tool under test asks the same question
                    // of the same number.
                    trim_free: s.old_max_end <= new_size,
                }
            }
        }
    }
}

/// The eleven variant classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// `old.md5 == new.md5`: the skip branch. Size and MD5 match, 0 bytes.
    Identical,
    /// `reused == 0`, `nchunks == 1`: one download into an empty target.
    ZeroReuseSingle,
    /// `reused == 0`, `nchunks > 1`: N sequential downloads, no reuse.
    ZeroReuseMulti,
    /// `0 < reused < nchunks`, `moved == 0`: the surviving chunks are already
    /// correct where they belong — **kept**, not copied.
    KeptInPlace,
    /// `moved > 0`, every move goes **down** (the source is after the
    /// destination): the bytes move earlier.
    MovedAscending,
    /// `moved > 0`, every move goes **up** (the source is before the
    /// destination): the bytes move later.
    MovedDescending,
    /// `moved > 0`, moves in **both** directions: the chunking re-flowed.
    MovedMixed,
    /// `0 < reused < nchunks`, the file on disk is longer than the target, and
    /// no source reaches past the target: the dead tail is released before the
    /// first write.
    TrimEager,
    /// `0 < reused < nchunks`, the file on disk is longer than the target, and
    /// a source reaches past it: the trim waits for the end.
    TrimDeferred,
    /// Only in the new manifest: the missing-local-file branch.
    Added,
    /// Only in the old manifest: not planned; a `--purge-after` target.
    Removed,
}

/// Whether a shape is *partially* reusable: some chunks survive and some do not.
///
/// Three of the eleven classes are exactly this — `kept_in_place` and the two
/// trim classes — and it was spelled out several times inside
/// [`Class::matches`]. Each spelling is a place where the definition of
/// "partial" can drift without anything noticing — the same failure the
/// `select` sort comparator has already had once.
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
    /// The order is a decision, not an accident: the direction classes are
    /// admitted **before** the trim classes, because `trim_deferred`'s cheapest
    /// candidate is the same file as `moved_ascending`'s. Admitting the trim
    /// first would push `moved_ascending` onto its next candidate — 24.9 MB
    /// for a second copy of the same shape.
    pub const ALL: [Class; 11] = [
        Class::Identical,
        Class::ZeroReuseSingle,
        Class::ZeroReuseMulti,
        Class::KeptInPlace,
        Class::MovedAscending,
        Class::MovedDescending,
        Class::MovedMixed,
        Class::TrimEager,
        Class::TrimDeferred,
        Class::Added,
        Class::Removed,
    ];

    /// The order admission walks them in.
    ///
    /// **`= ALL`, not a second list.** The two were spelled out separately and
    /// had drifted into the same contents; the day they stop agreeing, a class
    /// added to one and not the other becomes either invisible to `parse` or
    /// invisible to admission, and neither failure names the other list.
    pub const REQUIRED_ORDER: [Class; 11] = Class::ALL;

    /// The wire spelling, and the directory name a capture is filed under.
    pub const fn id(self) -> &'static str {
        match self {
            Class::Identical => "identical",
            Class::ZeroReuseSingle => "zero_reuse_single",
            Class::ZeroReuseMulti => "zero_reuse_multi",
            Class::KeptInPlace => "kept_in_place",
            Class::MovedAscending => "moved_ascending",
            Class::MovedDescending => "moved_descending",
            Class::MovedMixed => "moved_mixed",
            Class::TrimEager => "trim_eager",
            Class::TrimDeferred => "trim_deferred",
            Class::Added => "added",
            Class::Removed => "removed",
        }
    }

    /// What this class exercises, for `--help` and the generated README.
    ///
    /// The vocabulary is the repair spec's: a chunk is **kept** (already
    /// correct where it belongs), **copied** (its bytes live elsewhere in the
    /// file) or **fetched** (no usable local source), and the write order is
    /// the one that keeps the most sources alive.
    pub const fn exercises(self) -> &'static str {
        match self {
            Class::Identical => {
                "the skip branch: the scan's whole-file MD5 matches, 0 bytes fetched"
            }
            Class::ZeroReuseSingle => {
                "no local source: the one chunk is fetched and written in place"
            }
            Class::ZeroReuseMulti => {
                "no local source: every chunk is fetched, one at a time, into the target"
            }
            Class::KeptInPlace => {
                "the surviving chunks are already correct where they belong: kept, not copied; \
                 the rest are fetched"
            }
            Class::MovedAscending => {
                "the bytes move earlier: the ascending write order keeps every source alive"
            }
            Class::MovedDescending => {
                "the bytes move later: the descending write order keeps every source alive"
            }
            Class::MovedMixed => {
                "the chunking re-flowed: the ordered tier keeps the larger half and fetches \
                 the rest"
            }
            Class::TrimEager => {
                "the file on disk is longer than the target and no source reaches past it: \
                 the dead tail is released before the first write"
            }
            Class::TrimDeferred => {
                "the file on disk is longer than the target and a source reaches past it: \
                 the trim waits for the end"
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
    /// **`identical` is exclusive with everything else.** A file whose two
    /// whole-file MD5s agree is *skipped* — the repair reads its MD5, sees the
    /// target is already there, and touches nothing. It makes no content
    /// decision at all, so it cannot also be a statement about what the
    /// repair would do to its chunks. Every other class therefore requires
    /// `same_md5: false`, and the live pair does contain a file that would
    /// otherwise be in two classes at once: `Music4.pck` has 336 chunks, all
    /// of them reused, four of them moved, and the same whole-file MD5 on both
    /// sides. Its chunking was re-tiled and the file's *content* did not
    /// change, so it is the skip branch and nothing else.
    ///
    /// The exclusion is not cosmetic: A19 says a captured file differs between
    /// `game/` and `expected/` exactly when its class says it should, so a
    /// same-MD5 file admitted for a content class would fail the capture.
    pub fn matches(self, s: Shape) -> bool {
        use Shape::{Added, Removed, Scored};
        match self {
            Class::Identical => matches!(s, Scored { same_md5: true, .. }),
            Class::ZeroReuseSingle => {
                matches!(
                    s,
                    Scored {
                        same_md5: false,
                        nchunks: 1,
                        reused: 0,
                        ..
                    }
                )
            }
            Class::ZeroReuseMulti => {
                matches!(s, Scored { same_md5: false, reused: 0, nchunks, .. } if nchunks > 1)
            }
            // The surviving chunks are already correct where they belong: a
            // same-offset source *is* the chunk's own destination, so the
            // chunk is kept rather than copied. The payload is 0, so the
            // staged tier stages nothing.
            Class::KeptInPlace => matches!(
                s,
                Scored { same_md5: false, moved: 0, reused, nchunks, .. }
                    if is_partial(reused, nchunks)
            ),
            // The direction is the *sign* of the moves, and the two direction
            // classes are exclusive: a file whose every move goes down is not
            // a file whose every move goes up.
            Class::MovedAscending => {
                matches!(s, Scored { same_md5: false, moved, moved_up, .. } if moved > 0 && moved_up == 0)
            }
            Class::MovedDescending => {
                matches!(s, Scored { same_md5: false, moved, moved_down, .. } if moved > 0 && moved_down == 0)
            }
            Class::MovedMixed => matches!(
                s,
                Scored { same_md5: false, moved, moved_up, moved_down, .. }
                    if moved > 0 && moved_up > 0 && moved_down > 0
            ),
            // The trim is eager when the dead tail can be released before the
            // first write — which is exactly when no source reaches past the
            // target's end — and deferred when a source does.
            Class::TrimEager => matches!(
                s,
                Scored { same_md5: false, old_size, new_size, trim_free, reused, nchunks, .. }
                    if trim_free && old_size > new_size && is_partial(reused, nchunks)
            ),
            Class::TrimDeferred => matches!(
                s,
                Scored { same_md5: false, old_size, new_size, trim_free, reused, nchunks, .. }
                    if !trim_free && old_size > new_size && is_partial(reused, nchunks)
            ),
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
/// Not a *requirement* — the matrix reports on all eleven whether or not any
/// of them is required, because "this pair has no `moved_mixed` file" is worth
/// knowing even to a caller who asked for nothing in particular. What
/// `--require-class` does is promote a subset to fatal.
pub fn all_in_admission_order() -> Vec<Class> {
    Class::REQUIRED_ORDER.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scored shape, spelled the way the predicates read it.
    ///
    /// `size_equal` and `moved` are the two knobs the old table had; the
    /// direction and the trim flag are spelled separately, because they are
    /// what the new classes turn on.
    fn scored(same_md5: bool, size_equal: bool, nchunks: u64, reused: u64, moved: u64) -> Shape {
        shape(
            same_md5,
            nchunks,
            reused,
            moved,
            0,
            moved,
            100,
            if size_equal { 100 } else { 200 },
            true,
        )
    }

    /// The full shape, every field spoken for.
    #[allow(clippy::too_many_arguments)]
    fn shape(
        same_md5: bool,
        nchunks: u64,
        reused: u64,
        moved: u64,
        moved_up: u64,
        moved_down: u64,
        old_size: u64,
        new_size: u64,
        trim_free: bool,
    ) -> Shape {
        Shape::Scored {
            same_md5,
            nchunks,
            reused,
            moved,
            moved_up,
            moved_down,
            old_size,
            new_size,
            trim_free,
        }
    }

    /// The eleven classes against the shapes the spec's table describes. Each
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
            // 0 < reused < nchunks, moved == 0
            (
                Class::KeptInPlace,
                shape(false, 3, 1, 0, 0, 0, 100, 100, true),
            ),
            // moved > 0, every move goes down (the source is after the destination)
            (
                Class::MovedAscending,
                shape(false, 3, 2, 1, 0, 1, 100, 100, true),
            ),
            // moved > 0, every move goes up (the source is before the destination)
            (
                Class::MovedDescending,
                shape(false, 3, 2, 1, 1, 0, 100, 100, true),
            ),
            // moved > 0, moves in both directions
            (
                Class::MovedMixed,
                shape(false, 4, 3, 2, 1, 1, 100, 100, true),
            ),
            // 0 < reused < nchunks, the file is longer than the target, no source
            // reaches past the target
            (
                Class::TrimEager,
                shape(false, 3, 2, 0, 0, 0, 200, 100, true),
            ),
            // ... and a source does
            (
                Class::TrimDeferred,
                shape(false, 3, 2, 1, 0, 1, 200, 100, false),
            ),
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
    /// about which decision a capture forces.
    ///
    /// The sets are not all singletons, and that is a fact about the classes
    /// rather than about the fixture: **every trim class is also a
    /// `kept_in_place` or a direction class**, because the trim needs a
    /// partially-reusable file and the direction is a property of the same
    /// file's moves. The two are what `covered_by` is built from.
    #[test]
    fn each_documented_shape_matches_exactly_its_documented_classes() {
        let cases: Vec<(Vec<Class>, Shape)> = vec![
            (vec![Class::Identical], scored(true, true, 1, 1, 0)),
            (vec![Class::ZeroReuseSingle], scored(false, true, 1, 0, 0)),
            (vec![Class::ZeroReuseMulti], scored(false, false, 3, 0, 0)),
            (
                vec![Class::KeptInPlace],
                shape(false, 3, 1, 0, 0, 0, 100, 100, true),
            ),
            (
                vec![Class::MovedAscending],
                shape(false, 3, 2, 1, 0, 1, 100, 100, true),
            ),
            (
                vec![Class::MovedDescending],
                shape(false, 3, 2, 1, 1, 0, 100, 100, true),
            ),
            (
                vec![Class::MovedMixed],
                shape(false, 4, 3, 2, 1, 1, 100, 100, true),
            ),
            (
                // Nothing moves, the file shrinks, and no source reaches past
                // the target: both the keep and the eager trim.
                vec![Class::KeptInPlace, Class::TrimEager],
                shape(false, 3, 2, 0, 0, 0, 200, 100, true),
            ),
            (
                // One move down, the file shrinks, and a source reaches past
                // the target: both the direction and the deferred trim.
                vec![Class::MovedAscending, Class::TrimDeferred],
                shape(false, 3, 2, 1, 0, 1, 200, 100, false),
            ),
        ];
        for (want, shape) in cases {
            assert_eq!(Class::all_satisfied_by(shape), want, "{shape:?}");
        }
    }

    /// The two direction classes are exclusive, and the mixed one is exactly
    /// "neither of them". A file whose every move goes one way is a file whose
    /// chunking did not re-flow, and saying otherwise would put the same file
    /// in two direction classes.
    #[test]
    fn the_direction_classes_are_exclusive() {
        let all_down = shape(false, 4, 3, 2, 0, 2, 100, 100, true);
        assert!(Class::MovedAscending.matches(all_down));
        assert!(!Class::MovedDescending.matches(all_down));
        assert!(!Class::MovedMixed.matches(all_down));

        let all_up = shape(false, 4, 3, 2, 2, 0, 100, 100, true);
        assert!(Class::MovedDescending.matches(all_up));
        assert!(!Class::MovedAscending.matches(all_up));
        assert!(!Class::MovedMixed.matches(all_up));

        let both = shape(false, 4, 3, 2, 1, 1, 100, 100, true);
        assert!(Class::MovedMixed.matches(both));
        assert!(!Class::MovedAscending.matches(both));
        assert!(!Class::MovedDescending.matches(both));

        // And a file that does not move at all is none of them.
        let still = shape(false, 4, 3, 0, 0, 0, 100, 100, true);
        for c in [
            Class::MovedAscending,
            Class::MovedDescending,
            Class::MovedMixed,
        ] {
            assert!(!c.matches(still), "{c} must not take moved == 0");
        }
    }

    /// The two trim classes are exclusive, and the split is the *whole* point
    /// of having them: the eager trim releases the dead tail before the first
    /// write, the deferred one waits for the end.
    #[test]
    fn the_trim_classes_are_exclusive_and_need_a_shrinking_file() {
        let eager = shape(false, 3, 2, 0, 0, 0, 200, 100, true);
        assert!(Class::TrimEager.matches(eager));
        assert!(!Class::TrimDeferred.matches(eager));

        let deferred = shape(false, 3, 2, 1, 0, 1, 200, 100, false);
        assert!(Class::TrimDeferred.matches(deferred));
        assert!(!Class::TrimEager.matches(deferred));

        // A file that does not shrink has no dead tail to release.
        let same = shape(false, 3, 2, 0, 0, 0, 100, 100, true);
        assert!(!Class::TrimEager.matches(same));
        assert!(!Class::TrimDeferred.matches(same));

        let grows = shape(false, 3, 2, 0, 0, 0, 100, 200, true);
        assert!(!Class::TrimEager.matches(grows));
        assert!(!Class::TrimDeferred.matches(grows));
    }

    /// `reused` must be *strictly* between 0 and `nchunks` for the partial
    /// classes; the ends belong to the zero-reuse classes.
    #[test]
    fn partial_means_strictly_between_and_the_ends_belong_elsewhere() {
        let none_reused = scored(false, true, 4, 0, 0);
        for c in [Class::KeptInPlace, Class::TrimEager, Class::TrimDeferred] {
            assert!(!c.matches(none_reused), "{c} must not take reused == 0");
        }
        assert!(Class::ZeroReuseMulti.matches(none_reused));

        let all_reused = scored(false, true, 4, 4, 0);
        for c in [Class::KeptInPlace, Class::TrimEager, Class::TrimDeferred] {
            assert!(
                !c.matches(all_reused),
                "{c} must not take reused == nchunks"
            );
        }
        // A file whose every chunk is reused and none of them moved is the
        // keep-in-place shape *only* when some chunk is missing — with nothing
        // missing there is nothing to keep, so it matches no class at all.
        assert!(Class::all_satisfied_by(all_reused).is_empty());
    }

    /// `identical` is separated from everything else by the whole-file MD5: a
    /// file whose every chunk is reused is the skip branch when the MD5s
    /// agree and nothing else when they do not.
    #[test]
    fn identical_is_separated_from_the_rest_by_the_file_md5() {
        let identical = scored(true, true, 4, 4, 0);
        assert_eq!(Class::all_satisfied_by(identical), vec![Class::Identical]);
        let reordered = scored(false, true, 4, 4, 0);
        assert!(
            Class::all_satisfied_by(reordered).is_empty(),
            "a file whose every chunk is reused and none of them moved is not a \
             class of its own: it is the shape the removed `full_reuse_reordered` \
             used to claim, and there is no decision behind it"
        );
    }

    /// **The live pair's one file that would be in two classes at once.**
    ///
    /// `Music4.pck`: 336 chunks, all reused, four moved up, and the same
    /// whole-file MD5 on both sides. The chunking was re-tiled; the file's
    /// *content* did not change, so the repair skips it and it makes no
    /// content decision at all.
    ///
    /// Without the exclusion it satisfies `moved_descending` as well, and
    /// three things go wrong: the published population for that class is one
    /// too high, the instrument (which short-circuits on the MD5) stops
    /// agreeing with the crate, and A19 would fail a capture that admitted it
    /// — "is unchanged but its class says it should differ".
    #[test]
    fn a_file_whose_md5s_agree_is_only_the_skip_branch() {
        // The shape of `Music4.pck`, as `diff/candidates.json` records it.
        let music4 = shape(true, 336, 336, 4, 4, 0, 398_535_566, 398_535_566, true);
        assert_eq!(
            Class::all_satisfied_by(music4),
            vec![Class::Identical],
            "the whole-file MD5 is the skip branch, and nothing else"
        );
        // And the class it would have joined says so about itself.
        assert!(!Class::MovedDescending.matches(music4));
        assert!(!Class::MovedAscending.matches(music4));
        assert!(!Class::MovedMixed.matches(music4));
        assert!(!Class::KeptInPlace.matches(music4));
        assert!(!Class::TrimEager.matches(music4));
        assert!(!Class::TrimDeferred.matches(music4));
        assert!(!Class::ZeroReuseSingle.matches(music4));
        assert!(!Class::ZeroReuseMulti.matches(music4));
    }

    /// Every content class refuses a file whose two MD5s agree, and the
    /// skip branch is the only one that accepts it.
    ///
    /// The exclusion is a property of the *class set*, not of one class, so it
    /// is asserted for all of them at once: a class added later without the
    /// exclusion would put a skipped file into a content class and A19 would
    /// catch it in a capture rather than here.
    #[test]
    fn every_content_class_excludes_the_skip_branch() {
        let same = scored(true, true, 4, 2, 1);
        for c in Class::ALL {
            let matched = c.matches(same);
            assert_eq!(
                matched,
                c == Class::Identical,
                "{c} vs a file whose two MD5s agree: {matched}"
            );
        }
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
        let empty = scored(false, true, 0, 0, 0);
        assert!(
            Class::all_satisfied_by(empty).is_empty(),
            "{:?}",
            Class::all_satisfied_by(empty)
        );
        // An empty *identical* file is still the skip branch.
        assert_eq!(
            Class::all_satisfied_by(scored(true, true, 0, 0, 0)),
            vec![Class::Identical]
        );
    }

    /// Most shapes satisfy exactly one class. Where they do not, the tag list
    /// has to say so, because `covered_by` is built from it.
    ///
    /// The overlap the live pair actually has: a file whose chunking re-flowed
    /// *and* whose file on disk is longer than the target answers both the
    /// direction class and the trim class. That is the file
    /// `trim_deferred` is covered by, and the reason the admission has to
    /// learn `covered_by` at all.
    #[test]
    fn the_common_case_is_exactly_one_class_and_the_overlap_is_named() {
        for shape in [
            scored(true, true, 1, 1, 0),
            scored(false, true, 1, 0, 0),
            scored(false, true, 3, 1, 0),
            scored(false, false, 3, 0, 0),
            shape(false, 4, 3, 2, 0, 2, 100, 100, true),
            shape(false, 4, 3, 2, 1, 1, 100, 100, true),
        ] {
            let all = Class::all_satisfied_by(shape);
            assert_eq!(all.len(), 1, "{shape:?} -> {all:?}");
        }

        // The overlap: moves that go down, in a file that shrinks, with a
        // source reaching past the target's end.
        let both = shape(false, 10, 8, 4, 0, 4, 200, 100, false);
        assert_eq!(
            Class::all_satisfied_by(both),
            vec![Class::MovedAscending, Class::TrimDeferred],
            "the order is the admission order"
        );
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

    /// The order is a decision, not an accident: the **direction** classes are
    /// admitted before the **trim** classes, because `trim_deferred`'s cheapest
    /// candidate is the same file as `moved_ascending`'s. Admitting the trim
    /// first would push `moved_ascending` onto its next candidate — 24.9 MB
    /// for a second copy of the same shape.
    #[test]
    fn the_direction_classes_are_admitted_before_the_trim_classes() {
        let order = all_in_admission_order();
        let at = |c: Class| order.iter().position(|x| *x == c).expect("in the order");
        assert!(
            at(Class::MovedAscending) < at(Class::TrimEager),
            "the direction classes come first"
        );
        assert!(
            at(Class::MovedDescending) < at(Class::TrimEager),
            "the direction classes come first"
        );
        assert!(at(Class::MovedMixed) < at(Class::TrimEager));
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
