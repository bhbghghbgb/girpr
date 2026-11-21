# T13 — girsync: planner + effective cycle crate

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F24 (lines 300–308) —
  `planner` (~2,161) and `effective` (~2,593) are a cycle and would move
  together.
- §A10 (lines 86–94) — a boundary must sit around a whole strongly-connected
  group.
- Assumptions to validate: §A2 (27–33), §A4 (40–46), §A6 (55–61).

## Scope
- `tools/sync/src/{planner,effective}/**` and their cycle edge (e.g.
  `effective/phase_c.rs:13-21`).
- Dependents in `tools/sync/src/commands/sync/**` (~7,625 lines, including
  `plan/tests.rs` 1212).
- Locate the full SCC in the current tree before editing.

## Implementation plan
1. Validate the cycle against current code and confirm no larger SCC drags in
   cache or commands (§A10). If the group is larger than expected, stop and
   raise a follow-up rather than making a partial split.
2. Move the whole planner+effective group into one crate with a narrow interface
   to commands.
3. Make girsync's command layer depend on the crate directly (§A2) so a change
   in cache or other subsystems leaves planning cached.
4. Preserve plan byte contracts (§A6) and the test-only seams via §A7; update
   nextest filters (§F31) and §A5/§F39 doc tables if needed.

## Constraints / validation
- Do not cut the cycle — move the group whole (§A10).
- No byte/behaviour change in sync plans (§A6).
- §A2 payoff must be real: commands must depend on the crate directly.

## Done when
Planner+effective are one crate behind a narrow interface, commands depend on it
directly, plan bytes are unchanged, and all planner/effective/plan tests pass.
