//! The chunk-identity rule shared by the repair binary and the capture tool.
//!
//! A chunk is identified by its **digest and its decoded length**: the same MD5
//! with a different length names different bytes, so neither half alone is an
//! identity. This crate owns only the value types and the spelling rule around
//! that claim. It holds no selection policy, no collision handling and no
//! published bytes: each caller keeps its own index or set, its own first-match
//! order and its own artifact format.
//!
//! ## Two representations, one rule
//!
//! The programs deliberately keep different digest representations — girpr
//! parses a strict 16-byte [`Md5`], gircap publishes lowercase hex strings — so
//! the leaf offers both rather than collapsing them into one semantic digest
//! type:
//!
//! - [`IdentityKey`] is the **owned** byte key: a parsed digest plus a length.
//!   The repair path builds one once per local chunk and compares bytes from
//!   then on.
//! - [`ChunkIdentity`] is the **borrowed** view over a hex digest and a length.
//!   It allocates nothing per comparison, which is what a set-membership test
//!   over a manifest needs. Its derived equality is deliberately *spelling*
//!   equality: a caller that normalises at its decode boundary (gircap) can use
//!   it as a `Hash`/`Eq` key, while a caller that has not normalised uses
//!   [`ChunkIdentity::matches`] / [`ChunkIdentity::parse`] for a
//!   case-insensitive comparison.
//!
//! [`normalize_hex`] is the one way to lower a digest spelling at that decode
//! boundary. It lives here rather than in `girpr-proto` because the wire leaf
//! stays logic-free: renaming or normalising a field there changes dump bytes.
//! What each program *does* with a normalised digest — which fields it
//! publishes, and which chunk it fetches first on a collision — stays local.

use girpr_digest::Md5;
use girpr_proto::SophonChunk;

/// The owned identity key: parsed digest bytes plus the decoded length.
///
/// Parsing happens once at the caller's boundary; after that a key is compared
/// by bytes, so it is insensitive to the manifest's hex spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IdentityKey {
    /// The 16 MD5 bytes the manifest digest spells.
    pub digest: Md5,
    /// The decoded length; part of the identity, not an attribute of it.
    pub size: i64,
}

impl IdentityKey {
    /// The key for a digest that has already been parsed and a length.
    pub const fn new(digest: Md5, size: i64) -> Self {
        Self { digest, size }
    }
}

/// A borrowed view of a chunk's identity.
///
/// `(&str, i64)` is already `Hash + Eq`, so a caller asking "is this chunk in
/// that file" need not allocate a key per comparison. Equality and hashing are
/// on the **spelling** of the digest, exactly as on a tuple: a caller that has
/// normalised its manifests at the decode boundary gets case-insensitive
/// membership for free, and one that has not can use [`Self::matches`] or
/// [`Self::parse`], which parse the digest case-insensitively.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkIdentity<'a> {
    /// The digest as the manifest spells it.
    pub digest: &'a str,
    /// The decoded length; part of the identity, not an attribute of it.
    pub size: i64,
}

impl<'a> ChunkIdentity<'a> {
    /// A view over a digest spelling and a length.
    pub const fn new(digest: &'a str, size: i64) -> Self {
        Self { digest, size }
    }

    /// The identity of a decoded wire chunk.
    pub fn of(chunk: &'a SophonChunk) -> Self {
        Self::new(&chunk.uncompressed_md5, chunk.uncompressed_size)
    }

    /// Parse the borrowed spelling into the owned, byte-comparing key.
    ///
    /// Either hex case is accepted. A string that is not a 32-digit MD5 (the
    /// empty string above all) is not an identity: a chunk carrying one names
    /// no bytes and degrades to whatever "unknown" means for the caller.
    pub fn parse(&self) -> Option<IdentityKey> {
        Md5::parse_hex(self.digest)
            .ok()
            .map(|digest| IdentityKey::new(digest, self.size))
    }

    /// Does this view name the same chunk as `key`?
    ///
    /// The digest is compared case-insensitively by parsing it; the length
    /// must match exactly. A non-digest spelling matches nothing.
    pub fn matches(&self, key: IdentityKey) -> bool {
        self.size == key.size
            && Md5::parse_hex(self.digest).is_ok_and(|digest| digest == key.digest)
    }
}

/// Lowercase a digest spelling in place at a caller's decode boundary.
///
/// A digest is an identity, not a spelling: comparing two manifests that spell
/// one digest in different cases would make two programs disagree about the
/// same bytes. Normalising once where a document enters a program makes every
/// later key agree instead of relying on each comparison to remember. Live
/// manifests are already lowercase, so a string with no uppercase byte is left
/// completely untouched — no allocation on the common path.
pub fn normalize_hex(s: &mut String) {
    if s.bytes().any(|b| b.is_ascii_uppercase()) {
        *s = s.to_ascii_lowercase();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: &str = "d41d8cd98f00b204e9800998ecf8427e";
    const UPPER: &str = "D41D8CD98F00B204E9800998ECF8427E";

    fn chunk(md5: &str, size: i64) -> SophonChunk {
        SophonChunk {
            id: "c".into(),
            uncompressed_md5: md5.into(),
            offset: 0,
            compressed_size: size + 1,
            uncompressed_size: size,
            unknown_field6: 0,
            compressed_md5: "compressed".into(),
        }
    }

    /// The length is part of the identity: the same digest at a different size
    /// is a different claim about bytes.
    #[test]
    fn identity_is_the_digest_and_the_length() {
        let a = ChunkIdentity::new(EMPTY, 20);
        let b = ChunkIdentity::new(EMPTY, 21);
        assert_ne!(a, b);
        let c = ChunkIdentity::new("0cc175b9c0f1b6a831c399e269772661", 20);
        assert_ne!(a, c);
        assert_eq!(a, ChunkIdentity::new(EMPTY, 20));
    }

    /// The borrowed view reads the wire type without copying the digest.
    #[test]
    fn the_view_borrows_the_decoded_chunk() {
        let c = chunk(EMPTY, 690_719);
        let id = ChunkIdentity::of(&c);
        assert_eq!(id.digest, c.uncompressed_md5.as_str());
        assert_eq!(id.size, c.uncompressed_size);
        assert_eq!(
            id,
            ChunkIdentity::new(&c.uncompressed_md5, c.uncompressed_size)
        );
    }

    /// Parsing and matching fold case but keep the length exact, so a caller
    /// that has not normalised still agrees with one that has.
    #[test]
    fn parse_and_matches_are_case_insensitive_and_size_sensitive() {
        let lower_chunk = chunk(EMPTY, 20);
        let upper_chunk = chunk(UPPER, 20);
        let lower = ChunkIdentity::of(&lower_chunk);
        let upper = ChunkIdentity::of(&upper_chunk);
        let key = lower.parse().expect("a digest");
        assert_eq!(key, upper.parse().expect("the same digest"));
        assert!(lower.matches(key));
        assert!(upper.matches(key));

        let wrong_size = chunk(UPPER, 21);
        assert!(!ChunkIdentity::of(&wrong_size).matches(key));
        let other = chunk("0cc175b9c0f1b6a831c399e269772661", 20);
        assert!(!ChunkIdentity::of(&other).matches(key));
    }

    /// A string that is not a digest is unknown, not a wildcard identity.
    #[test]
    fn a_non_digest_is_not_an_identity() {
        for s in ["", "md5-a", &"g".repeat(32), &"a".repeat(31)] {
            let c = chunk(s, 20);
            let id = ChunkIdentity::of(&c);
            assert_eq!(id.parse(), None, "{s:?}");
            assert!(!id.matches(IdentityKey::new(Md5::EMPTY, 20)), "{s:?}");
        }
    }

    /// The spelling view stays spelling-exact: a caller that normalises at its
    /// decode boundary (gircap) gets case-insensitive set membership from the
    /// derived `Hash`/`Eq`; the case fold is offered separately above.
    #[test]
    fn the_borrowed_view_is_spelling_exact_until_normalised() {
        let lower = ChunkIdentity::new(EMPTY, 20);
        let upper = ChunkIdentity::new(UPPER, 20);
        assert_ne!(lower, upper);
        let set: std::collections::HashSet<ChunkIdentity<'_>> = [lower].into_iter().collect();
        assert!(!set.contains(&upper));

        let mut upper_owned = UPPER.to_string();
        normalize_hex(&mut upper_owned);
        assert_eq!(upper_owned, EMPTY);
        assert!(set.contains(&ChunkIdentity::new(&upper_owned, 20)));
    }

    /// Lowercasing is idempotent, and on the common path it does not reallocate
    /// the string: live manifests already spell their digests in lowercase.
    #[test]
    fn normalize_lowercases_only_when_needed_and_does_not_reallocate() {
        let mut s = UPPER.to_string();
        normalize_hex(&mut s);
        assert_eq!(s, EMPTY);

        let mut already = EMPTY.to_string();
        let ptr = already.as_ptr();
        normalize_hex(&mut already);
        normalize_hex(&mut already);
        assert_eq!(already, EMPTY);
        assert_eq!(
            already.as_ptr(),
            ptr,
            "a lowercase digest is left untouched"
        );
    }
}
