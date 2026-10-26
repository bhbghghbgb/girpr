//! [`AlgoSet`] and the `--hash` / `--hash-any-of` parsers.

use anyhow::{Result, anyhow, bail};

use super::algo::HashAlg;

/// A set of algorithms, as a bitfield: two bytes, no allocations, no order.
///
/// ## Why a set and not a list
///
/// This is the per-path and per-pair answer — "which algorithms settle this
/// pair" — and there is one of them per path, for every path in the tree. As a
/// `Vec<String>` that is a fresh heap allocation per path holding a fresh clone
/// of the run-wide request; as two bytes it is a field in a `HashMap` value that
/// was already there.
///
/// ## No order of its own
///
/// A set has no order to preserve, so [`Self::iter`] yields [`HashAlg::ALL`]
/// order and *that* is what everything downstream relies on. The one place where
/// a user's ordering genuinely matters — `crate::planner::pick_one`'s `--hash-any-of`
/// tie-break — reads the **request** (`&[HashAlg]`), not a set. If a set were
/// ever allowed to stand in for the request, the tie-break would silently become
/// "alphabetical" and the flag would quietly stop honouring the order the user
/// typed. Read the request there.
///
/// ## No `Display`
///
/// Deliberate. Three call sites need three separators — `+` in a `--why` tag, `, `
/// in a coverage error, a space in a remedy — so callers iterate and join with
/// what they need. One `Display` would force the other two to split it back apart.
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct AlgoSet(u16);

impl AlgoSet {
    /// No algorithms at all: `--hash-all-of none`, or a path that needs no read.
    pub const NONE: AlgoSet = AlgoSet(0);

    /// A one-algorithm set. [`crate::planner::HashMode::AnyOf`] settles every pair
    /// with exactly one, so this is what the planner's pick becomes.
    pub const fn one(a: HashAlg) -> AlgoSet {
        AlgoSet(a.bit())
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// How many algorithms are in the set. The count of bits, so it cannot
    /// disagree with [`Self::iter`] — there is no second list to fall out of step.
    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    pub fn insert(&mut self, a: HashAlg) {
        self.0 |= a.bit();
    }

    pub const fn contains(self, a: HashAlg) -> bool {
        self.0 & a.bit() != 0
    }

    /// `self` minus `other`: what this side still owes.
    ///
    /// The set-arithmetic form of "everything requested, minus what the row
    /// already holds", which is how [`crate::planner::HashPlan`] decides what to
    /// compute. An `other` holding an algorithm outside `self` cannot widen the
    /// result, so a caller need not pre-intersect.
    pub const fn difference(self, other: AlgoSet) -> AlgoSet {
        AlgoSet(self.0 & !other.0)
    }

    /// Canonical order — [`HashAlg::ALL`] order, always. The order every
    /// caller that renders or compares algorithms can rely on.
    pub fn iter(self) -> impl Iterator<Item = HashAlg> {
        HashAlg::ALL.into_iter().filter(move |a| self.contains(*a))
    }
}

impl FromIterator<HashAlg> for AlgoSet {
    fn from_iter<I: IntoIterator<Item = HashAlg>>(iter: I) -> Self {
        let mut set = AlgoSet::NONE;
        for a in iter {
            set.insert(a);
        }
        set
    }
}

impl std::fmt::Debug for AlgoSet {
    /// `["md5", "sha256"]` — byte-identical to what a `Vec<String>` of the same
    /// algorithms renders as, so every `?algos` log line keeps its spelling.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// Validate repeated `--hash` values.
///
/// Returns the requested algorithms — each one once, in the order it was first
/// typed — or an empty list for `--hash none` (which is exclusive: decisions fall
/// back to size+mtime).
///
/// Strings stop here. Below this point an algorithm is a [`HashAlg`], so "which
/// algorithms can a row hold" and "which algorithms did the user ask for" are
/// asked with the same vocabulary rather than two spellings of it.
pub fn parse_hash_list(input: &[String]) -> Result<Vec<HashAlg>> {
    let mut v: Vec<String> = input.iter().map(|s| s.to_lowercase()).collect();
    v.retain(|s| !s.is_empty());
    if v.iter().any(|s| s == "none") {
        if v.len() != 1 {
            bail!("--hash none cannot be combined with other algorithms");
        }
        return Ok(vec![]);
    }
    if v.is_empty() {
        return Ok(vec![]);
    }
    // Validate and dedup in one pass, so the *first* unsupported name is still the
    // one reported: `["md5", "nope"]` names `nope`, exactly as it did when the two
    // were separate loops over the same list.
    //
    // Full dedup in first-seen order, not `Vec::dedup`'s adjacent-only pass:
    // `--hash-all-of md5 --hash-all-of sha256 --hash-all-of md5` parsed as three
    // algorithms, which reached `HashPlan::digest_count` (three digests
    // promised) and a `digest-differs:md5+sha256+md5` reason tag. Order is the
    // user's, because `pick_one`'s tie-break is defined in terms of it.
    //
    // `contains` over a list bounded by the argv length: two short scans, and a
    // `HashSet` here would be an allocation to save less than the deduped list.
    let mut seen: Vec<HashAlg> = Vec::with_capacity(v.len());
    for a in &v {
        // `UnknownHash` names the offender; the supported list is this message's
        // to add. The resulting text is unchanged from the hand-rolled one.
        let algo = a
            .parse::<HashAlg>()
            .map_err(|e| anyhow!("{e} (supported: {} or none)", HashAlg::names()))?;
        if !seen.contains(&algo) {
            seen.push(algo);
        }
    }
    Ok(seen)
}

/// Validate repeated `--hash-any-of` values.
///
/// Separate from [`parse_hash_list`] for one reason: **`none` is not a value here.**
/// "Any of the requested algorithms", with nothing requested, is not a weaker
/// request — it is no request — and `--hash-all-of none` is the flag that says so.
/// Accepting it would give a stat-only audit a name claiming a digest was optional
/// but not required, which is the ambiguity all-of exists to remove.
pub fn parse_any_of(input: &[String]) -> Result<Vec<HashAlg>> {
    let algos = parse_hash_list(input)?;
    if algos.is_empty() {
        bail!(
            "--hash-any-of needs at least one algorithm; for a stat-only audit use \
             --hash-all-of none"
        );
    }
    Ok(algos)
}
