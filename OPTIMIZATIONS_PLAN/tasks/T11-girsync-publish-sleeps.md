# T11 — Replace girsync publish fixed sleeps with signals

Priority P1 · crate girsync · checkpoint CP3

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L284–L291` (§6.1 C6).
* Report priority row: `docs/08-test-runtime-optimization.md:L475` (§8, P1).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T11-girsync-publish-sleeps.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

Four publish tests set `PUBLISH_PAUSED` and call `pause_publish()`, which sleeps a
fixed **500 ms** before the test's `join()` — about 2 s of pure dead time. A
similar 150 ms sleep exists in `cache/write.rs`. Replace the fixed sleeps with a
one-shot signal/condvar: block in `pause_publish()` until the main test has
observed the claim, then release. This makes the race window deterministic and
removes the dead time.

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/sync/src/cache/open.rs:1064` — `pause_publish()` (product code, test
  support); publish tests at `:2208`, `:2367`, `:2456`, `:2502`.
* `tools/sync/src/cache/write.rs:586` — the 150 ms interval sleep.

Possible commits:

1. Introduce a test-only signal primitive (e.g. a one-shot condvar/channel in the
   existing test-support area) and have `pause_publish()` wait on it instead of
   sleeping. Give the four tests a handle to release it once the claim/state is
   observed.
2. Apply the same pattern to the 150 ms interval test in `write.rs`.
3. Remove the fixed sleeps and confirm no test still depends on wall-clock
   timing.

## Acceptance criteria

* No fixed 500 ms / 150 ms sleeps remain in these tests.
* The race window is still exercised deterministically — the paused thread is
  actually observed in the expected state before it is released.
* Tests pass repeatedly (run them more than once) to confirm no flakiness.

## Guardrails

* Validate against current code first; if the sleep/pause structure differs,
  stop and mark `skipped` with a note.
* This is test-support behavior inside product files — change only the test-only
  hooks; do not alter production publish/write behavior. If the wait primitive
  would need to be always-compiled, validate before doing so.
* You may improvise the primitive (channel, Barrier, condvar) as long as it is
  dead-time-free and deterministic. Run the affected girsync cache tests
  repeatedly; report follow-up ideas without implementing them.
