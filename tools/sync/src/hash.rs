//! Hash algorithm selection and streaming file digests.

use anyhow::{Context, Result, anyhow, bail};
use sha2::Digest;
use std::collections::HashMap;
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
/// an enum without giving that up. See [`crate::cache::codec`], which keeps names
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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
/// a user's ordering genuinely matters — [`crate::planner::pick_one`]'s `--hash-any-of`
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
    /// Build the contexts `algos` asks for. An empty list builds none, so
    /// [`update`](Self::update) does nothing and [`finish`](Self::finish)
    /// yields an empty map — the same answer [`hash_file`] gives for it.
    ///
    /// Borrowed comparison, not `algos.contains(&"md5".to_string())`. This runs
    /// once per file per phase C and once per copied file, and the `String`
    /// that form allocates exists only to be compared against borrowed data.
    pub fn new(algos: &[HashAlg]) -> Self {
        Self {
            md5ctx: algos.contains(&HashAlg::Md5).then(md5::Context::new),
            sha2ctx: algos.contains(&HashAlg::Sha256).then(sha2::Sha256::new),
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
    pub fn finish(self) -> HashMap<String, Vec<u8>> {
        let mut out = HashMap::new();
        if let Some(c) = self.md5ctx {
            out.insert("md5".into(), c.compute().0.to_vec());
        }
        if let Some(c) = self.sha2ctx {
            out.insert("sha256".into(), c.finalize().to_vec());
        }
        out
    }
}

/// Digest `path` with every requested algorithm in a single pass.
///
/// An empty `algos` returns an empty map without touching the file.
/// Values are **raw** digest bytes (16 for md5, 32 for sha256): the cache
/// stores them as-is instead of hex, halving hash disk cost.
pub fn hash_file(path: &Path, algos: &[HashAlg]) -> Result<HashMap<String, Vec<u8>>> {
    // Before any context is built: with nothing requested there is nothing to
    // digest, and opening the file at all would turn a stat-only decision into a
    // read. The empty map below is the whole answer.
    if algos.is_empty() {
        return Ok(HashMap::new());
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

    fn write(dir: &crate::testtemp::TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn algos(names: &[&str]) -> Vec<HashAlg> {
        names.iter().map(|s| s.parse().unwrap()).collect()
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

    /// **`Debug` is a log-line contract.** `info!(algos = ?algos)` prints in the
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
        let dir = crate::testtemp::TempDir::new("girsync_hash", "split");
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
        let dir = crate::testtemp::TempDir::new("girsync_hash", "empty");
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
        let mut d = Digests::new(&[]);
        d.update(b"these bytes must not be absorbed by anything");
        assert!(d.finish().is_empty());
        // ...and `hash_file` must agree, without opening the file at all.
        let dir = crate::testtemp::TempDir::new("girsync_hash", "absent");
        assert!(
            hash_file(&dir.join("not-there.bin"), &[])
                .unwrap()
                .is_empty(),
            "an empty request is answered without touching the disk"
        );
    }
}
