# T04 — `girpr-reuse-decision` leaf

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F7 (lines 220–230) — reuse/salvage
  ordering is a pure algorithm with a divergent twin; candidate boundary is a leaf
  parameterized by source provider with identity-only and order-survival modes.
- Assumptions to validate: §A5 (57–67), §A13 (122–128), §A2 (29–36), §A14 (129–135).

## Scope
- New leaf (proposed `girpr-reuse-decision`). Sources: product `src/repair/reuse.rs`
  (pure write-order decision with swappable deferred-optimum function) and gircap
  `tools/capture/src/blobs/chunks.rs` (`retention`/`Set::{Reuse,PatchFetch,Materialize}`).
  Locate before editing.

## Implementation plan
1. Validate §F7: confirm the write-order survival rule (ascending iff `r_i >= o_i`,
   descending iff `r_i <= o_i`, tie to ascending, conservative for holes) and gircap's
   identity-only decision.
2. Define the leaf parameterized by a source provider with an explicit mode for
   identity-only vs order-survival; keep the swappable optimum seam.
3. Migrate both callers directly (§A2); move through §A14 seams if gircap calls it.
4. Commit leaf, migrations, doc seams separately.

## Constraints / validation
- Load-bearing asymmetry (§F7): the writer uses the real file length as `usable_len`
  while the estimate uses `u64::MAX` because containment was already proved — a unified
  API must preserve this exactly.
- Reuse order decides which bytes survive and therefore `PLAN`/`SUMMARY` bytes (§A5).
  First-match/tie semantics must stay identical.
- Do not absorb digest identity (T01) or projection (T03).

## Done when
The decision is in the leaf with both modes, the `usable_len` asymmetry is preserved, all
reuse/salvage tests pin the same bytes, and doc seams are updated. If the two modes cannot
coexist safely, **stop** and record `skipped`.
