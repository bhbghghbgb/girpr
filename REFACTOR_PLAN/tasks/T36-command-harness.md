# T36 — Generic parse→plan→dispatch command harness (A4-conflicting, likely skip)

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F41 (lines 567–576) — three independent
  in-process command harnesses are test-only inversions; candidate boundary is a generic
  parse→plan→dispatch→typed-outcome harness. **High-risk**: each must name program-private
  construction and the outcome shapes differ; forcing a common surface directly contradicts
  §A4. Confidence high.
- Assumptions to validate: §A4 (48–56), §A2 (29–36), §A14 (129–135), §F2 (156–163), §F32 (479–487).

## Scope
- `crates/test-support/src/lib.rs` (`RunCtx` driving `girpr::run`),
  `tools/capture/test-support/src/lib.rs` (argv reparse + four `run::*` APIs + injectable
  transport), `crates/sync-test-support/src/lib.rs` (`run_cli*`/`run_args` over
  `girsync::run_captured_with_hooks`). Locate before editing.

## Implementation plan
1. Validate §F41: confirm the three harnesses and their incompatible typed outcomes
   (counts vs records vs typed `Record`+`Failure`).
2. Assess only: a common surface contradicts §A4 (program-private construction must be
   named, gircap independence). If so, record a validated skip with the one-line reason.
3. Implement only on hard evidence of a generic shape that keeps each program's construction
   and outcome local. Do not absorb T27's split or T30's fault seams.

## Constraints / validation
- gircap independence (§A4) and program-private construction are hard blockers. Do not
   merge distinct outcome shapes.
- Do not break env pinning / nextest isolation (§A10, §F2, §F32).

## Done when
A validated decision is recorded — expected `skipped` with the §A4 reason, or a safe generic
shape preserving all three outcome types.
