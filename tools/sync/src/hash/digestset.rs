//! [`DigestSet`]: the digests one cache row carries, as inline slots.

use super::algo::HashAlg;

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
/// during phase C. This is **80 bytes and zero allocations** for a normal row: two
/// fixed-size optional arrays, plus a vector that is empty and allocates nothing
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
    /// Algorithms this build does not know, carried verbatim. Empty in practice.
    /// See the type's docs.
    ///
    /// A **vector**, so an entry is changed where it stands: a digest that
    /// arrives under a name the arm already holds replaces that entry, and
    /// the arm is not rebuilt around it. The boxed slice this replaced could
    /// only be written by rebuilding it, so every insert — a replacement as
    /// much as an addition — copied the whole arm and allocated a new one.
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
        self.md5.is_some() as usize + self.sha256.is_some() as usize + self.foreign.len()
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
    /// zero". That distinction is load-bearing: [`crate::diff::hashes_differ`] is
    /// silent on a missing digest, and a zero digest substituted for `None` would turn
    /// a false clean back on. A known name sitting in `foreign` also reads as `None`
    /// here — see [`Self::get_algo`] for the typed lookup that finds it.
    pub fn get(&self, a: HashAlg) -> Option<&[u8]> {
        match a {
            HashAlg::Md5 => self.md5.as_ref().map(|d| &d[..]),
            HashAlg::Sha256 => self.sha256.as_ref().map(|d| &d[..]),
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
    /// the command line and the path key follow, and the same one
    /// [`HashAlg`]'s own parser applies.
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
    /// codec decodes through this and nothing else, so "a known name with a wrong
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
    /// it clones the prior row into `self` and passes this run's computed digests as
    /// `other`, so a recomputed digest replaces the prior one while everything the
    /// prior row held that this run did not compute rides along.
    /// See `crate::effective::merge_row`.
    pub fn merge(&mut self, other: &DigestSet) {
        if let Some(d) = other.md5 {
            self.md5 = Some(d);
        }
        if let Some(d) = other.sha256 {
            self.sha256 = Some(d);
        }
        if !other.foreign.is_empty() {
            // The arm is a vector, so each entry of `other` is written where it
            // belongs: over the entry of the same name when the arm holds one,
            // and at the end when it does not.
            //
            // The loop is skipped when `other` holds no foreign entry: a row
            // with no foreign entry is the overwhelmingly normal shape, and
            // the loop would walk the arm for nothing.
            for (name, digest) in &other.foreign {
                match self
                    .foreign
                    .iter_mut()
                    .find(|(n, _)| same_spelling(n, name))
                {
                    Some((_, d)) => {
                        d.clear();
                        d.extend_from_slice(digest);
                    }
                    None => self.foreign.push((name.clone(), digest.clone())),
                }
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
