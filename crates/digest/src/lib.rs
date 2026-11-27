//! The shared mechanics at the digest/text boundary: hex spelling, fixed-width
//! digest newtypes, and the published test vectors.
//!
//! ## Mechanics, not a program's data model
//!
//! Nothing here decides what a digest *means* to a program. girpr uses the
//! strict [`Md5`] newtype, where `EMPTY` means "unknown, not a match"; girsync
//! keeps raw bytes plus verbatim foreign algorithm names; gircap publishes
//! lowercase hex strings. Those three representations stay distinct. This
//! crate supplies the spelling and the fixed-width mechanics they share — it
//! is deliberately not a single semantic `Digest` type used by all three.
//!
//! The `stream` module adds the byte-absorbing sink shared by the three programs' hash
//! readers and writers. It runs a caller-chosen set of algorithms and yields
//! raw digest bytes; byte ceilings, accepted-byte rules and restart policies
//! stay with each owner.

mod stream;

pub use stream::{Algorithms, DigestBytes, Sink, hash_reader};

use std::fmt;

/// A byte sink whose content can be thrown away between attempts.
///
/// A streamed body is written as it decodes, so a failed attempt can leave half
/// a body in the sink and the next attempt must not append to it. The one
/// requirement this trait adds to [`std::io::Write`] is that reset: [`Self::restart`]
/// is called before a retry starts over. Nothing here decides *when* to restart
/// — that policy stays with the owner.
///
/// The methods are called synchronously from the streaming decode, on the
/// thread that polls the stream's future. An implementation backed by blocking
/// I/O therefore relies on the caller honouring its blocking-context contract.
pub trait RestartableWrite: std::io::Write {
    /// Discard everything written so far and start the body again.
    fn restart(&mut self) -> std::io::Result<()>;
}

impl RestartableWrite for Vec<u8> {
    fn restart(&mut self) -> std::io::Result<()> {
        // `clear` keeps the capacity, so a retried body does not pay for the
        // buffer twice.
        self.clear();
        Ok(())
    }
}

impl<T: RestartableWrite + ?Sized> RestartableWrite for &mut T {
    fn restart(&mut self) -> std::io::Result<()> {
        (**self).restart()
    }
}

/// The narrow view of a caller-owned file that an HTTP fetcher streams a body
/// into: a restartable byte sink that is safe to hand across threads.
///
/// The owner keeps the file, the path and its digests; a fetcher only writes
/// bytes through [`std::io::Write`] and calls [`RestartableWrite::restart`]
/// before a retry. The trait lives in this logic-free leaf because both the
/// fetcher and the digesting-file owner already depend on it: keeping it in
/// either of those crates would make the other depend on it and rebuild
/// whenever it changes. No program policy crosses the boundary — this is one
/// byte-sink method plus `Send` for the `dyn` use, not a repair-path type.
///
/// `BodyFile` adds no methods of its own: every restartable, `Send` byte sink is
/// one, so a concrete owner implements [`RestartableWrite`] once and gets this
/// view automatically.
pub trait BodyFile: RestartableWrite + Send {}

impl<T: RestartableWrite + Send + ?Sized> BodyFile for T {}

use anyhow::{Result, anyhow, bail};
// md-5 and sha2 re-export the same `digest::Digest` trait, so one import
// brings `digest()` into scope for both hashers.
use md5::Digest as _;

/// The MD5 of the empty file — what a missing (or empty) file hashes to.
///
/// The lowercase hex spelling every artifact carries. Named rather than
/// inlined so "the file is not there" and "the file is there and empty" cannot
/// drift apart, and so every program's oracle is the same constant.
pub const EMPTY_MD5: &str = "d41d8cd98f00b204e9800998ecf8427e";

/// The SHA-256 of the empty file, in the same lowercase hex spelling.
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// The XXH64 (seed 0) of the empty file, in the same lowercase hex spelling.
/// Eight bytes, printed as the big-endian bytes of the 64-bit value — the
/// spelling every published XXH64 vector uses.
pub const EMPTY_XXHASH64: &str = "ef46db3751d8e999";

/// Lowercase hex, two characters per byte.
pub fn encode_hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

/// Uppercase hex, two characters per byte — the form percent-encoding requires.
pub fn encode_hex_upper(bytes: &[u8]) -> String {
    hex::encode_upper(bytes)
}

/// Decode hex into bytes. The caller keeps its own context on a bad row.
pub fn decode_hex(text: &str) -> std::result::Result<Vec<u8>, hex::FromHexError> {
    hex::decode(text)
}

/// Lowercase a digest spelling in place at a caller's decode boundary.
///
/// A digest is an identity, not a spelling: two manifests that spell one
/// digest in different cases would make their programs disagree about the
/// same bytes. Normalising once where a document enters a program makes every
/// later key agree instead of relying on each comparison to remember. Live
/// manifests already spell digests in lowercase, so a string with no uppercase
/// byte is left completely untouched — no allocation on the common path.
///
/// Re-exported by `girpr-chunk-identity` for callers that already depend on
/// that crate. *When* to normalise stays with the caller: the wire leaf does
/// not fold fields, since renaming or normalising one there changes dump bytes.
pub fn normalize_hex(s: &mut String) {
    if s.bytes().any(|b| b.is_ascii_uppercase()) {
        *s = s.to_ascii_lowercase();
    }
}

/// One-shot lowercase-hex MD5 of `bytes`.
pub fn md5_hex(bytes: &[u8]) -> String {
    encode_hex(&md5::Md5::digest(bytes))
}

/// One-shot lowercase-hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    encode_hex(&sha2::Sha256::digest(bytes))
}

/// One-shot numeric XXH64 (seed 0) of `bytes`.
///
/// The value [`xxhash64_hex`] spells, exposed so a caller that compares
/// numbers need not hash bytes and then parse its own hex back out.
pub fn xxhash64_u64(bytes: &[u8]) -> u64 {
    xxhash_rust::xxh64::xxh64(bytes, 0)
}

/// Spell an XXH64 value as 16 lowercase hex digits: big-endian bytes.
///
/// The leaf owns the conversion; whether this is the spelling an artifact
/// needs stays with the caller.
pub fn format_xxhash64_hex(value: u64) -> String {
    encode_hex(&value.to_be_bytes())
}

/// One-shot lowercase-hex XXH64 (seed 0) of `bytes`, big-endian bytes.
pub fn xxhash64_hex(bytes: &[u8]) -> String {
    format_xxhash64_hex(xxhash64_u64(bytes))
}

/// Parse the 16-digit XXH64 hex spelling into its numeric value.
///
/// Either case is accepted, and the byte width is enforced, so a string that
/// is not exactly 16 hex digits is an error. The caller keeps its own wording
/// for that error.
pub fn parse_xxhash64_hex(text: &str) -> Result<u64, hex::FromHexError> {
    let mut bytes = [0u8; 8];
    hex::decode_to_slice(text, &mut bytes)?;
    Ok(u64::from_be_bytes(bytes))
}

/// A 16-byte MD5 digest.
///
/// The manifest's spelling is not canonical — either case is legal — so the
/// digest is parsed once at the boundary and compared as bytes from then on.
/// [`Md5::parse_hex`] is the only way in from a `String`; [`Md5::to_hex`] is
/// the only way back out. A string that is not 32 hex digits (the empty string
/// above all) is not a digest, which keeps "an empty MD5 is unknown, not a
/// match" an `Option<Md5>` rather than a string sentinel.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Md5([u8; 16]);

impl Md5 {
    /// The MD5 of the empty file, as a digest.
    pub const EMPTY: Self = Self([
        0xd4, 0x1d, 0x8c, 0xd9, 0x8f, 0x00, 0xb2, 0x04, 0xe9, 0x80, 0x09, 0x98, 0xec, 0xf8, 0x42,
        0x7e,
    ]);

    /// Parse a hex MD5 into the 16 bytes it spells.
    ///
    /// Either case is accepted. Anything that is not exactly 32 hex digits is
    /// an error, so a caller that wants "unknown" can turn it into `None`.
    /// Callers that only want "is this a digest" should use
    /// [`Self::try_parse_hex`] instead: this one builds an `anyhow::Error`
    /// describing the bad spelling, which those callers immediately discard.
    pub fn parse_hex(s: &str) -> Result<Self> {
        Self::try_parse_hex(s).ok_or_else(|| {
            if s.len() != 32 {
                anyhow!("an MD5 is 32 hex digits, not {} bytes", s.len())
            } else {
                anyhow!("not a hex MD5: {s:?}")
            }
        })
    }

    /// Parse a hex MD5 into the 16 bytes it spells, without allocating on
    /// failure.
    ///
    /// The same spelling rule as [`Self::parse_hex`] — either case, exactly 32
    /// hex digits — but returns `None` instead of an `anyhow::Error`, so the
    /// common "unknown digest" path allocates nothing. The two are kept in
    /// lockstep by sharing this one parse: `parse_hex` only adds the reason.
    pub fn try_parse_hex(s: &str) -> Option<Self> {
        if s.len() != 32 {
            return None;
        }
        let mut out = [0u8; 16];
        hex::decode_to_slice(s, &mut out).ok()?;
        Some(Self(out))
    }

    /// The digest as lowercase hex.
    pub fn to_hex(self) -> String {
        encode_hex(&self.0)
    }

    /// The digest bytes, for a key or a byte comparison.
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Does `s` spell this digest?
    ///
    /// Either case is accepted; a string that is not a digest (the empty MD5
    /// above all) matches nothing.
    pub fn matches_hex(self, s: &str) -> bool {
        Self::try_parse_hex(s).is_some_and(|other| other == self)
    }
}

impl fmt::Display for Md5 {
    /// Lowercase hex, without allocating an intermediate `String`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = [0u8; 32];
        hex::encode_to_slice(self.0, &mut buf).map_err(|_| fmt::Error)?;
        f.write_str(std::str::from_utf8(&buf).map_err(|_| fmt::Error)?)
    }
}

impl fmt::Debug for Md5 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Md5({self})")
    }
}

impl From<md5::digest::Output<md5::Md5>> for Md5 {
    fn from(digest: md5::digest::Output<md5::Md5>) -> Self {
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest);
        Self(bytes)
    }
}

/// Build the newtype from raw digest bytes — the shape a streaming sink yields.
impl From<[u8; 16]> for Md5 {
    fn from(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

/// A 32-byte SHA-256 digest, with the same hex mechanics as [`Md5`].
///
/// girpr does not use this newtype — it only needs the MD5 one — but gircap
/// publishes SHA-256 hex too, so the spelling and the empty-file oracle live
/// here alongside the MD5 ones rather than in a third copy.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha256([u8; 32]);

impl Sha256 {
    /// The SHA-256 of the empty file, as a digest.
    pub const EMPTY: Self = Self([
        0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9,
        0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52,
        0xb8, 0x55,
    ]);

    /// Parse a hex SHA-256 into the 32 bytes it spells. Either case is accepted.
    pub fn parse_hex(s: &str) -> Result<Self> {
        if s.len() != 64 {
            bail!("a SHA-256 is 64 hex digits, not {} bytes", s.len());
        }
        let mut out = [0u8; 32];
        hex::decode_to_slice(s, &mut out).map_err(|_| anyhow!("not a hex SHA-256: {s:?}"))?;
        Ok(Self(out))
    }

    /// The digest as lowercase hex.
    pub fn to_hex(self) -> String {
        encode_hex(&self.0)
    }

    /// The digest bytes, for a key or a byte comparison.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Does `s` spell this digest? Either case is accepted.
    pub fn matches_hex(self, s: &str) -> bool {
        Self::parse_hex(s).is_ok_and(|other| other == self)
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = [0u8; 64];
        hex::encode_to_slice(self.0, &mut buf).map_err(|_| fmt::Error)?;
        f.write_str(std::str::from_utf8(&buf).map_err(|_| fmt::Error)?)
    }
}

impl fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256({self})")
    }
}

impl From<sha2::digest::Output<sha2::Sha256>> for Sha256 {
    fn from(digest: sha2::digest::Output<sha2::Sha256>) -> Self {
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&digest);
        Self(bytes)
    }
}

/// Build the newtype from raw digest bytes — the shape a streaming sink yields.
impl From<[u8; 32]> for Sha256 {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

/// Published digest vectors, shared so each implementation is checked against
/// the same oracles instead of a copy that can drift.
///
/// The helpers take the caller's own hex function: each program tests the code
/// it ships, only the expected values come from here.
pub mod vectors {
    use super::EMPTY_MD5;

    /// (input, expected lowercase hex) from RFC 1321, plus the widely published
    /// "quick brown fox" vector.
    pub const RFC1321_MD5: &[(&[u8], &str)] = &[
        (b"", EMPTY_MD5),
        (b"a", "0cc175b9c0f1b6a831c399e269772661"),
        (b"abc", "900150983cd24fb0d6963f7d28e17f72"),
        (b"message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
        (
            b"abcdefghijklmnopqrstuvwxyz",
            "c3fcd3d76192e4007dfb496cca67e13b",
        ),
        (
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
            "d174ab98d277d9f5a5611c2c9f419d9f",
        ),
        (
            b"12345678901234567890123456789012345678901234567890123456789012345678901234567890",
            "57edf4a22be3c955ac49da2e2107b67a",
        ),
        (
            b"The quick brown fox jumps over the lazy dog",
            "9e107d9d372bb6826bd81d3542a419d6",
        ),
    ];

    /// The RFC 1321 million-`a` vector, kept out of the table because its input
    /// is a megabyte rather than a literal.
    pub const RFC1321_MD5_MILLION_A: &str = "7707d6ae4e027c70eea2a935c2296f21";

    /// (input, expected lowercase hex) from FIPS 180-4.
    pub const FIPS180_4_SHA256: &[(&[u8], &str)] = &[
        (
            b"",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            b"abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
    ];

    /// Assert `hex_of` reproduces every shared MD5 vector, including the
    /// million-`a` case that crosses many block boundaries.
    pub fn assert_md5_vectors(hex_of: impl Fn(&[u8]) -> String) {
        for (input, want) in RFC1321_MD5 {
            assert_eq!(hex_of(input), *want, "{input:?}");
        }
        let million_a = "a".repeat(1_000_000);
        assert_eq!(hex_of(million_a.as_bytes()), RFC1321_MD5_MILLION_A);
    }

    /// Assert `hex_of` reproduces every shared SHA-256 vector.
    pub fn assert_sha256_vectors(hex_of: impl Fn(&[u8]) -> String) {
        for (input, want) in FIPS180_4_SHA256 {
            assert_eq!(hex_of(input), *want, "{input:?}");
        }
    }

    /// XXH64 vectors at seed 0, spelled as the big-endian bytes of the 64-bit
    /// value. The first is the empty-file oracle; the rest are the reference
    /// values published with the algorithm.
    pub const XXH64: &[(&[u8], &str)] = &[
        (b"", super::EMPTY_XXHASH64),
        (b"a", "d24ec4f1a98c6e5b"),
        (b"abc", "44bc2cf5ad770999"),
        (
            b"The quick brown fox jumps over the lazy dog",
            "0b242d361fda71bc",
        ),
    ];

    /// Assert `hex_of` reproduces every shared XXH64 vector.
    pub fn assert_xxhash64_vectors(hex_of: impl Fn(&[u8]) -> String) {
        for (input, want) in XXH64 {
            assert_eq!(hex_of(input), *want, "{input:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_constants_agree_with_the_newtypes() {
        assert_eq!(Md5::EMPTY.to_hex(), EMPTY_MD5);
        assert_eq!(Md5::EMPTY.to_string(), EMPTY_MD5);
        assert_eq!(Sha256::EMPTY.to_hex(), EMPTY_SHA256);
        assert_eq!(Sha256::EMPTY.to_string(), EMPTY_SHA256);
        assert_eq!(xxhash64_hex(b""), EMPTY_XXHASH64);
    }

    #[test]
    fn xxhash64_matches_the_shared_vectors() {
        vectors::assert_xxhash64_vectors(xxhash64_hex);
    }

    /// The numeric hash, the one-shot hex and the streaming sink agree,
    /// including across the algorithm's block boundaries and chunk splits.
    #[test]
    fn xxhash64_numeric_hex_and_stream_agree() {
        use std::hash::Hasher as _;
        for len in [0, 1, 16, 31, 32, 33, 63, 64, 65, 1_000] {
            let bytes: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let value = xxhash64_u64(&bytes);
            assert_eq!(xxhash64_hex(&bytes), format_xxhash64_hex(value), "{len}");

            let mut hasher = xxhash_rust::xxh64::Xxh64::new(0);
            hasher.update(&bytes);
            assert_eq!(value, hasher.finish(), "{len}");

            let mut sink = Sink::new(Algorithms::XXHASH64);
            for chunk in bytes.chunks(7) {
                sink.update(chunk);
            }
            let out = sink.finish();
            assert_eq!(out.xxhash64_u64(), Some(value), "{len}");
            assert_eq!(
                out.xxhash64_hex().as_deref(),
                Some(xxhash64_hex(&bytes).as_str()),
                "{len}"
            );
        }
    }

    /// The formatter emits exactly the 16 lowercase digits callers publish.
    #[test]
    fn format_xxhash64_hex_is_the_published_spelling() {
        for value in [0, 1, 0xf7c6_7301_db67_13f0, u64::MAX] {
            assert_eq!(format_xxhash64_hex(value), format!("{value:016x}"));
        }
        assert_eq!(format_xxhash64_hex(0), "0000000000000000");
        assert_eq!(
            parse_xxhash64_hex(EMPTY_XXHASH64).unwrap(),
            0xef46_db37_51d8_e999
        );
    }

    /// Parsing accepts either case, enforces the width, and round-trips the
    /// value through the formatter.
    #[test]
    fn parse_xxhash64_hex_round_trips_and_checks_width() {
        for value in [0, 1, 0xef46_db37_51d8_e999, u64::MAX] {
            let hex = format_xxhash64_hex(value);
            assert_eq!(parse_xxhash64_hex(&hex).unwrap(), value);
            assert_eq!(
                parse_xxhash64_hex(&hex.to_ascii_uppercase()).unwrap(),
                value
            );
        }
        let short = "0".repeat(15);
        let long = "0".repeat(17);
        let non_hex = "g".repeat(16);
        for text in ["", "123", &short, &long, &non_hex] {
            assert!(parse_xxhash64_hex(text).is_err(), "{text:?}");
        }
    }

    /// The numeric accessor agrees with the byte and hex accessors and makes
    /// "not requested" explicit.
    #[test]
    fn digest_bytes_numeric_accessor_agrees_with_its_bytes_and_hex() {
        let value = 0x44bc_2cf5_ad77_0999u64;
        let out = DigestBytes {
            bytes: 3,
            md5: None,
            sha256: None,
            xxhash64: Some(value.to_be_bytes()),
        };
        assert_eq!(out.xxhash64_u64(), Some(value));
        assert_eq!(out.xxhash64_bytes(), Some(value.to_be_bytes()));
        assert_eq!(out.xxhash64_hex().as_deref(), Some("44bc2cf5ad770999"));

        let none = DigestBytes::default();
        assert_eq!(none.xxhash64_u64(), None);
        assert_eq!(none.xxhash64_bytes(), None);
        assert_eq!(none.xxhash64_hex(), None);
    }

    #[test]
    fn md5_parses_either_case_and_prints_lowercase() {
        let upper = Md5::parse_hex("D41D8CD98F00B204E9800998ECF8427E").unwrap();
        let lower = Md5::parse_hex(EMPTY_MD5).unwrap();
        assert_eq!(upper, lower);
        assert_eq!(upper.to_hex(), EMPTY_MD5);
        assert_eq!(upper.to_string(), lower.to_hex());
        assert_eq!(format!("{upper:?}"), format!("Md5({EMPTY_MD5})"));
        assert!(upper.matches_hex(EMPTY_MD5));
        assert!(upper.matches_hex("D41D8CD98F00B204E9800998ECF8427E"));
        assert!(!upper.matches_hex(""));
        assert!(!upper.matches_hex("md5-a"));
        assert!(Md5::parse_hex("").is_err());
        assert!(Md5::parse_hex(&"g".repeat(32)).is_err());
        assert!(Md5::parse_hex(&"a".repeat(31)).is_err());
    }

    /// The non-allocating parse agrees with `parse_hex` for every input, so a
    /// caller can swap one for the other without changing which digests match.
    #[test]
    fn try_parse_hex_agrees_with_parse_hex() {
        let bs = "g".repeat(32);
        let short = "a".repeat(31);
        let long = "a".repeat(33);
        let inputs = [
            EMPTY_MD5,
            "D41D8CD98F00B204E9800998ECF8427E",
            "0cc175b9c0f1b6a831c399e269772661",
            "",
            "md5-a",
            &bs,
            &short,
            &long,
            "d41d8cd98f00b204e9800998ecf8427f",
        ];
        for s in inputs {
            assert_eq!(Md5::try_parse_hex(s), Md5::parse_hex(s).ok(), "{s:?}");
        }
    }

    #[test]
    fn sha256_parses_either_case_and_prints_lowercase() {
        let upper = Sha256::parse_hex(&EMPTY_SHA256.to_ascii_uppercase()).unwrap();
        assert_eq!(upper.to_hex(), EMPTY_SHA256);
        assert_eq!(upper.to_string(), EMPTY_SHA256);
        assert!(upper.matches_hex(EMPTY_SHA256));
        assert!(Sha256::parse_hex(EMPTY_MD5).is_err());
    }

    /// Folding happens only when needed: live manifests are already lowercase,
    /// so the common path neither rewrites nor reallocates the string.
    #[test]
    fn normalize_hex_folds_only_uppercase_and_is_idempotent() {
        let mut upper = "D41D8CD98F00B204E9800998ECF8427E".to_string();
        normalize_hex(&mut upper);
        assert_eq!(upper, EMPTY_MD5);

        let mut already = EMPTY_MD5.to_string();
        let ptr = already.as_ptr();
        normalize_hex(&mut already);
        normalize_hex(&mut already);
        assert_eq!(already, EMPTY_MD5);
        assert_eq!(
            already.as_ptr(),
            ptr,
            "a lowercase digest is left untouched"
        );
    }

    #[test]
    fn hex_helpers_round_trip_every_byte_in_both_cases() {
        let all: Vec<u8> = (0..=255u8).collect();
        let lower = encode_hex(&all);
        let upper = encode_hex_upper(&all);
        assert_eq!(lower.len(), 512);
        assert_eq!(upper, lower.to_ascii_uppercase());
        assert_eq!(decode_hex(&lower).unwrap(), all);
        assert_eq!(decode_hex(&upper).unwrap(), all);
    }
}
