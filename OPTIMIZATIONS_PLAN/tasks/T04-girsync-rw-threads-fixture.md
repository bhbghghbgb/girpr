# T04 — Shrink the girsync rw_threads 96-file fixture

Priority P0 · crate girsync · checkpoint CP1

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L248–L257` (§6.1 A3).
* Report priority row: `docs/08-test-runtime-optimization.md:L472` (§8, P0).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T04-girsync-rw-threads-fixture.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

The rw_threads tests build a 96-file × 2-side, 512 B fixture and rebuild it about
12 times across limiter/through-the-binary tests. Two of the slowest tests in
the whole suite are the consumers, and a `hashed > 50` guard forces >100 files to
be re-hashed each pass. Reduce the pad count and the guard proportionally, and
build the fixture once where a test makes several passes. The invariant these
tests check is "more files than workers" and report parity across rw settings —
both hold with a much smaller fixture.

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/sync/tests/rw_threads/main.rs:116–132` — the 96-file fixture builder;
  `:127` is the pad count cited in §8.
* Rebuild sites: `limiter.rs:114,154,202`, `through_the_binary.rs:39` (×6).
* `limiter.rs:119` — `hashed > 50` guard (per side).
* Affected tests: `sync_reports_the_same_actions_under_every_rw_setting`
  (10.99 s nextest) and `every_rw_setting_answers_the_same_question` (5.91 s).

Possible commits:

1. Introduce one shared pad-count constant (report suggests 16 — still greater
   than the widest 8-worker limit) and use it everywhere the 96 is hard-coded;
   lower the `hashed > 50` guard to match.
2. Refactor tests that run several passes to construct the fixture once and
   reuse it, rather than rebuilding per pass. Keep the file content/bytes
   identical between sides and passes.
3. Confirm the largest worker-limit setting is still strictly less than the
   pad count so "more files than workers" remains true.

## Acceptance criteria

* All rw_threads tests pass with the reduced fixture; "more files than workers"
  and cross-setting parity are still genuinely exercised.
* The `hashed` guard is consistent with the new count and still asserts
  meaningful hashing.
* Fixture construction happens once per test where multiple passes occur.

## Guardrails

* Validate against current code first; if the fixture/guard structure differs,
  stop and mark `skipped` with a note.
* Do not reduce the count below the widest worker setting (the report's 8), or
  the invariant under test is lost.
* Test-only change; do not touch rw-limit product code.
* You may choose a different small count if justified. Run the girsync
  `rw_threads` suite to verify; report follow-up ideas without implementing them.
