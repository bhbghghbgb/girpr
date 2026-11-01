# T11 - P5: size the runtime for the file-task window

Report: `../../OPTIMIZATIONS_REPORT.md` lines 386-408 (P5).

Dependencies: T10.

## Goal

The repair loop spawns async file tasks whose bodies perform blocking
`std::fs` I/O. With `--io-threads` greater than the runtime's worker count, the
tasks compete for workers and the runtime can stall its own timers and network
polling. The flag's documented meaning ("concurrent files") is not what the
runtime actually provides.

This task implements the report's **cheap version**, not a redesign of the
per-file driver: build the runtime with enough worker threads for the requested
file window (plus headroom), and make the relationship visible in the startup
log. Do not wrap whole files in `spawn_blocking` here; the per-file loop mixes
async network with sync disk, and that redesign is explicitly out of scope
before P1-P4.

## Where to start

- `src/main.rs`: `#[tokio::main]`, `parse_args`, `args.into_ctx()`,
  `repair::run(ctx)`.
- `src/cli.rs`: `Args::io_threads` (around line 41) and `into_ctx` (around line
  255) where it becomes `jobs`.
- `src/config.rs`: `RunCtx::jobs`.
- `README.md`: the `--io-threads` description in the usage table, if it claims
  runtime threads.

## Steps

1. Replace the `#[tokio::main]` attribute with an explicit
   `tokio::runtime::Builder`:
   - parse args and install logging as today;
   - build the `RunCtx` first (usage errors still exit before any runtime is
     built);
   - `worker_threads` = `max(available_parallelism, ctx.jobs + 1)`; use
     `std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)` and
     saturate the addition so a huge `--io-threads` cannot overflow;
   - `.enable_all()` so time/IO drivers remain available;
   - if `build()` fails, print the error and exit `1` (a usage/environment
     failure, not a run failure).
2. Move the async part of `main` into a helper that returns the process code
   instead of calling `std::process::exit` in the middle. Keep the existing
   usage/fatal `eprintln!` and exit codes. T22 will later replace the remaining
   `process::exit` with an `ExitCode`; leave that to T22.
3. Log the chosen worker count and the file window at `info`:
   `worker_threads=... jobs=...` (the spelling is not a contract, but the two
   facts must be visible).
4. Do not change the repair loop's `jobs` bound, the `JoinSet` window, or the
   chunk order. This task only changes the runtime size.
5. Update the `--io-threads` help/README wording if it says "tokio worker
   threads"; it bounds concurrent files. T19 changes the flag surface; keep
   this task's wording compatible with that (no rename here).

## Tests

- The full `cargo test -p girpr` suite, including the binary-level tests, must
  pass. Runtime construction is exercised by every subprocess test.
- Add a small unit test if the worker-count calculation is extracted into a
  pure function: `jobs=1` on a 1-core host yields at least 2; a huge `jobs`
  saturates rather than overflowing. Do not test `available_parallelism`
  itself.
- Do not add a timing or thread-count assertion to a flaky integration test.

## Acceptance

- `--io-threads N` no longer starves the runtime when `N` exceeds the host's
  default worker count.
- Usage errors still exit `1` without building a runtime.
- No repair behavior or output changes.
- `cargo test -p girpr` passes.

## Commit plan

1. `perf(main): size the runtime for the requested file window`
2. `test(main): pin the worker-count calculation` (only if extracted)

## Notes

- This is a ceiling fix, not a concurrency model. The report's real version is
  a per-file driver redesign; do not start it here.
- A2/T19 handles the user-facing name and warning. Coordinate the help text,
  not the runtime.
