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
}
