---
id: T19
title: Benchmark harness over three tree shapes
batch: G
checkpoint: G
report: [L590-L593]
design_notes: "Benchmarks are not gates; the crate's tests are about meaning, not speed"
depends_on: []
size: S
risk: low
---

# T19 — Benchmark harness over three tree shapes

## Goal

Add a small, dependency-free benchmark binary under `benches/` that builds
three tree shapes and reports elapsed time and throughput:

- **many tiny files** — a wide directory of small files;
- **few large files** — a small number of multi-megabyte files;
- **deep nesting** — a chain of directories with a few files at each level.

It exercises the live walk (and, cheaply, hashing) so the performance claims in
the report are checkable. It is **not** a CI gate and has no assertions about
time.

## Why

Report §5.6: a harness over three shapes makes the performance discussion
checkable, but turning it into a gate would make the suite flaky and would
change what the tests are for. The crate currently has no benchmark target.

## Files to read

- `Cargo.toml` — dev-dependencies and target layout
- `src/scan.rs` — `walk_live`, `LiveEnt`
- `src/hash.rs` — `hash_file`, `HashAlg`, `AlgoSet`
- `girpr-tempdir` (dev-dependency) — `TempDir::new`, `subdir`, `join`
- `README.md` — how to run tests/builds

## Constraints

- **No `criterion`** and no new dependency. `harness = false` with a plain
  `main`.
- No wall-clock assertion. The harness prints numbers and exits 0; a slow
  machine is not a failure.
- The default fixture must finish in a few seconds on an ordinary disk. Gate
  the size behind an environment variable (for example
  `GIRSYNC_BENCH_SCALE`, default `1`) so a user can ask for a larger run.
- Do not touch production code or add a `cfg(bench)` path to it.
- Clean up the generated tree; do not leave fixtures in the repo or in a
  shared temp directory.
- Hashing is optional but useful: hash one representative file per shape (or
  a small sample), not every multi-megabyte file, unless the scale is small.
  The walk is the phase the report's scan claims are about.

## Implementation

1. `Cargo.toml`:
   ```toml
   [[bench]]
   name = "tree_shapes"
   path = "benches/tree_shapes.rs"
   harness = false
   ```
2. `benches/tree_shapes.rs`:
   - `fn main()` reads `GIRSYNC_BENCH_SCALE` (default 1, parse errors -> 1),
     builds each shape under a `girpr_tempdir::TempDir`, times
     `walk_live(&root, 64)`, and prints one line per shape:
     ```
     shape=many-tiny files=... bytes=... walk_ms=... files_per_s=...
     ```
   - a `hash_one` sample: pick one file per shape and time
     `hash_file(path, &AlgoSet::one(HashAlg::Md5))`, printing `hash_ms` and
     `mib_per_s`.
   - shapes:
     - many tiny: `scale * 10_000` files of 256 bytes (or 2_000 at scale 1 if
       10k is too slow on the host; document the chosen count);
     - few large: `scale * 4` files of 8 MiB (write with a repeated buffer,
       not one 32 MiB allocation);
     - deep: `scale * 200` levels, 2 files per level, each 128 bytes.
   - create directories with `std::fs::create_dir_all`, files with
     `std::fs::write` or a `BufWriter`.
   - return a nonzero exit only on an actual I/O error.
3. Add a short README “Benchmarks” note:
   ```text
   cargo bench -p girsync --bench tree_shapes
   GIRSYNC_BENCH_SCALE=4 cargo bench -p girsync --bench tree_shapes
   ```
   Say it is a local measurement aid, not a gate.
4. No test is required for the bench itself. If a shape helper is shared, keep
   it in the bench file; do not add a public API to the library solely for the
   bench.

## Commit plan

1. `bench(sync): add a three-shape tree benchmark`
2. `docs(sync): explain how to run the benchmark`

## Verification

```text
cargo fmt --all -- --check
cargo bench -p girsync --bench tree_shapes --no-run
cargo bench -p girsync --bench tree_shapes
cargo test -p girsync
```

## Acceptance

- `cargo bench -p girsync --bench tree_shapes` builds and runs without
  `criterion`.
- Three shapes are reported, with walk time and a throughput number.
- The fixture size scales with `GIRSYNC_BENCH_SCALE` and is cleaned up.
- No assertion fails on a slow host.
- No production code changes.

## Reviewer focus

- Is the bench target `harness = false` and dependency-free?
- Does the default run finish quickly?
- Are the generated trees removed on every exit path?
- Is the reported byte count the actual file bytes, not the directory size?
- Did any production file change?
