# T15 — Remove girpr `--help`/argument subprocess spawns

Priority P1 · crate girpr · checkpoint CP4

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L387–L394` (§6.3 2).
* Report priority row: `docs/08-test-runtime-optimization.md:L476` (§8, P1).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T15-girpr-arg-subprocess.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

`tests/cli_contract.rs` spawns the binary ~30×; every run initializes tracing and
leaves a `logs/girpr_<ts>.log` file beside the binary whose contents are never
read. Convert pure text/help cases to in-process clap (`Args::command().render_help()`
etc.), keeping a couple of binary cases for the process exit-code contract; or
add a default-off `GIRPR_TEST_NO_LOG=1` hook to skip file logging for
argument-only runs. The same idea applies to the run-lock file (lock behavior has
its own 15-test `run_lock.rs` suite).

## Implementation plan

Code anchors from the report (validate before editing):

* `tests/cli_contract.rs` — the ~30 binary spawns for argument/help cases
  (header at `:9–14`; a `--log-file-level` case at `:1256`).
* `tests/run_lock.rs:207` already uses `Args::command()` in-process — reuse that
  pattern.
* Exit-code contract must keep at least one/two real binary spawns.

Possible commits:

1. Convert pure help/parse/text cases to in-process clap assertions
   (`Args::command().render_help()`, `try_parse_from`, etc.).
2. Keep a minimal set of binary spawns that genuinely test exit codes and
   end-to-end argument handling.
3. If any remaining argument-only spawns still create log/run-lock files and do
   not need them, add a test-only, default-off hook to skip that file creation —
   validate before touching product logging/lock code.

## Acceptance criteria

* The number of binary spawns in `cli_contract.rs` drops substantially; pure
  help/parse cases are in-process.
* The process exit-code contract is still covered by real binary cases.
* Argument/help output assertions are unchanged.

## Guardrails

* Validate against current code first; if the spawn structure differs, stop and
  mark `skipped` with a note.
* Do not remove all binary cases — keep the exit-code contract. Do not change
  production logging or lock behavior; any hook must be test-only, default-off.
* Run the girpr `cli_contract` suite to verify; report follow-up ideas without
  implementing them.
