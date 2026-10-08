//! Deciding which digests a run has to compute.
//!
//! Phase A reports what a side already knows, [`HashPlan`] turns that into the set
//! of digests each side still has to produce, and phase C executes it. Three
//! properties follow from the split, and each is load-bearing:
//!
//! - **Phase A never hashes.** It reports what the cache knows, nothing more.
//!   That is what makes the cache *consultable* rather than fused with disk
//!   access, and it is what lets this module pick an algorithm per path instead
//!   of being handed whatever a scan already decided to read.
//! - **The decision is made here and nowhere else.** Phase A does not widen the
//!   request, and phase C does not narrow it. `--hash-any-of` is a different
//!   predicate in [`HashPlan`] rather than a new pass over the tree.
//! - **The decision is made before any read.** A plan is total over the run, so
//!   a run that cannot answer everything fails having read nothing.
//!
//! There are two predicates, and they answer different questions. One side with no
//! counterpart is [`HashPlan::plan_one_side`] — every requested algorithm for every
//! file, which is `update`. Two sides are [`plan_pairs`], which is
//! [`plan_one_side`] plus a pairing step and a coverage requirement.
//!
//! ## Two questions, kept apart
//!
//! [`HashPlan`] answers *what must be computed* and honours trust. Coverage
//! answers *whether the run can be answered at all*, and it does **not**: a side's
//! availability is `cached ∪ hashable`, with no trust term. Both halves matter,
//! because a side whose availability included trust could not be short of
//! anything:
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

/// How many of the requested algorithms an undecided pair must be able to answer
/// with.
///
/// Both modes ask a real question about content, and the split between them is the
/// whole of their point. Without a requirement, a pair whose two sides hold no
/// comparable digest falls back to a size+mtime that already agreed, and the run
/// reports a content comparison it never performed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HashMode {
    /// `--hash-all-of`, and the default: **every** requested algorithm must be
    /// available on both sides of every undecided pair.
    ///
    /// A folder side that lacks one backfills it by hashing. A record side cannot,
    /// so that is fatal — which is what makes a comparison a comparison.
    #[default]
    AllOf,
    /// `--hash-any-of`: **at least one** must be, and the planner picks one per
    /// pair, cheapest first and then in the order the user gave.
    ///
    /// Weaker than all-of by design, and never weaker than the pick: every
    /// algorithm it chooses is obtainable on *both* sides, so a pair settled this
    /// way is still settled by a digest both sides hold rather than by a digest one
    /// of them is missing. That is why the narrower mode cannot admit a pair whose
    /// content was never read.
    AnyOf,
}

/// Which parts of a file's stat a run will accept as evidence that two sides differ.
///
/// The short circuit in [`plan_pairs`] is an **optimisation**, so it has to be
/// switchable — and naming the two fields it consults is better than adding a switch
/// for the optimisation itself: `--no-trust-size --no-trust-mtime` together *is*
/// "disable it", and no third spelling is needed to reach it.
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

    /// True when this pair's stat, as trusted, settles it without a digest.
    ///
    /// The single definition of the short circuit's condition, so the planner and
    /// anything else that has to agree with it read the same rule. `size` and
    /// `mtime` are both present rather than compared here, so a caller cannot
    /// accidentally ask about one and infer the other.
    pub fn settles(&self, a_size: u64, a_mtime: i64, b_size: u64, b_mtime: i64) -> bool {
        (self.size && a_size != b_size) || (self.mtime && a_mtime != b_mtime)
    }

    /// False when neither field is trusted — i.e. the short circuit is off.
    ///
    /// For a caller that has no pairing step and so never consults [`Self::settles`],
    /// such as `update`. It asks whether there is anything left to switch off rather
    /// than repeating the negation of two fields.
    pub fn settles_any(&self) -> bool {
        self.size || self.mtime
    }
}

/// The algorithms that settle each undecided pair, from one decision.
///
/// Produced by [`plan_pairs`] and consumed by [`crate::diff::diff_maps`]. Handing
/// the diff *this* rather than the algorithm list is what lets one run settle path
/// X by md5 and path Y by sha256: under `all-of` every entry is the whole requested
/// list, so a uniform run would be described just as well by passing that list
/// straight through.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Required {
    /// src relative path -> the algorithms that settle it.
    pub by_rel: HashMap<String, Vec<String>>,
    /// Consulted for a path with no entry. See [`Required::of`].
    pub fallback: Vec<String>,
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
    pub fn of(&self, srel: &str) -> &[String] {
        self.by_rel.get(srel).unwrap_or(&self.fallback)
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
    /// Which algorithms settle each pair — see [`Required`].
    pub required: Required,
    /// Which stat fields settled a pair, as this run decided it.
    ///
    /// Carried out of the planner so the diff reads the *same* value rather than
    /// taking the flags a second time. The planner's short circuit and the diff's are
    /// the same rule, and reading it twice is how they drift — silently, and in both
    /// directions: a digest computed that nobody consults, or a pair the planner
    /// called settled that needed a digest nobody computed.
    pub stat: StatTrust,
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
/// algorithm". A path needs a digest only when it is a file on **both** sides with
/// equal size and equal mtime; everything else is already decided:
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
/// [`StatTrust::without_size`] and [`StatTrust::without_mtime`] put a field back into
/// the undecided set, and both together leave the short circuit with nothing to act
/// on.
///
/// Coverage follows the same boundary as the short circuit, and for the same reason: a
/// pair settled by a trusted stat field needed no digest, so none is required. Distrust
/// a field and its pairs become answerable questions — which for a record side means
/// they can now fail coverage, and that is the intended consequence rather than an
/// accident of the check's placement.
///
/// ## The two modes
///
/// [`HashMode::AllOf`] requires the whole requested list on an undecided pair, which
/// is what the diff then compares. [`HashMode::AnyOf`] requires one and picks it per
/// pair, so the answer is per *path* rather than per run — that is the entire reason
/// [`Required`] exists instead of passing `algos` to the diff.
///
/// The availability bookkeeping below is shared by both, and deliberately so: "can
/// this side obtain algorithm `a`" has the same answer either way — it holds it, or
/// it has a filesystem to read it from. Only the *requirement* changes: every `a`, or
/// one `a`. Under `any-of`, a side with a filesystem can therefore never be the
/// short one, which is why an `any-of` coverage failure always names a record.
pub fn plan_pairs(
    src: SideRequest<'_>,
    dst: SideRequest<'_>,
    case_sensitive: bool,
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
    plan.required.fallback = src.algos.to_vec();
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
                let slot = t.coverage.entry(algo.clone()).or_insert((0, 0));
                slot.1 += 1;
                if e.cached.contains_key(algo) {
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
                    let short: Vec<String> = req
                        .algos
                        .iter()
                        .filter(|a| !obtainable(e, req.cap, a))
                        .cloned()
                        .collect();
                    if !short.is_empty() {
                        tally[idx].missing.insert((*rel).clone(), short);
                    }
                }
                plan.required
                    .by_rel
                    .insert((*srel).clone(), src.algos.to_vec());
                plan_one(&src, s, srel, src.algos, &mut plan.src);
                plan_one(&dst, d, drel, dst.algos, &mut plan.dst);
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
                        let short: Vec<String> = req
                            .algos
                            .iter()
                            .filter(|a| !obtainable(e, req.cap, a))
                            .cloned()
                            .collect();
                        if !short.is_empty() {
                            t.missing.insert((*rel).clone(), short);
                        }
                    }
                    continue;
                };
                let chosen = [pick];
                plan.required
                    .by_rel
                    .insert((*srel).clone(), chosen.to_vec());
                plan_one(&src, s, srel, &chosen, &mut plan.src);
                plan_one(&dst, d, drel, &chosen, &mut plan.dst);
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

/// Add whatever `req` still owes for `rel` to `into`.
///
/// Skipped for a side that cannot hash, which is why `plan.required` — not this
/// function — is the record of what the run must produce: a side with no filesystem
/// is never *asked*, so a plan entry for it would be a request nothing could honour.
fn plan_one(
    req: &SideRequest<'_>,
    e: &SideEntry,
    rel: &str,
    algos: &[String],
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
    algos: &[String],
    s: &SideEntry,
    d: &SideEntry,
    sc: SideCapability,
    dc: SideCapability,
) -> Option<String> {
    algos
        .iter()
        .filter(|a| obtainable(s, sc, a) && obtainable(d, dc, a))
        .min_by_key(|a| {
            usize::from(!s.cached.contains_key(*a)) + usize::from(!d.cached.contains_key(*a))
        })
        .cloned()
}

/// Whether a side can supply `algo` at all: it holds it, or it can go and read it.
///
/// **No trust term**, for the reason the coverage check has none: trust asks a side
/// to re-read rather than reuse, and a record has no filesystem to re-read. Folding
/// it in would make `--no-trust-cached-hashes src` fatal for every record.
fn obtainable(e: &SideEntry, cap: SideCapability, algo: &str) -> bool {
    e.cached.contains_key(algo) || cap.can_hash_from_disk
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
    /// Which mode was asked for. Carried because the two modes fail differently and
    /// so are *remedied* differently — an all-of failure can be answered by asking
    /// for any one algorithm, which is the whole of `any-of`.
    pub mode: HashMode,
    /// rel path -> the requested algorithms it could not obtain.
    pub missing: BTreeMap<String, Vec<String>>,
    /// Requested algorithm -> (paths holding it, undecided paths).
    pub coverage: BTreeMap<String, (usize, usize)>,
}

impl fmt::Display for Coverage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let undecided: usize = self.coverage.values().next().map_or(0, |c| c.1);
        let flag = match self.mode {
            HashMode::AllOf => "--hash-all-of",
            HashMode::AnyOf => "--hash-any-of",
        };
        writeln!(
            f,
            "this run cannot compare: the {} side cannot supply every digest it was asked for",
            self.side
        )?;
        // Named up front, because the two modes are different requests and the
        // remedies below are mode-specific — a reader who does not know which one
        // failed cannot tell whether `--hash-any-of` is offered as the fix or as the
        // thing that was already tried.
        writeln!(f, "\n  asked under {flag}")?;
        writeln!(f, "  {}: {}", self.side, self.label)?;
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
        // The requirement, stated in the mode the user actually asked for. The two
        // sentences differ in exactly one clause, and that clause is the whole
        // reason the mode exists — so it is worth the two branches.
        let requirement = match self.mode {
            HashMode::AllOf => {
                "Every requested algorithm must be available on both sides of a \
                 pair that\nsize+mtime cannot settle."
            }
            HashMode::AnyOf => {
                "At least one requested algorithm must be available on both sides \
                 of a pair that\nsize+mtime cannot settle, and none of them is."
            }
        };
        writeln!(
            f,
            "\n{requirement} This side has no filesystem to read a missing digest from, so the \
             pair\nwould fall back to size+mtime and this run would report a confident answer \
             about\ncontent it never read."
        )?;
        // Remedies a user can act on without reading the rest of the message. Every
        // one of them names a flag that exists.
        writeln!(f, "\nTo answer this question, one of:")?;
        // Only worth offering when some single algorithm would actually cover the
        // undecided set — otherwise it is a command that fails the same way.
        let narrower = self
            .coverage
            .iter()
            .find(|(_, (have, total))| *total > 0 && have == total)
            .map(|(a, _)| a.clone());
        match self.mode {
            HashMode::AllOf => {
                if let Some(a) = narrower {
                    writeln!(
                        f,
                        "  - narrow the request to what it already holds: --hash-all-of {a}"
                    )?;
                } else if self.coverage.len() > 1 {
                    writeln!(
                        f,
                        "  - require only one of them: --hash-any-of {}",
                        self.coverage.keys().cloned().collect::<Vec<_>>().join(" ")
                    )?;
                }
            }
            HashMode::AnyOf => {
                writeln!(
                    f,
                    "  - require every one of them, which is stricter but equally \
                     unanswerable:\n    --hash-all-of {}",
                    self.coverage.keys().cloned().collect::<Vec<_>>().join(" ")
                )?;
            }
        }
        writeln!(
            f,
            "  - repopulate it, then run this again: girsync update --dir <the folder \
             this record describes>"
        )?;
        writeln!(
            f,
            "  - compare by stat only, and accept that content is unchecked: \
             --hash-all-of none"
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

    /// A complete cache row costs no read, an incomplete one costs only what is
    /// missing.
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
            .insert("z.txt".to_string(), vec!["md5".to_string()]);
        p.by_rel
            .insert("a.txt".to_string(), vec!["md5".to_string()]);
        p.by_rel.insert("done.txt".to_string(), vec![]);
        assert_eq!(p.pending(), vec!["a.txt", "z.txt"]);
        assert!(p.is_settled("done.txt"));
        assert_eq!(p.digest_count(), 2);
    }

    // -- pair planning -------------------------------------------------------
    //
    // The predicate, in isolation. Each path state from the table gets one case,
    // because the five states divide cleanly into "needs a digest" and "does not".

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
        /// Which answerability rule the run asked for. Defaults to all-of, because
        /// that is the default and these cases are about coverage rather than the
        /// choice between the two.
        mode: HashMode,
    }

    impl Shape {
        /// Both sides folders: nothing can be short of coverage, which is what most
        /// laziness cases need and what makes `sync` safe to share this check.
        const FOLDERS: Shape = Shape {
            hashable: (true, true),
            algos: &["md5"],
            no_trust: (false, false),
            mode: HashMode::AllOf,
        };

        /// A record on the src side. The common coverage shape: the record cannot
        /// fill a gap and the folder can.
        fn record_src(algos: &'static [&'static str]) -> Shape {
            Shape {
                hashable: (false, true),
                algos,
                no_trust: (false, false),
                mode: HashMode::AllOf,
            }
        }

        /// Both sides records, for the case where either may be the short one.
        fn both_records(algos: &'static [&'static str]) -> Shape {
            Shape {
                hashable: (false, false),
                algos,
                no_trust: (false, false),
                mode: HashMode::AllOf,
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
            stat: StatTrust,
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
                self.mode,
                stat,
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
            .run(s, d, sensitive, StatTrust::default())
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

    /// A record missing a requested digest fails the run rather than letting the
    /// pair fall back to size+mtime.
    #[test]
    fn a_record_missing_a_requested_algorithm_fails_the_run() {
        let s = map(vec![("a.txt", cached_entry(&["sha256"]))]);
        let d = map(vec![("a.txt", cached_entry(&["md5", "sha256"]))]);
        let err = Shape::record_src(&["md5", "sha256"])
            .run(&s, &d, true, StatTrust::default())
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("src"), "names the side: {msg}");
        assert!(msg.contains("a.txt"), "names the path: {msg}");
        assert!(msg.contains("md5"), "names the algorithm: {msg}");
        assert!(msg.contains("1/1"), "reports coverage: {msg}");
    }

    /// **Scoped to the undecided set.** A record full of stat-differing or
    /// one-sided paths needs no coverage, and failing on it would fail runs that
    /// are perfectly answerable — which is why this lives here and not in the
    /// record loader, which sees the whole record and cannot tell which of its paths
    /// are answerable.
    #[test]
    fn coverage_is_scoped_to_the_undecided_set() {
        // Undecided but covered: fine.
        let s = map(vec![("ok.txt", cached_entry(&["md5"]))]);
        let d = map(vec![("ok.txt", cached_entry(&["md5"]))]);
        assert!(
            Shape::record_src(&["md5"])
                .run(&s, &d, true, StatTrust::default())
                .is_ok()
        );

        // Stat differs: already decided, so an empty record row costs nothing.
        let s = map(vec![("d.txt", file(10, 100))]);
        let d = map(vec![("d.txt", file(11, 100))]);
        assert!(
            Shape::record_src(&["md5"])
                .run(&s, &d, true, StatTrust::default())
                .is_ok()
        );

        // Record-only path: a MISSING verdict, decided by presence.
        let s = map(vec![("only_rec.txt", file(10, 100))]);
        let d = map(vec![]);
        assert!(
            Shape::record_src(&["md5"])
                .run(&s, &d, true, StatTrust::default())
                .is_ok()
        );

        // A dir on both sides: compared by presence, needs no digest.
        let s = map(vec![("sub", dir())]);
        let d = map(vec![("sub", dir())]);
        assert!(
            Shape::record_src(&["md5"])
                .run(&s, &d, true, StatTrust::default())
                .is_ok()
        );
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
                .run(&s, &d, true, StatTrust::default())
                .is_ok()
        );
        // The folder side trusts: it already holds md5.
        let p = Shape::record_src(&["md5"])
            .distrusting(true, false)
            .run(&s, &d, true, StatTrust::default())
            .unwrap();
        assert!(p.dst.pending().is_empty());
        // The folder side distrusts, and it *can* re-read, so it does.
        let p = Shape::record_src(&["md5"])
            .distrusting(true, true)
            .run(&s, &d, true, StatTrust::default())
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
            mode: HashMode::AllOf,
        }
        .run(&s, &d, true, StatTrust::default())
        .unwrap();
        assert_eq!(p.src.pending(), vec!["a.txt"]);
        assert_eq!(p.dst.pending(), vec!["a.txt"]);
    }

    /// A record that *can* meet the request is never asked to compute, because it
    /// cannot. `plan_one` skips such a side, so the plan carries no entry for it at
    /// all — which is asserted here rather than left as an unobservable consequence of
    /// the skip.
    #[test]
    fn a_covered_record_is_never_asked_to_hash() {
        let s = map(vec![("a.txt", cached_entry(&["md5"]))]);
        let d = map(vec![("a.txt", file(10, 100))]);
        let p = Shape::record_src(&["md5"])
            .run(&s, &d, true, StatTrust::default())
            .unwrap();
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
        let err = Shape::both_records(&["md5"])
            .run(&s, &d, true, StatTrust::default())
            .unwrap_err();
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
            Shape::record_src(&["md5"])
                .run(&s, &d, true, StatTrust::default())
                .is_ok(),
            "md5 is what was asked for and it is there"
        );
        // And an algorithm the record does *not* hold is a shortfall even though
        // it holds something else.
        let s = map(vec![("a.txt", cached_entry(&["blake3"]))]);
        let d = map(vec![("a.txt", cached_entry(&["md5", "blake3"]))]);
        assert!(
            Shape::record_src(&["md5"])
                .run(&s, &d, true, StatTrust::default())
                .is_err()
        );
    }
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
