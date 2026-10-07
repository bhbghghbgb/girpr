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
//! There are two predicates, and they answer different questions. One side with no
//! counterpart is [`HashPlan::plan_one_side`] — every requested algorithm for every
//! file, which is `update` and the pre-W2 request it reproduces exactly. Two sides
//! are [`plan_pairs`], which is [`plan_one_side`] plus a pairing step and a coverage
//! requirement.
//!
//! ## Two questions, kept apart
//!
//! [`HashPlan`] answers *what must be computed* and honours trust. Coverage
//! answers *whether the run can be answered at all*, and it does **not**: a side's
//! availability is `cached ∪ hashable`, with no trust term. Both halves matter,
//! because conflating them produces the lenient default W2 exists to remove:
//!
//! | question | honours `no_trust` | who can fail |
//! | --- | --- | --- |
//! | what must be computed | **yes** — it means "re-read rather than reuse" | nobody; a side that cannot re-read is simply never asked |
//! | can the pair be answered | **no** — a record has nothing to distrust | a side that cannot produce a digest from disk, i.e. a record |
//!
//! Folding trust into coverage would make `--no-trust-cached-hashes src` fatal for
//! every record, including ones holding exactly the right digests — and a record is
//! not a cache with a stale entry, it is a fixed body of data with no filesystem
//! behind it.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use anyhow::Result;

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
    /// ## Why this needs no coverage check
    ///
    /// `plan_pairs` fails a side that cannot supply a requested digest, because a
    /// comparison cannot answer the question without it. `update` asks a folder for
    /// everything it has, and a folder can always go and compute any digest of any
    /// file it holds — so its own availability was never in question, and it needs
    /// no exemption from the rule. That is the same reason a record fails all-of
    /// *by construction* rather than by a special case: the predicate is "can this
    /// side obtain it", and one side always can and the other never can.
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
/// That gap used to be silent, and the silence was the bug:
/// [`crate::diff::hashes_differ`] skips any algorithm either side lacks, on the
/// reasonable grounds that it has nothing to compare. So a record missing `md5`
/// made the pair fall back to the size+mtime that had already agreed, and the run
/// reported `CHANGED = 0` — a confident answer about content nobody read. Now it
/// is [`Err`].
///
/// Three properties of the failure, each deliberate:
///
/// - **Scoped to the undecided set.** A record full of paths that exist on one
///   side only, or whose size differs, needs no coverage at all, and failing on
///   them would fail runs that are perfectly answerable. This is why the check is
///   here and not in the record loader — a whole-record preflight was written and
///   reverted once for exactly that reason.
/// - **Trust is not part of availability.** `--no-trust-cached-hashes src` on a
///   record asks it to re-read, which it cannot do, so it changes nothing it owes.
///   Folding trust in would make the flag fatal for every record.
/// - **Collected, not thrown at the first path.** Every uncovered path across both
///   sides is reported together, because a user who learns about one at a time will
///   not get to the end.
pub fn plan_pairs(
    src: SideRequest<'_>,
    dst: SideRequest<'_>,
    case_sensitive: bool,
) -> Result<PairPlan> {
    let mut plan = PairPlan::default();
    if src.algos.is_empty() {
        // `--hash none`: nothing to ask for, so nothing to read and nothing to be
        // short of.
        return Ok(plan);
    }
    // Per side, over the undecided set. `BTreeMap` so the error is deterministic:
    // a message that reshuffles between runs is a message nobody can read twice.
    let mut tally = [Tally::default(), Tally::default()];
    for (srel, drel) in shared_pairs(src.entries, dst.entries, case_sensitive) {
        let s = &src.entries[srel];
        let d = &dst.entries[drel];
        // Dirs compare by presence, and a kind conflict is decided by kind.
        if !s.is_file() || !d.is_file() {
            continue;
        }
        // The short circuit. Size or mtime differing decides the pair outright,
        // and `diff_maps` would short-circuit the hash comparison anyway — the
        // bytes just used to be read long before that. Coverage follows the same
        // boundary: nothing was required of these paths, so nothing is owed.
        if s.size != d.size || s.mtime_ns != d.mtime_ns {
            continue;
        }
        let sides = [(0usize, &src, s, srel), (1usize, &dst, d, drel)];
        for (idx, req, e, rel) in sides {
            let t = &mut tally[idx];
            for algo in req.algos {
                let slot = t.coverage.entry(algo.clone()).or_insert((0, 0));
                slot.1 += 1;
                if e.cached.contains_key(algo) {
                    slot.0 += 1;
                } else if !req.cap.can_hash_from_disk {
                    // Nothing on this side can produce it: the row lacks it and
                    // there is no filesystem to read.
                    t.missing
                        .entry((*rel).clone())
                        .or_default()
                        .push(algo.clone());
                }
            }
        }
        // What to compute. Only a side with a filesystem is ever asked, because a
        // side that cannot hash would ignore the request and then quietly fail to
        // deliver it — which is the gap the coverage check above exists to close.
        if src.cap.can_hash_from_disk {
            let want = HashPlan::missing(s, src.algos, src.no_trust);
            if !want.is_empty() {
                plan.src.by_rel.insert((*srel).clone(), want);
            }
        }
        if dst.cap.can_hash_from_disk {
            let want = HashPlan::missing(d, dst.algos, dst.no_trust);
            if !want.is_empty() {
                plan.dst.by_rel.insert((*drel).clone(), want);
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
                missing: std::mem::take(&mut tally[idx].missing),
                coverage: std::mem::take(&mut tally[idx].coverage),
            }
            .into());
        }
    }
    Ok(plan)
}

/// What one side owes, and how much of it it could meet, over the undecided set.
#[derive(Debug, Default)]
struct Tally {
    /// Requested algorithm -> (paths that hold it, undecided paths).
    coverage: BTreeMap<String, (usize, usize)>,
    /// rel path -> requested algorithms this side cannot obtain at all.
    missing: BTreeMap<String, Vec<String>>,
}

/// A side that could not supply every digest the run required.
///
/// The run cannot answer its own question, so this is an error rather than a
/// degraded verdict. [`fmt::Display`] carries the message because the planner is
/// the only place that holds all four facts a user needs to act on it: the
/// shortfall, its size, where it is, and which algorithms are involved.
#[derive(Debug)]
pub struct Coverage {
    /// `src` or `dst`, as the two-sided commands name them.
    pub side: &'static str,
    /// The caller's own description of that side — a record or folder path.
    ///
    /// Owned rather than borrowed because this becomes an [`anyhow::Error`],
    /// which is `'static`; the alternative would be for every caller to keep its
    /// labels alive for the life of the error.
    pub label: String,
    /// rel path -> the requested algorithms it could not obtain.
    pub missing: BTreeMap<String, Vec<String>>,
    /// Requested algorithm -> (paths holding it, undecided paths).
    pub coverage: BTreeMap<String, (usize, usize)>,
}

impl fmt::Display for Coverage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let undecided: usize = self.coverage.values().next().map_or(0, |c| c.1);
        writeln!(
            f,
            "this run cannot compare: the {} side cannot supply every digest it was asked for",
            self.side
        )?;
        writeln!(f, "\n  {}: {}", self.side, self.label)?;
        writeln!(
            f,
            "  {} of {} undecided path(s) uncovered:",
            self.missing.len(),
            undecided
        )?;
        for (rel, algos) in self.missing.iter().take(EXAMPLE_PATHS) {
            writeln!(f, "\n    {rel:<32} {}", algos.join(", "))?;
        }
        if self.missing.len() > EXAMPLE_PATHS {
            writeln!(
                f,
                "\n    ... and {} more",
                self.missing.len() - EXAMPLE_PATHS
            )?;
        }
        if undecided > 0 {
            writeln!(f, "\n  coverage over the {undecided} undecided path(s):")?;
            for (algo, (have, total)) in &self.coverage {
                writeln!(f, "    {algo:<12} {have}/{total}")?;
            }
        }
        writeln!(
            f,
            "\nEvery requested algorithm must be available on both sides of a pair that \
             size+mtime\ncannot settle. This side has no filesystem to read the missing \
             digest from, so the\npair would fall back to size+mtime and this run would \
             report a confident answer\nabout content it never read."
        )?;
        // Remedies a user can act on without reading the rest of the message.
        // `--hash any-of` is missing here because it does not exist yet; it is the
        // fourth remedy and lands with the flag in stage 7.
        let narrower = self
            .coverage
            .iter()
            .find(|(_, (have, total))| *total > 0 && have == total)
            .map(|(a, _)| a.clone());
        writeln!(f, "\nTo answer this question, one of:")?;
        if let Some(a) = narrower {
            writeln!(
                f,
                "  - narrow the request to what it already holds: --hash {a}"
            )?;
        }
        writeln!(
            f,
            "  - repopulate it, then run this again: girsync update --dir <the folder \
             this record describes>"
        )?;
        writeln!(
            f,
            "  - compare by stat only, and accept that content is unchecked: --hash none"
        )?;
        Ok(())
    }
}

impl std::error::Error for Coverage {}

/// How many uncovered paths the message names in full.
///
/// Enough to recognise the shape of the problem, few enough that the message stays
/// readable on a tree with thousands of uncovered paths — the count and the
/// per-algorithm coverage carry the rest.
const EXAMPLE_PATHS: usize = 5;

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

    /// Which side is a record, which algorithms were asked for, and the trust of
    /// each side — everything a case needs to vary. Named rather than a long
    /// argument list so a coverage case reads as "a record on the src side".
    #[derive(Clone, Copy)]
    struct Shape {
        /// (src can hash from disk, dst can)
        hashable: (bool, bool),
        algos: &'static [&'static str],
        /// (no_trust_src, no_trust_dst)
        no_trust: (bool, bool),
    }

    impl Shape {
        /// Both sides folders: nothing can be short of coverage, which is what most
        /// laziness cases need and what makes `sync` safe to share this check.
        const FOLDERS: Shape = Shape {
            hashable: (true, true),
            algos: &["md5"],
            no_trust: (false, false),
        };

        /// A record on the src side. The common coverage shape: the record cannot
        /// fill a gap and the folder can.
        fn record_src(algos: &'static [&'static str]) -> Shape {
            Shape {
                hashable: (false, true),
                algos,
                no_trust: (false, false),
            }
        }

        /// Both sides records, for the case where either may be the short one.
        fn both_records(algos: &'static [&'static str]) -> Shape {
            Shape {
                hashable: (false, false),
                algos,
                no_trust: (false, false),
            }
        }

        /// Same, with each side's distrust set independently.
        fn distrusting(mut self, src: bool, dst: bool) -> Shape {
            self.no_trust = (src, dst);
            self
        }

        fn run(
            &self,
            s: &HashMap<String, SideEntry>,
            d: &HashMap<String, SideEntry>,
            sensitive: bool,
        ) -> Result<PairPlan> {
            let cap = |ok: bool| SideCapability {
                can_hash_from_disk: ok,
                can_write_cache: ok,
            };
            let algos: Vec<String> = self.algos.iter().map(|a| a.to_string()).collect();
            plan_pairs(
                SideRequest {
                    entries: s,
                    algos: &algos,
                    no_trust: self.no_trust.0,
                    cap: cap(self.hashable.0),
                    label: "src",
                },
                SideRequest {
                    entries: d,
                    algos: &algos,
                    no_trust: self.no_trust.1,
                    cap: cap(self.hashable.1),
                    label: "dst",
                },
                sensitive,
            )
        }
    }

    /// The everyday case: both sides folders, `md5`, both trusting (or both not).
    fn plan(
        s: &HashMap<String, SideEntry>,
        d: &HashMap<String, SideEntry>,
        sensitive: bool,
        no_trust: bool,
    ) -> PairPlan {
        Shape::FOLDERS
            .distrusting(no_trust, no_trust)
            .run(s, d, sensitive)
            .unwrap()
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
        let p = Shape {
            hashable: (true, true),
            algos: &[],
            no_trust: (false, false),
        }
        .run(&s, &d, true)
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
            Shape::record_src(&[]).run(&s, &d, true).is_ok(),
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

    // -- coverage -----------------------------------------------------------
    //
    // The rule in one line: a side must hold, or be able to compute, every
    // requested algorithm for every *undecided* pair. A folder can always
    // compute, so only a record can be short - which is the property, not a
    // special case.

    /// The whole point of the change: a record missing a requested digest fails
    /// the run instead of letting the pair fall back to size+mtime.
    #[test]
    fn a_record_missing_a_requested_algorithm_fails_the_run() {
        let s = map(vec![("a.txt", cached_entry(&["sha256"]))]);
        let d = map(vec![("a.txt", cached_entry(&["md5", "sha256"]))]);
        let err = Shape::record_src(&["md5", "sha256"])
            .run(&s, &d, true)
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("src"), "names the side: {msg}");
        assert!(msg.contains("a.txt"), "names the path: {msg}");
        assert!(msg.contains("md5"), "names the algorithm: {msg}");
        assert!(msg.contains("1/1"), "reports coverage: {msg}");
    }

    /// **Scoped to the undecided set.** A record full of stat-differing or
    /// one-sided paths needs no coverage, and failing on it would fail runs that
    /// are perfectly answerable - which is why this lives here and not in the
    /// record loader, where such a preflight was written and reverted once.
    #[test]
    fn coverage_is_scoped_to_the_undecided_set() {
        // Undecided but covered: fine.
        let s = map(vec![("ok.txt", cached_entry(&["md5"]))]);
        let d = map(vec![("ok.txt", cached_entry(&["md5"]))]);
        assert!(Shape::record_src(&["md5"]).run(&s, &d, true).is_ok());

        // Stat differs: already decided, so an empty record row costs nothing.
        let s = map(vec![("d.txt", file(10, 100))]);
        let d = map(vec![("d.txt", file(11, 100))]);
        assert!(Shape::record_src(&["md5"]).run(&s, &d, true).is_ok());

        // Record-only path: a MISSING verdict, decided by presence.
        let s = map(vec![("only_rec.txt", file(10, 100))]);
        let d = map(vec![]);
        assert!(Shape::record_src(&["md5"]).run(&s, &d, true).is_ok());

        // A dir on both sides: compared by presence, needs no digest.
        let s = map(vec![("sub", dir())]);
        let d = map(vec![("sub", dir())]);
        assert!(Shape::record_src(&["md5"]).run(&s, &d, true).is_ok());
    }

    /// Trust is **not** part of availability. `--no-trust-cached-hashes src` on a
    /// record asks it to re-read, which it cannot do, so it changes nothing it
    /// owes. Folding trust in would make the flag fatal for every record - and
    /// `tests/convert.rs` passes that flag against a record, so this is live.
    ///
    /// And the two halves stay independent: distrusting the *record* changes
    /// nothing, while distrusting the folder side still makes it re-read.
    #[test]
    fn distrusting_a_record_does_not_change_what_it_owes() {
        let s = map(vec![("a.txt", cached_entry(&["md5"]))]);
        let d = map(vec![("a.txt", cached_entry(&["md5"]))]);
        // The record is fully covered, so distrusting it is not a shortfall.
        assert!(
            Shape::record_src(&["md5"])
                .distrusting(true, false)
                .run(&s, &d, true)
                .is_ok()
        );
        // The folder side trusts: it already holds md5.
        let p = Shape::record_src(&["md5"])
            .distrusting(true, false)
            .run(&s, &d, true)
            .unwrap();
        assert!(p.dst.pending().is_empty());
        // The folder side distrusts, and it *can* re-read, so it does.
        let p = Shape::record_src(&["md5"])
            .distrusting(true, true)
            .run(&s, &d, true)
            .unwrap();
        assert_eq!(p.dst.pending(), vec!["a.txt"]);
    }

    /// A folder is never short of anything it can reach, so `sync` - two folders -
    /// cannot fail coverage. That is what makes the shared check safe there.
    #[test]
    fn a_folder_side_is_never_uncovered_however_little_it_has_cached() {
        let s = map(vec![("a.txt", file(10, 100))]);
        let d = map(vec![("a.txt", file(10, 100))]);
        let p = Shape {
            hashable: (true, true),
            algos: &["md5", "sha256"],
            no_trust: (false, false),
        }
        .run(&s, &d, true)
        .unwrap();
        assert_eq!(p.src.pending(), vec!["a.txt"]);
        assert_eq!(p.dst.pending(), vec!["a.txt"]);
    }

    /// A record that *can* meet the request is never asked to compute, because it
    /// cannot. The old planner inserted it anyway and `resolve_record` ignored the
    /// entry, so this asserts the plan is clean rather than merely unused.
    #[test]
    fn a_covered_record_is_never_asked_to_hash() {
        let s = map(vec![("a.txt", cached_entry(&["md5"]))]);
        let d = map(vec![("a.txt", file(10, 100))]);
        let p = Shape::record_src(&["md5"]).run(&s, &d, true).unwrap();
        assert!(
            p.src.pending().is_empty(),
            "a record has nothing to compute"
        );
        assert_eq!(p.dst.pending(), vec!["a.txt"], "the folder side pays");
    }

    /// Both sides can be records (`compare --src A/girpr-cache --dst B/girpr-cache`),
    /// and then either may be the short one. `src` is reported first so the message
    /// leads with a fixed side rather than whichever happened to be scanned first.
    #[test]
    fn two_records_report_the_src_shortfall_first() {
        let s = map(vec![("a.txt", file(10, 100))]);
        let d = map(vec![("a.txt", file(10, 100))]);
        let err = Shape::both_records(&["md5"]).run(&s, &d, true).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("the src side"), "{msg}");
    }

    /// An unrequested algorithm is neither a shortfall nor a substitute. A record
    /// carrying `blake3` that this crate has never heard of covers `md5`
    /// perfectly well - which is `tests/convert.rs`'s shape, and the reason
    /// availability is a set membership test rather than a count.
    #[test]
    fn an_unrequested_algorithm_is_neither_a_shortfall_nor_a_substitute() {
        let s = map(vec![("a.txt", cached_entry(&["md5", "blake3"]))]);
        let d = map(vec![("a.txt", cached_entry(&["md5", "blake3"]))]);
        assert!(
            Shape::record_src(&["md5"]).run(&s, &d, true).is_ok(),
            "md5 is what was asked for and it is there"
        );
        // And an algorithm the record does *not* hold is a shortfall even though
        // it holds something else.
        let s = map(vec![("a.txt", cached_entry(&["blake3"]))]);
        let d = map(vec![("a.txt", cached_entry(&["md5", "blake3"]))]);
        assert!(Shape::record_src(&["md5"]).run(&s, &d, true).is_err());
    }
}
