# T09 — Lift value types + `girpr-repair-plan` crate

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F12 (lines 270–280) — the plan/purge
  decision graph is concentrated in a cyclic 1,344-line `repair/mod.rs`; candidate boundary
  is a pure crate owning `PlanInputs`/`PlanWork`/`DownloadEstimate`/`PlanSummaryValues`
  and the "scope selected something" rule. `RepairPlan`, `PlannedFile`, `Layout`,
  `PurgePlan`, `Mode`, `PathScope` must become leaf-owned values first — the same move the
  ledger cycle required.
- Bottleneck context: §F16 (317–323). Ledger precedent: §A12 (114–121).
- Assumptions to validate: §A2 (29–36), §A5 (57–67), §A14 (129–135).

## Scope
- New crate (proposed `girpr-repair-plan`). Sources: `src/repair/mod.rs` (plan_phase,
  build_and_assert_plan, plan_work_and_purge, estimate_download, within_limits,
  publish_plan, deprecated normalization, apply_run_flags), `scope/limits.rs`,
  `phases/plan.rs`, `scope/report.rs`. T07's `Mode` leaf is a prerequisite. Locate before editing.

## Implementation plan
1. Validate §F12: confirm the concentration and the back-pointing types; confirm the
   ledger-cycle move pattern applies.
2. First lift the value types (`RepairPlan`, `PlannedFile`, `Layout`, `PurgePlan`, `Mode`,
   `PathScope`) into leaf-owned values, reusing T07's `Mode` leaf.
3. Define the pure crate owning the named inputs/estimates/summary values and the scope
   rule; leave I/O, fetch and publish policies in the facade.
4. Migrate callers directly (§A2); move through §A14 seams. Deliver in multiple commits.

## Constraints / validation
- Byte contracts (§A5): `PLAN`/`SUMMARY` keys, wording and flag conditions pinned; the
  1,344-line file encodes byte-safety proofs — do not weaken them.
- Do not absorb the rw gate, lock ordering or other local items (§N5). Do not create a
  re-exporting facade (§A2).
- Record the lifted value-type locations in the note so later tasks reuse them.

## Done when
The plan decision values live in a pure crate, the cycle is reduced, the facade consumes
it directly, and all plan/report bytes are unchanged.
