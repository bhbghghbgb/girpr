# T15 — Remove cross-file duplicate process-level assertions

## Goal / pointers
- `docs/10-test-optimization-research.md` §F22 (lines 327–332) — `process.rs:229-279`
  asserts REPORT/SUMMARY, repair, scope and `getBuild` hits already asserted in-process
  by `repair_offline.rs` and `cli_contract.rs`; `repair_post.rs`, `repair_scan.rs` and
  `repair_plan.rs` duplicate in-process rigs. Fix: keep one binary smoke per flag path
  and assert details in-process.
- Assumptions to validate: §A1 (46–52), §A12 (118–121), §A10 (108–110), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- `tests/process.rs`, `tests/repair_offline.rs`, `tests/cli_contract.rs`,
  `tests/repair_post.rs`, `tests/repair_scan.rs`, `tests/repair_plan.rs`. Locate the
  exact duplicate assertions in the current tree before editing.

## Implementation plan
1. Validate §F22: for each duplicated assertion, confirm an in-process test already
   pins the same semantic fact; list assertions that are process-only and must stay.
2. Reduce the duplicate process-level cases to one binary smoke per flag path; keep the
   detailed assertions in the in-process rigs.
3. Remove redundant in-process rigs that duplicate an existing rig, keeping the stronger
   assertion.
4. Update nextest filters if a named case is removed or moved (§A10). Deliver in a
   couple of commits if removal and filter updates are easier apart.

## Constraints / validation
- Preserve process-only coverage (§A12): only remove a spawn when its assertion is
  genuinely covered in-process; never remove the last binary smoke for a flag path.
- No byte or log-contract change (§6); do not pull in gircap or another program.
- Do not silently drop a test named in a nextest filter (§A10). Preserve messages.

## Done when
Each flag path retains one binary smoke, duplicated detailed assertions are removed,
no semantic fact loses its assertion, filters match, and both process and in-process
suites pass with fewer spawns.
