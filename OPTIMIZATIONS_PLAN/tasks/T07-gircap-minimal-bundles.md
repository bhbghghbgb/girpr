# T07 — Hand-build minimal bundles for gircap "delete a doc" tests

Priority P0 · crate gircap · checkpoint CP2

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L332–L338` (§6.2 2, first
  bullet).
* Report priority row: `docs/08-test-runtime-optimization.md:L473` (§8, P0).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T07-gircap-minimal-bundles.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

Six `capture_layout.rs` tests capture an entire bundle just to delete one JSON
document and verify the failure; `schema.rs` goes further and `copy_tree()`s the
whole bundle before deleting a key. The bundle bytes are irrelevant to the
assertion — only the missing-document error matters. Build a minimal valid
bundle directly with the crate's writer (as `offline_bundle.rs::Bundle::build`
does) and assert the error from that. Removes a full capture + tree write/delete
per test.

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/capture/tests/capture_layout.rs:387–517` — six "delete one required
  document → verify" tests.
* `tools/capture/tests/schema.rs:439–488` — `copy_tree()` duplicate before
  deleting a key; `copy_tree` at `:490`.
* `tools/capture/tests/offline_bundle.rs` — `Bundle::build` is the example of a
  hand-built minimal bundle.

Possible commits:

1. Add a small helper (in `tests/common/mod.rs` or local to the suite) that
   writes the smallest bundle satisfying the schema writer, parameterized by
   which required document to omit/delete.
2. Convert the six `capture_layout.rs` cases to use it and assert the resulting
   error/verdict.
3. Convert the `schema.rs` `copy_tree` cases to use it instead of duplicating a
   captured tree.

## Acceptance criteria

* Those tests no longer spawn a capture or copy an entire captured tree.
* The same missing-document errors are asserted; the error contract is not
  weakened.
* The minimal bundles are genuinely valid inputs except for the one deleted
  document (so the failure is for the intended reason).

## Guardrails

* Validate against current code first; if these tests no longer capture a full
  bundle, stop and mark `skipped` with a note.
* Do not change product validation/error behavior; these are test fixtures.
* Run the gircap `capture_layout` and `schema` suites to verify; report
  follow-up ideas without implementing them.
