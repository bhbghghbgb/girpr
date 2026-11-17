# T17 — Merge redundant girsync dry-run/hard-link test groups

Priority P1 · crate girsync · checkpoint CP4

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L299–L306` (§6.1 D8 and D9).
* Report priority row: `docs/08-test-runtime-optimization.md:L477` (§8, P1).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T17-girsync-redundant-tests.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

Two redundant groups in `tests/sync.rs`: (D8) the dry-run trio where
`run_sync_dry_run_writes_nothing` and `…creates_no_cache` overlap almost
entirely — merge into one run with all checks, keep `…with_ignore_cache_leaves_caches_intact`
(distinct). (D9) the hard-link parity trio that each spawn the binary 2–4× for
the same "hard links refused at plan time" invariant in three dimensions — batch
into a shared fixture with fewer spawns.

## Implementation plan

Code anchors from the report (validate before editing):

* Dry-run trio: `tools/sync/tests/sync.rs:124–150`, `:155–179`, `:184–206`.
* Hard-link trio: `tools/sync/tests/sync.rs:477–517`, `:527–574`, `:583–623`.

Possible commits:

1. Merge the two overlapping dry-run tests into one test that performs the run
   once and asserts all the original checks; leave the ignore-cache test alone.
2. Refactor the three hard-link cases to share a fixture so the binary is
   spawned fewer times, while still asserting the invariant in all three
   dimensions (the report notes they are not strictly subsumed).

## Acceptance criteria

* Fewer binary spawns in `sync.rs`; the overlapping dry-run checks all still run.
* All three hard-link dimensions remain asserted.
* No assertion is silently dropped when merging.

## Guardrails

* Validate against current code first; if the groups differ, stop and mark
  `skipped` with a note.
* Be conservative with the hard-link trio — the report says they are not strictly
  subsumed; reduce spawns via a shared fixture rather than deleting a dimension.
* Test-only change; do not touch product hard-link/plan behavior. Run the girsync
  `sync` suite; report follow-up ideas without implementing them.
