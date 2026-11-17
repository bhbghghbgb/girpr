# T08 — `repair_plan` scope table + optional root-integration merges

Priority P0 · root product integration tests · checkpoint CP3

## Pointers (give these to the implementing session)

* Report findings: `docs/08-test-suite-reduction-audit.md:L107–L109` (§B.3),
  `L110–L115` (§B.4 including the do-not-merge list).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T08-root-integration-optional-merges.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

Low / low–medium risk merges in root integration tests:

* `tests/repair_plan.rs` — three audio `--print-plan` process tests (117, 170,
  225) differ only by `--include`; add a scope column to a case table.
  **2 saved.**
* Smaller optional pairs: `tests/repair_offline.rs` branch-failure pairs
  (514/560, 662/699), `tests/repair_post.rs` non-UTF-8 switch pair (485/497),
  `tests/repair_scope.rs` include-nothing pair (184/414). **~5 saved.**

Expected ~7 tests removed.

## Implementation plan

Code anchors from the report (validate before editing):

* `tests/repair_plan.rs:117/170/225`.
* `tests/repair_offline.rs:514/560, 662/699`; `tests/repair_post.rs:485/497`;
  `tests/repair_scope.rs:184/414`.

Possible commits:

1. Add the `--include` scope column and collapse the three `repair_plan` tests.
2. Merge each optional pair, preserving all assertions as table rows / combined
   assertions.

## Acceptance criteria

* Every original assertion preserved; the `--include` dimension still covered.
* Affected integration suites pass; no product code touched.

## Guardrails

* **Do not merge** the complementary pairs listed in §B.4:
  `repair_post.rs:512/596`, `repair_purge.rs:421/454`, `repair_scope.rs:212/342`,
  `repair_plan.rs:477/503` — they pin different layers or complementary cases.
* Validate each pair differs only by the named dimension; skip any that does
  not.
* Do not touch log-file process tests (T13). Report follow-up ideas without
  implementing them.
