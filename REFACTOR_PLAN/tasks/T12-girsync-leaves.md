# T12 — girsync: extract the acyclic leaves

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F24 (lines 300–308) — true
  acyclic production leaves in girsync: `hash/*` ~941 lines (+ 1,036-line test
  file), `rw/*` ~628, `filter.rs` 405, `hardlink.rs` 171, `link.rs` 218,
  `scan.rs` 373.
- Assumptions to validate: §A2 (27–33), §A4 (40–46), §A13 (108–114 — the `rw`
  gate is a deliberate keeper), §F37 (424–430).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/sync/src/hash/**`, `rw/**`, `filter.rs`, `hardlink.rs`, `link.rs`,
  `scan.rs`.
- Note `girpr-digest` already provides hashing mechanics (§F34); do not
   duplicate them. `girpr-filter` exists as a shared leaf (§A3).
- Locate each file and its `use crate::` edges in the current tree before editing.

## Implementation plan
1. Validate that each candidate is acyclic in production and identify a direct
   high-level consumer (§A2). Group only related leaves that can share one
   logic-free crate.
2. Reuse/extend existing shared leaves (`girpr-digest`, `girpr-filter`) where the
   code is already shared (§F34, §F37) rather than creating duplicates.
3. Move the leaves behind crate boundaries and point girsync at them directly,
   so a change in cache/planner/commands leaves them cached (§A2).
4. Keep the `rw` gate, FNV lock key and symlink classification local per §A13 —
   move files, do not change policies.
5. Update nextest filters (§F31) and §A5/§F39 doc tables if gircap gains a leaf.

## Constraints / validation
- Payoff test §A2: a pure "move into a crate that the facade re-exports" is not
   a win — confirm a direct consumer.
- Divergent policies are deliberate keepers (§A13); no behaviour change.
- Do not reopen §F37 leaves where further splitting was judged low-value.

## Done when
The validated acyclic leaves live in crates that girsync depends on directly,
policies and bytes are unchanged, and all hash/rw/filter/link/scan tests pass.
