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

use std::collections::HashMap;

use crate::effective::SideEntry;

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

    /// The algorithms this side still has to compute for `entry`, in `algos`
    /// order.
    ///
    /// `no_trust` overrides the subtraction: distrusting the cache is a refusal
    /// to *reuse* a digest, never an instruction to forget it — see
    /// [`crate::effective::merge_row`], which stores the same row either way —
    /// so the plan asks again while phase A keeps reporting the old value.
    fn missing(entry: &SideEntry, algos: &[String], no_trust: bool) -> Vec<String> {
        algos
            .iter()
            .filter(|a| no_trust || !entry.cached.contains_key(*a))
            .cloned()
            .collect()
    }

    /// The plan for a side with no counterpart, and the one `update` uses: every
    /// requested algorithm, for every file, minus what the cache already holds.
    ///
    /// There is no pairing step, so every file is a candidate. That is the right
    /// answer for a command with no other side to be lazy *relative to*, and the
    /// wrong one for a comparison — see [`plan_pairs`].
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
            let want = Self::missing(e, algos, no_trust);
            if !want.is_empty() {
                by_rel.insert(rel.clone(), want);
            }
        }
        Self { by_rel }
    }
}

/// One side's inputs to a pair plan.
#[derive(Clone, Copy)]
pub struct SideRequest<'a> {
    /// Phase A's output for this side.
    pub entries: &'a HashMap<String, SideEntry>,
    /// The algorithms the run asked for.
    pub algos: &'a [String],
    /// `--no-trust-cached-hashes <side>`.
    pub no_trust: bool,
}

/// Both sides' plans, from one decision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PairPlan {
    pub src: HashPlan,
    pub dst: HashPlan,
}

/// The paths both sides hold, paired the way [`crate::diff::diff_maps`] pairs
/// them: by exact key when case-sensitive, by lowercase when not.
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
    case_sensitive: bool,
) -> Vec<(&'a String, &'a String)> {
    let mut out: Vec<(&String, &String)> = Vec::new();
    if case_sensitive {
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
    } else {
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
    out.sort();
    out
}

/// Decide what a two-sided run must compute, from both sides at once.
///
/// This is where laziness comes from, and it is a *pair* decision, not a
/// per-side one. A single side cannot know that its counterpart exists, or that
/// their sizes already differ, so it can only answer "every file, every
/// algorithm" — which is why the eager path read whole trees to conclude things
/// size had already decided. A path needs a digest only when it is a file on
/// **both** sides with equal size and equal mtime; everything else is already
/// decided:
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
///   empty set that happens to pass.
/// - `--no-trust-cached-hashes` is a **no-op for a stat-differing pair**, and
///   correctly so: distrusting the cache does not make an unequal size uncertain.
///
/// Note there is no coverage check here. A side that cannot produce a digest it
/// was asked for — a record missing an algorithm — is not an error yet; it
/// degrades to size+mtime, as it always has. Making that fatal is the all-of
/// rule, and it arrives with the flag that names it, not before.
pub fn plan_pairs(src: SideRequest<'_>, dst: SideRequest<'_>, case_sensitive: bool) -> PairPlan {
    let mut plan = PairPlan::default();
    if src.algos.is_empty() {
        return plan; // `--hash none`: nothing to ask for, so nothing to read
    }
    for (srel, drel) in shared_pairs(src.entries, dst.entries, case_sensitive) {
        let s = &src.entries[srel];
        let d = &dst.entries[drel];
        // Dirs compare by presence, and a kind conflict is decided by kind.
        if !s.is_file() || !d.is_file() {
            continue;
        }
        // The short circuit. Size or mtime differing decides the pair outright,
        // and `diff_maps` would short-circuit the hash comparison anyway — the
        // bytes just used to be read long before that.
        if s.size != d.size || s.mtime_ns != d.mtime_ns {
            continue;
        }
        let want_s = HashPlan::missing(s, src.algos, src.no_trust);
        let want_d = HashPlan::missing(d, dst.algos, dst.no_trust);
        if !want_s.is_empty() {
            plan.src.by_rel.insert((*srel).clone(), want_s);
        }
        if !want_d.is_empty() {
            plan.dst.by_rel.insert((*drel).clone(), want_d);
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

    // -- pair planning -----------------------------------------------------
    //
    // The predicate, in isolation. Each path state from the table gets one case,
    // because the whole claim of W2 is that the five states divide cleanly into
    // "needs a digest" and "does not".

    fn file(size: u64, mtime: i64) -> SideEntry {
        SideEntry {
            kind: "file".into(),
            size,
            mtime_ns: mtime,
            cached: HashMap::new(),
            fresh: false,
        }
    }

    fn map(entries: Vec<(&str, SideEntry)>) -> HashMap<String, SideEntry> {
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect()
    }

    fn md5() -> Vec<String> {
        vec!["md5".to_string()]
    }

    fn plan(
        s: &HashMap<String, SideEntry>,
        d: &HashMap<String, SideEntry>,
        sensitive: bool,
        no_trust: bool,
    ) -> PairPlan {
        plan_pairs(
            SideRequest {
                entries: s,
                algos: &md5(),
                no_trust,
            },
            SideRequest {
                entries: d,
                algos: &md5(),
                no_trust,
            },
            sensitive,
        )
    }

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
        let p = plan_pairs(
            SideRequest {
                entries: &s,
                algos: &[],
                no_trust: false,
            },
            SideRequest {
                entries: &d,
                algos: &[],
                no_trust: false,
            },
            true,
        );
        assert!(p.src.by_rel.is_empty() && p.dst.by_rel.is_empty());
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

    fn cached_entry(cached: &[&str]) -> SideEntry {
        SideEntry {
            kind: "file".into(),
            size: 10,
            mtime_ns: 100,
            cached: cached
                .iter()
                .map(|a| (a.to_string(), vec![1u8; 16]))
                .collect(),
            fresh: true,
        }
    }
}
