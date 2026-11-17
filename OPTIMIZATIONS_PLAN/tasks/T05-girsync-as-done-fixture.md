# T05 — Shrink the girsync `as_done` 11 MB fixture

Priority P0 · crate girsync · checkpoint CP1

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L241–L246` (§6.1 A2).
* Report priority row: `docs/08-test-runtime-optimization.md:L472` (§8, P0).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T05-girsync-as-done-fixture.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

`tests/sync_plan/as_done.rs` builds 24 files of 1 KB…~920 KB (~11 MB), which the
binary copies again (~22 MB total), only to assert the *set* of copy events.
Content and size are never read back. Shrink to small files while keeping enough
files that `--rw-threads 4` still makes completion order diverge and the
multiset assertion remains meaningful. Removes ~1.8 s of storage delta on this
test and reduces parallel contention.

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/sync/tests/sync_plan/as_done.rs:145–179` — fixture (`:153` cited in §8);
  multiset assertion at `:166–178`.
* The suite sets `GIRSYNC_TEST_CACHE=memory` (around `:111`); keep it.

Possible commits:

1. Replace the large file sizes with small ones (report suggests 24 or 8 files
   of 1–10 KB). Keep distinct names/content so the event multiset is still
   well-defined.
2. Keep `--rw-threads 4` (or equivalent) so completion order still diverges;
   confirm the assertion checks the multiset of copy events, not sizes.

## Acceptance criteria

* The test still passes and still verifies the *set/multiset* of copy events;
  it does not depend on file sizes or content.
* Fixture size drops from ~11 MB to a few tens of KB; copy volume drops
  correspondingly.
* Memory-cache env remains in place.

## Guardrails

* Validate against current code first; if the fixture/assertion structure has
  changed, stop and mark `skipped` with a note.
* Keep enough files to preserve the out-of-order completion that the test is
  designed to observe.
* Test-only change; do not touch product copy logic. Run the girsync `sync_plan`
  suite to verify; report follow-up ideas without implementing them.
