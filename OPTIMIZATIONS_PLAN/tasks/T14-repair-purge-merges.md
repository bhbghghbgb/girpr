# T14 — `repair_purge.rs` plan-out + purge-only merges

Priority P1 · root product integration tests · checkpoint CP5 · report finding A7

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L185–L191` (§A7).
* Report synthesis: `docs/09-test-scale-reduction-research.md:L804–L805`
  (pattern 4), `L815–L816` (Wave 2).
* Report "do-not-merge" list: `docs/09-test-scale-reduction-research.md:L826–L841`
  (constraints; note the `repair_purge.rs:421/454` pair is called complementary —
  do not conflate it with these clusters).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T14-repair-purge-merges.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Merge two clusters of same-fixture-shape tests in `tests/repair_purge.rs`:

* **plan-out before/after** (:422, :455).
* **purge-only trio** (:310, :359, :492).

Two tables over `{purge_before, purge_after}` and
`{purge_only, purge_only+dry-run, purge_only+check_only}`. ~2 tests merged,
~2 mock servers and temp dirs saved.

## Implementation plan

Code anchors (validate before editing): `tests/repair_purge.rs:310`, `:359`,
`:422`, `:455`, `:492`, and their fixtures/assertions.

Steps (one or two commits, one per table):

1. Table-drive plan-out before/after.
2. Table-drive the purge-only trio.

Preserve each case's distinct assertions as table rows (exit code, plan content,
disk state, chunk hits, `config_bumped`/`scoped` counters as applicable).

## Acceptance criteria

* Every case and assertion preserved as table rows; ~2 tests merged, ~2 mock
  servers/temp dirs saved.
* The complementary pair at `repair_purge.rs:421/454` (from the report's
  do-not-merge list) is **not** folded into these tables.
* The process-level "removed capture" case situation (A6) is out of scope here —
  see guardrails. Affected suite passes.

## Guardrails

* A6 (`tests/process.rs:282–330` vs `repair_purge.rs:257–307`, report L172–L183)
  is a separate finding, not part of this task. If the process case is still
  present, do not remove it here; note it as a follow-up suggestion instead.
* Validate anchors; skip a cluster that no longer matches. Test-only change.
  Report follow-up ideas without implementing them.
