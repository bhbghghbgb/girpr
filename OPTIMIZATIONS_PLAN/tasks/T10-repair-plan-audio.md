# T10 — `repair_plan.rs` audio-plan partial downshift

Priority P1 · root product integration tests · checkpoint CP3 · report finding A3

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L120–L129` (§A3).
* Report synthesis: `docs/09-test-scale-reduction-research.md:L790–L795`
  (pattern 1), `L815–L816` (Wave 2).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T10-repair-plan-audio.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Cut two mock servers and process spawns from the audio-plan test by running the
library-observable cases in-process.

`tests/repair_plan.rs:117–219` has one test looping three cases, each spawning a
fresh `MockServer` and binary. Only `no_scope` asserts a binary-level fact
(`audio_langs=en-us` in stdout, :189–195). `scan_out`/`scan_in` assert only exit
0, a PLAN line and filesystem negatives — all library-observable.

## Implementation plan

Code anchors (validate before editing): `tests/repair_plan.rs:117–219`,
`tests/common/mod.rs` (`ctx_for`, `run`, `field`), plus the library harness pattern
used by `repair_writer.rs`/`repair_limits.rs` and introduced by T09's
`Rig::run_lib` (if T09 is already merged, reuse it). `RunCtx` has the audio/scope
fields and `print_plan`.

Steps (one commit):

1. Keep the binary spawn for `no_scope` (it asserts stdout `audio_langs=en-us`).
2. Run `scan_out` and `scan_in` through `ctx` with `print_plan = true` and the
   audio/scope fields set; assert exit 0, the PLAN line and the filesystem
   negatives in-process.

## Acceptance criteria

* All three cases' assertions preserved; only the two library-observable cases
  are downshifted.
* −2 spawns and mock servers; test count unchanged (the three cases stay in the
  loop/test structure).
* Affected suite passes. No production behavior change.

## Guardrails

* Keep the `no_scope` stdout assertion exactly as-is — do not try to downshift a
  process-level fact.
* Validate anchors (line numbers from the report snapshot). Report follow-up
  ideas without implementing them.
