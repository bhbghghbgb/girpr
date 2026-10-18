//! The two-sided decision: [`plan_pairs`].
//!
//! This is where laziness comes from, and it is a *pair* decision rather than a
//! per-side one. A single side cannot know that its counterpart exists, or that
//! their sizes already differ, so it can only answer "every file, every algorithm".
//! Everything here exists to narrow that to the pairs that genuinely need a digest,
//! and to refuse the run when a pair cannot be answered at all.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;

use crate::effective::{SideCapability, SideEntry};
use crate::hash::{AlgoSet, HashAlg};

use super::HashMode;
use super::coverage::Coverage;
use super::plan::{HashPlan, plan_one};
use super::stat_trust::StatTrust;

/// One side's inputs to a pair plan.
#[derive(Clone, Copy)]
pub struct SideRequest<'a> {
    /// Phase A's output for this side.
    pub entries: &'a HashMap<String, SideEntry>,
    /// The algorithms the run asked for, **in the order the user typed them**.
    pub algos: &'a [HashAlg],
    /// `--no-trust-cached-hashes <side>`.
    pub no_trust: bool,
    /// What this side is able to do.
    ///
    /// Needed because a side's *availability* is `cached` plus `hashable`, and
    /// `can_hash_from_disk` is the whole of what a record lacks. Without it the
    /// planner cannot tell "already has it" from "could go and get it", which is
    /// precisely the difference between backfilling a gap and being unable to fill
    /// one.
    pub cap: SideCapability,
    /// How to name this side when a coverage error has to point at it — a record
    /// or folder path, ideally saying which it is. The planner knows only what a
    /// side *can do*, not what it *is*, so this is the one chance a caller has to
    /// put its own vocabulary into the message.
    pub label: &'a str,
}

/// Both sides' plans, from one decision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PairPlan {
    pub src: HashPlan,
    pub dst: HashPlan,
    /// Which algorithms settle each pair — see [`crate::planner::Required`].
    pub required: crate::planner::Required,
    /// Which stat fields settled a pair, as this run decided it.
    ///
    /// Carried out of the planner so the diff reads the *same* value rather than
    /// taking the flags a second time. The planner's short circuit and the diff's are
    /// the same rule, and reading it twice is how they drift — silently, and in both
    /// directions: a digest computed that nobody consults, or a pair the planner
    /// called settled that needed a digest nobody computed.
    pub stat: StatTrust,
}

/// How two sides are paired into the paths they both hold.
///
/// The same rule [`crate::diff::diff_maps`] pairs by, and a **value rather than a
/// `bool`** so the two rules a run applies are each named at the call site. They
/// are not always the same, and the case where they differ is the one worth being
/// careful about:
///
/// | pairing | means | used by |
/// | --- | --- | --- |
/// | [`Exact`](Self::Exact) | `a.txt` and `A.txt` are two different paths | a diff over paths that already line up |
/// | [`Lowercase`](Self::Lowercase) | `a.txt` and `A.txt` are one path, spelled two ways | pairing what is on disk, before any rename |
///
/// `sync` plans with [`Lowercase`](Self::Lowercase) in insensitive mode and then
/// diffs with [`Exact`](Self::Exact), because the rename pass has re-keyed dst
/// onto src's spelling by then. Read as two bare booleans that is a pair of calls
/// that must disagree and nothing says so; read as two values it is a sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CasePairing {
    /// Pair by exact key — the paths must match character for character.
    Exact,
    /// Pair by lowercase key — spelling is not part of a path's identity.
    Lowercase,
}

impl From<bool> for CasePairing {
    /// From the run's `--case-sensitive` flag: `true` pairs exactly.
    fn from(case_sensitive: bool) -> Self {
        if case_sensitive {
            Self::Exact
        } else {
            Self::Lowercase
        }
    }
}

/// The paths both sides hold, paired the way [`crate::diff::diff_maps`] pairs
/// them.
///
/// **This must not be re-derived differently.** `diff_maps` decides the verdict;
/// if the planner paired by a different rule it would compute digests for paths
/// the diff never compares and skip pairs it does. In insensitive mode a
/// `CASE-MISMATCH` is *also* compared, so a case-only pair is a real pair and
/// needs a digest like any other — reading the match as "already a difference"
/// is the easiest way to get this wrong.
fn shared_pairs<'a>(
    src: &'a HashMap<String, SideEntry>,
    dst: &'a HashMap<String, SideEntry>,
    pairing: CasePairing,
) -> Vec<(&'a String, &'a String)> {
    let mut out: Vec<(&String, &String)> = Vec::new();
    match pairing {
        CasePairing::Exact => {
            // Within one side, mixed-case collisions are already fatal (phase A), so
            // a first-wins insert cannot lose a pair.
            let mut seen: HashMap<&str, &String> = HashMap::new();
            for k in src.keys() {
                seen.insert(k.as_str(), k);
            }
            for drel in dst.keys() {
                if let Some(srel) = seen.get(drel.as_str()) {
                    out.push((srel, drel));
                }
            }
        }
        CasePairing::Lowercase => {
            let mut slow: HashMap<String, &String> = HashMap::new();
            for k in src.keys() {
                slow.insert(k.to_lowercase(), k);
            }
            for (lkey, drel) in dst.keys().map(|k| (k.to_lowercase(), k)) {
                if let Some(srel) = slow.get(&lkey) {
                    out.push((srel, drel));
                }
            }
        }
    }
    out.sort();
    out
}

/// Decide what a two-sided run must compute, from both sides at once.
///
/// A path needs a digest only when it is a file on **both** sides with equal size and
/// equal mtime; everything else is already decided:
///
/// | path state                        | verdict         | digest needed |
/// | --------------------------------- | --------------- | ------------- |
/// | src-only                          | `MISSING`       | no            |
/// | dst-only                          | `EXTRA`         | no            |
/// | kind differs                      | `TYPE-CONFLICT` | no            |
/// | file/file, size or mtime differs  | `CHANGED`       | no            |
/// | file/file, size **and** mtime equal | undecided     | **yes**       |
///
/// Two consequences worth stating, because both are deliberate:
///
/// - `--hash none` is trivially lazy: an empty `algos` means there is nothing to
///   ask for, so no file is opened. It is a distinct mode, not all-of over an
///   empty set that happens to pass — and it is also why an empty request is never
///   a coverage failure, since the early return happens before any pair is even
///   considered.
/// - `--no-trust-cached-hashes` is a **no-op for a stat-differing pair**, and
///   correctly so: distrusting the cache does not make an unequal size uncertain.
///
/// ## The coverage requirement
///
/// A side must be able to supply every requested algorithm for every undecided
/// pair, where "able" means `cached` plus `can_hash_from_disk`. A folder side can
/// always go and compute a missing digest; a record cannot, and a digest it does
/// not hold is one the pair will never be compared on.
///
/// The gap this closes: [`crate::diff::hashes_differ`] skips any algorithm either
/// side lacks, on the reasonable grounds that it has nothing to compare. So a
/// record missing `md5` would let the pair fall back to the size+mtime that had
/// already agreed, and the run would report `CHANGED = 0` — a confident answer
/// about content nobody read. A shortfall is therefore an [`Err`], not a verdict.
///
/// Three properties of the failure, each deliberate:
///
/// - **Scoped to the undecided set.** A record full of paths that exist on one
///   side only, or whose size differs, needs no coverage at all, and failing on
///   them would fail runs that are perfectly answerable. This is why the check is
///   here and not in the record loader, which sees the whole record and cannot
///   tell which of its paths are answerable.
/// - **Trust is not part of availability.** `--no-trust-cached-hashes src` on a
///   record asks it to re-read, which it cannot do, so it changes nothing it owes.
///   Folding trust in would make the flag fatal for every record.
/// - **Collected, not thrown at the first path.** Every uncovered path across both
///   sides is reported together, because a user who learns about one at a time will
///   not get to the end.
///
/// ## The short circuit, and how to switch it off
///
/// [`StatTrust`] decides which stat fields may settle a pair without a digest. Under
/// the default both are trusted, which is what makes the whole laziness story work;
/// [`StatTrust::without_size`] and [`StatTrust::without_mtime`] put a field back into the
/// undecided set, and both together leave the short circuit with nothing to act
/// on.
///
/// Coverage follows the same boundary as the short circuit, and for the same reason: a
/// pair settled by a trusted stat field needed no digest, so none is required. Distrust
/// a field and its pairs become answerable questions — which for a record side means
/// they can now fail coverage, and that is the intended consequence rather than an
/// accident of the check's placement.
///
/// ## `pairing`
///
/// [`CasePairing`] is which paths count as the same path, and it must be the same
/// answer [`crate::diff::diff_maps`] is given for the pairs this planner decides —
/// except where a rename has already made them line up. `sync` is exactly that
/// case: it plans with [`CasePairing::Lowercase`] in insensitive mode (the paths
/// are on disk under two spellings) and then diffs with [`CasePairing::Exact`],
/// because the rename pass re-keyed dst onto src's spelling in between.
///
/// ## The two modes
///
/// [`HashMode::AllOf`] requires the whole requested list on an undecided pair, which
/// is what the diff then compares. [`HashMode::AnyOf`] requires one and picks it per
/// pair, so the answer is per *path* rather than per run — that is the entire reason
/// [`crate::planner::Required`] exists instead of passing `algos` to the diff.
///
/// The availability bookkeeping below is shared by both, and deliberately so: "can
/// this side obtain algorithm `a`" has the same answer either way — it holds it, or
/// it has a filesystem to read it from. Only the *requirement* changes: every `a`, or
/// one `a`. Under `any-of`, a side with a filesystem can therefore never be the
/// short one, which is why an `any-of` coverage failure always names a record.
pub fn plan_pairs(
    src: SideRequest<'_>,
    dst: SideRequest<'_>,
    pairing: CasePairing,
    mode: HashMode,
    stat: StatTrust,
) -> Result<PairPlan> {
    // `stat` is set in the initialiser, and that is load-bearing rather than
    // tidiness: it has to happen even on the early return below, because the diff
    // reads `plan.stat` to decide its own short circuit. An unflagged default there
    // would make `--hash-all-of none --no-trust-mtime` trust mtime in the diff while
    // the user asked it not to, and the verdict would come from the field the run
    // was told to ignore.
    let mut plan = PairPlan {
        stat,
        ..PairPlan::default()
    };
    if src.algos.is_empty() {
        // `--hash none`: nothing to ask for, so nothing to read and nothing to be
        // short of.
        return Ok(plan);
    }
    plan.required.fallback = AlgoSet::from_iter(src.algos.iter().copied());
    // Per side, over the undecided set. `BTreeMap` so the error is deterministic:
    // a message that reshuffles between runs is a message nobody can read twice.
    let mut tally = [Tally::default(), Tally::default()];
    for (srel, drel) in shared_pairs(src.entries, dst.entries, pairing) {
        let s = &src.entries[srel];
        let d = &dst.entries[drel];
        // Dirs compare by presence, and a kind conflict is decided by kind.
        if !s.is_file() || !d.is_file() {
            continue;
        }
        // The short circuit. A stat field this run trusts settles the pair outright,
        // and `diff_maps` would report the same `CHANGED` without reading anything, so
        // the read would buy nothing. Coverage follows the same boundary: nothing was
        // required of these paths, so nothing is owed.
        //
        // Distrusting a field (`StatTrust`) is what puts it back here. The condition
        // is read through `StatTrust::settles` rather than inlined, so the planner
        // and the diff cannot disagree about which pairs are undecided — the one way
        // this could silently break is the planner hashing something the diff never
        // consults, or the other way round.
        if stat.settles(s.size, s.mtime_ns, d.size, d.mtime_ns) {
            continue;
        }
        // Mode-independent. See the note above.
        //
        // A shortfall here means a requested algorithm this side cannot obtain, and
        // whether that is a failure depends on the mode: under `all-of` it *is* the
        // failure, so recording every one is right. Under `any-of` it is not — the run
        // only needs one algorithm, and a record short of sha256 is perfectly
        // answerable by md5, so recording every shortfall would make any-of fatal for
        // exactly the records it exists to rescue. `any-of` therefore records nothing
        // in this loop and defers entirely to `pick_one`, which is the only place that
        // knows whether *nothing* is obtainable on both sides.
        let sides = [(0usize, &src, s, srel), (1usize, &dst, d, drel)];
        // `coverage` is always counted: it is what the error message reports per
        // algorithm, and it is the same arithmetic under both modes.
        for (idx, req, e, _) in sides {
            let t = &mut tally[idx];
            for algo in req.algos {
                let slot = t
                    .coverage
                    .entry(algo.as_str().to_string())
                    .or_insert((0, 0));
                slot.1 += 1;
                if e.cached.contains_key(algo.as_str()) {
                    slot.0 += 1;
                }
            }
        }
        // What settles this pair, and therefore what the diff will compare. Only a
        // side with a filesystem is ever asked to compute, because a side that
        // cannot hash would ignore the request and then quietly fail to deliver it
        // — which is the gap the coverage check above exists to close.
        match mode {
            HashMode::AllOf => {
                // Every algorithm this side cannot obtain is a failure, because the
                // run needs all of them.
                for (idx, req, e, rel) in sides {
                    if req.cap.can_hash_from_disk {
                        continue;
                    }
                    let short = AlgoSet::from_iter(
                        req.algos
                            .iter()
                            .copied()
                            .filter(|a| !obtainable(e, req.cap, a)),
                    );
                    if !short.is_empty() {
                        tally[idx].missing.insert((*rel).clone(), short);
                    }
                }
                let want = AlgoSet::from_iter(src.algos.iter().copied());
                plan.required.by_rel.insert((*srel).clone(), want);
                plan_one(&src, s, srel, want, &mut plan.src);
                plan_one(
                    &dst,
                    d,
                    drel,
                    AlgoSet::from_iter(dst.algos.iter().copied()),
                    &mut plan.dst,
                );
            }
            HashMode::AnyOf => {
                // `None` means no requested algorithm is obtainable on both sides.
                // Nothing is planned and no entry is written; the failure is recorded
                // against whichever side cannot hash, and reported at the bottom.
                let Some(pick) = pick_one(src.algos, s, d, src.cap, dst.cap) else {
                    // A side with a filesystem can obtain any of the requested
                    // algorithms, so if nothing qualifies then the side that cannot
                    // obtain anything is always a record — which makes this branch
                    // naming a non-hashable side, by construction rather than by a
                    // guess at which one it was.
                    for (idx, req, e, rel) in sides {
                        if req.cap.can_hash_from_disk {
                            continue;
                        }
                        let t = &mut tally[idx];
                        let short = AlgoSet::from_iter(
                            req.algos
                                .iter()
                                .copied()
                                .filter(|a| !obtainable(e, req.cap, a)),
                        );
                        if !short.is_empty() {
                            t.missing.insert((*rel).clone(), short);
                        }
                    }
                    continue;
                };
                let chosen = AlgoSet::one(pick);
                plan.required.by_rel.insert((*srel).clone(), chosen);
                plan_one(&src, s, srel, chosen, &mut plan.src);
                plan_one(&dst, d, drel, chosen, &mut plan.dst);
            }
        }
    }
    // src first, so the message leads with the side a reader is most likely to be
    // surprised by — in practice the record, because a record cannot fill a gap.
    for (idx, req) in [(0usize, &src), (1usize, &dst)] {
        if !tally[idx].missing.is_empty() {
            return Err(Coverage {
                side: ["src", "dst"][idx],
                label: req.label.to_string(),
                mode,
                missing: std::mem::take(&mut tally[idx].missing),
                coverage: std::mem::take(&mut tally[idx].coverage),
            }
            .into());
        }
    }
    Ok(plan)
}

/// The algorithm that settles one pair under [`HashMode::AnyOf`], or `None` when
/// none is obtainable on both sides.
///
/// **Cheapest first**, which is the tier that makes the flag worth having for
/// folder-vs-folder: an algorithm already on *both* rows costs no read at all,
/// while one already on one row costs a read and one on neither costs two. Without
/// this preference the mode would hash whenever it could have reused, because both
/// sides are always able to backfill — so "pick something" would be a claim about
/// nothing.
///
/// **Then the user's flag order.** `min_by_key` keeps the *first* minimum, and
/// `algos` is the flag order, so `--hash-any-of md5 --hash sha256` prefers md5 and
/// the choice is visible in the command they typed rather than in a rule they would
/// have to look up.
fn pick_one(
    algos: &[HashAlg],
    s: &SideEntry,
    d: &SideEntry,
    sc: SideCapability,
    dc: SideCapability,
) -> Option<HashAlg> {
    algos
        .iter()
        .filter(|a| obtainable(s, sc, a) && obtainable(d, dc, a))
        .min_by_key(|a| {
            usize::from(!s.cached.contains_key(a.as_str()))
                + usize::from(!d.cached.contains_key(a.as_str()))
        })
        .copied()
}

/// Whether a side can supply `algo` at all: it holds it, or it can go and read it.
///
/// **No trust term**, for the reason the coverage check has none: trust asks a side
/// to re-read rather than reuse, and a record has no filesystem to re-read. Folding
/// it in would make `--no-trust-cached-hashes src` fatal for every record.
fn obtainable(e: &SideEntry, cap: SideCapability, algo: &HashAlg) -> bool {
    e.cached.contains_key(algo.as_str()) || cap.can_hash_from_disk
}

/// What one side owes, and how much of it it could meet, over the undecided set.
#[derive(Debug, Default)]
struct Tally {
    /// Requested algorithm -> (paths that hold it, undecided paths).
    coverage: BTreeMap<String, (usize, usize)>,
    /// rel path -> requested algorithms this side cannot obtain at all.
    missing: BTreeMap<String, AlgoSet>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::fixtures::{Shape, cached_entry, dir, file, map, plan};

    // -- pair planning -------------------------------------------------------
    //
    // The predicate, in isolation. Each path state from the table gets one case,
    // because the five states divide cleanly into "needs a digest" and "does not".

    /// The one path state that costs a read. Everything else in the table is a
    /// free decision, and this test is the one that would notice if the short
    /// circuit had been widened.
    #[test]
    fn only_a_stat_equal_file_pair_is_planned() {
        let s = map(vec![
            ("equal.txt", file(10, 100)),
            ("size.txt", file(10, 100)),
            ("mtime.txt", file(10, 100)),
        ]);
        let d = map(vec![
            ("equal.txt", file(10, 100)),
            ("size.txt", file(11, 100)),
            ("mtime.txt", file(10, 101)),
        ]);
        let p = plan(&s, &d, true, false);
        assert_eq!(p.src.pending(), vec!["equal.txt"]);
        assert_eq!(p.dst.pending(), vec!["equal.txt"]);
    }

    #[test]
    fn one_sided_paths_and_kind_conflicts_are_never_planned() {
        let s = map(vec![
            ("only_src.txt", file(10, 100)),
            ("clash", file(10, 100)),
            ("dir", dir()),
        ]);
        let d = map(vec![
            ("only_dst.txt", file(10, 100)),
            ("clash", dir()),
            ("dir", dir()),
        ]);
        let p = plan(&s, &d, true, false);
        assert!(p.src.pending().is_empty());
        assert!(p.dst.pending().is_empty());
    }

    /// A dir on both sides is compared by presence alone, so it never needs a
    /// digest even though both sides "have" it.
    #[test]
    fn a_shared_dir_never_needs_a_digest() {
        let s = map(vec![("d", dir())]);
        let d = map(vec![("d", dir())]);
        assert!(plan(&s, &d, true, false).src.pending().is_empty());
    }
    /// `--hash none` is a mode, not all-of over an empty set: no path may reach
    /// either plan, so phase C never runs and no file is opened.
    #[test]
    fn an_empty_algorithm_set_plans_nothing_on_any_pair_state() {
        let s = map(vec![("equal.txt", file(10, 100))]);
        let d = map(vec![("equal.txt", file(10, 100))]);
        let p = Shape {
            hashable: (true, true),
            algos: &[],
            no_trust: (false, false),
            mode: HashMode::AllOf,
        }
        .run(&s, &d, true, StatTrust::default())
        .unwrap();
        assert!(p.src.by_rel.is_empty() && p.dst.by_rel.is_empty());
    }

    /// `--hash none` is also not a coverage failure, and that needs no special
    /// case: nothing was asked for, so nothing can be short of it. With a record
    /// on one side and a completely empty row, the run still succeeds.
    #[test]
    fn an_empty_algorithm_set_is_not_a_coverage_failure() {
        let s = map(vec![("equal.txt", file(10, 100))]);
        let d = map(vec![("equal.txt", file(10, 100))]);
        assert!(
            Shape::record_src(&[])
                .run(&s, &d, true, StatTrust::default())
                .is_ok(),
            "a stat-only audit asks for no digests and owes none"
        );
    }

    /// The short circuit applies before trust: an unequal size is not made
    /// uncertain by refusing to reuse a digest, so a stat-differing pair still
    /// costs nothing under `--no-trust-cached-hashes`.
    #[test]
    fn no_trust_does_not_widen_a_stat_differing_pair() {
        let s = map(vec![("differs.txt", file(10, 100))]);
        let d = map(vec![("differs.txt", file(11, 100))]);
        let p = plan(&s, &d, true, true);
        assert!(p.src.pending().is_empty());
        assert!(p.dst.pending().is_empty());
    }

    #[test]
    fn no_trust_does_widen_a_stat_equal_pair() {
        let s = map(vec![("equal.txt", cached_entry(&["md5"]))]);
        let d = map(vec![("equal.txt", cached_entry(&["md5"]))]);
        // Trusting: both sides already hold it.
        let p = plan(&s, &d, true, false);
        assert!(p.src.pending().is_empty() && p.dst.pending().is_empty());
        // Distrusting: both sides pay for it again.
        let p = plan(&s, &d, true, true);
        assert_eq!(p.src.pending(), vec!["equal.txt"]);
        assert_eq!(p.dst.pending(), vec!["equal.txt"]);
    }

    /// A cached digest satisfies the pair on its own; only the *missing* side
    /// pays. This is what makes a mixed-algorithm history cheap instead of
    /// forcing a full rehash of both sides.
    #[test]
    fn a_cached_digest_satisfies_only_its_own_side() {
        let s = map(vec![("a.txt", cached_entry(&["md5"]))]);
        let d = map(vec![("a.txt", file(10, 100))]);
        let p = plan(&s, &d, true, false);
        assert!(p.src.pending().is_empty());
        assert_eq!(p.dst.pending(), vec!["a.txt"]);
    }

    /// **Pairing must match `diff_maps` exactly.** In sensitive mode `a.txt` and
    /// `A.txt` are different paths, so the run is missing+extra and nothing is
    /// hashed.
    #[test]
    fn sensitive_mode_pairs_by_exact_key() {
        let s = map(vec![("a.txt", file(10, 100))]);
        let d = map(vec![("A.txt", file(10, 100))]);
        let p = plan(&s, &d, true, false);
        assert!(p.src.pending().is_empty());
        assert!(p.dst.pending().is_empty());
    }

    /// In insensitive mode they are the *same* path, so the pair is compared —
    /// and since it is stat-equal it needs a digest. A planner that read a
    /// case-only match as "already a difference" would plan nothing here, and the
    /// content disagreement would go unreported.
    #[test]
    fn insensitive_mode_pairs_by_lowercase() {
        let s = map(vec![("a.txt", file(10, 100))]);
        let d = map(vec![("A.txt", file(10, 100))]);
        let p = plan(&s, &d, false, false);
        assert_eq!(p.src.pending(), vec!["a.txt"]);
        assert_eq!(p.dst.pending(), vec!["A.txt"]);
    }

    /// And the pairing is per-key, not per-side: an insensitive run over a tree
    /// with a case-only difference *and* an ordinary stat-equal pair must plan
    /// both, and nothing else.
    #[test]
    fn insensitive_mode_pairs_each_key_independently() {
        let s = map(vec![
            ("a.txt", file(10, 100)),
            ("same.txt", file(10, 100)),
            ("differs.txt", file(10, 100)),
        ]);
        let d = map(vec![
            ("A.txt", file(10, 100)),
            ("same.txt", file(10, 100)),
            ("differs.txt", file(99, 100)),
        ]);
        let p = plan(&s, &d, false, false);
        assert_eq!(p.src.pending(), vec!["a.txt", "same.txt"]);
        assert_eq!(p.dst.pending(), vec!["A.txt", "same.txt"]);
    }
}
