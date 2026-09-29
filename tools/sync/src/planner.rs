//! Deciding which digests a run has to compute.
//!
//! This module is the seam W2 introduces. Phase A reports what a side already
//! knows, [`HashPlan`] turns that into the set of digests each side still has to
//! produce, and phase C executes it. Three properties follow from the split, and
//! each is load-bearing:
//!
//! - **Phase A never hashes.** It reports what the cache knows, nothing more.
//!   That is what makes the cache *consultable* rather than fused with disk
//!   access, and it is what lets this module pick an algorithm per path instead
//!   of being handed whatever a monolithic scan already decided to read.
//! - **The decision is made here and nowhere else.** Phase A does not widen the
//!   request, and phase C does not narrow it. `--hash-any-of` later becomes a
//!   different predicate in [`HashPlan`] rather than a new pass over the tree.
//! - **The decision is made before any read.** A plan is total over the run, so
//!   a run that cannot answer everything fails having read nothing.
//!
//! Today there is only one predicate, [`HashPlan::plan_one_side`], and it
//! deliberately reproduces the pre-W2 behaviour exactly: request every requested
//! algorithm for every file, minus whatever the cache already holds.

use std::collections::{BTreeMap, HashMap};

use crate::effective::{SideCapability, SideEntry};

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
    pub by_rel: HashMap<String, Vec<String>>,
}

impl HashPlan {
    /// The algorithms to compute for `rel`; empty when the path needs no read.
    pub fn get(&self, rel: &str) -> &[String] {
        self.by_rel.get(rel).map(|v| &v[..]).unwrap_or(&[])
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

    /// The plan for a side with no counterpart, and the one `update` uses: every
    /// requested algorithm, for every file, minus what the cache already holds.
    ///
    /// `no_trust` overrides the subtraction. Distrusting the cache is a refusal
    /// to *reuse* a digest, never an instruction to forget it — see
    /// [`crate::effective::merge_row`], which stores the same row either way —
    /// so the plan asks again while phase A keeps reporting the old value.
    ///
    /// `algos` order is preserved, because it is the user's flag order and the
    /// tie-break `--hash-any-of` will use is defined in terms of it.
    pub fn plan_one_side(
        entries: &HashMap<String, SideEntry>,
        algos: &[String],
        no_trust: bool,
    ) -> Self {
        let mut by_rel = HashMap::new();
        for (rel, e) in entries {
            if !e.is_file() || algos.is_empty() {
                continue;
            }
            let want: Vec<String> = algos
                .iter()
                .filter(|a| no_trust || !e.cached.contains_key(*a))
                .cloned()
                .collect();
            if !want.is_empty() {
                by_rel.insert(rel.clone(), want);
            }
        }
        Self { by_rel }
    }
}

/// One side's inputs to the two-sided planner.
#[derive(Debug, Clone, Copy)]
pub struct SideSpec<'a> {
    /// Phase A's output for this side.
    pub entries: &'a HashMap<String, SideEntry>,
    /// What this side is able to do.
    pub cap: SideCapability,
    /// `--no-trust-cached-hashes` for this side.
    pub no_trust_cached_hashes: bool,
}

/// One plan per side of a comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PairPlan {
    pub src: HashPlan,
    pub dst: HashPlan,
}

impl PairPlan {
    /// Total digests the pair will compute.
    pub fn digest_count(&self) -> usize {
        self.src.digest_count() + self.dst.digest_count()
    }
}

/// The (src, dst) path correspondences `diff_maps` will compare, sorted.
///
/// **This must mirror `diff_maps`'s matching exactly.** In insensitive mode the
/// two sides are paired by lowercase, so `src/a.txt` faces `dst/A.txt`; a
/// planner that paired them differently would hash the wrong files and leave the
/// genuinely undecided ones unexamined — and the mistake is invisible on a tree
/// with no case differences, which is most trees. Hence a test on this function
/// rather than only an end-to-end one.
///
/// Matching by exact key in sensitive mode is the same rule with the lowercase
/// step dropped.
pub fn pair_keys<'a>(
    src: &'a HashMap<String, SideEntry>,
    dst: &'a HashMap<String, SideEntry>,
    case_sensitive: bool,
) -> Vec<(&'a str, &'a str)> {
    let mut out: Vec<(&str, &str)> = Vec::new();
    if case_sensitive {
        for srel in src.keys() {
            if dst.contains_key(srel) {
                out.push((srel.as_str(), srel.as_str()));
            }
        }
    } else {
        // Both sides are already known to be collision-free by case: the folder
        // side checks the live walk (`check_mixed_case`) and the record side
        // checks its rows (`load_record_side_from`). So the lowercasing is a
        // bijection and one dst key cannot be claimed by two src keys.
        let dlow: BTreeMap<String, &str> = dst
            .keys()
            .map(|drel| (drel.to_lowercase(), drel.as_str()))
            .collect();
        for srel in src.keys() {
            if let Some(&drel) = dlow.get(&srel.to_lowercase()) {
                out.push((srel.as_str(), drel));
            }
        }
    }
    out.sort();
    out
}

/// True when a pair's verdict is not yet decided by stat alone, so a digest is
/// needed to settle it.
///
/// The complement is the whole lazy win, and it is a table rather than a
/// heuristic:
///
/// | path state | verdict | digest needed |
/// | --- | --- | --- |
/// | src-only | `MISSING` | no |
/// | dst-only | `EXTRA` | no |
/// | kind differs | `TYPE-CONFLICT` | no |
/// | file/file, size or mtime differs | `CHANGED` | no |
/// | file/file, size **and** mtime equal | **undecided** | **yes** |
///
/// Size and mtime differing means `diff_maps` short-circuits and never reaches
/// `hashes_differ` at all, so the bytes behind those files are never consulted.
/// A dir is compared by presence alone, so it never needs one either.
pub fn undecided(s: &SideEntry, d: &SideEntry) -> bool {
    s.is_file() && d.is_file() && s.size == d.size && s.mtime_ns == d.mtime_ns
}

/// The algorithms one side must compute for an undecided pair.
///
/// A side that cannot read files asks for nothing: a record has no filesystem to
/// hash, so whatever it holds is all it will ever have. That a record short of
/// coverage then degrades to size+mtime is today's behaviour and stays it until
/// the all-of rule lands; making it fatal here instead would fail a run on the
/// first path it happened to be short of, which is the reverted attempt's
/// mistake.
fn required(e: &SideEntry, algos: &[String], side: &SideSpec<'_>) -> Vec<String> {
    if !side.cap.can_hash_from_disk {
        return Vec::new();
    }
    if side.no_trust_cached_hashes {
        return algos.to_vec();
    }
    algos
        .iter()
        .filter(|a| !e.cached.contains_key(*a))
        .cloned()
        .collect()
}

/// Plan a two-sided comparison: digests only for the undecided pairs.
///
/// This is where laziness lives. Every other path state is already decided by
/// stat, and the diff would short-circuit before looking at a hash, so a side
/// reads a file only when the pair it belongs to could still go either way.
///
/// It runs to completion before either side resolves, and it is pure: the same
/// entries and options give the same plan every time, with no filesystem access.
/// That is what makes an unanswerable run able to fail having read nothing, once
/// the coverage check arrives.
pub fn plan_pairs(
    src: SideSpec<'_>,
    dst: SideSpec<'_>,
    case_sensitive: bool,
    algos: &[String],
) -> PairPlan {
    let mut plan = PairPlan::default();
    if algos.is_empty() {
        // `--hash none` is a distinct mode, not all-of over an empty set: no path
        // is undecided because no path is waiting on a digest.
        return plan;
    }
    for (srel, drel) in pair_keys(src.entries, dst.entries, case_sensitive) {
        if !undecided(&src.entries[srel], &dst.entries[drel]) {
            continue;
        }
        let want_src = required(&src.entries[srel], algos, &src);
        if !want_src.is_empty() {
            plan.src.by_rel.insert(srel.to_string(), want_src);
        }
        let want_dst = required(&dst.entries[drel], algos, &dst);
        if !want_dst.is_empty() {
            plan.dst.by_rel.insert(drel.to_string(), want_dst);
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(cached: &[&str]) -> SideEntry {
        SideEntry {
            kind: "file".into(),
            size: 10,
            mtime_ns: 99,
            cached: cached
                .iter()
                .map(|a| (a.to_string(), vec![1u8; 16]))
                .collect(),
            fresh: !cached.is_empty(),
        }
    }

    fn dir() -> SideEntry {
        SideEntry {
            kind: "dir".into(),
            size: 0,
            mtime_ns: 0,
            cached: HashMap::new(),
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

    fn algos() -> Vec<String> {
        vec!["md5".to_string(), "sha256".to_string()]
    }

    /// The predicate's whole job, before W2 adds laziness to it: a complete
    /// cache row costs no read, an incomplete one costs only what is missing.
    #[test]
    fn one_side_plans_only_what_the_cache_lacks() {
        let p = HashPlan::plan_one_side(&entries(), &algos(), false);
        assert_eq!(p.get("cached.txt"), &["sha256".to_string()]);
        assert_eq!(
            p.get("stale.txt"),
            &["md5".to_string(), "sha256".to_string()]
        );
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
        assert_eq!(p.get("cached.txt"), &["sha256".to_string()]);
    }

    /// A distrusting run must pay for the read even against a complete cache —
    /// that is the flag's entire purpose.
    #[test]
    fn no_trust_plans_every_algorithm_regardless_of_the_cache() {
        let p = HashPlan::plan_one_side(&entries(), &algos(), true);
        assert_eq!(
            p.get("cached.txt"),
            &["md5".to_string(), "sha256".to_string()]
        );
        assert_eq!(
            p.get("stale.txt"),
            &["md5".to_string(), "sha256".to_string()]
        );
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

    /// An empty plan is the shape stage 3 will produce for the common lazy case,
    /// so its accessors have to be right when nothing is pending.
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
            .insert("z.txt".to_string(), vec!["md5".to_string()]);
        p.by_rel
            .insert("a.txt".to_string(), vec!["md5".to_string()]);
        p.by_rel.insert("done.txt".to_string(), vec![]);
        assert_eq!(p.pending(), vec!["a.txt", "z.txt"]);
        assert!(p.is_settled("done.txt"));
        assert_eq!(p.digest_count(), 2);
    }

    // -- pair_keys ----------------------------------------------------------
    //
    // `pair_keys` has to agree with `diff_maps` exactly. A planner that paired
    // them differently would hash the wrong files and leave the undecided ones
    // unexamined, and the mistake is invisible on a tree with no case
    // differences — which is most trees. Hence tests on the function itself
    // rather than only an end-to-end one.

    fn side(rels: &[&str]) -> HashMap<String, SideEntry> {
        rels.iter()
            .map(|r| ((*r).to_string(), entry(&["md5"])))
            .collect()
    }

    #[test]
    fn sensitive_mode_pairs_by_exact_key() {
        let src = side(&["a.txt", "B.txt"]);
        let dst = side(&["a.txt", "b.txt"]);
        assert_eq!(
            pair_keys(&src, &dst, true),
            vec![("a.txt", "a.txt")],
            "only the exactly-spelled path pairs"
        );
    }

    #[test]
    fn insensitive_mode_pairs_by_lowercase() {
        let src = side(&["Data/Game.TXT", "b.txt"]);
        let dst = side(&["data/game.txt", "B.TXT"]);
        assert_eq!(
            pair_keys(&src, &dst, false),
            vec![("Data/Game.TXT", "data/game.txt"), ("b.txt", "B.TXT")]
        );
    }

    #[test]
    fn pairing_ignores_paths_present_on_one_side_only() {
        let src = side(&["shared.txt", "onlysrc.txt"]);
        let dst = side(&["shared.txt", "onlydst.txt"]);
        for case_sensitive in [true, false] {
            assert_eq!(
                pair_keys(&src, &dst, case_sensitive),
                vec![("shared.txt", "shared.txt")]
            );
        }
    }

    #[test]
    fn pairing_covers_a_nested_case_difference() {
        // The lowercase key includes the separators, so a path whose directory
        // casing differs still matches its counterpart.
        let src = side(&["Sub/Deep/File.bin"]);
        let dst = side(&["sub/deep/file.BIN"]);
        assert_eq!(
            pair_keys(&src, &dst, false),
            vec![("Sub/Deep/File.bin", "sub/deep/file.BIN")]
        );
    }

    // -- the undecided predicate -------------------------------------------

    fn at(size: u64, mtime: i64) -> SideEntry {
        SideEntry {
            size,
            mtime_ns: mtime,
            ..entry(&["md5"])
        }
    }

    #[test]
    fn only_a_stat_equal_file_pair_is_undecided() {
        assert!(undecided(&at(10, 99), &at(10, 99)), "size and mtime agree");
        assert!(!undecided(&at(10, 99), &at(11, 99)), "size differs");
        assert!(!undecided(&at(10, 99), &at(10, 100)), "mtime differs");
    }

    /// A dir is compared by presence alone, so it never needs a digest — and a
    /// file against a dir is a type conflict, already decided.
    #[test]
    fn a_dir_never_needs_a_digest() {
        assert!(!undecided(&dir(), &dir()));
        assert!(!undecided(&at(10, 99), &dir()));
        assert!(!undecided(&dir(), &at(10, 99)));
    }

    #[test]
    fn a_zero_byte_stat_equal_pair_is_undecided() {
        // The degenerate case a "non-empty and same size" shortcut would get wrong.
        assert!(undecided(&at(0, 0), &at(0, 0)));
    }

    // -- plan_pairs ---------------------------------------------------------

    fn spec<'a>(
        entries: &'a HashMap<String, SideEntry>,
        cap: SideCapability,
        no_trust: bool,
    ) -> SideSpec<'a> {
        SideSpec {
            entries,
            cap,
            no_trust_cached_hashes: no_trust,
        }
    }

    fn folder() -> SideCapability {
        SideCapability {
            can_hash_from_disk: true,
            can_write_cache: true,
        }
    }

    /// Both algorithms cached and fresh: nothing to plan.
    fn complete() -> HashMap<String, SideEntry> {
        let mut m = HashMap::new();
        m.insert(
            "a.txt".to_string(),
            SideEntry {
                cached: [
                    ("md5".to_string(), vec![1u8; 16]),
                    ("sha256".to_string(), vec![2u8; 32]),
                ]
                .into_iter()
                .collect(),
                ..entry(&[])
            },
        );
        m
    }

    /// The headline: a stat-differing pair is planned for nothing at all.
    #[test]
    fn a_stat_differing_pair_is_planned_for_nothing() {
        let src = side(&["bigger.txt"]);
        let mut dst = side(&["bigger.txt"]);
        dst.get_mut("bigger.txt").unwrap().size = 11;
        let plan = plan_pairs(
            spec(&src, folder(), false),
            spec(&dst, folder(), false),
            true,
            &algos(),
        );
        assert_eq!(plan.digest_count(), 0);
    }

    #[test]
    fn an_undecided_pair_is_planned_on_both_sides() {
        let src = side(&["a.txt"]);
        let dst = side(&["a.txt"]);
        let plan = plan_pairs(
            spec(&src, folder(), false),
            spec(&dst, folder(), false),
            true,
            &algos(),
        );
        assert_eq!(plan.src.get("a.txt"), &["sha256".to_string()]);
        assert_eq!(plan.dst.get("a.txt"), &["sha256".to_string()]);
    }

    /// A digest cached on both sides costs nothing — the tier that makes the
    /// cache worth consulting even though backfill would be available.
    #[test]
    fn a_digest_cached_on_both_sides_is_planned_for_nothing() {
        let full = complete();
        let plan = plan_pairs(
            spec(&full, folder(), false),
            spec(&full, folder(), false),
            true,
            &algos(),
        );
        assert_eq!(plan.digest_count(), 0);
    }

    /// A record side asks for nothing, whatever it is short of: it has no
    /// filesystem to hash, so a request would be unsatisfiable.
    #[test]
    fn a_record_side_never_asks_for_a_digest() {
        let src = side(&["a.txt"]);
        let dst = side(&["a.txt"]);
        let plan = plan_pairs(
            spec(&src, folder(), false),
            spec(&dst, SideCapability::record(), false),
            true,
            &algos(),
        );
        assert_eq!(plan.src.get("a.txt"), &["sha256".to_string()]);
        assert!(
            plan.dst.by_rel.is_empty(),
            "a record cannot read, so it must not be asked to"
        );
    }

    /// Trust widens the request even against a complete cache — the flag's whole
    /// purpose — and still cannot make a record read.
    #[test]
    fn no_trust_asks_for_everything_and_still_cannot_make_a_record_read() {
        let full = complete();
        let other = side(&["a.txt"]);
        let plan = plan_pairs(
            spec(&full, folder(), true),
            spec(&other, SideCapability::record(), true),
            true,
            &algos(),
        );
        assert_eq!(
            plan.src.get("a.txt"),
            &["md5".to_string(), "sha256".to_string()]
        );
        assert!(plan.dst.by_rel.is_empty());
    }

    #[test]
    fn hash_none_plans_nothing_for_any_tree() {
        let src = side(&["a.txt", "b.txt"]);
        let dst = side(&["a.txt", "b.txt"]);
        let plan = plan_pairs(
            spec(&src, folder(), false),
            spec(&dst, folder(), false),
            true,
            &[],
        );
        assert_eq!(plan.digest_count(), 0);
    }

    /// Each side is planned under *its own* spelling, which differs from the
    /// other's only in insensitive mode. Keying both by one spelling would make
    /// the resolve phase look up a path that side does not have.
    #[test]
    fn each_side_is_planned_under_its_own_spelling() {
        let src = side(&["File.TXT"]);
        let dst = side(&["file.txt"]);
        let plan = plan_pairs(
            spec(&src, folder(), false),
            spec(&dst, folder(), false),
            false,
            &algos(),
        );
        assert_eq!(plan.src.get("File.TXT"), &["sha256".to_string()]);
        assert_eq!(plan.dst.get("file.txt"), &["sha256".to_string()]);
    }
}
