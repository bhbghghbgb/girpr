//! Hash algorithm selection and streaming file digests.

use anyhow::{Context, Result, bail};
use sha2::Digest;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

const SUPPORTED_HASHES: &[&str] = &["md5", "sha256"];

/// Chunk size for every read of file *content* this crate does.
///
/// One constant, not a per-call-site literal: a digest is correct only if it is
/// fed every byte, and a caller already moving the bytes should not have to
/// know that to stay in step with [`hash_file`].
pub const CHUNK: usize = 512 * 1024;

/// Validate repeated `--hash` values.
///
/// Returns the lowercased algorithm list, or an empty list for `--hash none`
/// (which is exclusive: decisions fall back to size+mtime).
pub fn parse_hash_list(input: &[String]) -> Result<Vec<String>> {
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
    for a in &v {
        if !SUPPORTED_HASHES.contains(&a.as_str()) {
            bail!(
                "unsupported hash '{}' (supported: {} or none)",
                a,
                SUPPORTED_HASHES.join(",")
            );
        }
    }
    v.dedup();
    Ok(v)
}

/// Validate repeated `--hash-any-of` values.
///
/// Separate from [`parse_hash_list`] for one reason: **`none` is not a value here.**
/// "Any of the requested algorithms", with nothing requested, is not a weaker
/// request — it is no request — and `--hash-all-of none` is the flag that says so.
/// Accepting it would give a stat-only audit a name claiming a digest was optional
/// but not required, which is the ambiguity all-of exists to remove.
pub fn parse_any_of(input: &[String]) -> Result<Vec<String>> {
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
    pub fn new(algos: &[String]) -> Self {
        let wants = |a: &str| algos.iter().any(|x| x == a);
        Self {
            md5ctx: wants("md5").then(md5::Context::new),
            sha2ctx: wants("sha256").then(sha2::Sha256::new),
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
pub fn hash_file(path: &Path, algos: &[String]) -> Result<HashMap<String, Vec<u8>>> {
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

    fn algos(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
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
