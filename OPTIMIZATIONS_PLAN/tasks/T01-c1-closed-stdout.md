# T01 - C1: a closed stdout pipe is not a panic

Report: `../../OPTIMIZATIONS_REPORT.md` lines 73-93 (C1), and lines 32-63 for
the contract it protects.

Dependencies: none.

## Goal

`report::emit` is the single writer for `REPORT` / `PROGRESS` / `PLAN` /
`SUMMARY`. It currently uses `println!`, which panics when stdout's reader has
gone away. A completed repair then exits `101` with no `SUMMARY`, even though
the install was mutated.

Make a closed stdout a benign end of the data plane: the line is still mirrored
to the log, the run keeps its documented exit code, and the final `SUMMARY` is
attempted once more (it may still have no reader). Other stdout I/O errors must
also be non-fatal and diagnosable, not panics.

## Where to start

- `src/report/emit.rs`: `emit` (around line 20), `emit_progress`,
  `spawn_progress_reporter`.
- `src/main.rs`: the final `report::emit(&report::summary_record(...), ...)`.
- `src/report/tests.rs` for the record-rendering tests.
- Any call site of `report::emit` is a consumer of the same rule; do not add a
  second writer.

## Steps

1. Add a small internal function in `src/report/emit.rs` that takes a
   `&mut impl std::io::Write` and a rendered line, writes the line plus `\n`,
   and returns `std::io::Result<()>`. Keep it private; this is the unit-test
   seam, not a new public API.
2. In `emit`, render the line once. Mirror it to the log **before** or
   independently of the stdout write, so a failed stdout does not suppress the
   log record. Then lock stdout once (`std::io::stdout().lock()`) and write
   through the helper.
3. Handle the result:
   - `ErrorKind::BrokenPipe`: treat as a benign stop. A `debug!`/`trace!` line
     is enough; do not print to stderr, because that would make a piped run
     noisy after the consumer has gone.
   - any other error: `warn!` once per call with the error kind, but do not
     panic and do not change the exit code. Do not add a global "stdout is
     dead" latch unless the repeated warnings are actually a problem in a
     test; a latch adds cross-thread state that the contract does not need.
4. Leave the record order and the log mirror unchanged. The progress ticker
   already calls `emit`, so fixing the writer fixes the ticker too; verify no
   other code calls `println!` for a data-plane line.
5. Do not change `OutputFormat`, `Record`, or the `SUMMARY` fields.

## Tests

- Unit test the helper with a writer that returns
  `io::ErrorKind::BrokenPipe`: it returns the error, and the caller's `emit`
  path does not panic. Because `emit` writes to the real process stdout, test
  the helper directly; do not try to redirect the global stdout in a unit test.
- Unit test the helper with a short writer that fails on the first write and
  with one that succeeds; assert the exact bytes written include the trailing
  newline.
- The process-level broken-pipe test belongs to T44 (spawn the binary with a
  pipe, close the reader, assert exit `0` and no panic). Do not add a flaky
  global-stdout test here.

## Acceptance

- No path in `report::emit` or the progress ticker can panic on stdout I/O.
- A normal run's stdout bytes are unchanged.
- The log still receives every record even when stdout fails.
- `cargo test -p girpr` passes.

## Commit plan

1. `fix(report): make the data-plane writer survive a closed stdout`
2. `test(report): pin the line writer and broken-pipe handling`

## Notes

- The README's automation contract is the reason this is first. Do not "fix" it
  by catching the panic at `main`; the record would still be lost and the
  contract would still be broken.
- A closed stdout is not an error the process should report via exit code. The
  run's documented code is the one the repair earned.
