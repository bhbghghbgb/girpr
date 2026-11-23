//! [`Digests`], [`hash_file`] and [`hash_file_with_buf`]: streaming file digests in a single pass.

use anyhow::{Context, Result};
use girpr_digest::{Algorithms, Sink};
use std::io::Read;
use std::path::Path;

use super::{AlgoSet, DigestSet, HashAlg};

/// Chunk size for every read of file *content* this crate does.
///
/// One constant, not a per-call-site literal: a digest is correct only if it is
/// fed every byte, and a caller already moving the bytes should not have to
/// know that to stay in step with [`hash_file`].
pub const CHUNK: usize = 512 * 1024;

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
    sink: Sink,
}

impl Digests {
    /// Build the contexts `algos` asks for. An empty set builds none, so
    /// [`update`](Self::update) does nothing and [`finish`](Self::finish)
    /// yields an empty map — the same answer [`hash_file`] gives for it.
    ///
    /// The shared sink runs only the requested algorithms. The mapping from
    /// this crate's `AlgoSet` to the leaf's plain algorithm set happens here,
    /// at the call site: `AlgoSet`, `HashAlg` and `DigestSet` stay local.
    pub fn new(algos: &AlgoSet) -> Self {
        Self {
            sink: Sink::new(Algorithms::new(
                algos.contains(HashAlg::Md5),
                algos.contains(HashAlg::Sha256),
            )),
        }
    }

    /// Absorb the next chunk. Chunks must arrive in file order — the split is
    /// the caller's business, and [`hash_file`]'s is its read loop's.
    pub fn update(&mut self, chunk: &[u8]) {
        self.sink.update(chunk);
    }

    /// The digests of everything fed so far. Values are **raw** bytes (16 for
    /// md5, 32 for sha256), which is what the cache stores.
    ///
    /// Assigned to the slots directly rather than through `insert_named`: the
    /// sink knows exactly which algorithm each of its two halves is, so there
    /// is no name to route and no decision to get wrong.
    pub fn finish(self) -> DigestSet {
        let raw = self.sink.finish();
        DigestSet {
            md5: raw.md5,
            sha256: raw.sha256,
            ..Default::default()
        }
    }
}

/// Digest `path` with every requested algorithm in a single pass.
///
/// An empty `algos` returns an empty set without touching the file.
/// Values are **raw** digest bytes (16 for md5, 32 for sha256): the cache
/// stores them as-is instead of hex, halving hash disk cost.
pub fn hash_file(path: &Path, algos: &AlgoSet) -> Result<DigestSet> {
    // Before any buffer is allocated: with nothing requested there is nothing to
    // digest, and opening the file at all would turn a stat-only decision into a
    // read. The empty set below is the whole answer.
    if algos.is_empty() {
        return Ok(DigestSet::default());
    }
    let mut buf = vec![0u8; CHUNK];
    hash_file_with_buf(path, algos, &mut buf)
}

/// Digest `path` with every requested algorithm, reading through the caller's
/// buffer.
///
/// This is [`hash_file`] with the scratch space handed in rather than allocated
/// per call: a caller hashing many files (a rayon worker, say) allocates once
/// and reuses the same slice for each one. `buf` may be any non-empty size; its
/// length only decides how many reads the loop makes, never the digest. An empty
/// `algos` returns an empty set without touching the file or the buffer.
pub fn hash_file_with_buf(path: &Path, algos: &AlgoSet, buf: &mut [u8]) -> Result<DigestSet> {
    if algos.is_empty() {
        return Ok(DigestSet::default());
    }
    // A zero-length slice would make `read` return 0 forever rather than report
    // EOF, so refuse it explicitly instead of spinning.
    anyhow::ensure!(!buf.is_empty(), "hash buffer must not be empty");
    let mut f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut digests = Digests::new(algos);
    loop {
        let n: usize = f.read(buf)?;
        if n == 0 {
            break;
        }
        digests.update(&buf[..n]);
    }
    Ok(digests.finish())
}
