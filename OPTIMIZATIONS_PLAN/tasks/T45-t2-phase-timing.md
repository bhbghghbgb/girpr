# T45 - T2: log phase and per-file durations

Report: `../../OPTIMIZATIONS_REPORT.md` lines 1102-1116 (T2).

Dependencies: T44. Keep the seam small; the regression tests may use it.

## Goal

Every phase logs start/end facts but no durations. The performance findings are
theoretical without a way to see where a real run spends its time. Add one
`duration_ms` field per phase at `info` and per-file totals at `debug`, so the
next optimization decision is evidence-based and P1-P8 can be validated.

## Where to start

- `src/repair/mod.rs`: `run`, `plan_phase`, `execute_phase`, and the phase
  calls (metadata, manifest, scan, purge, repair, post).
- `src/repair/file/mod.rs`: `file_task`/`repair_one_file` (per-file).
- `src/repair/phases/check.rs`: check-only.
- `src/report/emit.rs` / `src/logging.rs`: the log fields; do not add a new
  record type.
- `src/sleep.rs`: a clock seam if the tests need deterministic durations.

## Steps

1. Add a small `PhaseTimer` (or a pair of `Instant` values) that records a
   start and emits one line on drop/finish:
   ```text
   phase=<name> duration_ms=<n> <existing fields>
   ```
   Keep the existing start/end facts; add the duration to the completion line
   rather than emitting a new record. Use `tracing::info!`.
2. Instrument the top-level phase boundaries:
   - metadata;
   - manifest (latest + local map);
   - plan build + purge planning;
   - scan (`--scan`);
   - repair loop (whole);
   - check-only;
   - post-phase;
   - the whole run.
   One duration per phase; the whole-run duration is not a substitute.
3. Per-file: in `file_task`/`repair_one_file`, log `duration_ms` at `debug`
   with the file rel. Do not log every chunk; that is too noisy. A per-file
   total is enough for T2.
4. Use a monotonic clock (`std::time::Instant`). Report milliseconds as
   `u128`/`u64` consistently; do not use floating point in the data plane.
5. Do not add a `PROGRESS` or `SUMMARY` field for durations. This is
   observability in the log, not an output-contract change.
6. If the tests need a deterministic seam, put the clock behind a tiny trait
   or a function pointer only where the test asserts; do not add timing
   assertions to CI. The report says `info` at phase boundaries and `debug`
   per file.

## Tests

- A unit test for the `PhaseTimer`/duration helper with an injected clock
  (start, end, expected ms) if one is added.
- A process test (or a log-capture test) asserts the phase lines contain
  `duration_ms=`. Do not assert a value.
- `SUMMARY` and the stdout data plane are unchanged.
- `cargo test -p girpr` passes.

## Acceptance

- Every phase completion line carries `duration_ms`.
- Per-file completion carries `duration_ms` at debug.
- No output-contract change and no timing assertion in CI.
- `cargo test -p girpr` passes.

## Commit plan

1. `feat(logging): log phase durations`
2. `feat(file): log per-file repair duration`
3. `test(logging): pin the duration field without timing`

## Notes

- Keep the field name stable: `duration_ms`. A reader will grep it.
- Do not log a duration for a phase that did not run (a skipped scan, an early
  return). Absence is meaningful.
