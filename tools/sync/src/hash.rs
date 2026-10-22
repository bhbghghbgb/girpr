//! Hash algorithm selection and streaming file digests.

use anyhow::{Context, Result, anyhow, bail};
use sha2::Digest;
use std::io::Read;
use std::path::Path;

/// Chunk size for every read of file *content* this crate does.
///
/// One constant, not a per-call-site literal: a digest is correct only if it is
/// fed every byte, and a caller already moving the bytes should not have to
/// know that to stay in step with [`hash_file`].
pub const CHUNK: usize = 512 * 1024;

/// One algorithm a run may **ask for**.
///
/// ## What this is, and what it is not
///
/// This is the *request* side of the vocabulary: the closed set of names
/// `--hash-all-of` / `--hash-any-of` accept. It is one byte per algorithm and
/// there is no string anywhere near it, because the request is read once per run.
///
/// **It is not what a cache row stores.** A row may carry a digest under a name
/// this build has never heard of — a cache written by a newer binary with
/// `--hash blake3` has to stay readable here — so the persistence layer cannot be
/// an enum without giving that up. See `crate::cache::codec`, which keeps names
/// inline for exactly that reason, and [`DigestSet`]'s `foreign` arm for the
/// in-memory end of the same guarantee. A closed enum here and an open set there
/// is the whole design; the two are deliberately not reconciled.
///
/// ## Ord
///
/// Derived, so it is declaration order — which **is** [`Self::ALL`] order and is
/// the one order every canonical iteration in this crate uses. A third algorithm
/// gets a canonical slot at the end of the enum and nothing else has to change.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum HashAlg {
    Md5,
    Sha256,
}

impl HashAlg {
    /// Every algorithm, in canonical order.
    ///
    /// The single list that defines "canonical order": [`AlgoSet::iter`] filters
    /// this, [`DigestSet::iter`] walks it, and the codec emits rows in it.
    pub const ALL: [HashAlg; 2] = [HashAlg::Md5, HashAlg::Sha256];

    /// The command-line spelling. Also the cache key, verbatim.
    pub const fn as_str(self) -> &'static str {
        match self {
            HashAlg::Md5 => "md5",
            HashAlg::Sha256 => "sha256",
        }
    }

    /// Raw digest bytes — never hex. See `FileRec::hashes`.
    ///
    /// Also the length a decoded row of this algorithm must have: a name with a
    /// digest of any other length is not this algorithm's digest, and the codec
    /// routes it to [`DigestSet`]'s `foreign` arm rather than rejecting it.
    pub const fn digest_len(self) -> usize {
        match self {
            HashAlg::Md5 => 16,
            HashAlg::Sha256 => 32,
        }
    }

    /// The bit this algorithm occupies in an [`AlgoSet`].
    ///
    /// Private because it is an encoding detail: `AlgoSet` is the only thing that
    /// should be manipulating a bitfield, and a public `bit()` would invite a
    /// caller to build one by hand and bypass its invariants.
    const fn bit(self) -> u16 {
        1u16 << (self as u8)
    }

    /// `md5,sha256` — the supported list as the existing `--hash` refusal names it.
    pub fn names() -> String {
        HashAlg::ALL.map(|a| a.as_str()).join(",")
    }
}

/// The name an algorithm was parsed from that this build does not know.
///
/// Carries the offending name and nothing else. **The supported list is not part
/// of it**, because that list is the caller's sentence to finish: `Display` here
/// says what was wrong, and the `--hash` refusal appends what would have been
/// right. Two copies of one message is two spellings to keep in step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownHash {
    name: String,
}

impl std::fmt::Display for UnknownHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unsupported hash '{}'", self.name)
    }
}

impl std::error::Error for UnknownHash {}

impl std::fmt::Debug for HashAlg {
    /// The command-line spelling, quoted — **not** the derived `Md5`/`Sha256`.
    ///
    /// `info!(algos = ?algos)` appears in the span of all four subcommands and
    /// prints `["md5", "sha256"]` today. A derive would silently rewrite every one
    /// of those log lines to `[Md5, Sha256]` and quietly break anyone grepping a
    /// log, which is exactly the sort of change that costs a day and is never in
    /// the diff that caused it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_str(), f)
    }
}

impl std::fmt::Display for HashAlg {
    /// `f.pad`, not `f.write_str`.
    ///
    /// A `Display` impl is responsible for honouring the width/fill/alignment in the
    /// format spec, and `write_str` does not: it is `pad` that applies them. The
    /// visible consequence is the coverage error's per-algorithm column, which is
    /// rendered `{:<12}` — `write_str` would silently print `md5 0/1` instead of
    /// `md5         0/1`, with no error anywhere.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

impl std::str::FromStr for HashAlg {
    type Err = UnknownHash;

    /// Case-insensitive **on its own**, not because a caller remembered to
    /// lowercase first. `--hash MD5` works today for a reason that lives in
    /// `parse_hash_list`, which is one caller's private detail; a parser whose
    /// contract depends on someone else having normalised the input first is a
    /// parser every new caller gets wrong. ASCII case only, which is all an
    /// algorithm name can be.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        HashAlg::ALL
            .into_iter()
            .find(|a| a.as_str().eq_ignore_ascii_case(s))
            .ok_or_else(|| UnknownHash {
                name: s.to_string(),
            })
    }
}

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

/// The digests one cache row carries, as inline slots rather than a map.
///
/// ## Not `Digests`
///
/// [`Digests`] is the streaming *context* that absorbs bytes; this is the *result*.
/// Different jobs, and they must not be merged: the context is per-file-per-phase and
/// thrown away, this one lives in the cache and in both sides' effective maps.
///
/// ## Why not a `HashMap<String, Vec<u8>>`
///
/// The map version was five allocations per row — the table, two `String` keys, two
/// `Vec<u8>` digests — and a row is allocated once per file in phase A and held twice
/// during phase C. This is **72 bytes and zero allocations** for a normal row: two
/// fixed-size optional arrays, plus a boxed slice that is empty and allocates nothing
/// unless something needs carrying. `size_of` is asserted in this module's tests.
///
/// ## The foreign arm is not optional
///
/// `HashAlg` is a closed enum, and this is where that stops being the whole truth. A
/// cache written by a newer build may carry a digest under a name this one has never
/// heard of, and `crate::cache::codec` keeps names inline precisely so that row stays
/// readable. `foreign` is where such a name lives in memory, and it is **carried,
/// merged and written back** rather than merely decoded — a scan that dropped it would
/// lose the row's contents on the next write.
///
/// A *known* name with an *unexpected* length lands here too, and must: today such a
/// row lives in a `HashMap` and `x != y` compares two unequal `Vec`s, reporting a
/// difference. Refusing it instead would turn a corrupt row into a hard failure rather
/// than a reported difference, which is not what this format has always done.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct DigestSet {
    md5: Option<[u8; 16]>,
    sha256: Option<[u8; 32]>,
    /// Algorithms this build does not know, carried verbatim. Empty in practice.
    /// See the type's docs.
    foreign: Box<[(Box<str>, Vec<u8>)]>,
}

impl DigestSet {
    /// How many digests this holds, foreign ones included.
    ///
    /// Not `const`, unlike [`AlgoSet::len`]: counting `foreign` means dereferencing a
    /// boxed slice, and a count that silently ignored the foreign arm would be wrong
    /// for exactly the rows forward compatibility exists to preserve.
    pub fn len(&self) -> usize {
        self.md5.is_some() as usize + self.sha256.is_some() as usize + self.foreign.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The digest for a known algorithm.
    ///
    /// `None` means "this row has no digest under that name", **not** "the digest is
    /// zero". That distinction is load-bearing: [`crate::diff::hashes_differ`] is
    /// silent on a missing digest, and a zero digest substituted for `None` would turn
    /// a false clean back on. A known name sitting in `foreign` also reads as `None`
    /// here — see [`Self::get_named`] for the by-name lookup that finds it.
    pub fn get(&self, a: HashAlg) -> Option<&[u8]> {
        match a {
            HashAlg::Md5 => self.md5.as_ref().map(|d| &d[..]),
            HashAlg::Sha256 => self.sha256.as_ref().map(|d| &d[..]),
        }
    }

    /// The digest under *any* name, known or not.
    ///
    /// This is what availability tests and a name-keyed comparison need: a row written
    /// by a newer build holds `blake3`, and asking for it by name has to find it.
    pub fn get_named(&self, name: &str) -> Option<&[u8]> {
        if let Ok(a) = name.parse::<HashAlg>()
            && let Some(d) = self.get(a)
        {
            return Some(d);
        }
        // Not `else`: a known name with an unexpected length is in `foreign`, and
        // `get` cannot see it.
        self.foreign
            .iter()
            .find(|(n, _)| &**n == name)
            .map(|(_, d)| d.as_slice())
    }

    /// Known algorithms in canonical order. Foreign ones are not included;
    /// [`Self::foreign_iter`] is separate and a caller that wants them asks.
    pub fn iter(&self) -> impl Iterator<Item = (HashAlg, &[u8])> {
        HashAlg::ALL
            .into_iter()
            .filter_map(|a| self.get(a).map(|d| (a, d)))
    }

    /// The names this build does not know, in whatever order they were recorded.
    pub fn foreign_iter(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.foreign.iter().map(|(n, d)| (&**n, d.as_slice()))
    }

    /// Record a digest under `name`.
    ///
    /// **The routing rule, in one place.** A name this build knows *and* a digest of
    /// that algorithm's length goes to its slot; anything else becomes foreign. The
    /// codec decodes through this and nothing else, so "a known name with a wrong
    /// length is still a digest, not an error" cannot be spelled two ways.
    pub fn insert_named(&mut self, name: &str, digest: &[u8]) {
        let known = name.parse::<HashAlg>().ok();
        if known == Some(HashAlg::Md5)
            && let Ok(d) = <[u8; 16]>::try_from(digest)
        {
            self.md5 = Some(d);
            return;
        }
        if known == Some(HashAlg::Sha256)
            && let Ok(d) = <[u8; 32]>::try_from(digest)
        {
            self.sha256 = Some(d);
            return;
        }
        let mut entries = std::mem::take(&mut self.foreign).into_vec();
        // An explicit loop rather than `find`: the tuple destructuring is what makes
        // `&**n` a `&str` here, and it reads as "replace the entry with this name"
        // rather than "find a slot and poke at it".
        let mut replaced = false;
        for (n, d) in entries.iter_mut() {
            if &**n == name {
                *d = digest.to_vec();
                replaced = true;
                break;
            }
        }
        if !replaced {
            entries.push((name.into(), digest.to_vec()));
        }
        self.foreign = entries.into_boxed_slice();
    }

    /// Fold `other` in, with **`self` winning** on every name both hold.
    ///
    /// That direction is the merge-write rule: this run's computed digests replace the
    /// prior row's, and everything the prior row held that this run did not compute
    /// rides along. See `crate::effective::merge_row`.
    pub fn merge(&mut self, other: &DigestSet) {
        if let Some(d) = other.md5 {
            self.md5 = Some(d);
        }
        if let Some(d) = other.sha256 {
            self.sha256 = Some(d);
        }
        if other.foreign.is_empty() {
            // The common case, and the only one that has to allocate nothing: a row
            // with no foreign entry is the overwhelmingly normal shape, and reaching
            // the loop below would rebuild the boxed slice on every merge.
            return;
        }
        let mut entries = std::mem::take(&mut self.foreign).into_vec();
        for (name, digest) in &other.foreign {
            let mut replaced = false;
            for (n, d) in entries.iter_mut() {
                if n == name {
                    *d = digest.clone();
                    replaced = true;
                    break;
                }
            }
            if !replaced {
                entries.push((name.clone(), digest.clone()));
            }
        }
        self.foreign = entries.into_boxed_slice();
    }
}

impl std::fmt::Debug for DigestSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut m = f.debug_map();
        m.entries(self.iter());
        m.entries(self.foreign_iter());
        m.finish()
    }
}

/// The digest contexts for a requested set of algorithms, fed chunk by chunk.
///
/// This is the part of [`hash_file`] that is not "open the file and read it".
/// It is split out because the copy phase is *already moving every byte of
/// src* and would otherwise read the whole file a second time purely to hash
/// it. One implementation rather than two on purpose: "absorb these bytes" is
/// exactly the kind of thing that looks identical in two places and drifts in
/// one of them, and a drift here is not a wrong number in a log — it is a
/// spurious `verify hash mismatch`, or a silently weakened one.
pub struct Digests {
    md5ctx: Option<md5::Context>,
    sha2ctx: Option<sha2::Sha256>,
}

impl Digests {
    /// Build the contexts `algos` asks for. An empty set builds none, so
    /// [`update`](Self::update) does nothing and [`finish`](Self::finish)
    /// yields an empty map — the same answer [`hash_file`] gives for it.
    ///
    /// A membership test on a bitfield rather than a scan for a name: this runs
    /// once per file per phase C and once per copied file, so a `String` search
    /// would be the whole of what this function does.
    pub fn new(algos: &AlgoSet) -> Self {
        Self {
            md5ctx: algos.contains(HashAlg::Md5).then(md5::Context::new),
            sha2ctx: algos.contains(HashAlg::Sha256).then(sha2::Sha256::new),
        }
    }

    /// Absorb the next chunk. Chunks must arrive in file order — the split is
    /// the caller's business, and [`hash_file`]'s is its read loop's.
    pub fn update(&mut self, chunk: &[u8]) {
        if let Some(c) = self.md5ctx.as_mut() {
            c.consume(chunk);
        }
        if let Some(c) = self.sha2ctx.as_mut() {
            c.update(chunk);
        }
    }

    /// The digests of everything fed so far. Values are **raw** bytes (16 for
    /// md5, 32 for sha256), which is what the cache stores.
    ///
    /// Assigned to the slots directly rather than through `insert_named`: the
    /// context knows exactly which algorithm each of its two halves is, so there is
    /// no name to route and no decision to get wrong.
    pub fn finish(self) -> DigestSet {
        let mut out = DigestSet::default();
        if let Some(c) = self.md5ctx {
            out.md5 = Some(c.compute().0);
        }
        if let Some(c) = self.sha2ctx {
            let d = c.finalize();
            out.sha256 = Some(d.into());
        }
        out
    }
}

/// Digest `path` with every requested algorithm in a single pass.
///
/// An empty `algos` returns an empty set without touching the file.
/// Values are **raw** digest bytes (16 for md5, 32 for sha256): the cache
/// stores them as-is instead of hex, halving hash disk cost.
pub fn hash_file(path: &Path, algos: &AlgoSet) -> Result<DigestSet> {
    // Before any context is built: with nothing requested there is nothing to
    // digest, and opening the file at all would turn a stat-only decision into a
    // read. The empty set below is the whole answer.
    if algos.is_empty() {
        return Ok(DigestSet::default());
    }
    let mut f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut digests = Digests::new(algos);
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n: usize = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        digests.update(&buf[..n]);
    }
    Ok(digests.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &girpr_tempdir::TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn algos(names: &[&str]) -> AlgoSet {
        AlgoSet::from_iter(names.iter().map(|s| s.parse().unwrap()))
    }

    fn parse(input: &[&str]) -> Vec<HashAlg> {
        parse_hash_list(&input.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    fn parse_any(input: &[&str]) -> Vec<HashAlg> {
        parse_any_of(&input.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    // -- HashAlg --------------------------------------------------------------

    /// Every name round-trips, and `as_str` is the inverse of `FromStr`. The
    /// identity has to hold for the *whole* enum: a name with no parse, or a
    /// parse with no name, is an algorithm the crate cannot store.
    #[test]
    fn every_algorithm_name_round_trips() {
        for a in HashAlg::ALL {
            assert_eq!(a.as_str().parse::<HashAlg>().unwrap(), a);
            assert_eq!(a.as_str().to_string().parse::<HashAlg>().unwrap(), a);
        }
    }

    /// Case-insensitive **on its own**. `--hash MD5` works today because
    /// `parse_hash_list` lowercases before validating, which is a private detail
    /// of one caller; a parser that only works when someone else has normalised
    /// the input is a parser every new caller gets wrong.
    #[test]
    fn from_str_is_case_insensitive_without_help() {
        assert_eq!("MD5".parse::<HashAlg>().unwrap(), HashAlg::Md5);
        assert_eq!("Md5".parse::<HashAlg>().unwrap(), HashAlg::Md5);
        assert_eq!("SHA256".parse::<HashAlg>().unwrap(), HashAlg::Sha256);
        assert_eq!("Sha256".parse::<HashAlg>().unwrap(), HashAlg::Sha256);
    }

    /// An unknown name is refused, and the refusal names the offender rather
    /// than echoing it back bare — the caller supplies what would have been right.
    #[test]
    fn an_unknown_name_is_refused_and_named() {
        let err = "blake3".parse::<HashAlg>().unwrap_err();
        assert_eq!(err.to_string(), "unsupported hash 'blake3'");
        // Not part of the error: the supported list belongs to the message that
        // frames the refusal, and carrying it here too would be two spellings.
        assert!(!err.to_string().contains("md5"));
    }

    // -- AlgoSet --------------------------------------------------------------

    /// Every subset of the enum, so "for all sets" is a real claim rather than
    /// "for the two sets somebody thought of".
    fn subsets() -> Vec<AlgoSet> {
        let mut out = vec![AlgoSet::NONE];
        for a in HashAlg::ALL {
            let mut next = out.clone();
            for s in &mut next {
                s.insert(a);
            }
            out.extend(next);
        }
        out
    }

    /// **The canonical-order claim**, which is what every downstream consumer of
    /// `AlgoSet::iter` relies on. A set has no order of its own, so if this ever
    /// yields anything but `HashAlg::ALL` order then a `--why` tag or a coverage
    /// message becomes a function of a `HashMap`'s iteration order.
    #[test]
    fn iteration_is_always_canonical_order() {
        for set in subsets() {
            let got: Vec<HashAlg> = set.iter().collect();
            let want: Vec<HashAlg> = HashAlg::ALL
                .into_iter()
                .filter(|a| set.contains(*a))
                .collect();
            assert_eq!(got, want, "{set:?} iterates in ALL order");
        }
        assert_eq!(subsets().len(), 4, "NONE, md5, sha256, both");
    }

    /// `len` is a bit count, so it cannot disagree with `iter`. Both properties
    /// are stated because `digest_count()` reports the first and the reason tag
    /// is built from the second.
    #[test]
    fn the_length_is_the_number_iterated() {
        for set in subsets() {
            assert_eq!(set.len(), set.iter().count(), "{set:?}");
        }
    }

    /// `HashMode::AnyOf` settles a pair with exactly one algorithm, so this is
    /// the shape the planner produces for every such path.
    #[test]
    fn a_single_algorithm_set_holds_exactly_that_one() {
        for a in HashAlg::ALL {
            let one = AlgoSet::one(a);
            assert_eq!(one.len(), 1);
            assert!(one.contains(a));
        }
    }

    /// `difference` is the "everything requested, minus what the row already
    /// holds" the planner needs, so its identities and its non-narrowing case
    /// are the whole contract.
    #[test]
    fn difference_subtracts_and_narrows_never_widens() {
        let both = AlgoSet::from_iter(HashAlg::ALL);
        assert_eq!(both.difference(AlgoSet::NONE), both, "identity");
        assert_eq!(AlgoSet::NONE.difference(AlgoSet::NONE), AlgoSet::NONE);
        assert!(
            AlgoSet::NONE.difference(both).is_empty(),
            "nothing asked for, nothing left"
        );
        assert_eq!(both.difference(AlgoSet::one(HashAlg::Md5)).len(), 1);
        assert!(
            both.difference(both).is_empty(),
            "everything already held, nothing owed"
        );
        // An `other` outside `self` cannot widen the result.
        assert_eq!(
            AlgoSet::one(HashAlg::Md5).difference(both),
            AlgoSet::NONE,
            "a set cannot gain a member by subtraction"
        );
    }

    /// **`Display` honours the width in the format spec.** A `Display` impl that
    /// writes with `write_str` prints the string and silently drops the column,
    /// which is how a coverage table loses its alignment with nothing failing.
    #[test]
    fn display_pads_to_the_width_in_the_format_spec() {
        for a in HashAlg::ALL {
            assert_eq!(format!("{a:>12}|"), format!("{:>12}|", a.as_str()));
            assert_eq!(format!("{a:<12}|"), format!("{:<12}|", a.as_str()));
            assert_eq!(format!("{a:012}|"), format!("{:012}|", a.as_str()));
            assert_eq!(
                format!("{a:<12} 0/1"),
                format!("{:<12} 0/1", a.as_str()),
                "and it is wider than the name, which is the only case that can tell"
            );
        }
    }

    /// **The memory claim, as an assertion.** A cache row used to be a
    /// `HashMap<String, Vec<u8>>` — five allocations per row, one row per file, held
    /// twice during phase C. This pins the replacement's size so a later edit that
    /// adds a `HashMap` or a `Vec` field fails here rather than quietly costing 25–50 MB
    /// on a 200k-file tree.
    ///
    /// Derived, not guessed: a `Box<[T]>` fat pointer (16), plus `Option<[u8; 16]>` at
    /// 17 and `Option<[u8; 32]>` at 33 — an `Option` over an array has no niche to
    /// spare, so it costs a discriminant byte. Alignment rounds the total up to 72.
    #[test]
    fn a_digest_set_is_seventy_two_bytes_and_allocates_nothing() {
        assert_eq!(std::mem::size_of::<DigestSet>(), 72);
        assert!(
            DigestSet::default().foreign.is_empty(),
            "the foreign slice is empty and points nowhere"
        );
    }

    /// Round-trip through `insert_named` / `get` / `get_named`, for every subset of
    /// the known algorithms. The identity is `get` against `get_named` — a row that
    /// can be found one way and not the other is a row some caller will misread.
    #[test]
    fn every_subset_of_the_known_algorithms_round_trips() {
        for set in subsets() {
            let mut d = DigestSet::default();
            for a in set.iter() {
                d.insert_named(a.as_str(), &vec![0xABu8; a.digest_len()]);
            }
            assert_eq!(d.len(), set.len(), "{set:?}");
            for a in HashAlg::ALL {
                let want = set.contains(a).then(|| vec![0xABu8; a.digest_len()]);
                assert_eq!(d.get(a), want.as_deref(), "{set:?} get({a:?})");
                assert_eq!(d.get_named(a.as_str()), want.as_deref(), "{set:?} named");
            }
            assert_eq!(
                d.iter().count(),
                set.len(),
                "{set:?}: iter sees every known digest and nothing else"
            );
            assert_eq!(d.is_empty(), set.is_empty());
        }
    }

    /// **The foreign arm, and the guarantee it exists for.** A name this build has
    /// never heard of must be findable by name, must not be mistaken for a known
    /// algorithm, and must not stop the known ones being found.
    #[test]
    fn an_unknown_algorithm_is_carried_verbatim_and_found_by_name() {
        let mut d = DigestSet::default();
        d.insert_named("blake3", b"\x07\x07\x07\x07");
        d.insert_named("md5", &[1u8; 16]);

        assert_eq!(d.get_named("blake3"), Some(&b"\x07\x07\x07\x07"[..]));
        assert_eq!(
            d.get(HashAlg::Md5),
            Some(&[1u8; 16][..]),
            "and it does not disturb the known slots"
        );
        assert_eq!(
            d.iter().count(),
            1,
            "iter reports known algorithms only -- a caller that wants the rest asks"
        );
        let foreign: Vec<_> = d.foreign_iter().collect();
        assert_eq!(foreign, vec![("blake3", &b"\x07\x07\x07\x07"[..])]);
        assert_eq!(
            d.len(),
            2,
            "and len counts it, because it is part of the row"
        );
    }

    /// **A known name with an unexpected length is still a digest.** A corrupt or
    /// hand-written row that says `md5` with eight bytes must decode, must compare
    /// byte-wise against whatever else claims to be an md5, and must therefore report
    /// a difference rather than be refused or silently accepted.
    ///
    /// This is exactly what a `HashMap` did: `x != y` on two unequal `Vec`s. Routing it
    /// to `foreign` preserves that; rejecting it would turn a reported difference into a
    /// hard failure.
    #[test]
    fn a_known_name_with_the_wrong_length_routes_to_foreign() {
        let mut d = DigestSet::default();
        d.insert_named("md5", b"\x01\x02\x03\x04\x05\x06\x07\x08");

        assert_eq!(
            d.get(HashAlg::Md5),
            None,
            "it is not this algorithm's digest"
        );
        assert_eq!(
            d.get_named("md5"),
            Some(&b"\x01\x02\x03\x04\x05\x06\x07\x08"[..]),
            "but it is still the digest this row holds under that name"
        );
        assert_eq!(d.foreign_iter().count(), 1);
        assert_eq!(d.len(), 1);
        assert_ne!(
            d,
            DigestSet::default(),
            "and it differs from a row that has nothing, which is the comparison that reports it"
        );
    }

    /// `merge` is the write rule: computed digests win, everything else rides along.
    /// Stated over all three shapes at once — a name only one side holds, a name both
    /// hold, and a foreign name — because "rides along" and "wins" are two different
    /// promises and a merge that got one right could still get the other wrong.
    #[test]
    fn merging_keeps_what_was_not_recomputed_and_prefers_what_was() {
        let mut prior = DigestSet::default();
        prior.insert_named("sha256", &[2u8; 32]);
        prior.insert_named("blake3", &[7u8; 32]);

        let mut computed = DigestSet::default();
        computed.insert_named("md5", &[1u8; 16]);
        computed.insert_named("sha256", &[9u8; 32]);
        computed.insert_named("blake3", &[8u8; 32]);

        let mut row = prior.clone();
        row.merge(&computed);

        assert_eq!(
            row.get(HashAlg::Md5),
            Some(&[1u8; 16][..]),
            "newly computed"
        );
        assert_eq!(
            row.get(HashAlg::Sha256),
            Some(&[9u8; 32][..]),
            "recomputed: the new value wins"
        );
        assert_eq!(
            row.get_named("blake3"),
            Some(&[8u8; 32][..]),
            "and a foreign digest merges like any other"
        );
        assert_eq!(row.foreign_iter().count(), 1, "not appended beside itself");

        // The other direction is the real invariant: merging something empty changes
        // nothing, which is what a `--hash none` scan writes.
        let mut untouched = prior.clone();
        untouched.merge(&DigestSet::default());
        assert_eq!(untouched, prior);
    }

    /// **The memory claim, as an assertion.** One `AlgoSet` sits in
    /// `HashPlan.by_rel` for every path the run will read, and one in
    /// `Required.by_rel` for every undecided pair — and under `--hash-all-of`
    /// every entry of the latter is the same value. As a `Vec` that is a heap
    /// allocation per path holding a fresh clone of the run-wide request; as two
    /// bytes it is part of a map value that already exists.
    #[test]
    fn a_set_is_two_bytes() {
        assert_eq!(std::mem::size_of::<AlgoSet>(), 2);
    }

    /// `Debug` is a log-line contract. `info!(algos = ?algos)` prints in the
    /// span of all four subcommands, and it renders `["md5", "sha256"]` today; a
    /// derive on either type would rewrite every one of those lines to
    /// `[Md5, Sha256]` without failing a single test elsewhere.
    #[test]
    fn debug_renders_the_spellings_the_logs_already_carry() {
        assert_eq!(format!("{:?}", AlgoSet::NONE), "[]");
        assert_eq!(format!("{:?}", AlgoSet::one(HashAlg::Md5)), "[\"md5\"]");
        assert_eq!(
            format!("{:?}", AlgoSet::from_iter(HashAlg::ALL)),
            "[\"md5\", \"sha256\"]"
        );
        assert_eq!(format!("{:?}", HashAlg::Md5), "\"md5\"");
    }

    /// A repeated algorithm is **one** request, however the flags were typed.
    ///
    /// The non-adjacent case comes first because it is the one that used to slip
    /// through: `Vec::dedup` only collapses *consecutive* repeats, so
    /// `md5 sha256 md5` survived as three algorithms and reached
    /// `HashPlan::digest_count` as a promise of three digests.
    #[test]
    fn a_repeated_algorithm_is_one_request() {
        assert_eq!(
            parse(&["md5", "sha256", "md5"]),
            vec![HashAlg::Md5, HashAlg::Sha256]
        );
        assert_eq!(
            parse(&["md5", "md5", "sha256", "md5", "sha256"]),
            vec![HashAlg::Md5, HashAlg::Sha256]
        );
    }

    /// First-seen order is the order kept. **This is the property `pick_one`
    /// depends on** — `--hash-any-of`'s tie-break is the user's flag order, so a
    /// dedup that sorted, or that kept the *last* occurrence, would change which
    /// algorithm a pair is settled by.
    #[test]
    fn first_seen_order_is_the_order_kept() {
        assert_eq!(
            parse(&["sha256", "md5", "sha256"]),
            vec![HashAlg::Sha256, HashAlg::Md5]
        );
        assert_eq!(
            parse(&["sha256", "md5"]),
            vec![HashAlg::Sha256, HashAlg::Md5]
        );
    }

    /// The adjacent case `Vec::dedup` already handled, pinned so the fix above
    /// cannot be read as "dedup was always broken".
    #[test]
    fn adjacent_repeats_collapse() {
        assert_eq!(parse(&["md5", "md5"]), vec![HashAlg::Md5]);
        assert_eq!(
            parse(&["sha256", "md5", "md5"]),
            vec![HashAlg::Sha256, HashAlg::Md5]
        );
    }

    /// `parse_any_of` delegates, so it inherits the dedup. Worth pinning because
    /// it is the mode where a duplicate mattered most: the flag asks for *one*
    /// algorithm, and a repeated one had nothing to change.
    #[test]
    fn any_of_dedupes_through_the_delegate() {
        assert_eq!(parse_any(&["md5", "md5"]), vec![HashAlg::Md5]);
        assert_eq!(
            parse_any(&["md5", "sha256", "md5"]),
            vec![HashAlg::Md5, HashAlg::Sha256]
        );
    }

    /// The refusal names the offender and the alternatives, in the wording the
    /// README documents. `UnknownHash` carries only the name and this framing
    /// supplies the rest, so there is exactly one place that sentence is written.
    #[test]
    fn an_unsupported_name_is_refused_with_the_supported_list() {
        let err = parse_hash_list(&["bogus".to_string()]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "unsupported hash 'bogus' (supported: md5,sha256 or none)"
        );
        // The first unsupported name is the one named, whatever follows it.
        let err = parse_hash_list(&[
            "md5".to_string(),
            "nope".to_string(),
            "also-nope".to_string(),
        ])
        .unwrap_err();
        assert!(
            err.to_string().contains("'nope'"),
            "the first refusal, not the last: {err}"
        );
    }

    /// **The property the whole split rests on.** `copy_and_digest` folds the
    /// digest into the copy loop and `hash_file` keeps its own; if those two
    /// ever disagreed, every copy in a real sync would fail its verify — or,
    /// worse, the copy path would stop being checked at all.
    ///
    /// Fed in every awkward shape on purpose: one byte at a time (the split
    /// lands mid-digest), the whole file at once, and a chunk larger than the
    /// file. The digest must not care how the bytes were divided, and only the
    /// first case proves it is reading `update` and not something incidental.
    #[test]
    fn a_digest_is_the_same_however_the_bytes_were_split() {
        let dir = girpr_tempdir::TempDir::new("girsync_hash", "split");
        // Longer than `CHUNK` is not practical here, but it must exceed one
        // buffer's worth of a *small* file to make "chunk bigger than the file"
        // a real case rather than a trivially-true one.
        let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let path = write(&dir, "f.bin", &body);

        for set in [
            algos(&["md5", "sha256"]),
            algos(&["md5"]),
            algos(&["sha256"]),
        ] {
            let reference = hash_file(&path, &set).unwrap();

            let mut by_byte = Digests::new(&set);
            for b in &body {
                by_byte.update(std::slice::from_ref(b));
            }
            assert_eq!(
                by_byte.finish(),
                reference,
                "{set:?}: one byte per update is the split hash_file never uses"
            );

            let mut whole = Digests::new(&set);
            whole.update(&body);
            assert_eq!(whole.finish(), reference, "{set:?}: one update");

            let mut oversized = Digests::new(&set);
            oversized.update(&body[..body.len() / 2]);
            oversized.update(&body[body.len() / 2..]);
            assert_eq!(oversized.finish(), reference, "{set:?}: uneven halves");
        }
    }

    /// An empty file is the degenerate case of the same property: a digest loop
    /// that never calls `update` has still absorbed every byte there is.
    #[test]
    fn an_empty_file_hashes_to_the_digest_of_nothing() {
        let dir = girpr_tempdir::TempDir::new("girsync_hash", "empty");
        let path = write(&dir, "f.bin", b"");

        let set = algos(&["md5", "sha256"]);
        let never_fed = Digests::new(&set);
        assert_eq!(never_fed.finish(), hash_file(&path, &set).unwrap());
    }

    /// `Digests::new(&[])` must be the inert object, because `--hash none` is
    /// the mode where a copy never builds one. A context built and thrown away is
    /// the per-file allocation [`hash_file`] is written to avoid.
    #[test]
    fn no_algorithms_asked_for_means_no_context_and_no_digest() {
        let mut d = Digests::new(&AlgoSet::NONE);
        d.update(b"these bytes must not be absorbed by anything");
        assert!(d.finish().is_empty());
        // ...and `hash_file` must agree, without opening the file at all.
        let dir = girpr_tempdir::TempDir::new("girsync_hash", "absent");
        assert!(
            hash_file(&dir.join("not-there.bin"), &AlgoSet::NONE)
                .unwrap()
                .is_empty(),
            "an empty request is answered without touching the disk"
        );
    }
}
