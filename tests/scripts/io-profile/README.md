# Test I/O profiler

These scripts measure how long each test takes, how
much of that is CPU, and how much file-system work it does. They are Windows
tools because the accounting uses a job object, which aggregates the counters
of the test process *and every child it spawns*.

- `iotime/` — runs one test at a time inside a job object and records wall
  time, CPU time, and the job's read/write/other operation and byte counters.
- `iot-bench/` — microbenchmarks of the primitives the tests hit: fsync of a
  copied file, redb commits and opens, fresh cache creation.
- `make-tasks.ps1` — turns `cargo nextest list` metadata into the TSV `iotime`
  consumes.
- `analyze.ps1` — turns one or two sweeps into the per-test and per-suite
  tables; with a second sweep it also writes the storage-attributable delta.
- `run-sweep.ps1` — does the whole serial sweep and analysis.

Everything the scripts write goes under `target/io-profile/`, which is
git-ignored. The copies of the numbers quoted in the report live there too, so
they survive cleanup of the system temp directory but are not committed. In
that directory, `analysis/z-vs-e-tests.csv` and `analysis/z-vs-e-suites.csv`
are the recorded differential under the column names used when the report was
written; a fresh `analyze.ps1 -CompareCsv` run writes the same data as
`compare-tests.csv` and `compare-suites.csv`.

## Quick start

From the repository root, in PowerShell 7:

```pwsh
# Serial sweep on the disk under test (%TEMP% as configured).
pwsh -File tests/scripts/io-profile/run-sweep.ps1

# Same sweep with TEMP redirected to a RAM disk, for the differential.
pwsh -File tests/scripts/io-profile/run-sweep.ps1 `
    -TempRoot Z:\tmp -Out target/io-profile/ram

# Re-analyze, this time with the HDD sweep as baseline.
pwsh -File tests/scripts/io-profile/analyze.ps1 `
    -Tests target/io-profile/tests.json `
    -Csv   target/io-profile/tasks.tsv.csv `
    -CompareCsv target/io-profile/ram/tasks.tsv.csv `
    -Out   target/io-profile/analysis
```

`cargo nextest list` builds the test binaries on first use, so the first run is
slower. The sweep is deliberately serial (concurrency 1): parallel tests
overlap their I/O, which destroys per-test attribution. Compare its total with
`cargo nextest run --workspace` for the parallel wall-clock baseline; the gap
between the two is itself a measurement of how much the suite loses to I/O
queueing.

## Microbenchmarks

```pwsh
cargo build --release `
    --manifest-path tests/scripts/io-profile/iot-bench/Cargo.toml `
    --target-dir target/io-profile/iot-bench-target
target/io-profile/iot-bench-target/release/iot-bench.exe E:\scratch
target/io-profile/iot-bench-target/release/iot-bench.exe Z:\scratch
```

Use an empty scratch directory; the tool creates and removes its own files.
Run it once per disk and compare the `per_op` figures, not the totals: the
number of operations is fixed so the totals are comparable, but a single noisy
run is still only worth about 10%.

## Reading the output

- `WaitMs` in `tests-ranked.csv` is `wall - CPU`. It includes every deliberate
  sleep (`repair_offline`, the CLI contract tests, transport retry tests), so
  it is an upper bound on I/O wait, not I/O wait itself. The differential run
  separates them: a test whose wall time does not change when the same work
  runs from a RAM disk is sleeping, not waiting on the disk.
- `cpu_period_ms` is the CPU charged to the job during the run, including
  children. A test that builds or copies a large fixture shows it here.
- Bytes from `CopyFileEx` (`std::fs::copy`) are not reliably counted by the
  job object's write counter; treat `WriteMB` as a lower bound for tests that
  copy rather than write.
- Each run writes the CSV next to the task list. Keep the two runs you intend
  to compare in separate `-Out` directories so one does not overwrite the
  other.
