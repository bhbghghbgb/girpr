//! [`DigestSet`]: the digests one cache row carries, as inline slots.
//!
//! ## The fold here is over a name, not over a path
//!
//! This module folds an **algorithm name** with an ASCII lowercase, and the
//! fold answers one question: *does this stored name name this algorithm?* A
//! row may hold `MD5` where the request says `md5`, and the lookup has to find
//! it.
//!
//! **It is not one of the tool's path folds.** Those live in girsync's util module,
//! and each of them answers a question about paths: `path_key` is the key
//! every relative-path index uses and answers which bucket of a run's map a
//! path belongs to, `component_eq` asks whether two spellings open the same
//! place on disk, and `ascii_case_eq` states the cache reservation's
//! comparison. The fold here has no path in it at all, so it states its own
//! rule rather than borrowing one of theirs.

use super::algo::HashAlg;
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeSeq};

/// Whether two names are the same name.
///
/// The foreign arm matches a name **exactly** — the spelling it was stored
/// under and no other — so this is byte equality and nothing else.
///
/// **This is the one place the exact-match rule is written.** The arm is
/// matched by two rules and they answer different questions: a known
/// algorithm is found under any ASCII-case spelling of its name, and an
/// unknown name is matched exactly — this one. The three places that match a
/// name against the arm exactly all come through here: the lookup that
/// finds a digest by name ([`DigestSet::get_named`]) for a name no algorithm
/// answers to, and the two writers that put one there
/// ([`DigestSet::insert_named`] and [`DigestSet::merge`]). So they cannot
/// drift on what "the same name" means: the arm holds one entry per
/// spelling, and a repeat of a spelling replaces that entry rather than
/// adding a second one beside it.
///
/// A one-line helper earns its place for exactly that reason. Inlined, the
/// rule would be spelled at three call sites, and a second spelling of a
/// rule is a rule that can drift — and this one drifts quietly: one writer
/// would replace the entry under a name and another would add a second one
/// beside it, and the row would hold two entries under one name without
/// anything saying so.
///
/// If the arm's exact-match rule ever stops being exact, this is where it
/// changes.
fn same_spelling(a: &str, b: &str) -> bool {
    a == b
}

/// The digests one cache row carries, as inline slots rather than a map.
///
/// ## Not `Digests`
///
/// [`super::Digests`] is the streaming *context* that absorbs bytes; this is the *result*.
/// Different jobs, and they must not be merged: the context is per-file-per-phase and
/// thrown away, this one lives in the cache and in both sides' effective maps.
///
/// ## Why not a `HashMap<String, Vec<u8>>`
///
/// The map version was five allocations per row — the table, two `String` keys, two
/// `Vec<u8>` digests — and a row is allocated once per file in phase A and held twice
/// during phase C. This is **88 bytes and zero allocations** for a normal row: three
/// fixed-size optional arrays (16, 32 and 8 bytes of digest), plus a vector that is
/// empty and allocates nothing unless something needs carrying. `size_of` is asserted
/// in this module's tests.
///
/// ## The foreign arm is not optional
///
/// `HashAlg` is a closed enum, and this is where that stops being the whole truth. A
/// cache written by a newer build may carry a digest under a name this one has never
/// heard of, and `crate::wire` keeps names inline precisely so that row stays
/// readable. `foreign` is where such a name lives in memory, and it is **carried,
/// merged and written back** rather than merely decoded — a scan that dropped it would
/// lose the row's contents on the next write.
///
/// A *known* name with an *unexpected* length lands here too, and must: today such a
/// row lives in a `HashMap` and `x != y` compares two unequal `Vec`s, reporting a
/// difference. Refusing it instead would turn a corrupt row into a hard failure rather
/// than a reported difference, which is not what this format has always done.
///
/// ## The arm holds no room beyond its entries
///
/// The arm is a vector, and a vector grows in steps: a row that carries one
/// unknown name holds room for four. Two things keep that from costing a
/// caller anything it keeps:
///
/// - **A clone is sized for its length.** `Vec::clone` allocates exactly the
///   length today, so a derive would give the same answer — but the claim
///   this crate makes about a row is that it holds no more than it says, and
///   it should not rest on what `std` happens to do with a vector's spare
///   capacity. The clone is the row a caller keeps: the effective map holds
///   one per path, for the whole phase.
/// - **A finished row gives the room back.** The row the codec decoded and
///   the row a merge built are finished when they are made, and both give
///   the arm its room back there — not on every insert, which would be a
///   reallocation per insert and the cost the vector was chosen to avoid.
#[derive(Default, PartialEq, Eq)]
pub struct DigestSet {
    pub(super) md5: Option<[u8; 16]>,
    pub(super) sha256: Option<[u8; 32]>,
    /// XXH64's 8 raw bytes, in the big-endian order its hex spelling uses.
    pub(super) xxhash64: Option<[u8; 8]>,
    /// Algorithms this build does not know, carried verbatim. Empty in practice.
    /// See the type's docs.
    ///
    /// A **vector**, so an entry is changed where it stands: a digest that
    /// arrives under a name the arm already holds replaces that entry, and
    /// the arm is not rebuilt around it. The boxed slice this replaced could
    /// only be written by rebuilding it, so every insert — a replacement as
    /// much as an addition — copied the whole arm and allocated a new one.
    ///
    /// **The arm owns its entries.** A name is a boxed string and a digest a
    /// byte vector, both heap buffers the row holds, and a merge that adds
    /// an entry takes the other row's entry over rather than copying it.
    /// See [`DigestSet::merge`].
    ///
    /// The arm holds no room beyond its entries; see the type's docs.
    pub(super) foreign: Vec<(Box<str>, Vec<u8>)>,
}

/// A clone carries no room beyond the arm's entries.
///
/// Written out rather than derived, for the reason the type's docs give: the
/// property is the crate's, and a derive would leave it resting on what
/// `std` happens to do with a vector's spare capacity.
impl Clone for DigestSet {
    fn clone(&self) -> Self {
        Self {
            md5: self.md5,
            sha256: self.sha256,
            xxhash64: self.xxhash64,
            // Collected rather than cloned entry by entry: the iterator knows
            // how many entries there are, so the vector is sized for exactly
            // that many.
            foreign: self
                .foreign
                .iter()
                .map(|(name, digest)| (name.clone(), digest.clone()))
                .collect(),
        }
    }
}

impl DigestSet {
    /// How many digests this holds, foreign ones included.
    ///
    /// Not `const`, unlike [`super::AlgoSet::len`]: counting `foreign` means reading
    /// the arm, and a count that silently ignored it would be wrong for exactly the
    /// rows forward compatibility exists to preserve.
    pub fn len(&self) -> usize {
        self.md5.is_some() as usize
            + self.sha256.is_some() as usize
            + self.xxhash64.is_some() as usize
            + self.foreign.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Give the arm back the room it is not using.
    ///
    /// **The arm holds no room beyond its entries.** A vector grows in steps,
    /// so a row that carries one unknown name holds room for four; this is
    /// the call that gives the room back.
    ///
    /// It is called where a row is *finished* — the row the codec decoded and
    /// the row a merge built — because that is the only point at which the
    /// arm's length is known to be final. Calling it on every insert would be
    /// a reallocation per insert, which is the cost the vector was chosen to
    /// avoid.
    pub(crate) fn shrink_to_fit(&mut self) {
        self.foreign.shrink_to_fit();
    }

    /// The arm's capacity, for a test that asks how much room it holds.
    #[cfg(test)]
    pub(crate) fn foreign_capacity(&self) -> usize {
        self.foreign.capacity()
    }

    /// The digest for a known algorithm.
    ///
    /// `None` means "this row has no digest under that name", **not** "the digest is
    /// zero". That distinction is load-bearing: `girsync::diff::hashes_differ` is
    /// silent on a missing digest, and a zero digest substituted for `None` would turn
    /// a false clean back on. A known name sitting in `foreign` also reads as `None`
    /// here — see [`Self::get_algo`] for the typed lookup that finds it.
    pub fn get(&self, a: HashAlg) -> Option<&[u8]> {
        match a {
            HashAlg::Md5 => self.md5.as_ref().map(|d| &d[..]),
            HashAlg::Sha256 => self.sha256.as_ref().map(|d| &d[..]),
            HashAlg::Xxhash64 => self.xxhash64.as_ref().map(|d| &d[..]),
        }
    }

    /// The digest for `a`, including a malformed known-name entry carried in
    /// `foreign` (a known name with an unexpected length).
    ///
    /// The typed counterpart to [`Self::get_named`]: a caller that already has a
    /// `HashAlg` should not have to spell it back into a string. The fallback is
    /// not optional — a known name whose digest length was unexpected is in
    /// `foreign`, and [`Self::get`] cannot see it.
    ///
    /// The `foreign` match is by **name**, not by spelling: a known algorithm is
    /// found under any ASCII-case spelling of its name, which is the same ASCII
    /// case rule the algorithm's own parser applies to the command line.
    ///
    /// The fold is over that name alone — it answers *does this stored name name
    /// this algorithm?* — and it never touches a path. The crate's path folds
    /// are elsewhere, in girsync's util module.
    pub fn get_algo(&self, a: HashAlg) -> Option<&[u8]> {
        self.get(a).or_else(|| self.foreign_known(a))
    }

    /// The digest under *any* name, known or not.
    ///
    /// This is what availability tests and a name-keyed comparison need: a row written
    /// by a newer build holds `blake3`, and asking for it by name has to find it.
    ///
    /// A **known** name is answered the way [`Self::get_algo`] answers it, so the
    /// two lookups cannot disagree about a row that holds `MD5` where the request
    /// says `md5`. An **unknown** name is matched **exactly**: it is not an
    /// algorithm, so nothing this build knows how to ask for could mean it.
    pub fn get_named(&self, name: &str) -> Option<&[u8]> {
        if let Some(a) = HashAlg::from_name(name) {
            // Not `else`: a known name with an unexpected length is in `foreign`, and
            // `get` cannot see it.
            return self.get_algo(a);
        }
        // An unknown name is exact, and only ever the spelling it was given.
        self.foreign
            .iter()
            .find(|(n, _)| same_spelling(n, name))
            .map(|(_, d)| d.as_slice())
    }

    /// The digest a known algorithm's **foreign** entry carries, if any.
    ///
    /// The arm is scanned for the first entry whose **stored name names `a`**,
    /// and that entry's digest is returned. The stored name is compared with the
    /// algorithm's canonical spelling, folding ASCII case only — the same rule
    /// the command line follows, and the same one [`HashAlg`]'s own parser
    /// applies.
    ///
    /// **The fold is over an algorithm name, and it answers one question:**
    /// *does this stored name name this algorithm?* It is not one of the
    /// crate's path folds, and it answers no question about paths: the path
    /// key (the path key) asks which bucket of a run's map a path
    /// belongs to, the identity rule (the component-identity rule) asks whether
    /// two spellings open the same place on disk, and the cache reservation's
    /// comparison (the cache-reservation comparison) asks whether a component is
    /// one of the tool's own names. This one has no path in it.
    ///
    /// The comparison allocates nothing. It is the resolver's, and the
    /// resolver is a `const fn`: a const fn may not allocate, so the
    /// comparison cannot. A **match** has no refusal to build, and a name
    /// that names no algorithm matches nothing, so there is nothing to keep
    /// either: the only spelling of a name that ever needs a copy is the one
    /// a refusal has to name.
    ///
    /// **First match wins.** More than one entry can resolve to the same
    /// algorithm — `MD5` and `md5`, both with an unexpected length — and the arm
    /// is an ordered list, so the answer is a property of the row rather than of
    /// an iteration order. Both lookups go through here, so they cannot disagree
    /// about which entry that is.
    fn foreign_known(&self, a: HashAlg) -> Option<&[u8]> {
        self.foreign
            .iter()
            .find(|(name, _)| HashAlg::from_name(name) == Some(a))
            .map(|(_, digest)| digest.as_slice())
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
    /// wire deserializes through this and nothing else, so "a known name with a wrong
    /// length is still a digest, not an error" cannot be spelled two ways.
    ///
    /// The name is resolved the same way the lookups resolve one, so a name that
    /// names an algorithm here names it there.
    pub fn insert_named(&mut self, name: &str, digest: &[u8]) {
        let known = HashAlg::from_name(name);
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
        if known == Some(HashAlg::Xxhash64)
            && let Ok(d) = <[u8; 8]>::try_from(digest)
        {
            self.xxhash64 = Some(d);
            return;
        }
        // The arm is a vector, so the entry is changed where it stands: a
        // repeat of a spelling replaces that entry, and a spelling that is
        // not there yet is appended. Nothing is copied around it.
        match self
            .foreign
            .iter_mut()
            .find(|(n, _)| same_spelling(n, name))
        {
            Some((_, d)) => {
                // The digest's own buffer is reused when it is big enough, so
                // replacing a digest allocates nothing.
                d.clear();
                d.extend_from_slice(digest);
            }
            None => self.foreign.push((name.into(), digest.to_vec())),
        }
    }

    /// Fold `other` in, with **`other` winning** on every name both hold.
    ///
    /// That direction is the merge-write rule, and it is what `merge_row` relies on:
    /// it clones the prior row into `self` and hands this run's computed digests
    /// over as `other`, so a recomputed digest replaces the prior one while
    /// everything the prior row held that this run did not compute rides along.
    /// See the merge-row rule in girsync's effective module.
    ///
    /// **`other` is taken over, not borrowed.** The arm owns its entries — a
    /// name is a boxed string and a digest a byte vector, both heap buffers —
    /// so a name `other` holds and this one does not is *added* by moving the
    /// other side's entry in: the added entry is the other side's entry, and
    /// it costs no allocation at all. A name both hold is replaced where it
    /// stands, and the arm's own digest buffer is refilled from the other's
    /// bytes, so a replacement costs no allocation either.
    ///
    /// The insert path ([`Self::insert_named`]) has to copy, because it is
    /// handed a name and a digest it does not own. The merge path is the one
    /// place where the crate holds the other whole row, so it need not.
    ///
    /// The rule that says *which* entry is written is the arm's own and is
    /// unchanged: an entry is replaced only under the spelling it was stored
    /// with, and a spelling the arm does not hold is added beside the others.
    /// See `same_spelling`.
    pub fn merge(&mut self, other: DigestSet) {
        if let Some(d) = other.md5 {
            self.md5 = Some(d);
        }
        if let Some(d) = other.sha256 {
            self.sha256 = Some(d);
        }
        if let Some(d) = other.xxhash64 {
            self.xxhash64 = Some(d);
        }
        // The arm is a vector, so each entry of `other` is written where it
        // belongs: over the entry of the same name when the arm holds one,
        // and at the end when it does not.
        //
        // `other` is consumed, so an entry that is added is moved in and the
        // arm keeps the buffers it came with: no name is copied and no digest
        // is copied. A row with no foreign entry is the overwhelmingly normal
        // shape, and the loop then does nothing at all.
        for (name, digest) in other.foreign {
            // The entry is found by position rather than by reference, so the
            // name is free to be moved into the arm below.
            let at = self
                .foreign
                .iter()
                .position(|(n, _)| same_spelling(n, &name));
            match at {
                Some(i) => {
                    // The arm's own digest buffer is reused, so replacing a
                    // digest allocates nothing.
                    let (_, d) = &mut self.foreign[i];
                    d.clear();
                    d.extend_from_slice(&digest);
                }
                None => self.foreign.push((name, digest)),
            }
        }
        // The row is finished here, so the arm gives back the room it grew
        // into: a vector grows in steps, and a row that carries one unknown
        // name would otherwise hold room for four.
        self.shrink_to_fit();
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

/// The wire shape is a sequence of `(name, digest)` pairs: known algorithms
/// first in canonical order, then foreign names sorted by name. Both halves
/// are deterministic, so a row's bytes are reproducible for the same content.
///
/// This is deliberately **not** a derive. A derive would make the closed set of
/// known slots the whole schema and would drop an unrecognised name on decode;
/// the forward-compatibility rule says such a name is carried verbatim, and the
/// only place that rule is written is [`DigestSet::insert_named`]. Serialize and
/// deserialize both go through the set's own iterators and router so the rule
/// cannot be spelled a second way.
impl Serialize for DigestSet {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Foreign names are sorted so two rows carrying the same entries encode
        // to the same bytes regardless of the order they were inserted in — the
        // same determinism the old wire format had.
        let mut foreign: Vec<(&str, &[u8])> = self.foreign_iter().collect();
        foreign.sort_by_key(|(name, _)| *name);

        let mut seq = serializer.serialize_seq(Some(self.len()))?;
        for (algo, digest) in self.iter() {
            seq.serialize_element(&(algo.as_str(), digest))?;
        }
        for (name, digest) in foreign {
            seq.serialize_element(&(name, digest))?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for DigestSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let pairs = Vec::<(String, Vec<u8>)>::deserialize(deserializer)?;
        let mut set = DigestSet::default();
        for (name, digest) in pairs {
            // **Never reject an algorithm name.** This is the whole forward-compat
            // guarantee: a row written by a build that knows `blake3` has to stay
            // readable here, so an unrecognised name is carried under the exact
            // spelling it was written with. `insert_named` also routes a known
            // name with an unexpected digest length into `foreign`, which is what
            // a map did and what keeps a corrupt row a reported difference rather
            // than a hard failure.
            set.insert_named(&name, &digest);
        }
        // The row is finished here, so the arm gives back the room it grew into:
        // a vector grows in steps, and a row that carries one unknown name would
        // otherwise hold room for four.
        set.shrink_to_fit();
        Ok(set)
    }
}
