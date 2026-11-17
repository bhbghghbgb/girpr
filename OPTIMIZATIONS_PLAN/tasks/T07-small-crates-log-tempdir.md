# T07 — `crates/log` + `tempdir` low-risk merges

Priority P0 · crates log, tempdir · checkpoint CP3

## Pointers (give these to the implementing session)

* Report findings: `docs/08-test-suite-reduction-audit.md:L218–L220` (§F.4),
  `L221–L222` (§F.5).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T07-small-crates-log-tempdir.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

Two low-risk reductions:

* `crates/log/src/plan.rs:428`
  `a_debug_build_and_a_release_build_name_the_default_the_same_way` restates
  `default_file_is_written() == !cfg!(debug_assertions)`; the next test (446)
  covers the policy through `LogPlan::resolve`. **1 saved.**
* `crates/tempdir/src/lib.rs` — two overlapping pairs: name-shape tests 266/334
  and root-placement tests 291/320. Fold the weaker into the stronger.
  **2 saved.**

Expected 3 tests removed.

## Implementation plan

Code anchors from the report (validate before editing):

* `crates/log/src/plan.rs:428`, `:446`.
* `crates/tempdir/src/lib.rs:266/334`, `:291/320`.

Possible commits: one per crate.

## Acceptance criteria

* The policy remains covered via `LogPlan::resolve` (446).
* Name-shape and root-placement properties still asserted after folding.
* Affected crate suites pass; no product code touched.

## Guardrails

* Keep the stronger test of each pair and all its assertions; do not drop a
  property for the count.
* Validate anchors; skip a pair that is not actually overlapping.
* Report follow-up ideas without implementing them.
