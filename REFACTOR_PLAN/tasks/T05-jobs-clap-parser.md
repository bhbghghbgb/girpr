# T05 — `jobs` range validation in clap

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §B3 (lines 415–417).

## Scope
- Package: root `girpr`, `src/cli.rs` (lines 294–308), `src/main.rs` (lines
  94–96).

## Implementation plan
1. Move the manual `jobs` range checks into a clap value parser using
   `.range(1..=1024)` (or an equivalent typed parser).
2. Preserve the `Option<usize>` meaning ("flag absent") at the call site.

## Constraints / validation
- Low payoff (§B3). The project remaps clap's default exit code 2 to 1 for usage
  errors (implementation plan `docs/03-repair-rust-implementation-plan.md`, CLI
  section) — confirm argument-error behavior is unchanged.
- If the parser cannot preserve the semantics cleanly, stop and record as
  `skipped`.

## Done when
Range validation lives in the clap layer and existing CLI behavior/tests are
preserved.
