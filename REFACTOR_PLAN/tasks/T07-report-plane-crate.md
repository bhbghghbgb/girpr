# T07 — Report plane crate

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F18 (lines 257–259) —
  `src/report/{mod,record,summary,emit}.rs` (820) + `src/phase.rs` (143) ≈ 963
  lines, widely imported but small; keeps `Record` / `emit` / `Summary` /
  `ProgressGuard` cached when repair internals change. Constrained by §F13.
- Assumptions to validate: §A2 (27–33), §A6 (55–61), §A10 (86–94).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/report/{mod,record,summary,emit,tests}.rs`, `src/phase.rs`.
- Cycle partners: `src/repair/scope/ledger.rs:17`, `src/repair/scope/report.rs`,
  `src/report/emit.rs:25` (provenance).
- `girpr-report-core` already exists (§A3); extend/reuse it rather than creating
  a duplicate ordered-JSON core.

## Implementation plan
1. Validate the cycle claims (§F13) and the three report value vocabularies.
   A single shared `Record` that forces one key order or bool rendering is
   forbidden (§A6) — keep the vocabularies distinct.
2. Create/extend the report crate holding the shared plane mechanics and make
   its high-level consumers depend on it directly (§A2), so repair-internal
   edits leave reporting cached.
3. Preserve key order, casing, spacing, `preserve_order`-free JSON, and the
   pinned `format_timestamp` (§A6). Do not "fix" divergent report formats.
4. Update nextest filters for moved report tests (§F31). If gircap gains a leaf
   dependency, update the §A5/§F39 doc tables.

## Constraints / validation
- Byte contracts are load-bearing (§A6): no change to report/dump bytes,
   key order or flag conditions.
- Do not cut the `report::summary ↔ repair::scope::ledger` cycle illegally
  (§A10). Move shared types down first or keep the boundary around the group.
- Do not merge the three report planes or enable `preserve_order`.

## Done when
Report plane mechanics live in a crate consumed directly by high-level code,
all report bytes are byte-identical, and all report/phase tests pass.
