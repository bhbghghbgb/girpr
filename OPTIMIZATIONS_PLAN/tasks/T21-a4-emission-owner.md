# T21 - A4: one emitter policy and one progress driver

Report: `../../OPTIMIZATIONS_REPORT.md` lines 904-923 (A4).

Dependencies: T01 (the writer error policy). T10 may have already moved the
check loop onto the blocking pool; adapt to its shape.

## Goal

`report::emit` is called from metadata/plan, check, and main; the progress
reporter owns a ticker; the check loop also emits progress inline. There is no
single owner of "records are emitted here, in this order, with one error
policy, and exactly one final SUMMARY".

Keep the free `emit` function as the writer (the report says a new abstraction
is not needed). Add the missing pieces: one progress ownership rule, one
emission seam for the final SUMMARY, and one place that documents the order.

## Where to start

- `src/report/emit.rs`: `emit`, `emit_progress`, `spawn_progress_reporter`.
- `src/report/record.rs`: `PROGRESS_INTERVAL_SECS`, `Record`.
- `src/repair/file/mod.rs`: `repair_all` starts the ticker (around line 223).
- `src/repair/phases/check.rs`: `verify_plan` emits progress inline (around
  line 107). If T10 has already replaced the loop, this is the new async fold.
- `src/main.rs`: the final `report::emit(&summary_record(...))`.
- `src/repair/phases/metadata.rs` / `src/repair/mod.rs`: the `REPORT`/`PLAN`
  emissions.

## Steps

1. Add a small `ProgressGuard` (or equivalent RAII helper) in
   `report::emit` that starts the ticker and aborts/joins it on drop. Give it
   the shared `Arc<Summary>` and the `OutputFormat`.
2. Use exactly one progress owner per phase that has long-running work:
   - the repair loop (`repair_all`) uses it;
   - check-only uses it instead of its inline `last_emit` timer;
   - do not emit progress from a blocking task or from a per-file loop.
   The ticker remains the only `PROGRESS` producer.
3. Remove the inline progress emission from `verify_plan` (or its T10
   replacement). The counters are still updated as work completes; the ticker
   reads them.
4. Make the SUMMARY emission a single call site in `main` (or one helper it
   calls) and state in a comment that it is the last record of the run. Do not
   emit SUMMARY from a phase.
5. Do not add a global writer or a trait object. `emit` remains the function;
   the policy is: render once, mirror to the log, write stdout through the T01
   helper, never panic, never change the exit code.
6. Keep the record order: `REPORT` (metadata), `PLAN` (when requested),
   zero or more `PROGRESS`, `SUMMARY` last. `--print-plan` still ends with
   `PLAN` and its `SUMMARY` (whatever the current contract is); do not reorder.
7. Document the order and the ownership rule in `report/emit.rs` or the README
   section that describes the data plane.

## Tests

- A repair and a check-only run each emit at most one PROGRESS stream (no
  duplicate tickers) and exactly one SUMMARY. Use a test subscriber/format
  seam if one exists; otherwise assert the helper's start/stop behavior in a
  unit test with a short interval.
- The ticker stops when the phase returns, including an early return/error.
  Test the guard's drop.
- Existing `report` tests and `tests/cli_contract.rs` pass; update only the
  expected progress cadence if a test pinned the inline check emission.
- `cargo test -p girpr` passes.

## Acceptance

- One progress ticker per long-running phase; check-only no longer emits
  progress from its loop.
- One final SUMMARY, emitted by one call site.
- The emission error policy is the T01 policy everywhere.
- No new global writer or trait abstraction.
- `cargo test -p girpr` passes.

## Commit plan

1. `refactor(report): own progress with one guard`
2. `refactor(check): drop the inline progress emission`
3. `test(report): pin one ticker and one final summary`

## Notes

- A guard is chosen over a global so two phases cannot silently run two
  tickers. `JoinHandle::abort` is enough; do not block the runtime waiting for
  a ticker that may be sleeping.
- If T10 has not landed yet, coordinate the check-loop shape rather than
  reintroducing the inline timer.
