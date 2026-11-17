# T12 — Capture: remove redundant process test; fold assertion-free test

Priority P1 · crate gircap · checkpoint CP5

## Pointers (give these to the implementing session)

* Report findings: `docs/08-test-suite-reduction-audit.md:L181–L184` (§E.2),
  `L185–L187` (§E.3).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T12-capture-medium-removes.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

Two medium-risk reductions in gircap:

1. **REMOVE redundant process test:** `tests/cli_contract.rs:214`
   `a_bad_log_level_is_refused_by_the_parser` — its three assertions are covered
   by `every_argument_error_is_one` (142), `src/cli/mod.rs:1002`, and the stdout
   contract (197/229). **1 saved.**
2. **FOLD assertion-free test:** `src/emit/closeout.rs:1000`
   `a_clean_close_out_returns_ok` calls `seed()/run()/remove_dir_all()` with no
   assertion; neighbouring tests (931/956/1008) run the same happy path and
   assert more. Fold it into a neighbour (or delete it if fully redundant).
   **1 saved.**

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/capture/tests/cli_contract.rs:214`, `:142`, `src/cli/mod.rs:1002`,
  `:197/229`.
* `tools/capture/src/emit/closeout.rs:1000/931/956/1008`.

Possible commits: one per bullet.

## Acceptance criteria

* Bad-log-level rejection still covered by the named tests (exit code/stdout
  contracts retained).
* The happy-path behavior formerly in the assertion-free test is still exercised
  by a neighbour that asserts more.
* Affected suites pass; no product code touched.

## Guardrails

* Confirm the covering tests exist and assert what the report claims before
  deleting; otherwise keep and mark `skipped`/partial.
* Do not touch the capture log-file process tests (529, 565, 586, 610, 698, 635,
  665) — those belong to T13.
* Report follow-up ideas without implementing them.
