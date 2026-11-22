# T05 — `girpr-manifest-stats` leaf

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F9 (lines 240–247) — manifest-stats
  reconciliation exists only as capture assertions; candidate boundary is a leaf with
  sum/compare primitives parameterized on fatal-versus-recorded and preserving the string
  wire form. Confidence low-to-medium.
- Assumptions to validate: §A5 (57–67), §A13 (122–128), §A2 (29–36), §A14 (129–135).

## Scope
- New leaf (proposed `girpr-manifest-stats`). Sources: gircap
  `tools/capture/src/hyp/types.rs` (getBuild stats reconciliation), `verify/checks.rs`
  (structural properties), and the product's fatal plan-time range validation in
  `src/repair/phases/plan.rs`. Locate before editing.

## Implementation plan
1. Validate §F9: confirm capture records reconciliation/structural properties as
   assertions (never fatal) while the product's validation is fatal; confirm the string
   wire form is load-bearing.
2. If a safe boundary exists, define sum/compare primitives parameterized on
   fatal-versus-recorded; leave each caller's severity policy local.
3. Migrate callers directly (§A2); move through §A14 seams if gircap calls it.
4. Low-confidence: if the only "shared" part is a trivial sum over different severities,
   prefer a validated skip over a thin leaf.

## Constraints / validation
- High-risk (§F9): the artifact records a violation as evidence; normalizing a bad value
  destroys the finding (§A13). Preserve string wire form and recorded-vs-fatal distinction.
- Do not change published wording or key sets (§A5).
- Do not reopen closed cache/codec decisions (§N15).

## Done when
Reconciliation primitives are shared without changing severity or wire form and all
assertion tests pass — or a validated skip is recorded with the one-line reason (expected
default given low confidence).
