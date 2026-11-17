# T14 — Reduce girpr trace-level logging in repair_machinery

Priority P1 · crate girpr · checkpoint CP4

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L377–L385` (§6.3 1).
* Report priority row: `docs/08-test-runtime-optimization.md:L476` (§8, P1).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T14-girpr-trace-logging.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

Each `Rig::run()` in `tests/repair_machinery.rs` spawns the binary with
`--log-file-level trace`, writes a trace log to TEMP, reads it back and then
`snapshot()`s/MD5s every file. Several tests call `run()` 2–3×. Only ~3 tests
actually assert chunk-level log records. Make the log level configurable per
test and use `trace` only for those; default to `debug`/`warn` elsewhere, cutting
log IO and hashing without losing any log-content assertion.

## Implementation plan

Code anchors from the report (validate before editing):

* `tests/repair_machinery.rs:104–205` — `Rig::run()`;
  `--log-file-level trace` at `:176–184`; log read at `:203`; `snapshot()` at
  `:239–262`.
* Tests that call `run()` more than once: `:791`, `:1145`, `:1176`.
* Tests that genuinely need trace/chunk-level records: around `:1535`, `:1210`,
  `:1278`.

Possible commits:

1. Add a per-`run()` log-level parameter (default `debug`/`warn`) and keep
   `trace` only where chunk-level records are asserted.
2. Remove log read-back/MD5 of the log where the test no longer inspects it,
   without changing the file-tree `snapshot()` assertions.
3. Confirm the ~3 trace-dependent tests still receive and assert the records
   they need.

## Acceptance criteria

* Only the few tests that assert chunk-level log records request `trace`.
* All `repair_machinery` tests still pass; log-content assertions are unchanged.
* The general file-tree/hash assertions still run (do not drop them to save time).

## Guardrails

* Validate against current code first; if the log-level wiring differs, stop and
  mark `skipped` with a note.
* Test-only change; do not alter the binary's logging implementation or the
  shared log policy in `crates/log`.
* Run the girpr `repair_machinery` suite to verify; report follow-up ideas without
  implementing them.
