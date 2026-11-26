//! A benchmark over three tree shapes: **many tiny files**, **few large
//! files** and **deep nesting**.
//!
//! ## What it is, and what it is not
//!
//! A **local measurement aid**, not a gate. criterion owns the harness: it
//! warms up, gathers statistics and can compare against a saved baseline,
//! but it asserts no timing bound — a slow disk is not a failure and nothing
//! in CI runs this target as a gate. What it makes checkable is a claim about
//! where a run's time goes: that a tree of many small files is dominated by
//! the per-entry cost of the walk, and a tree of few large files by the bytes.
//!
//! Two things are measured per shape. `walk` reports throughput in walked
//! files and tree bytes; `hash` reports throughput in MiB/s for one
//! representative file. The old hand-rolled `files_per_s`/`mib_per_s` lines
//! become criterion's throughput estimates, with warm-up, distributions and
//! regression output in place of a single hand-computed ratio.
//!
//! ## The fixture
//!
//! Each shape is built in its own scratch directory ([`TempDir`]), which is
//! removed when the value goes out of scope, so nothing is left behind
//! however the run ends. Fixture construction is the `iter_batched` setup:
//! it runs outside the measured region, so only the walk or the hash is
//! timed. `GIRSYNC_BENCH_SCALE` multiplies the fixture size; the default is
//! `1`, and a value that is not a positive whole number is read as `1`.
//!
//! The walk is capped at [`WALK_DEPTH`] levels, and the deep shape's chain is
//! deeper than that — 200 levels at scale 1 — so the deep shape's numbers are
//! the walk of the chain's top 64 levels.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
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
/// even measured, and the whole run then takes longer than a measurement should.
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
///
/// The scratch guard is the first field so it is dropped when the fixture is:
/// the tree stays alive for the whole measured routine and is removed
/// however the benchmark ends.
struct Fixture {
    /// Owns the scratch tree; kept solely for its `Drop`.
    _dir: TempDir,
    /// The tree's root.
    root: PathBuf,
    /// One representative file, for the hashing sample.
    sample: PathBuf,
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

/// The three measured shapes and the throughput their fixtures imply at the
/// current scale.
///
/// The counts are derived from the same constants the builders use, so
/// criterion can attach a files/s (and MiB/s) estimate to each `BenchmarkId`
/// without building an extra fixture just to count it.
fn shapes(scale: u64) -> [ShapeSpec; 3] {
    let tiny = scale.saturating_mul(TINY_FILES);
    let large = scale.saturating_mul(LARGE_FILES);
    let deep = scale.saturating_mul(DEEP_LEVELS);
    [
        ShapeSpec {
            name: "many-tiny",
            build: build_many_tiny,
            count: tiny,
            walked_files: tiny,
            tree_bytes: tiny * TINY_BYTES as u64,
            sample_bytes: TINY_BYTES as u64,
        },
        ShapeSpec {
            name: "few-large",
            build: build_few_large,
            count: large,
            walked_files: large,
            tree_bytes: large * LARGE_BYTES as u64,
            sample_bytes: LARGE_BYTES as u64,
        },
        ShapeSpec {
            name: "deep",
            build: build_deep,
            count: deep,
            // The walk stops at WALK_DEPTH levels, so the chain's deeper
            // levels are not counted in the walk's throughput.
            walked_files: deep.min(WALK_DEPTH as u64) * DEEP_FILES_PER_LEVEL,
            tree_bytes: deep * DEEP_FILES_PER_LEVEL * DEEP_BYTES as u64,
            sample_bytes: DEEP_BYTES as u64,
        },
    ]
}

/// What a builder returns: tree root, sample file, file count, byte count.
type Built = Result<(PathBuf, PathBuf, u64, u64)>;

/// A fixture builder: scratch directory and scaled count to a built tree.
type Builder = fn(&Path, u64) -> Built;

/// What distinguishes one measured shape from the others.
struct ShapeSpec {
    /// The shape's name, used as the benchmark function name.
    name: &'static str,
    /// The fixture builder.
    build: Builder,
    /// The scaled file/level count handed to the builder.
    count: u64,
    /// Files the capped walk sees, for the walk's element throughput.
    walked_files: u64,
    /// Bytes the tree holds, for the walk's byte throughput.
    tree_bytes: u64,
    /// Bytes of the one file the hash routine reads.
    sample_bytes: u64,
}

/// Build a fresh tree for one iteration of a benchmark.
///
/// This is the `iter_batched` setup: it is deliberately outside the measured
/// region, so writing the fixture is never mistaken for the walk or the hash.
fn build_fixture(spec: &ShapeSpec) -> Fixture {
    let dir = TempDir::new("girsync_bench", spec.name);
    let (root, sample, _, _) =
        (spec.build)(&dir, spec.count).unwrap_or_else(|e| panic!("build {}: {e:#}", spec.name));
    Fixture {
        _dir: dir,
        root,
        sample,
    }
}

/// The measured walk: one capped traversal of the freshly built tree.
///
/// The fixture is handed back rather than dropped here: criterion drops the
/// routine's output *after* it stops the timer, so removing the tree stays
/// outside the measured region just as building it does.
fn walk(fixture: Fixture) -> Fixture {
    let live = walk_live(&fixture.root, WALK_DEPTH)
        .unwrap_or_else(|e| panic!("walk {}: {e:#}", fixture.root.display()));
    std::hint::black_box(live.len());
    fixture
}

/// The measured hash: one representative file of the freshly built tree.
///
/// As in `walk`, the fixture is returned so its teardown is not timed.
fn hash(fixture: Fixture) -> Fixture {
    let digest = hash_file(&fixture.sample, &AlgoSet::one(HashAlg::Md5))
        .unwrap_or_else(|e| panic!("hash {}: {e:#}", fixture.sample.display()));
    std::hint::black_box(digest);
    fixture
}

/// Measure the capped walk over all three shapes.
fn walk_bench(c: &mut Criterion) {
    let scale = scale();
    let mut group = c.benchmark_group("walk");
    for spec in shapes(scale) {
        group.throughput(Throughput::ElementsAndBytes {
            elements: spec.walked_files,
            bytes: spec.tree_bytes,
        });
        group.bench_function(BenchmarkId::new(spec.name, format!("scale={scale}")), |b| {
            b.iter_batched(|| build_fixture(&spec), walk, BatchSize::LargeInput);
        });
    }
    group.finish();
}

/// Measure the hash of one representative file over all three shapes.
fn hash_bench(c: &mut Criterion) {
    let scale = scale();
    let mut group = c.benchmark_group("hash");
    for spec in shapes(scale) {
        group.throughput(Throughput::Bytes(spec.sample_bytes));
        group.bench_function(BenchmarkId::new(spec.name, format!("scale={scale}")), |b| {
            b.iter_batched(|| build_fixture(&spec), hash, BatchSize::LargeInput);
        });
    }
    group.finish();
}

criterion_group!(benches, walk_bench, hash_bench);
criterion_main!(benches);

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
