# testprof — the test suite's time-cost instrument

The scripts behind
[`docs/07-test-suite-time-cost.md`](../../docs/07-test-suite-time-cost.md).

They answer one question per test: **how long did it take, how much of that
was CPU, and how much was waiting?** The suite spends 94 % of its time
blocked, so the second number is the interesting one.

| script | what it does |
|---|---|
| `measure.ps1` | runs **one** test binary serially and writes one row per test: wall ms, CPU ms, read/write ops and bytes |
| `drive.ps1` | runs `measure.ps1` over every test binary in the workspace, serially and at the default thread count |
| `aggregate.ps1` | turns the per-test CSVs into the tables the report quotes |

## Running it

```powershell
cargo test --workspace --no-run     # build the harnesses first
.\tools\testprof\drive.ps1           # per-test rows  -> target\testprof\*.csv
.\tools\testprof\aggregate.ps1       # the tables     -> target\testprof\agg\
```

`drive.ps1` takes about 15 minutes on the machine the report was measured on
(serial plus parallel over every binary). `aggregate.ps1` is instant.

## Why not `cargo nextest --report-time`

`cargo-nextest` is not installed, and `--report-time` / `--format json` are
nightly-only libtest features. The toolchain here is stable, so the
per-test boundaries are read off libtest's own output instead: with
`--test-threads=1`, every `test <name> ... ok` line is a clean boundary, and
the harness timestamps it and samples the child process's CPU time and I/O
counters at the same instant.

## The three Windows details that are easy to get wrong

They are all in `measure.ps1`, and all three produce *plausible but wrong*
numbers rather than errors:

1. **`$p.StandardOutput` is null until `Start()` has been called**, and
   enabling `BeginOutputReadLine` detaches it again. The reader has to be
   captured in between, or the harness dies on a null-valued expression.
2. **stderr must be drained asynchronously**, or a chatty test deadlocks
   against a full pipe. The handler has to be attached from C# (`Add-Type`):
   a PowerShell script-block delegate needs a runspace and fails on the
   thread-pool thread the event arrives on.
3. **The child must be the test binary itself**, not `cmd.exe` wrapping it.
   `cmd.exe`'s CPU time does not include its grandchild's, which would
   silently under-report every CPU figure.

## Reading the output

- **wall_ms** — the test's elapsed time, boundary to boundary.
- **cpu_ms** — the child process's user + kernel CPU over the same interval.
- **wait = wall − cpu** — the time the test spent *blocked*: sleeping, in a
  file lock, in an fsync, in a connect, or waiting for a child process. This
  is the number the report is about.
- **read/write ops and KB** — the *test process's* I/O counters. Work done in
  a spawned child (a `gircap` capture, a `girsync` run) does not appear here;
  it shows up in `wall_ms` only.
