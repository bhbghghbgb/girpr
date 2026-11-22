# T26 — Generic `EvidencePolicy` with `CannotTell`

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F31 (lines 469–477) — fail-open/fail-closed
  ("gray-box") policy is repeated as ad-hoc branching; candidate boundary is a generic
  `EvidencePolicy` with an explicit `CannotTell` arm and a "fail closed unless the missing
  input contributed nothing" rule. Confidence high.
- Assumptions to validate: §A13 (122–128), §A5 (57–67), §A9 (88–97), §A2 (29–36), §A14 (129–135).

## Scope
- girsync `tools/sync/src/effective/side.rs` (fail open on missing/ignored cache, closed on
  corrupt), `planner/coverage.rs`, `planner/pairs.rs`, `diff/verdict.rs`
  (`Unverifiable`/`CHANGED`); product `src/repair/scope/mod.rs` fail-closed ceiling policy.
  Locate before editing.

## Implementation plan
1. Validate §F31: confirm the branches and that "cannot tell" is marked distinct from
   equality.
2. Define a generic `EvidencePolicy` with an explicit `CannotTell` arm; leave each
   program's specific fail-open/closed choices as caller-supplied parameters (§A9 keepers).
3. Migrate callers directly (§A2); move through §A14 seams where relevant.
4. Commit policy type, migrations, seams.

## Constraints / validation
- High-risk (§F31): collapsing `CannotTell` into equality produces the false-clean outcome
  both tools guard against. Keep it distinct.
- Cache open fail-open-vs-closed is a keeper (§A9); do not unify.
- Preserve verdict wording/`Unverifiable`/`CHANGED` bytes (§A5). Do not reuse an observation
  buffer (§A13).

## Done when
`EvidencePolicy` is shared with `CannotTell` intact, each caller keeps its fail-open/closed
policy, and all verdict bytes are unchanged.
