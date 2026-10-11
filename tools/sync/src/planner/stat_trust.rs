//! Which parts of a file's stat a run will accept as evidence that two sides differ.
//!
//! [`StatTrust`] is the short circuit, and it is an **optimisation**, so it has to
//! be switchable. It lives beside the vocabulary it speaks rather than inside
//! `planner` or `diff` because two of the three modules in this tree consult it,
//! and a second definition anywhere would be a second spelling of a rule the
//! planner and the diff have to agree on.
//!
//! Its tests are about *switching the short circuit off*, which is only observable
//! through [`crate::planner::plan_pairs`] — a distrusted field has to put a pair
//! back into the undecided set, and that is a statement about the plan rather than
//! about the predicate. So they drive the pair planner and assert the consequence.

/// Which stat fields differ between two sides.
///
/// Two booleans rather than a `Vec<StatField>`, because the question is asked once
/// per pair on a path that may hold hundreds of thousands of them and the answer is
/// almost always "nothing". The list form exists too, as [`Self::fields`], for the
/// one caller that has to *name* the fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DifferingFields {
    /// The two sides' lengths differ.
    pub size: bool,
    /// The two sides' modification times differ.
    pub mtime: bool,
}

impl DifferingFields {
    /// True when at least one field differs.
    pub fn any(&self) -> bool {
        self.size || self.mtime
    }

    /// The fields that differ, **size first**.
    ///
    /// The order is the one [`StatTrust::settles`] tests them in, so it is both the
    /// first one to have settled a pair and the first one a reason names. A caller
    /// cannot depend on the reverse.
    pub fn fields(&self) -> impl Iterator<Item = StatField> {
        [(self.size, StatField::Size), (self.mtime, StatField::Mtime)]
            .into_iter()
            .filter_map(|(differing, field)| differing.then_some(field))
    }
}

/// One of the two stat fields a comparison consults.
///
/// A type rather than a string because the field is **named in output**: a reason
/// tag says which field settled a pair, and a tag assembled from `&str`s would have
/// two spellings and no compiler to catch them disagreeing. It lives beside
/// [`StatTrust`] rather than in [`crate::diff`] because it is the vocabulary
/// `StatTrust`'s own predicates speak — the diff reports it, but nothing here needs
/// to know that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StatField {
    /// Byte length.
    Size,
    /// Modification time.
    Mtime,
}

impl StatField {
    /// The spelling used inside a reason tag: `stat-size`, `unverifiable:size`.
    pub fn as_str(&self) -> &'static str {
        match self {
            StatField::Size => "size",
            StatField::Mtime => "mtime",
        }
    }
}

/// Which parts of a file's stat a run will accept as evidence that two sides differ.
///
/// The short circuit in [`crate::planner::plan_pairs`] is an **optimisation**, so
/// it has to be switchable — and naming the two fields it consults is better than
/// adding a switch for the optimisation itself: `--no-trust-size --no-trust-mtime`
/// together *is* "disable it", and no third spelling is needed to reach it.
///
/// An untrusted field does not settle a pair, so the pair becomes undecided and
/// needs a digest. For `mtime` that can change the *verdict*: a file whose mtime
/// moved but whose bytes did not is `CHANGED` on the strength of the timestamp
/// alone, and distrusting mtime is how you ask whether that is really so. For
/// `size` it cannot — different lengths are different content — so the verdict is
/// unchanged and the flag buys the read instead, which is a different and
/// narrower thing than `--no-trust-cached-hashes` (which distrusts a *digest*,
/// and so only ever affects pairs that were going to be read anyway).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatTrust {
    /// `--no-trust-size` absent: a size difference settles a pair.
    pub size: bool,
    /// `--no-trust-mtime` absent: an mtime difference settles a pair.
    pub mtime: bool,
}

impl Default for StatTrust {
    /// **Trusts both.** An unflagged run must mean exactly the thing the short
    /// circuit optimises, so the flags are additive and default to the behaviour
    /// they optimise.
    fn default() -> Self {
        StatTrust {
            size: true,
            mtime: true,
        }
    }
}

impl StatTrust {
    /// `--no-trust-size`
    pub fn without_size(mut self) -> Self {
        self.size = false;
        self
    }

    /// `--no-trust-mtime`
    pub fn without_mtime(mut self) -> Self {
        self.mtime = false;
        self
    }

    /// `--no-trust-size`, applied only when the flag was actually passed.
    ///
    /// The flag is a bool and the field is a `bool` meaning the opposite thing, and
    /// negating at the boundary is where that belongs — a `StatTrust` built by
    /// negating inside itself would be right only by accident.
    pub fn without_size_if(self, distrusted: bool) -> Self {
        if distrusted {
            self.without_size()
        } else {
            self
        }
    }

    /// `--no-trust-mtime`, applied only when the flag was actually passed.
    pub fn without_mtime_if(self, distrusted: bool) -> Self {
        if distrusted {
            self.without_mtime()
        } else {
            self
        }
    }

    /// Which fields differ **and are trusted** — so a pair for which this is
    /// non-empty is settled without a digest.
    ///
    /// The single definition of the short circuit's condition, so the planner and
    /// anything else that has to agree with it read the same rule. Naming the fields
    /// rather than comparing them here is what lets a caller that needs to *report*
    /// the reason ask the same question and get the answer as a value, instead of
    /// re-deriving which field happened to settle the pair.
    pub fn trusted_differs(
        &self,
        a_size: u64,
        a_mtime: i64,
        b_size: u64,
        b_mtime: i64,
    ) -> DifferingFields {
        DifferingFields {
            size: self.size && a_size != b_size,
            mtime: self.mtime && a_mtime != b_mtime,
        }
    }

    /// True when this pair's stat, as trusted, settles it without a digest.
    ///
    /// Reads [`Self::trusted_differs`] rather than writing the condition out, so the
    /// boolean and the reason it can give cannot drift apart: a predicate that has to
    /// be re-typed is a predicate that will be.
    pub fn settles(&self, a_size: u64, a_mtime: i64, b_size: u64, b_mtime: i64) -> bool {
        self.trusted_differs(a_size, a_mtime, b_size, b_mtime).any()
    }

    /// Which fields differ **and are not trusted**.
    ///
    /// The companion to [`Self::trusted_differs`], and the question the diff has to
    /// ask when it has *no* digest to fall back on. `trusted_differs` asks "is there
    /// trusted evidence of difference?"; this asks "was there evidence we chose to
    /// ignore?" — and a pair whose only evidence of difference was ignored has no
    /// evidence **of sameness** either, so it cannot be reported equal.
    ///
    /// It exists because [`crate::diff::hashes_differ`] is silent when handed nothing to
    /// compare, and that silence is only safe while every pair reaching it has a digest
    /// — which is what [`crate::planner::Required`] guarantees: the diff reaches the digest
    /// comparison only for an undecided pair, and the planner has an entry for every
    /// undecided pair. A run that requested **no** algorithm has no entry, so the silence
    /// stops being a fallback and becomes the whole of the answer.
    pub fn untrusted_differs(
        &self,
        a_size: u64,
        a_mtime: i64,
        b_size: u64,
        b_mtime: i64,
    ) -> DifferingFields {
        DifferingFields {
            size: !self.size && a_size != b_size,
            mtime: !self.mtime && a_mtime != b_mtime,
        }
    }

    // Deliberately no `untrusted_disagrees() -> bool` alongside this one. It would
    // have no caller, and a second spelling of a rule is a rule that can drift: when
    // this was a boolean, the mutation that made *it* disagree with the diff while the
    // diff agreed with the itemised form passed every test in the suite. One
    // definition, and the caller that wants a bool asks `.any()`.

    /// True when **at least one** field is not trusted.
    ///
    /// For a caller that never pairs anything, such as `update`, and for validating a
    /// request that pairs with no digest available. Both need the same question — "did
    /// the user take anything away?" — rather than the negation of two fields written
    /// out, because the two questions are *not* complements and conflating them is a
    /// real trap: `distrusts_any` is the safe condition, since with no digest to fall
    /// back on, one untrusted field is enough to leave some pair undecidable. Its
    /// complement would only be the right question if a single trusted field could
    /// settle every pair, which it cannot.
    pub fn distrusts_any(&self) -> bool {
        !self.size || !self.mtime
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::fixtures::{Shape, dir, file, map};

    // -- distrusting the short circuit --------------------------------------------

    /// `--no-trust-size` and `--no-trust-mtime` each put one stat field back into the
    /// undecided set, and both together leave the short circuit with nothing to act on.
    ///
    /// The first case states the **default** alongside the flagged one, because the
    /// property that matters is not that a flag does something — it is that it changes
    /// only the named field, and only pairs that field decides.

    #[test]
    fn a_stat_differing_pair_is_undecided_when_the_field_it_differs_in_is_distrusted() {
        let s = map(vec![("a.txt", file(11, 100))]);
        let d = map(vec![("a.txt", file(10, 100))]);

        // Size differs, mtime agrees: trusting size settles it for free.
        let free = Shape::FOLDERS
            .run(&s, &d, true, StatTrust::default())
            .unwrap();
        assert!(
            free.required.by_rel.is_empty(),
            "nothing to settle by digest"
        );
        assert!(free.src.pending().is_empty() && free.dst.pending().is_empty());

        // Distrust size and the same pair needs a digest.
        let hashing = Shape::FOLDERS
            .run(&s, &d, true, StatTrust::default().without_size())
            .unwrap();
        assert_eq!(
            hashing.required.of("a.txt"),
            ["md5"],
            "the pair is now undecided, so it must be comparable"
        );
        assert_eq!(
            hashing.dst.pending(),
            vec!["a.txt"],
            "and read to settle it"
        );
    }

    /// The same pair with the *other* field distrusted is still free. This is the case
    /// that distinguishes the two flags from one switch: `--no-trust-mtime` must not
    /// rehash a pair whose sizes already differ.
    #[test]
    fn distrusting_mtime_leaves_a_size_difference_settling_the_pair() {
        let s = map(vec![("a.txt", file(11, 100))]);
        let d = map(vec![("a.txt", file(10, 100))]);
        let p = Shape::FOLDERS
            .run(&s, &d, true, StatTrust::default().without_mtime())
            .unwrap();
        assert!(
            p.required.by_rel.is_empty() && p.src.pending().is_empty(),
            "size still settles it: distrusting mtime must not widen this"
        );
    }

    /// **Both flags is "disable the short circuit".** Four pair states, each of which is
    /// free under the default, and none of which is free here.
    #[test]
    fn distrusting_both_fields_leaves_no_pair_settled_by_stat() {
        for (a, b) in [
            (file(11, 100), file(10, 100)), // size differs
            (file(10, 101), file(10, 100)), // mtime differs
            (file(11, 101), file(10, 100)), // both differ
        ] {
            let (s, d) = (map(vec![("a.txt", a)]), map(vec![("a.txt", b)]));
            let off = StatTrust::default().without_size().without_mtime();
            let p = Shape::FOLDERS.run(&s, &d, true, off).unwrap();
            assert_eq!(
                p.required.of("a.txt"),
                ["md5"],
                "nothing is left for the short circuit to act on"
            );
        }
    }

    /// The **other** half of the point, and the reason `no-trust-size` is not
    /// `no-trust-cached-hashes`.
    ///
    /// A size difference implies different content, so the digest comparison will always
    /// disagree — the *verdict* cannot change. What changes is that the side is read, and
    /// so its cache row is rewritten with a digest it did not have.
    ///
    /// That is the useful part: a size-changed row arrives in phase A with its digests
    /// dropped (a stale row's are all suspect), so under the short circuit it keeps
    /// *no* digest forever, and the stat-only row can never become comparable again. This
    /// is what repairs it.
    #[test]
    fn a_distrusted_field_widens_the_undecided_set_without_changing_the_verdict() {
        let s = map(vec![("a.txt", file(11, 100))]);
        let d = map(vec![("a.txt", file(10, 100))]);
        let off = StatTrust::default().without_size().without_mtime();
        let p = Shape::FOLDERS.run(&s, &d, true, off).unwrap();
        // Both sides are read, and neither had a digest to reuse.
        assert_eq!(p.src.pending().len(), 1);
        assert_eq!(p.dst.pending().len(), 1);
        // Under all-of the pair is settled by the whole requested list, so this is
        // comparable rather than merely comparable-by-one-algorithm.
        assert_eq!(p.required.of("a.txt"), ["md5"]);
    }

    /// A distrusted field must not resurrect a pair that stat cannot decide *even so* —
    /// a dir, or a kind conflict. Those are settled by presence and kind, which no flag
    /// here touches, and asking for a digest for a directory would fail.
    #[test]
    fn a_distrusted_field_does_not_widen_the_set_past_what_stat_can_decide() {
        let off = StatTrust::default().without_size().without_mtime();
        // Dirs on both sides.
        let (s, d) = (map(vec![("sub", dir())]), map(vec![("sub", dir())]));
        let p = Shape::FOLDERS.run(&s, &d, true, off).unwrap();
        assert!(
            p.required.by_rel.is_empty(),
            "a dir is compared by presence"
        );

        // File against dir.
        let (s, d) = (
            map(vec![("a.txt", file(10, 100))]),
            map(vec![("a.txt", dir())]),
        );
        let p = Shape::FOLDERS.run(&s, &d, true, off).unwrap();
        assert!(
            p.required.by_rel.is_empty(),
            "a kind conflict is decided by kind"
        );

        // One-sided paths.
        let (s, d) = (map(vec![("only.txt", file(10, 100))]), map(vec![]));
        let p = Shape::FOLDERS.run(&s, &d, true, off).unwrap();
        assert!(
            p.required.by_rel.is_empty(),
            "MISSING is decided by presence"
        );
    }

    /// **Trusting both is the default**, and it is the field that must not move: every
    /// other case in this file is describing the unflagged run.
    #[test]
    fn the_default_distrusts_neither_field() {
        let t = StatTrust::default();
        assert!(t.size && t.mtime, "an unflagged run trusts both");
        let s = map(vec![("a.txt", file(11, 100))]);
        let d = map(vec![("a.txt", file(10, 100))]);
        assert!(
            Shape::FOLDERS
                .run(&s, &d, true, t)
                .unwrap()
                .required
                .by_rel
                .is_empty(),
            "so a size difference still settles the pair for free"
        );
    }
}