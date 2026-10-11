//! Hash algorithm selection and streaming file digests.

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

const SUPPORTED_HASHES: &[&str] = &["md5", "sha256"];

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

/// Digest `path` with every requested algorithm in a single pass.
///
/// An empty `algos` returns an empty map without touching the file.
/// Values are **raw** digest bytes (16 for md5, 32 for sha256): the cache
/// stores them as-is instead of hex, halving hash disk cost.
pub fn hash_file(path: &Path, algos: &[String]) -> Result<HashMap<String, Vec<u8>>> {
    use sha2::Digest;
    // Before any context is built: with nothing requested there is nothing to
    // digest, and opening the file at all would turn a stat-only decision into a
    // read. The empty map below is the whole answer.
    if algos.is_empty() {
        return Ok(HashMap::new());
    }
    // Borrowed comparison, not `algos.contains(&"md5".to_string())`. This runs
    // once per file per phase C and twice per copied file, and the `String` that
    // form allocates exists only to be compared against borrowed data.
    let wants = |a: &str| algos.iter().any(|x| x == a);
    let mut md5ctx = wants("md5").then(md5::Context::new);
    let mut sha2ctx = wants("sha256").then(sha2::Sha256::new);
    let mut f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut buf = vec![0u8; 512 * 1024];
    loop {
        let n: usize = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        if let Some(c) = md5ctx.as_mut() {
            c.consume(chunk);
        }
        if let Some(c) = sha2ctx.as_mut() {
            use sha2::Digest;
            c.update(chunk);
        }
    }
    let mut out = HashMap::new();
    if let Some(c) = md5ctx {
        out.insert("md5".into(), c.compute().0.to_vec());
    }
    if let Some(c) = sha2ctx {
        use sha2::Digest;
        out.insert("sha256".into(), c.finalize().to_vec());
    }
    Ok(out)
}
