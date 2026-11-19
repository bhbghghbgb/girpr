# T08 — `tracing-appender` as the single file-log writer

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A9 (lines 278–302).
- Assumption to validate: §I.4 (lines 825–827).

## Scope
- Package: root `girpr`, `src/logging.rs` (`FileSink`, `FILE_SINK`,
  `SharedWriter`, `init_tracing_with`, `flush_log`) and `src/main.rs` exit paths.
  The two tools already depend on `tracing-appender`.

## Implementation plan
1. Add `tracing-appender` to the root manifest.
2. Replace the custom `MakeWriter` / `Arc<Mutex<Box<dyn Write>>>` / global sink
   with `tracing_appender::non_blocking(f)` → `NonBlocking` + `WorkerGuard`.
3. Hold `Option<WorkerGuard>` in a static mutex; make `flush_log()` take and
   drop the guard.
4. Audit every `std::process::exit` path in `src/main.rs` so the explicit flush
   seam is called (guard drop will not otherwise run on exit).
5. `girpr-log` keeps deciding *where* the file goes.

## Constraints / validation
- Logging becomes non-blocking; `tail -f` still lags by a buffer.
- `WorkerGuard` has no public mid-run `flush()` — mid-run flush semantics are
  lost (§A9). If exit paths cannot be made safe, stop rather than regress flush
  on exit.
- Tests named in §A9 and `tests/cli_contract.rs` log assertions.

## Done when
The custom file sink is replaced and every exit path is verified to flush.
