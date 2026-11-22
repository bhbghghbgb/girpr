# T16 — `girpr-request-url` leaf

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F21 (lines 367–375) — region
  resolution shares values but not request building; candidate boundary is a leaf of pure
  functions over the shared region plus a base/override seam. Confidence medium.
- Assumptions to validate: §A5 (57–67), §A8 (81–87), §A4 (48–56), §A14 (129–135), §A9 (88–97).

## Scope
- New leaf (proposed `girpr-request-url`). Sources: gircap
  `tools/capture/src/biz/mod.rs` (`Biz::resolve`, `Endpoints` URL builders, `push_pair`/`enc`
  form encoding, `BranchParams`) and product resolve policy in `crate::hyp`
  (`girpr-model::region` is already shared). Locate before editing.

## Implementation plan
1. Validate §F21: confirm the shared region values and the local URL builders; confirm
   `--hyp-base`/`--sophon-base` is capture's testing seam.
2. Define pure functions over the shared region plus a caller-supplied base/override seam;
   keep URL-join strategy differences local (recorded divergence).
3. Migrate callers directly (§A2); move through §A14 seams if gircap calls it.
4. Commit leaf, callers, seams.

## Constraints / validation
- High-risk (§F21): query order and escaping are load-bearing when a reader diffs a
  captured URL against a fresh request (§A5); URL-join strategies are a recorded divergence
  (§A9) — keep them local.
- `--hyp-base`/`--sophon-base` seam must survive. gircap independence and doc tables
  (§A4, §A14) mandatory.
- Do not absorb digest semantics (§A8).

## Done when
URL building is shared over the region with the override seam intact, all captured URL
bytes and query orders are unchanged, and doc seams are updated.
