# T11 — `girpr-byte-accounting` leaf

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F14 (lines 295–306) — byte-ceiling
  accounting names three roles without a shared type; candidate boundary is a leaf naming
  and relating `Projected`/`Live`/`Settled`/`FirstStop` and the reservation arithmetic —
  explicitly **not** the `>` predicate or counters that prior report 08's N2 excluded.
- Assumptions to validate: §A5 (57–67), §A2 (29–36), §A9 (88–97), §A14 (129–135).

## Scope
- New leaf (proposed `girpr-byte-accounting`). Sources: product `src/repair/mod.rs`
  (projected estimate, `note_stopped`), `src/repair/file/temp.rs` and `file/mod.rs`
  (reserve/settle/adjust, `note_write`); gircap `tools/capture/src/blobs/plan.rs` and
  `run/mod.rs` (projected/live split, deliberately no mid-flight ledger). Locate before editing.

## Implementation plan
1. Validate §F14: confirm the three roles and that gircap deliberately has no mid-flight
   ledger; confirm `describe()` was copied for habit, not shared code.
2. Define the leaf naming/relating `Projected`/`Live`/`Settled`/`FirstStop` and the
   reservation arithmetic only — not the `>` predicate or counters.
3. Migrate callers directly (§A2); gircap keeps its projected/live split without being
   forced into a ledger. Move through §A14 seams if gircap calls it.
4. Commit leaf, migrations, seams.

## Constraints / validation
- High-risk (§F14): a wrong settle changes published `SUMMARY` bytes and `stopped=` (§A5);
  first-stop-wins must stay.
- Do not absorb the `>` predicate or counters (prior N2). Do not force gircap to adopt a
  mid-flight ledger it deliberately lacks (§A9).
- Divergent policies are keepers — if a split turns out intentional, stop and raise a
  follow-up.

## Done when
Byte roles and reservation arithmetic are shared, all `SUMMARY`/`stopped=` bytes stay
identical, callers consume directly, and doc seams are updated.
