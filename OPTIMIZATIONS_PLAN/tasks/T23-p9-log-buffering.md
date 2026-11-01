# T23 - P9: buffer the file log sink

Report: `../../OPTIMIZATIONS_REPORT.md` lines 480-500 (P9).

Dependencies: T22 (the explicit flush seam).

## Goal

The file log sink is an unbuffered `Mutex<Box<dyn Write>>`. Every event takes
the mutex, formats, and issues a write syscall; long runs with per-file and
per-chunk debug events serialize workers on it. Wrap the file in a buffered
writer to cut syscalls and contention, without losing the tail at exit.

## Where to start

- `src/logging.rs`: the writer construction (around line 95), the file layer
  (around line 105), `open_log_file` (around line 131), the tests.
- The `flush_log()` seam added by T22.
- `src/main.rs`: the flush call sites.

## Steps

1. Change the file writer's inner type from `File` to
   `std::io::BufWriter<File>` with an explicit capacity (for example 64 KiB).
   Keep the `Mutex` (or the T22 shared-writer wrapper) so the tracing layer's
   `MakeWriter` contract is unchanged.
2. Keep the console layer unbuffered; this task is the file only.
3. Ensure every intended flush actually flushes the buffer:
   - the T22 `flush_log()` at normal/fatal exit;
   - any other explicit flush site T22 added.
   Do not add a flush per event; that would undo the buffering.
4. Error policy: a failed write is the tracing subscriber's problem and is
   already swallowed by the writer; a failed flush at exit should be reported
   once to stderr, not change the run's exit code. If a buffered write fails
   mid-run, the layer's behavior must not panic.
5. Do not use `tracing-appender` or a non-blocking appender. The report's
   trade-off is explicit: that adds a dependency and can drop records.
6. Document the durability story in `logging.rs`: "buffered for the run;
   explicitly flushed at exit and on fatal paths; a kill -9 can lose the tail."
   Do not promise more than that.
7. Keep `--no-log-file` on `std::io::sink()`; buffering a sink is pointless, so
   either keep the sink unbuffered or use a buffered sink with the same flush
   semantics. Prefer the simple sink.

## Tests

- A writer-level test (not a full process) proves events land in the buffer and
  appear only after `flush_log()`, using a temporary file and a manual flush.
  If the tracing subscriber makes this awkward, test a small
  `BufferedFileSink` wrapper directly.
- A process-level run with `--log-file` asserts the log's last line is present
  after exit (this is the regression the buffer could cause). The existing
  process tests can be extended.
- `--no-log-file` leaves no file and does not panic.
- `cargo test -p girpr` passes.

## Acceptance

- The file sink is buffered; the console sink is not.
- The log tail is present after a normal and a fatal exit.
- No new dependency and no non-blocking appender.
- The durability limit (kill -9) is documented.
- `cargo test -p girpr` passes.

## Commit plan

1. `perf(logging): buffer the file log sink`
2. `test(logging): pin the flush-before-exit tail`

## Notes

- The mutex is still needed: `tracing` can write from several threads. The
  buffer reduces syscalls, not the lock; do not remove the lock.
- Do not buffer stdout; the data plane's ordering and broken-pipe behavior are
  separate contracts.
