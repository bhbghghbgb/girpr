# T24 — Generic engine `Plan`/`Execute`/`Verify` trait shape (low confidence)

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F29 (lines 448–456) — plan/execute
  engines have converged in shape, but the order is not expressible as a crate; candidate
  boundary is a `girpr-engine` leaf with `Plan`/`Execute` and a `Verify` trait requiring two
  independent observations. **Confidence low; most plausible outcome is a shared trait
  shape only.**
- Assumptions to validate: §A6 (68–75), §A7 (76–80), §A8 (81–87), §A12 (114–121), §A13 (122–128), §N5 (713–716).

## Scope
- girsync `tools/sync/src/commands/sync/mod.rs` (eight named phases, order is invariant),
  product `src/repair/mod.rs` ("plan, then assert, then execute"), gircap verify/materialize
  half. Locate before editing.

## Implementation plan
1. Validate §F29: confirm the three shapes and that phases are deliberately *named, not
   extracted*.
2. Assess only: a general engine crosses three exit contracts (§A7), three report planes
   (§A6), the open digest vocabulary (§A8), the tool SCCs (§A12) and the `Meter`/`Summary`
   constraint. If no safe boundary exists, record a validated skip. If anything, restrict
   to a transport-free trait shape with no shared order/executor.
3. Do not absorb the rw gate, lock ordering, bounded runner, symlink classification or
   budget (N5). Do not reuse an observation buffer (§A13).

## Constraints / validation
- Keep verification requiring two independent observations (§A13); no shared buffer.
- Do not move named phase order into a crate; it is the girsync invariant.
- No byte/exit/report drift (§A5, §A7, §A6).

## Done when
A validated decision is recorded — expected `skipped` or a minimal trait shape only, with
the one-line reason.
