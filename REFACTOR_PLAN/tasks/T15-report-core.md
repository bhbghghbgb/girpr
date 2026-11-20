# T15 — Ordered-JSON / `write_line` report core

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F21 (lines 305–314) — three large
  report data planes implementing the same idea.
- Assumptions to validate: §A3 (29–35), §A8 (69–76), §A2 (21–27).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (§A8, §N4).

## Scope
- Candidate shared crate `crates/report-core` (`girpr-report-core`) — a narrow
  core only.
- Report names: `src/report/{record.rs,emit.rs,summary.rs}`,
  `tools/sync/src/report/{mod.rs,record.rs}`,
  `tools/capture/src/report/mod.rs`, `tools/sled2redb/src/lib.rs` (reuses
  sync's writer). Locate before editing.

## Implementation plan
1. Validate §F21 and §A8: the three planes differ in key order (girpr event-first
   declaration order, girsync alphabetical, gircap event-first with a real
   `event` field), in bool-vs-1/0, and in percent-encoding. A monolithic shared
   `Record` would break §A3/§A8 and is forbidden.
2. Extract only a narrow ordered-JSON / `write_line` core parameterised by:
   emission order, event placement, escaping, and boolean rendering. Each tool
   supplies its own value vocabulary and keeps its own `Val`/`Record` types.
3. Migrate the emit/summary mechanics to the core while leaving each tool's
   field order and value types untouched. `sled2redb` may keep reusing sync's
   writer if its contracts stay identical (§F21).
4. Validate the named byte-level report oracles after migration.

## Constraints / validation
- No `preserve_order` feature unification; emission order is preserved by the
  core without it (§A8).
- Bool-vs-1/0, casing, spacing, flag conditions and percent-encoding stay
  per-tool (§A3, §A8, §N9).
- If the only viable shared core would require normalizing the three value
  vocabularies, stop and mark `skipped` with the §A8 reason.
- A new gircap leaf dep requires §A2 doc tables to be updated.

## Done when
A narrow ordered-JSON core is shared, all three tools keep their distinct
orders/value vocabularies, and every report byte oracle passes unchanged.
