# T05 — Plan / purge / scope decision crate

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F16 (lines 247–251) —
  `repair/phases/plan.rs` (1032), `repair/purge.rs` (1141),
  `scope/{limits,report,ledger,filter}.rs` (491+1039+920+196) ≈ 4,819 lines;
  isolates `RepairPlan`, `PurgePlan`, `Limits`, `PathScope`, `Meter`.
- Constrained by the `Meter`/`Summary` cycle (§F13, lines 228–232; §A10, 86–94).
- Assumptions to validate: §A2 (27–33), §A4 (40–46), §A6 (55–61).

## Scope
- `src/repair/phases/plan.rs`, `src/repair/purge.rs`,
  `src/repair/scope/{limits,report,ledger,filter}.rs`.
- Cycle partners: `src/report/summary.rs:4` ↔ `src/repair/scope/ledger.rs:17`,
  `src/repair/scope/report.rs:29-31` ↔ `src/repair/phases/plan.rs:25` and
  `src/repair/scope/limits.rs:26-27`.
- Test-only seams in `src/repair/purge.rs:450-489`.
- Locate all edges in the current tree before editing.

## Implementation plan
1. Validate the cycle claims against the current code. A boundary must sit at an
   acyclic leaf or around the whole strongly-connected group (§A10); move the
   shared types (`Meter`, `Summary`) down first so the decision logic can become
   a crate without dragging the report plane with it.
2. Create the crate holding `RepairPlan`, `PurgePlan`, `Limits`, `PathScope`,
   `Meter`, keeping policy divergence intact (§A6, §A13).
3. Make the high-level repair phases depend on the crate directly (§A2) so a
   change in writer/HTTP/report code leaves decision logic cached.
4. Preserve test-only seams via the §A7 pattern; update pinned nextest filters
   for `repair::scope::report::tests::…` and
   `repair::phases::plan::…` (§F31).
5. If gircap gains a leaf dependency, update the §A5/§F39 doc tables.

## Constraints / validation
- The `Meter`/`Summary` and `scope`↔`phases` cycles (§F13) are not free cuts —
  if type-moving cannot break them, stop and raise a follow-up rather than
  creating a half-split.
- No byte/behaviour change in plans, reports or purge output (§A6).
- Keep limits/counters/gates and the `rw` gate local (§A13).
- Do not couple the product to gircap or vice versa (§A5).

## Done when
Decision logic is a crate behind the cycle, high-level phases depend on it
directly, and all plan/purge/scope/ledger tests plus pinned nextest filters pass
with unchanged output.
