//! [`Digests`] and [`hash_file`]: streaming file digests in a single pass.

use anyhow::{Context, Result};
use sha2::Digest;
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
    /// context knows exactly which algorithm each of its two halves is, so there
    /// is no name to route and no decision to get wrong.
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
