# T02 — `girpr-chunk-layout` leaf

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F5 (lines 192–205) — chunk
  layout/range algebra is implemented twice with different verdict semantics; candidate
  boundary is a leaf with `Range`/`tiled`/`gaps`/`overlaps` and checked, saturating
  construction. `assemble/ranges.rs` imports only `serde`, making it the cleanest domain
  module; `verify/checks.rs` is not leaf-ready.
- Assumptions to validate: §A5 (57–67), §A12 (114–121), §A13 (122–128), §A14 (129–135).

## Scope
- New leaf (proposed `girpr-chunk-layout`). Sources: product `src/repair/phases/plan.rs`
  (`PlannedFile::layout`, `is_tiled`, gaps/overlaps, `validate_chunk_ranges`) and
  `src/repair/file/{inplace,temp}.rs` saturated folds, plus gircap
  `tools/capture/src/assemble/ranges.rs` and arithmetic in `verify/checks.rs`.
  Locate before editing.

## Implementation plan
1. Validate §F5: confirm the half-open model, saturating gap diagnostics and the
   "violations as data, never an error" gircap contract; confirm `verify/checks.rs` is not
   leaf-ready and keep it local.
2. Define the leaf owning `Range`, `tiled`, `gaps`, `overlaps` and checked saturating
   construction; each caller keeps its verdict policy.
3. Migrate callers directly (§A2). Move the crate through §A14 seams (workspace, README,
   gircap tables/manifest comment, bridge doc, nextest filters).
4. Deliver in commits: leaf, product migrations, gircap migration, doc seams.

## Constraints / validation
- The two verdicts must stay divergent: one caller treats a bad layout as a
  repaired-with-zero-fill file, the other as an assertion finding that must still be
  described (§A13). Do not unify them or normalize the finding away.
- Product-side validation wording is pinned by selfcheck-like output (§A5); key order,
  wording and flag conditions must not drift.
- A cycle must not be introduced (§A12); place the leaf at an acyclic boundary.

## Done when
Layout/range algebra lives in the leaf, both verdict policies survive verbatim, all
layout tests pass with unchanged output wording, and doc seams are updated. If the
semantic divergence cannot be preserved, **stop** and record `skipped`.
