# T06 — Consolidate gircap default captures and share one read-only bundle

Priority P0 · crate gircap · checkpoint CP2

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L321–L330` (§6.2 1).
* Report priority row: `docs/08-test-runtime-optimization.md:L473` (§8, P0).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T06-gircap-capture-consolidation.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

`capture_layout.rs` (37 tests, ~38 captures), `schema.rs` (21 tests, 22 captures)
and `matrix.rs` (10 tests, 14 captures) redo the same default capture many times
— `schema.rs` alone calls `bundle()` 19× for one document assertion each. Each
capture starts a mock server, spawns `gircap capture`, writes a whole tree,
re-hashes it and deletes it. Group "read document X from the default bundle"
cases into one test per capture variant (default / gzip threshold / ceiling)
and add a **file-based, file-locked, read-only bundle fixture** shared across
suites. Target: remove ~30 captures (~15–20 s on HDD). Note: nextest gives each
test its own process, so an in-process `OnceLock` cannot share a capture — the
sharing must be file-based or the tests must be merged.

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/capture/tests/schema.rs:125` — `bundle()` (19 callers).
* `tools/capture/tests/capture_layout.rs`, `matrix.rs` — capture sites.
* `tools/capture/tests/common/mod.rs` — where a shared fixture helper belongs.
* `World::all_shapes()` at `common/mod.rs:308` builds the source tree used by
  captures (related to T08 — coordinate, don't duplicate effort).

Possible commits (this is a large task — multiple commits expected):

1. Add a shared read-only bundle fixture in `tests/common/mod.rs`: capture once,
   guarded by a file lock / stable on-disk location, cleaned up safely; expose a
   helper that returns paths into it. Ensure concurrent nextest processes cannot
   race on creation or teardown.
2. Migrate `schema.rs` document-reading cases to one test per capture variant,
   reading from the shared bundle instead of calling `bundle()` each time.
3. Migrate the equivalent cases in `capture_layout.rs` and `matrix.rs` to the
   shared bundle / merged tests, keeping the distinct variants (default, gzip
   threshold, ceiling) covered.
4. Confirm the "delete a required document" tests are left for T07.

## Acceptance criteria

* The number of full capture subprocesses across these three files drops
  markedly (report target ~30 fewer).
* Every document/variant assertion that existed still exists (merged into
  grouped tests), and the shared fixture is race-free under parallel nextest.
* `capture_layout` deletion tests remain for T07; do not pre-empt them.

## Guardrails

* Validate against current code first; if capture/fixture structure differs,
  stop and mark `skipped` with a note.
* Keep all distinct capture variants covered — do not lose a variant for the
  sake of fewer captures.
* Test-only change; do not alter capture product behavior.
* The shared fixture must work when each test runs in its own process. Run the
  gircap `capture_layout`, `schema` and `matrix` suites to verify; report
  follow-up ideas without implementing them.
