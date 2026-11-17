# T04 — Table-drive sync hash dedup/order asserts

Priority P0 · crate girsync · checkpoint CP2

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-suite-reduction-audit.md:L154–L156` (§D.2).
* Report "kept" note: `docs/08-test-suite-reduction-audit.md:L168–L173` (§D.6).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T04-sync-hash-dedup.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

`tools/sync/src/hash/tests.rs:900, 916, 930, 942` are four short dedup/order
asserts. Fold them into one table-driven `dedup_keeps_first_seen_order`, keeping
a short comment per regression row. Expected up to 3 tests removed, low /
low–medium risk.

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/sync/src/hash/tests.rs:900/916/930/942`.

Possible commits:

1. Add the table with one row per original case (with a regression comment).
2. Remove the four single-case tests.

## Acceptance criteria

* Every original case is a row; first-seen order semantics still asserted.
* `hash` suite passes; no product code touched.
* Report follow-up ideas without implementing them.

## Guardrails

* The `hash/tests.rs` suite is in the report's "kept" list (§D.6) as a whole —
  only merge these four named cases, do not prune other tests in that file.
* Validate the anchors; if they no longer match, stop and mark `skipped`.
