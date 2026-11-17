# T01 — Lower the girsync heartbeat fixture via a test hook

Priority P0 · crate girsync · checkpoint CP1

## Pointers (give these to the implementing session)

Read these sections to understand the goal and the measurement; do not paste
them into the session:

* Report finding: `docs/08-test-runtime-optimization.md:L230–L239` (§6.1 A).
* Report priority row: `docs/08-test-runtime-optimization.md:L470` (§8, P0).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T01-girsync-heartbeat-fixture.md` (whole file).
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

`the_walk_counts_what_it_saw_and_the_map_counts_what_it_kept` creates 1,999
one-byte files only to trip the production walk heartbeat (`HEARTBEAT_ENTRIES =
2000`). Almost all of its 7 s on HDD is waiting on those creates/opens/removes;
the "counts what it saw / kept" assertions do not care about the real period.
Make the period configurable for tests and build only a handful of files, so the
assertions stay identical while ~99% of the fixture IO disappears.

## Implementation plan

Code anchors from the report (validate before editing — they may be stale):

* `tools/sync/tests/output.rs:840–1018` — the test; loop at `:846`, assertions
  at `:911`, `:936–943`, `:959`, `:975`, `:1000`, `:1012`, `:1018`.
* `tools/sync/src/scan.rs:26` — `HEARTBEAT_ENTRIES: usize = 2000`; used at
  `:137`, `:161`.

Possible commits (combine if small):

1. Add a test-only source of the heartbeat period in `scan.rs` — e.g. read an
   env var (`GIRSYNC_TEST_HEARTBEAT_ENTRIES`) with the production 2000 as the
   default, so behavior is unchanged unless the hook is set. Keep the variable
   test-only and default-off, matching the existing `GIRSYNC_TEST_*` convention
   (see `.config/nextest.toml`'s hook comments). Document it alongside the
   other hooks if that is where such knobs are listed.
2. In the test, set the hook to a small value (8 in the report) on the spawned
   child, build that many files, and express the expected counts in terms of
   the same value so the assertions remain self-consistent. Keep the memory
   cache env already used by this suite.

## Acceptance criteria

* The test still asserts the heartbeat counts and the kept/live totals, now
  derived from the reduced period — not from the literal 2000.
* Production (`HEARTBEAT_ENTRIES` with no hook) remains 2000 and its behavior is
  unchanged.
* The test's wall time and filesystem-operation count drop substantially
  (target: skip creating ~2,000 files).

## Guardrails

* Validate the current code matches the report before editing; if the fixture
  or heartbeat mechanism has changed, stop and mark the task `skipped` with a
  note rather than forcing it.
* This is test-only behavior; do not alter the production walk's heartbeat
  semantics. The env knob must default off in production.
* You may improvise the hook mechanism if it is cleaner (e.g. a helper on the
  existing test-config path), as long as the goal is met.
* Run the girsync `output` test suite to verify; report any follow-up ideas in
  your final message (do not implement them).
