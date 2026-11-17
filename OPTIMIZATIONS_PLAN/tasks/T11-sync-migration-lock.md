# T11 — Trim `migration_lock.rs` clear duplicates

Priority P1 · crate girsync · checkpoint CP5

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-suite-reduction-audit.md:L159–L164` (§D.4).
* Report "kept" note: `docs/08-test-suite-reduction-audit.md:L168–L173` (§D.6).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T11-sync-migration-lock.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

`tools/sync/tests/migration_lock.rs` (11 tests) has 4–5 tests that overlap
dedicated suites:

* 443 ≈ `sync.rs:140`
* 376 ≈ `sync.rs:1986`
* 241 ≈ `helpers.rs:459` / `lazy/compare.rs:182`
* 480/279 ≈ generic convergence

Trim only the clear duplicates. Its **stated purpose is a backend-agnostic
behavior lock** (asserts cache-row semantics without pinning format), which is
real — keep the row/content assertions. **~5 saved, medium risk**; deleting the
whole target is HIGH risk.

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/sync/tests/migration_lock.rs:443/376/241/480/279` and the counterparts
  in `tests/sync.rs:140`, `tests/sync.rs:1986`, `tests/helpers.rs:459`,
  `tests/lazy/compare.rs:182`.

Possible commits:

1. For each candidate, confirm the counterpart asserts the same property; remove
   only proven duplicates.
2. Keep the backend-agnostic row/content assertions intact.

## Acceptance criteria

* The behavior-lock purpose still covered; no row/content assertion lost.
* `migration_lock` suite passes; no product code touched.

## Guardrails

* **Do not delete the whole target** — the report rates that HIGH risk.
* Verify each removal is a true duplicate, not a different backend/format angle.
* Do not touch the `lazy`/`stat_trust`/`hash_mode`/`rw_threads`/`sync_plan`/`why`
  suites (§D.6). Report follow-up ideas without implementing them.
