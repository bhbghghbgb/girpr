# T12 — Replace girpr's 300 ms panic-ordering sleep with a signal

Priority P1 · crate girpr · checkpoint CP3

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L402–L406` (§6.3 4).
* Report priority row: `docs/08-test-runtime-optimization.md:L475` (§8, P1).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T12-girpr-panic-ordering-sleep.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

`src/repair/file/mod.rs` sleeps 300 ms in task 0 so that task 1's panic is
observed first — pure wasted time in a test. Replace it with a `Barrier`/channel
signal so task 0 waits until the panic has been observed, then proceeds. The
report also notes a 5 ms `ShortSleep` in `repair/phases/manifest.rs:609–616` that
can be removed by asserting attempt counts with `NoSleep` instead.

## Implementation plan

Code anchors from the report (validate before editing):

* `src/repair/file/mod.rs:1009–1037` — the 300 ms sleep in task 0 (PRODUCT CODE,
  test-only path).
* `src/repair/phases/manifest.rs:609–616` — 5 ms `ShortSleep`.

Possible commits:

1. Replace the 300 ms sleep with a synchronization primitive that task 1 signals
   after its panic/observation, so the ordering is guaranteed without waiting.
2. Replace the 5 ms `ShortSleep` case with `NoSleep` plus an assertion on the
   number of attempts (the behavior actually under test).
3. Remove the now-unused sleep imports/branches if nothing else uses them.

## Acceptance criteria

* The panic-ordering test still deterministically observes task 1's panic before
  task 0 proceeds, with no fixed sleep.
* The `ShortSleep` case still asserts the retry/attempt behavior with `NoSleep`.
* Tests pass repeatedly without flakiness.

## Guardrails

* Validate against current code first; if these sleeps no longer exist, stop and
  mark `skipped` with a note.
* Product files are involved — change only the test-only timing paths; do not
  alter production repair behavior or retry policy.
* You may improvise the synchronization primitive. Run the affected girpr unit
  tests repeatedly; report follow-up ideas without implementing them.
