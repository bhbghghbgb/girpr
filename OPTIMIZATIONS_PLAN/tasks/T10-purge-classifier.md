# T10 — Consolidate `purge.rs` classifier early tests

Priority P1 · root product unit tests · checkpoint CP4

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-suite-reduction-audit.md:L138–L142` (§C.5).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T10-purge-classifier.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

Four early tests in `src/repair/purge.rs` (667, 732, 751, 1204) re-assert cases
already in the parametrized `cached_keys_classify_every_keep_rule` (930). Verify
parity before deleting, then remove the redundant early tests. **~3–4 saved,
medium risk.**

Important nuance from the report: the early ones use `classify_purge_path`,
while 930 uses `classify_purge_path_visible` — parity is not automatic.

**Keep** `the_keep_set_is_config_and_the_lock` (1191) and
`a_hidden_path_is_kept...` (782).

## Implementation plan

Code anchors from the report (validate before editing):

* `src/repair/purge.rs:667/732/751/1204` (early tests).
* `src/repair/purge.rs:930` (`cached_keys_classify_every_keep_rule`).
* Kept: `src/repair/purge.rs:1191`, `:782`.

Possible commits:

1. Establish parity table: for each early case, show an equivalent row in 930
   (accounting for `classify_purge_path` vs `classify_purge_path_visible`).
2. Remove only the cases proven subsumed; keep any distinct case.

## Acceptance criteria

* Every removed case is demonstrably covered by 930 (or moved into it).
* The two explicitly kept tests remain unchanged.
* `purge` unit tests pass; no product change.

## Guardrails

* If parity cannot be established, keep the tests and mark the task `skipped` or
  partial with a note — do not delete on assumption (medium risk).
* Do not touch the classifier implementation. Report follow-up ideas without
  implementing them.
