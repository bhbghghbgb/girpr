//! [`HashAlg`] and [`UnknownHash`]: the closed, command-line side of the hash
//! vocabulary.

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
    ///
    /// Visible to the whole `hash` module tree (`pub(super)` relative to `algo`)
    /// because `AlgoSet` now lives in its own sibling module and needs it.
    pub(super) const fn bit(self) -> u16 {
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
