# T22 - A5: return an exit code and flush the log on the way out

Report: `../../OPTIMIZATIONS_REPORT.md` lines 925-943 (A5).

Dependencies: T21. T23 (buffered log sink) depends on this task, so define the
flush seam here and let T23 fill it.

## Goal

`main` calls `std::process::exit` after `SUMMARY` and on fatal errors.
`process::exit` skips destructors, so it is safe today only by accident; it
becomes a lost log tail the moment the file sink is buffered (P9). Replace the
post-run exits with a normal return of `ExitCode`, and add one explicit log
flush that runs on every path after logging is installed.

## Where to start

- `src/main.rs`: the whole entry point (`#[tokio::main]`, `parse_args`,
  `process::exit` at lines 40, 63, 68, 84).
- `src/logging.rs`: `init_tracing`, the `Mutex<Box<dyn Write + Send + Sync>>`
  writer (around line 95), `open_log_file`.
- `src/config.rs`: `RunFailure.exit_code`.
- `src/report/emit.rs`: the final SUMMARY emission.

## Steps

1. Change the async entry point to return `std::process::ExitCode`:
   `#[tokio::main] async fn main() -> ExitCode`. Replace the success and fatal
   `process::exit` calls with `return ExitCode::from(code as u8)` (or a small
   `exit_code(i32) -> ExitCode` helper that clamps/handles the documented
   values). Returning from `main` lets the runtime shut down normally and drops
   in-scope guards.
2. Keep argument-parse failures before logging. `parse_args` may still exit
   directly with clap's `0`/`1` handling, or return `Result<Args, ExitCode>`;
   choose one and keep `--help`/`--version` at `0`. A usage error after
   logging is installed must go through the normal return.
3. Add a logging flush seam:
   - make the file writer shareable (`Arc<Mutex<...>>` or a small `MakeWriter`
     wrapper) and keep a handle reachable from a `flush_log()` function;
   - `flush_log()` locks, flushes, and reports an error to stderr at most once;
   - it must be safe when no file was opened (sink) and when logging was never
     installed.
   T23 will replace the inner writer with a `BufWriter`; the seam stays the
   same.
4. Call `flush_log()` on all paths after `init_tracing`:
   - normal return after SUMMARY;
   - fatal `RunFailure`;
   - the post-parse usage error;
   - a panic is not required to be caught (do not add a panic hook unless the
     existing code already has one).
5. Do not rely on `Drop` alone for the log: the report's durability story is
   "the log file is the record of the run", so the flush is explicit.
6. Update the module doc in `main.rs` to say "parse, install tracing, run,
   emit SUMMARY, flush, return the exit code". Do not add report/plan
   references.

## Tests

- A normal run still emits the same SUMMARY and exits with the same code.
  The process-level tests in `tests/` already assert exit codes; keep them.
- A fatal run (metadata/write error) flushes the file: add a test that runs the
  binary with `--log-file`, forces a failure, and asserts the log's tail
  contains the fatal line. If forcing a failure is too broad, test
  `flush_log()` directly with a writer that records `flush` calls.
- A run with `--no-log-file` still works and `flush_log()` is a no-op.
- `cargo test -p girpr` passes, including the `CARGO_BIN_EXE_girpr` tests.

## Acceptance

- No `process::exit` after the run has started; the exit code is returned
  normally.
- Every logged path flushes the file before returning.
- The log flush is one seam, ready for T23's buffer.
- Exit codes are unchanged.
- `cargo test -p girpr` passes.

## Commit plan

1. `refactor(main): return ExitCode instead of exiting after the run`
2. `feat(logging): add an explicit log flush seam`
3. `test(logging): pin flushing on success and fatal paths`

## Notes

- Do not enable `panic = "abort"`; the report explicitly warns that it would
  bypass Drop-based cleanup (locks, temp files).
- Keep the lock's release in `execute_phase`; this task is about the top-level
  exit and the log, not about relocating lock ownership.
