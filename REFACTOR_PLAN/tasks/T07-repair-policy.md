# T07 — Lift `Mode` / `resolve_mode` / `RepairMode` into a policy leaf

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F15 (lines 308–315) — the write-mode
  decision is triplicated inside the facade; candidate boundary is a leaf owning
  `RepairMode`, `Mode` and the single resolve rule.
- Dependency note: §F11 (262–268) says `Mode`/`resolve_mode` must move to a neutral leaf
  **before** chunk verification can be extracted (T08). §A12 (114–121) cycles.
- Assumptions to validate: §A5 (57–67), §A2 (29–36), §A14 (129–135).

## Scope
- New leaf (proposed `girpr-repair-policy`). Sources: `src/repair/file/mod.rs`
  (`resolve_mode`, `Mode`), `src/config.rs` (`RepairMode`), `src/repair/scope/report.rs`
  (`DiskPlan::from_figures` test-injection seam), `src/repair/scan.rs`. Locate before editing.

## Implementation plan
1. Validate §F15: confirm the three derivations and that the disk-plan duplication exists
   to keep a test-injection seam.
2. Define the leaf owning `RepairMode`, `Mode` and the single resolve rule; keep the
   injection seam working (e.g. a builder/factory in the leaf or a preserved local seam).
3. Migrate all three call sites directly (§A2); do not keep re-exports in the facade.
4. Deliver in commits: leaf + migrations, doc seams.

## Constraints / validation
- High-risk (§F15): the estimate's temp/discard branches depend on the exact resolve
  outcome; `PLAN`/`SUMMARY` bytes must stay identical (§A5).
- Do not remove the injection seam that `DiskPlan::from_figures` provides; if consolidating
  it changes test behavior, stop and raise a follow-up.
- This task is a prerequisite for T08; record the chosen location of `Mode` in the note so
  T08 can build on it (§A12).

## Done when
The mode decision lives in one leaf, all three callers use it, the injection seam still
works, and no published figure changes.
