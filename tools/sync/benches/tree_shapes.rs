//! A benchmark over three tree shapes: **many tiny files**, **few large
//! files** and **deep nesting**.
//!
//! ## What it is, and what it is not
//!
//! A **local measurement aid**, not a gate. It prints numbers and asserts
//! nothing about them: there is no timing assertion anywhere in this file, a
//! slow disk is not a failure, and nothing in CI runs this target. What it
//! makes checkable is a claim about where a run's time goes — that a tree of
//! many small files is dominated by the per-entry cost of the walk, and a tree
//! of few large files by the bytes.
//!
//! ## The fixture
//!
//! Each shape is built in its own scratch directory ([`TempDir`]), which
//! removes the tree when the value goes out of scope, so nothing is left
//! behind however the run ends. `GIRSYNC_BENCH_SCALE` multiplies the fixture
//! size; the default is `1`, and a value that is not a positive whole number
//! is read as `1`.
//!
//! The walk is capped at [`WALK_DEPTH`] levels, and the deep shape's chain is
//! deeper than that — 200 levels at scale 1 — so the deep shape's numbers are
//! the walk of the chain's top 64 levels. `fixture_files` and `fixture_bytes`
//! say how much the tree holds, beside the `files` and `bytes` the walk saw.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use girpr_tempdir::TempDir;
use girsync::hash::{AlgoSet, HashAlg, hash_file};
use girsync::scan::walk_live;

/// How deep the walk goes.
///
/// The crate's own default is 10; this is deeper so that a chain of
/// directories is descended rather than truncated near the root.
const WALK_DEPTH: usize = 64;

/// How many small files the many-tiny shape holds, per unit of scale.
///
/// **2 000 at scale 1, not 10 000.** Writing the tree is the expensive half
/// of this shape — a file creation costs about 0.7 ms on an ordinary Windows
/// host, so ten thousand of them is nearly seven seconds before the walk is
/// even timed, and the whole run then takes longer than a measurement should.
/// Two thousand is a fifth of that and still walks for long enough to time.
/// `GIRSYNC_BENCH_SCALE` multiplies it, so a larger run is one variable away.
const TINY_FILES: u64 = 2_000;

/// How big a small file is.
const TINY_BYTES: usize = 256;

/// How many large files the few-large shape holds, per unit of scale.
const LARGE_FILES: u64 = 4;

/// How big a large file is.
const LARGE_BYTES: usize = 8 * 1024 * 1024;

/// How many levels the deep shape's chain has, per unit of scale.
const DEEP_LEVELS: u64 = 200;

/// How many files sit at each level of the deep shape.
const DEEP_FILES_PER_LEVEL: u64 = 2;

/// How big a file at each level of the deep shape is.
const DEEP_BYTES: usize = 128;

/// The buffer the large files are streamed out in. One buffer, reused for
/// every file: the fixture's own memory use does not scale with the file size.
const LARGE_CHUNK: usize = 1024 * 1024;

/// One tree the walk is measured over.
struct Shape {
    /// The name the line is printed under.
    name: &'static str,
    /// The tree's root.
    root: PathBuf,
    /// One representative file, for the hashing sample.
    sample: PathBuf,
    /// How many files the fixture holds.
    fixture_files: u64,
    /// How many bytes those files hold.
    fixture_bytes: u64,
}

fn main() {
    let scale = scale();
    if let Err(e) = run(scale) {
        eprintln!("bench: {e:#}");
        std::process::exit(1);
    }
}

/// The fixture multiplier: `GIRSYNC_BENCH_SCALE`, defaulting to `1`.
///
/// A value that is not a positive whole number is read as `1` rather than
/// refused: the fixture is a measurement, and a knob that can be mis-set into
/// an error is a knob nobody will dare turn.
fn scale() -> u64 {
    std::env::var("GIRSYNC_BENCH_SCALE")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(1)
}

/// Build and measure each shape in turn.
fn run(scale: u64) -> Result<()> {
    println!("scale={scale}");
    shape("many-tiny", |dir| {
        build_many_tiny(dir, scale.saturating_mul(TINY_FILES))
    })?;
    shape("few-large", |dir| {
        build_few_large(dir, scale.saturating_mul(LARGE_FILES))
    })?;
    shape("deep", |dir| {
        build_deep(dir, scale.saturating_mul(DEEP_LEVELS))
    })?;
    Ok(())
}

/// Build one shape in its own scratch directory, walk it, and print its line.
///
/// The scratch directory is dropped — and with it the tree — when this
/// returns, so the fixture is gone however the run ends.
fn shape<F>(name: &'static str, build: F) -> Result<()>
where
    F: FnOnce(&Path) -> Result<(PathBuf, PathBuf, u64, u64)>,
{
    let dir = TempDir::new("girsync_bench", name);
    let t0 = Instant::now();
    let (root, sample, fixture_files, fixture_bytes) = build(&dir)?;
    let build_s = t0.elapsed().as_secs_f64();
    let shape = Shape {
        name,
        root,
        sample,
        fixture_files,
        fixture_bytes,
    };
    measure(&shape, build_s)
}

/// Walk the tree, hash one file of it, and print what that cost.
///
/// `build_s` is how long the fixture took to write: it is the price of the
/// measurement rather than part of it, and it is printed so the two are not
/// confused for each other.
fn measure(shape: &Shape, build_s: f64) -> Result<()> {
    let t0 = Instant::now();
    let live = walk_live(&shape.root, WALK_DEPTH)
        .with_context(|| format!("walk {}", shape.root.display()))?;
    let walk_s = t0.elapsed().as_secs_f64();

    // The bytes are the files' own sizes, summed — the content, not the
    // directory's size on disk, which is a different number entirely.
    let files: u64 = live.iter().filter(|e| !e.is_dir).count() as u64;
    let bytes: u64 = live.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();

    let hash_bytes = std::fs::metadata(&shape.sample)
        .with_context(|| format!("stat {}", shape.sample.display()))?
        .len();
    let t1 = Instant::now();
    hash_file(&shape.sample, &AlgoSet::one(HashAlg::Md5))
        .with_context(|| format!("hash {}", shape.sample.display()))?;
    let hash_s = t1.elapsed().as_secs_f64();

    let files_per_s = rate(files as f64, walk_s);
    let mib_per_s = rate(hash_bytes as f64 / (1024.0 * 1024.0), hash_s);

    println!(
        "shape={} files={} bytes={} fixture_files={} fixture_bytes={} \
         build_ms={:.1} walk_ms={:.1} files_per_s={:.1} hash_bytes={} hash_ms={:.3} mib_per_s={:.2}",
        shape.name,
        files,
        bytes,
        shape.fixture_files,
        shape.fixture_bytes,
        build_s * 1000.0,
        walk_s * 1000.0,
        files_per_s,
        hash_bytes,
        hash_s * 1000.0,
        mib_per_s,
    );
    Ok(())
}

/// A rate per second, and zero rather than an infinity when the interval was
/// too short to measure.
fn rate(amount: f64, seconds: f64) -> f64 {
    if seconds > 0.0 { amount / seconds } else { 0.0 }
}

/// A wide directory of small files: the shape where the walk's per-entry cost
/// dominates.
fn build_many_tiny(dir: &Path, files: u64) -> Result<(PathBuf, PathBuf, u64, u64)> {
    let root = dir.join("many-tiny");
    std::fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
    let payload = vec![b'.'; TINY_BYTES];
    // The sample is the first file, named rather than remembered: the name is
    // known before the loop, so there is nothing to keep and nothing to
    // unwrap afterwards.
    let sample = root.join("f000000.bin");
    for i in 0..files {
        let path = root.join(format!("f{i:06}.bin"));
        std::fs::write(&path, &payload).with_context(|| format!("write {}", path.display()))?;
    }
    let bytes = files * TINY_BYTES as u64;
    Ok((root, sample, files, bytes))
}

/// A handful of multi-megabyte files: the shape where the bytes dominate.
fn build_few_large(dir: &Path, files: u64) -> Result<(PathBuf, PathBuf, u64, u64)> {
    let root = dir.join("few-large");
    std::fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
    let chunk = vec![b'.'; LARGE_CHUNK];
    let sample = root.join("big000.bin");
    for i in 0..files {
        let path = root.join(format!("big{i:03}.bin"));
        let mut out = BufWriter::with_capacity(
            LARGE_CHUNK,
            File::create(&path).with_context(|| format!("create {}", path.display()))?,
        );
        let mut left = LARGE_BYTES as u64;
        while left > 0 {
            let n = left.min(chunk.len() as u64) as usize;
            out.write_all(&chunk[..n])
                .with_context(|| format!("write {}", path.display()))?;
            left -= n as u64;
        }
        out.flush()
            .with_context(|| format!("flush {}", path.display()))?;
    }
    let bytes = files * LARGE_BYTES as u64;
    Ok((root, sample, files, bytes))
}

/// A chain of directories with a couple of files at each level: the shape
/// where the path gets longer and longer.
fn build_deep(dir: &Path, levels: u64) -> Result<(PathBuf, PathBuf, u64, u64)> {
    let root = dir.join("deep");
    std::fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
    let payload = vec![b'.'; DEEP_BYTES];
    let sample = root.join("f0.bin");
    let mut files = 0u64;
    let mut current = root.clone();
    for level in 0..levels {
        for k in 0..DEEP_FILES_PER_LEVEL {
            let path = current.join(format!("f{k}.bin"));
            std::fs::write(&path, &payload).with_context(|| format!("write {}", path.display()))?;
            files += 1;
        }
        // The next level's directory, unless this was the last one: a chain
        // with an empty directory hanging off its end is a level that holds
        // nothing, and the shape is about levels that hold files.
        if level + 1 < levels {
            current = current.join("d");
            std::fs::create_dir_all(&current)
                .with_context(|| format!("create {}", current.display()))?;
        }
    }
    let bytes = files * DEEP_BYTES as u64;
    Ok((root, sample, files, bytes))
}
