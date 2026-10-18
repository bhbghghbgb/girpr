//! What a run must compute: [`HashPlan`] and [`Required`].
//!
//! The two answers [`crate::planner::plan_pairs`] produces, and nothing else. A
//! [`HashPlan`] is per side and says *which digests that side still owes*;
//! [`Required`] is per pair and says *which digests settle it*. They are separate
//! because they answer separate questions, and because only the second one travels
//! into [`crate::diff`].
//!
//! [`plan_one`] lives here rather than beside the pair planner: it is the one
//! function that writes into a [`HashPlan`], so it belongs with the type whose
//! invariant it maintains — a side that cannot hash is never given an entry.

use std::collections::HashMap;

use crate::effective::SideEntry;
use crate::hash::{AlgoSet, HashAlg};

use super::pairs::SideRequest;

/// The algorithms that settle each undecided pair, from one decision.
///
/// Produced by [`crate::planner::plan_pairs`] and consumed by [`crate::diff::diff_maps`]. Handing
/// the diff *this* rather than the algorithm list is what lets one run settle path
/// X by md5 and path Y by sha256: under `all-of` every entry is the whole requested
/// list, so a uniform run would be described just as well by passing that list
/// straight through.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Required {
    /// src relative path -> the algorithms that settle it.
    pub by_rel: HashMap<String, AlgoSet>,
    /// Consulted for a path with no entry. See [`Required::of`].
    pub fallback: AlgoSet,
}

impl Required {
    /// The algorithms that settle `srel`, which is a **src** path.
    ///
    /// The fallback is the whole requested list, and it is unreachable while the
    /// planner and the diff agree on which pairs are undecided: a path reaches the
    /// digest comparison only when size and mtime already agreed, every such path is
    /// undecided, and the planner has an entry for every undecided file pair. It is
    /// still the right answer to give, because
    /// [`crate::diff::hashes_differ`] is *silent* when a digest is missing — a
    /// fallback of nothing would report every unplanned pair as equal. A safe fallback
    /// and the reachable answer coincide, so the safety costs nothing.
    pub fn of(&self, srel: &str) -> AlgoSet {
        self.by_rel.get(srel).copied().unwrap_or(self.fallback)
    }
}

/// What a run must compute for one side.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HashPlan {
    /// Relative path -> the algorithms phase C must produce for it. An absent
    /// or empty list means "this path needs no read".
    ///
    /// This is the set to *compute*, not the set to *end up holding*. The
    /// distinction is what makes `--no-trust-cached-hashes` expressible: it
    /// asks for the digest again, while phase A keeps reporting the cached one
    /// so the planner can still see what is on hand.
    pub by_rel: HashMap<String, AlgoSet>,
}

impl HashPlan {
    /// The algorithms to compute for `rel`; empty when the path needs no read.
    pub fn get(&self, rel: &str) -> AlgoSet {
        self.by_rel.get(rel).copied().unwrap_or(AlgoSet::NONE)
    }

    /// True when `rel` needs no read at all, because everything it needs is
    /// already cached.
    pub fn is_settled(&self, rel: &str) -> bool {
        self.get(rel).is_empty()
    }

    /// Every path the plan asks to hash, in sorted order.
    pub fn pending(&self) -> Vec<&String> {
        let mut v: Vec<&String> = self
            .by_rel
            .iter()
            .filter(|(_, a)| !a.is_empty())
            .map(|(r, _)| r)
            .collect();
        v.sort();
        v
    }

    /// Total number of digests the plan will compute.
    pub fn digest_count(&self) -> usize {
        self.by_rel.values().map(|a| a.len()).sum()
    }

    /// The algorithms this side still has to compute for `entry`, in
    /// [`AlgoSet::iter`] order.
    ///
    /// Everything in `want`, minus what the row already holds — set arithmetic
    /// rather than a filtered copy of the request, because the answer is the same
    /// one for every path under `--hash-all-of` and there is one of them per file
    /// in the tree.
    ///
    /// `no_trust` overrides the subtraction: distrusting the cache is a refusal
    /// to *reuse* a digest, never an instruction to forget it — see
    /// [`crate::effective::merge_row`], which stores the same row either way —
    /// so the plan asks again while phase A keeps reporting the old value.
    ///
    /// Held is decided by **name**, not by slot: a row whose `md5` is stored with an
    /// unexpected length is not held in the sense that matters, and must not be
    /// counted as covering a request. A name this build does not recognise cannot be
    /// in `want` at all, so it cannot widen the answer either.
    fn missing(entry: &SideEntry, want: AlgoSet, no_trust: bool) -> AlgoSet {
        if no_trust {
            return want;
        }
        let have = HashAlg::ALL
            .into_iter()
            .filter(|a| entry.cached.get_named(a.as_str()).is_some())
            .collect::<AlgoSet>();
        want.difference(have)
    }

    /// The plan for a side with no counterpart, and the one `update` uses: every
    /// requested algorithm, for every file, minus what the cache already holds.
    ///
    /// There is no pairing step, so every file is a candidate. That is the right
    /// answer for a command with no other side to be lazy *relative to*, and the
    /// wrong one for a comparison — see [`crate::planner::plan_pairs`].
    ///
    /// ## Why this needs no coverage check
    ///
    /// [`crate::planner::plan_pairs`] fails a side that cannot supply a requested digest, because a
    /// comparison cannot answer the question without it. `update` asks a folder for
    /// everything it has, and a folder can always go and compute any digest of any
    /// file it holds — so its own availability was never in question, and it needs
    /// no exemption from the rule. The predicate is the same one for both: "can this
    /// side obtain it", and a folder always can.
    ///
    /// The two consequences worth stating, because one is easy to get wrong:
    ///
    /// - It returns `Self`, not `Result<Self>`, and there is no capability input.
    ///   Not because coverage was skipped, but because a one-sided plan has nothing
    ///   to check a shortfall *against* — the check is about a pair.
    /// - `no_trust` is load-bearing here in a way it is not in `plan_pairs`.
    ///   `update` passes `true` unconditionally, which is what makes it recompute
    ///   every digest on every run rather than trusting a row whose stat still
    ///   matches. `update_recomputes_every_digest_and_repairs_a_wrong_one` in
    ///   `tests/update.rs` is the only test that would notice if it stopped, because
    ///   idempotence cannot distinguish a reused digest from a recomputed one.
    ///
    /// The plan's entries are in canonical order, which is [`AlgoSet`]'s and not
    /// the user's: the flag order is what `pick_one` reads, and it reads the
    /// **request** for that. Nothing downstream of here depends on which order the
    /// flags were typed in, which is what makes a `--why` tag stable across them.
    pub fn plan_one_side(
        entries: &HashMap<String, SideEntry>,
        algos: &[HashAlg],
        no_trust: bool,
    ) -> Self {
        let requested = AlgoSet::from_iter(algos.iter().copied());
        let mut by_rel = HashMap::new();
        for (rel, e) in entries {
            if !e.is_file() || requested.is_empty() {
                continue;
            }
            let want = Self::missing(e, requested, no_trust);
            if !want.is_empty() {
                by_rel.insert(rel.clone(), want);
            }
        }
        Self { by_rel }
    }
}

/// Add whatever `req` still owes for `rel` to `into`.
///
/// Skipped for a side that cannot hash, which is why `plan.required` — not this
/// function — is the record of what the run must produce: a side with no filesystem
/// is never *asked*, so a plan entry for it would be a request nothing could honour.
pub(super) fn plan_one(
    req: &SideRequest<'_>,
    e: &SideEntry,
    rel: &str,
    algos: AlgoSet,
    into: &mut HashPlan,
) {
    if !req.cap.can_hash_from_disk {
        return;
    }
    let want = HashPlan::missing(e, algos, req.no_trust);
    if !want.is_empty() {
        into.by_rel.insert(rel.to_string(), want);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::DigestSet;
    use std::collections::HashMap;

    fn entry(cached: &[&str]) -> SideEntry {
        SideEntry {
            kind: "file".into(),
            size: 10,
            mtime_ns: 99,
            cached: {
                let mut d = DigestSet::default();
                for a in cached {
                    d.insert_named(a, &[1u8; 16]);
                }
                d
            },
            fresh: !cached.is_empty(),
        }
    }

    fn dir() -> SideEntry {
        SideEntry {
            kind: "dir".into(),
            size: 0,
            mtime_ns: 0,
            cached: DigestSet::default(),
            fresh: true,
        }
    }

    fn entries() -> HashMap<String, SideEntry> {
        let mut m = HashMap::new();
        m.insert("cached.txt".to_string(), entry(&["md5"]));
        m.insert("stale.txt".to_string(), entry(&[]));
        m.insert("sub".to_string(), dir());
        m
    }

    fn algos() -> Vec<HashAlg> {
        vec![HashAlg::Md5, HashAlg::Sha256]
    }

    fn names(set: AlgoSet) -> Vec<&'static str> {
        set.iter().map(|a| a.as_str()).collect()
    }

    /// A complete cache row costs no read, an incomplete one costs only what is
    /// missing.
    #[test]
    fn one_side_plans_only_what_the_cache_lacks() {
        let p = HashPlan::plan_one_side(&entries(), &algos(), false);
        assert_eq!(names(p.get("cached.txt")), vec!["sha256"]);
        assert_eq!(names(p.get("stale.txt")), vec!["md5", "sha256"]);
        assert!(p.is_settled("sub"), "a dir never needs a read");
    }

    /// A row with nothing cached is indistinguishable, to this predicate, from
    /// a row whose stat moved — which is correct, because phase A carries no
    /// digests for a stat-changed path. Both need everything.
    #[test]
    fn one_side_ignores_digests_phase_a_declined_to_carry() {
        let mut e = entries();
        let stale = e.get("cached.txt").unwrap().clone();
        e.insert("cached.txt".to_string(), stale);
        let p = HashPlan::plan_one_side(&e, &algos(), false);
        assert_eq!(names(p.get("cached.txt")), vec!["sha256"]);
    }

    /// A distrusting run must pay for the read even against a complete cache —
    /// that is the flag's entire purpose.
    #[test]
    fn no_trust_plans_every_algorithm_regardless_of_the_cache() {
        let p = HashPlan::plan_one_side(&entries(), &algos(), true);
        assert_eq!(names(p.get("cached.txt")), vec!["md5", "sha256"]);
        assert_eq!(names(p.get("stale.txt")), vec!["md5", "sha256"]);
    }

    /// **The plan's entries are canonical, not the flag order.** With the two
    /// flags the other way round the plan still names md5 first, because
    /// `AlgoSet` iterates `HashAlg::ALL`. Everything downstream reads this through
    /// `AlgoSet::iter` and the `--why` tag is built from it, so a plan that
    /// inherited the flag order would quietly put that tag's behaviour back.
    #[test]
    fn the_plan_names_algorithms_in_canonical_order_not_the_flag_order() {
        let reversed = vec![HashAlg::Sha256, HashAlg::Md5];
        let p = HashPlan::plan_one_side(&entries(), &reversed, false);
        assert_eq!(names(p.get("stale.txt")), vec!["md5", "sha256"]);
    }

    /// A row carrying an algorithm this build has never heard of is neither
    /// refused nor counted as covering anything. `AlgoSet` has no slot for a name
    /// it does not know, so the key is dropped — and it could not have been in the
    /// request either, so the answer is still exactly "requested minus held".
    #[test]
    fn a_row_with_an_unknown_algorithm_still_plans_exactly_what_it_lacks() {
        let mut e = entries();
        e.insert("odd.txt".to_string(), entry(&["blake3"]));
        let p = HashPlan::plan_one_side(&e, &algos(), false);
        assert_eq!(names(p.get("odd.txt")), vec!["md5", "sha256"]);
    }

    /// `--hash none` is a distinct mode, not all-of over an empty set that
    /// happens to succeed: no path may appear in the plan at all, so phase C
    /// never runs and no file is opened.
    #[test]
    fn no_algorithms_plans_nothing() {
        let p = HashPlan::plan_one_side(&entries(), &[], false);
        assert!(p.by_rel.is_empty());
        assert_eq!(p.digest_count(), 0);
    }

    /// An empty plan is the common lazy case — everything already decided — so its
    /// accessors have to be right when nothing is pending.
    #[test]
    fn an_empty_plan_settles_everything() {
        let p = HashPlan::default();
        assert!(p.is_settled("anything"));
        assert!(p.pending().is_empty());
        assert_eq!(p.digest_count(), 0);
    }

    #[test]
    fn pending_is_sorted_and_excludes_settled_paths() {
        let mut p = HashPlan::default();
        p.by_rel
            .insert("z.txt".to_string(), AlgoSet::one(HashAlg::Md5));
        p.by_rel
            .insert("a.txt".to_string(), AlgoSet::one(HashAlg::Md5));
        p.by_rel.insert("done.txt".to_string(), AlgoSet::NONE);
        assert_eq!(p.pending(), vec!["a.txt", "z.txt"]);
        assert!(p.is_settled("done.txt"));
        assert_eq!(p.digest_count(), 2);
    }
}
