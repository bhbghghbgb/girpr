//! [`DigestSet`]: the digests one cache row carries, as inline slots.

use super::algo::HashAlg;

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
    pub(super) md5: Option<[u8; 16]>,
    pub(super) sha256: Option<[u8; 32]>,
    /// Algorithms this build does not know, carried verbatim. Empty in practice.
    /// See the type's docs.
    pub(super) foreign: Box<[(Box<str>, Vec<u8>)]>,
}

impl DigestSet {
    /// How many digests this holds, foreign ones included.
    ///
    /// Not `const`, unlike [`super::AlgoSet::len`]: counting `foreign` means dereferencing a
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
    /// found under any ASCII-case spelling of its name, which is the same rule
    /// the algorithm's own parser applies to the command line.
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
        if let Ok(a) = name.parse::<HashAlg>() {
            // Not `else`: a known name with an unexpected length is in `foreign`, and
            // `get` cannot see it.
            return self.get_algo(a);
        }
        // An unknown name is exact, and only ever the spelling it was given.
        self.foreign
            .iter()
            .find(|(n, _)| &**n == name)
            .map(|(_, d)| d.as_slice())
    }

    /// The digest a known algorithm's **foreign** entry carries, if any.
    ///
    /// The arm is scanned for the first entry whose **stored name parses to `a`**,
    /// and that entry's digest is returned. Parsing is the algorithm's own
    /// parser, so the match is ASCII-case-insensitive and the fold is ASCII case
    /// only — the same rule the command line and the path key follow. A name
    /// that does not parse to an algorithm never matches: there is nothing to
    /// match it against, so an unknown name cannot be pulled into a known
    /// algorithm's answer.
    ///
    /// **First match wins.** More than one entry can resolve to the same
    /// algorithm — `MD5` and `md5`, both with an unexpected length — and the arm
    /// is a slice, so the answer is a property of the row rather than of an
    /// iteration order. Both lookups go through here, so they cannot disagree
    /// about which entry that is.
    fn foreign_known(&self, a: HashAlg) -> Option<&[u8]> {
        self.foreign
            .iter()
            .find(|(name, _)| name.parse::<HashAlg>().ok() == Some(a))
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
