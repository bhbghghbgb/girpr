//! Streaming digest mechanics: a sink that absorbs bytes into a caller-chosen
//! set of algorithms, and a reader helper that drives it in caller-sized chunks.
//!
//! This is mechanics only. There is no byte ceiling, no forwarding, no file or
//! retry policy here and no program's digest vocabulary: the caller decides
//! which algorithms to run, how large its reads are, when to restart, and what
//! the raw result means to it. Those ordering rules are load-bearing — a sink
//! that hashes before a write lands, or after a limit check, produces a digest
//! that looks plausible and is wrong — so they stay at the call site.

use std::io::Read;
// `md-5` and `sha2` re-export the same `digest::Digest` trait, so one import
// brings `new`/`update`/`finalize` into scope for both hashers.
use md5::Digest as _;

/// Which algorithms a sink runs.
///
/// Plain mechanics, not a program's algorithm vocabulary: girsync's `AlgoSet`
/// and command-line names stay in girsync and are mapped to this at the call
/// site. Three booleans are enough for the fixed-width algorithms this crate owns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Algorithms {
    md5: bool,
    sha256: bool,
    xxhash64: bool,
}

impl Algorithms {
    /// Run no algorithms. The sink is inert: it counts bytes but yields no digests.
    pub const NONE: Self = Self::new(false, false, false);
    /// MD5 only.
    pub const MD5: Self = Self::new(true, false, false);
    /// SHA-256 only.
    pub const SHA256: Self = Self::new(false, true, false);
    /// XXH64 only.
    pub const XXHASH64: Self = Self::new(false, false, true);
    /// MD5 and SHA-256 over the same bytes, in one pass.
    pub const MD5_SHA256: Self = Self::new(true, true, false);
    /// All three over the same bytes, in one pass.
    pub const ALL: Self = Self::new(true, true, true);

    /// Build a set from three membership flags.
    pub const fn new(md5: bool, sha256: bool, xxhash64: bool) -> Self {
        Self {
            md5,
            sha256,
            xxhash64,
        }
    }

    /// Whether MD5 was requested.
    pub const fn md5(self) -> bool {
        self.md5
    }

    /// Whether SHA-256 was requested.
    pub const fn sha256(self) -> bool {
        self.sha256
    }

    /// Whether XXH64 was requested.
    pub const fn xxhash64(self) -> bool {
        self.xxhash64
    }

    /// Whether nothing was requested.
    pub const fn is_empty(self) -> bool {
        !self.md5 && !self.sha256 && !self.xxhash64
    }
}

/// The raw result of a finished [`Sink`]: a byte count and the digest each
/// requested algorithm computed, as fixed-width byte arrays.
///
/// No program's result type. A caller maps these bytes into its own shape —
/// girpr's `Md5` newtype, gircap's lowercase hex strings, girsync's raw-byte
/// cache row — without the mechanics choosing a single digest semantics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DigestBytes {
    /// Bytes absorbed.
    pub bytes: u64,
    /// MD5 bytes, when requested.
    pub md5: Option<[u8; 16]>,
    /// SHA-256 bytes, when requested.
    pub sha256: Option<[u8; 32]>,
    /// XXH64 bytes (big-endian), when requested.
    pub xxhash64: Option<[u8; 8]>,
}

impl DigestBytes {
    /// Bytes absorbed.
    pub fn len(&self) -> u64 {
        self.bytes
    }

    /// Whether no bytes were absorbed.
    pub fn is_empty(&self) -> bool {
        self.bytes == 0
    }

    /// The MD5 bytes, when requested.
    pub fn md5_bytes(&self) -> Option<[u8; 16]> {
        self.md5
    }

    /// The SHA-256 bytes, when requested.
    pub fn sha256_bytes(&self) -> Option<[u8; 32]> {
        self.sha256
    }

    /// The MD5 as lowercase hex, when requested.
    pub fn md5_hex(&self) -> Option<String> {
        self.md5.map(|d| super::encode_hex(&d))
    }

    /// The SHA-256 as lowercase hex, when requested.
    pub fn sha256_hex(&self) -> Option<String> {
        self.sha256.map(|d| super::encode_hex(&d))
    }

    /// The XXH64 bytes, when requested.
    pub fn xxhash64_bytes(&self) -> Option<[u8; 8]> {
        self.xxhash64
    }

    /// The XXH64 as its 64-bit value (big-endian bytes), when requested.
    pub fn xxhash64_u64(&self) -> Option<u64> {
        self.xxhash64.map(u64::from_be_bytes)
    }

    /// The XXH64 as lowercase hex, when requested.
    pub fn xxhash64_hex(&self) -> Option<String> {
        self.xxhash64.map(|d| super::encode_hex(&d))
    }
}

/// A byte-absorbing sink: feeds every byte to the requested hashers and counts
/// them.
///
/// It does **not** forward bytes, enforce a ceiling, touch a file, or decide
/// when a retry starts over. Those are the owner's policies. A writer that
/// wraps this sink checks its limit before forwarding and calls [`Self::update`]
/// only with the bytes that actually landed; a file wrapper writes before it
/// hashes and calls [`Self::reset`] when a retry discards the partial body.
pub struct Sink {
    md5: Option<md5::Md5>,
    sha256: Option<sha2::Sha256>,
    xxhash64: Option<xxhash_rust::xxh64::Xxh64>,
    bytes: u64,
    algos: Algorithms,
}

impl Sink {
    /// A fresh sink running exactly the requested algorithms.
    pub fn new(algos: Algorithms) -> Self {
        Self {
            md5: algos.md5().then(md5::Md5::new),
            sha256: algos.sha256().then(sha2::Sha256::new),
            // Seed 0: this is a content identity shared across runs, not a keyed hash.
            xxhash64: algos.xxhash64().then(|| xxhash_rust::xxh64::Xxh64::new(0)),
            bytes: 0,
            algos,
        }
    }

    /// Absorb the next chunk. Chunks must arrive in file order; the split is
    /// the caller's business, and the digest is the same for any split.
    pub fn update(&mut self, chunk: &[u8]) {
        if let Some(h) = self.md5.as_mut() {
            h.update(chunk);
        }
        if let Some(h) = self.sha256.as_mut() {
            h.update(chunk);
        }
        if let Some(h) = self.xxhash64.as_mut() {
            h.update(chunk);
        }
        self.bytes += chunk.len() as u64;
    }

    /// How many bytes have been absorbed so far.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// The algorithm set this sink was built with.
    pub fn algos(&self) -> Algorithms {
        self.algos
    }

    /// Read the current MD5 without finishing the sink, for a writer that must
    /// keep absorbing after it asks. `None` when MD5 was not requested.
    pub fn md5_bytes(&self) -> Option<[u8; 16]> {
        self.md5.as_ref().map(|h| bytes16(h.clone().finalize()))
    }

    /// Read the current SHA-256 without finishing the sink. `None` when
    /// SHA-256 was not requested.
    pub fn sha256_bytes(&self) -> Option<[u8; 32]> {
        self.sha256.as_ref().map(|h| bytes32(h.clone().finalize()))
    }

    /// Read the current XXH64 without finishing the sink. `None` when
    /// XXH64 was not requested.
    pub fn xxhash64_bytes(&self) -> Option<[u8; 8]> {
        self.xxhash64.as_ref().map(|h| bytes8(h.digest()))
    }

    /// Discard everything absorbed and start over with the same algorithm set.
    ///
    /// This is the reset *mechanic* only. When to call it — on a reader seek, a
    /// file truncation, or a fresh writer per attempt — is the owner's contract.
    pub fn reset(&mut self) {
        self.md5 = self.algos.md5().then(md5::Md5::new);
        self.sha256 = self.algos.sha256().then(sha2::Sha256::new);
        self.xxhash64 = self
            .algos
            .xxhash64()
            .then(|| xxhash_rust::xxh64::Xxh64::new(0));
        self.bytes = 0;
    }

    /// Finish and take the digests of everything absorbed.
    pub fn finish(self) -> DigestBytes {
        DigestBytes {
            bytes: self.bytes,
            md5: self.md5.map(|h| bytes16(h.finalize())),
            sha256: self.sha256.map(|h| bytes32(h.finalize())),
            xxhash64: self.xxhash64.map(|h| bytes8(h.digest())),
        }
    }
}

/// Absorb every byte, always accept the whole buffer. A `Write` sink with no
/// forwarding of its own: wrapping writers keep their own limit and ordering.
impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Read from `reader` in caller-sized chunks, forward each chunk to `forward`
/// **before** absorbing it into `sink`, and report how many bytes were read.
///
/// This is the one buffered read loop behind the file digesters and the copy
/// path. It owns neither a file nor a write policy: `forward` is where a caller
/// sends the same bytes onward (a copy does), and a hash-only caller passes a
/// no-op. Forward-before-absorb is the order a copy needs — a digest of bytes
/// that never landed is worse than no digest at all.
///
/// The caller supplies the buffer, so its size stays the caller's policy. An
/// empty buffer is refused: `read` would otherwise return 0 forever and look
/// like EOF. Seeking and deciding when to reset the sink stay with the caller.
pub fn copy_with_sink<R: Read>(
    reader: &mut R,
    sink: &mut Sink,
    buf: &mut [u8],
    mut forward: impl FnMut(&[u8]) -> std::io::Result<()>,
) -> std::io::Result<u64> {
    if buf.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "hash buffer must not be empty",
        ));
    }
    let mut total = 0u64;
    loop {
        let n = reader.read(buf)?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        forward(chunk)?;
        sink.update(chunk);
        total += n as u64;
    }
    Ok(total)
}

/// Hash everything a reader yields, in caller-sized chunks.
///
/// The caller supplies the read buffer, so the buffer size is its policy — a
/// 64 KiB file-digest read and a 512 KiB chunk read drive the same sink. The
/// reader starts wherever it currently is: seeking to the beginning (or not)
/// is the caller's reset contract, not this function's. The read loop itself is
/// shared with the copy path through [`copy_with_sink`].
pub fn hash_reader<R: Read>(
    mut reader: R,
    algos: Algorithms,
    buf: &mut [u8],
) -> std::io::Result<DigestBytes> {
    let mut sink = Sink::new(algos);
    copy_with_sink(&mut reader, &mut sink, buf, |_| Ok(()))?;
    Ok(sink.finish())
}

fn bytes16(output: md5::digest::Output<md5::Md5>) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&output);
    bytes
}

fn bytes32(output: sha2::digest::Output<sha2::Sha256>) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&output);
    bytes
}

/// Big-endian bytes, the spelling a hex-rendered XXH64 uses.
fn bytes8(value: u64) -> [u8; 8] {
    value.to_be_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn algorithms_membership_and_empty() {
        assert_eq!(Algorithms::default(), Algorithms::NONE);
        assert!(Algorithms::NONE.is_empty());
        assert!(Algorithms::MD5.md5() && !Algorithms::MD5.sha256());
        assert!(!Algorithms::SHA256.md5() && Algorithms::SHA256.sha256());
        assert!(Algorithms::XXHASH64.xxhash64());
        assert!(!Algorithms::XXHASH64.md5() && !Algorithms::XXHASH64.sha256());
        assert_eq!(Algorithms::MD5_SHA256, Algorithms::new(true, true, false));
        assert_eq!(Algorithms::ALL, Algorithms::new(true, true, true));
        assert!(!Algorithms::MD5_SHA256.is_empty());
        assert!(!Algorithms::ALL.is_empty());
    }

    #[test]
    fn sink_agrees_with_the_one_shot_vectors_for_every_algorithm_set() {
        for (input, want_md5) in super::super::vectors::RFC1321_MD5 {
            let want_sha = super::super::sha256_hex(input);
            let mut sink = Sink::new(Algorithms::MD5_SHA256);
            sink.update(input);
            let out = sink.finish();
            assert_eq!(out.len(), input.len() as u64);
            assert_eq!(out.md5_hex().as_deref(), Some(*want_md5), "{input:?}");
            assert_eq!(
                out.sha256_hex().as_deref(),
                Some(want_sha.as_str()),
                "{input:?}"
            );
        }
    }

    #[test]
    fn a_split_stream_matches_a_single_write_and_the_one_shot_helpers() {
        let data: Vec<u8> = (0..5000).map(|i| (i % 251) as u8).collect();
        let want_md5 = super::super::md5_hex(&data);
        let want_sha = super::super::sha256_hex(&data);
        let want_xx = super::super::xxhash64_hex(&data);
        let mut sink = Sink::new(Algorithms::ALL);
        for chunk in data.chunks(37) {
            sink.update(chunk);
        }
        let out = sink.finish();
        assert_eq!(out.len(), data.len() as u64);
        assert_eq!(out.md5_hex().as_deref(), Some(want_md5.as_str()));
        assert_eq!(out.sha256_hex().as_deref(), Some(want_sha.as_str()));
        assert_eq!(out.xxhash64_hex().as_deref(), Some(want_xx.as_str()));
    }

    #[test]
    fn xxhash64_only_yields_its_own_digest() {
        let mut sink = Sink::new(Algorithms::XXHASH64);
        sink.update(b"abc");
        let out = sink.finish();
        assert_eq!(out.md5, None);
        assert_eq!(out.sha256, None);
        assert_eq!(out.xxhash64_hex().as_deref(), Some("44bc2cf5ad770999"));
    }

    #[test]
    fn empty_algorithm_set_yields_no_digests_but_still_counts() {
        let mut sink = Sink::new(Algorithms::NONE);
        sink.update(b"bytes are counted even with no hasher");
        let out = sink.finish();
        assert_eq!(out.len(), 37);
        assert_eq!(out.md5, None);
        assert_eq!(out.sha256, None);
        assert_eq!(out.xxhash64, None);
    }

    #[test]
    fn reset_starts_a_fresh_digest_and_count() {
        let mut sink = Sink::new(Algorithms::MD5_SHA256);
        sink.update(b"first");
        let partial = sink.md5_bytes();
        assert!(partial.is_some());
        sink.reset();
        assert_eq!(sink.bytes(), 0);
        let first = sink.clone_finish();
        sink.update(b"second");
        let second = sink.clone_finish();
        let empty_md5 = super::super::md5_hex(b"");
        let second_md5 = super::super::md5_hex(b"second");
        assert_eq!(first.md5_hex().as_deref(), Some(empty_md5.as_str()));
        assert_eq!(second.md5_hex().as_deref(), Some(second_md5.as_str()));
    }

    #[test]
    fn write_impl_accepts_every_byte() {
        use std::io::Write as _;
        let want = super::super::md5_hex(b"abc");
        let mut sink = Sink::new(Algorithms::MD5);
        assert_eq!(sink.write(b"abc").unwrap(), 3);
        sink.flush().unwrap();
        assert_eq!(sink.finish().md5_hex().as_deref(), Some(want.as_str()));
    }

    /// The shared loop hands every byte onward as well as to the sink, and an
    /// empty buffer is refused instead of reading zero forever.
    #[test]
    fn copy_with_sink_forwards_every_byte_and_refuses_an_empty_buffer() {
        let data: Vec<u8> = (0..500).map(|i| (i % 251) as u8).collect();
        let want = super::super::md5_hex(&data);
        let mut sink = Sink::new(Algorithms::MD5);
        let mut forwarded = Vec::new();
        let mut buf = [0u8; 37];
        let n = copy_with_sink(&mut data.as_slice(), &mut sink, &mut buf, |chunk| {
            forwarded.extend_from_slice(chunk);
            Ok(())
        })
        .unwrap();
        assert_eq!(n, data.len() as u64);
        assert_eq!(forwarded, data);
        assert_eq!(sink.finish().md5_hex().as_deref(), Some(want.as_str()));

        let mut sink = Sink::new(Algorithms::MD5);
        let err = copy_with_sink(&mut data.as_slice(), &mut sink, &mut [], |_| Ok(())).unwrap_err();
        assert_eq!(err.to_string(), "hash buffer must not be empty");
    }

    #[test]
    fn hash_reader_agrees_across_buffer_sizes_and_reader_types() {
        let data: Vec<u8> = (0..100_000).map(|i| (i % 251) as u8).collect();
        let want = (
            super::super::md5_hex(&data),
            super::super::sha256_hex(&data),
        );
        for capacity in [1, 7, 4096, 512 * 1024] {
            let mut buf = vec![0u8; capacity];
            let got = hash_reader(data.as_slice(), Algorithms::MD5_SHA256, &mut buf).unwrap();
            assert_eq!(got.len(), data.len() as u64);
            assert_eq!(got.md5_hex().as_deref(), Some(want.0.as_str()));
            assert_eq!(got.sha256_hex().as_deref(), Some(want.1.as_str()));
        }
    }

    impl Sink {
        fn clone_finish(&self) -> DigestBytes {
            DigestBytes {
                bytes: self.bytes,
                md5: self.md5_bytes(),
                sha256: self.sha256_bytes(),
                xxhash64: self.xxhash64_bytes(),
            }
        }
    }
}
