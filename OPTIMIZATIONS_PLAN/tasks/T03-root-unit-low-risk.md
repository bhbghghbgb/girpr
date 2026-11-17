# T03 — Consolidate remaining low-risk root-unit cases

Priority P0 · root product unit tests · checkpoint CP2

## Pointers (give these to the implementing session)

* Report findings: `docs/08-test-suite-reduction-audit.md:L129–L132` (§C.2),
  `L133–L135` (§C.3), `L136–L137` (§C.4).
* Report "kept intentionally" note: `docs/08-test-suite-reduction-audit.md:L143–L148` (§C.6).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T03-root-unit-low-risk.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

Three independent, low-risk consolidations in root product unit tests:

* `src/main.rs` worker-thread arithmetic: `a_single_job_keeps_runtime_headroom_on_one_core`
  (166), `host_parallelism_wins_when_it_exceeds_the_file_window` (171),
  `a_huge_file_window_saturates_instead_of_overflowing` (176) → one parametrized
  test over the pure `worker_threads(jobs, window)`.
* `src/repair/reuse.rs` `salvage_order`: seven 2–4-line asserts (224, 232, 240,
  250, 258, 264, 294) → one table-driven test. **Keep** the four distinct cases
  at 272, 283, 306, 324.
* `src/util/ini.rs:1075/1091` NUL game-dir inspection: both call
  `game_dir_refusal` with a NUL path — combine.

Expected ~9 tests removed, low / low–medium risk.

## Implementation plan

Code anchors from the report (validate before editing):

* `src/main.rs:166/171/176` and the `worker_threads` function.
* `src/repair/reuse.rs:224–324` (table rows and the four kept distinct cases).
* `src/util/ini.rs:1075/1091`.

Possible commits: one per bullet (three small commits).

## Acceptance criteria

* All original cases rows are preserved in the tables; the four distinct
  `salvage_order` cases remain separate.
* No product logic changed (pure-function tables and test-local helpers only).
* Affected unit suites pass.

## Guardrails

* Do not touch tests in the "kept intentionally" list (§C.6), especially the
  `md5.rs` throughput benchmarks and the table-driven bad-path tests.
* Validate each anchor; skip a bullet that no longer matches rather than
  forcing it. Test-only change.
* Report follow-up ideas without implementing them.
