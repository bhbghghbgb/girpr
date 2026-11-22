# T03 — `girpr-chunk-cost` download-projection fold

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F6 (lines 207–218) — the
  download-projection formula is literally documented as a cross-program copy; candidate
  boundary is a pure fold over `(identity, compressed_size) × local-reuse membership`
  with each program supplying the set.
- Assumptions to validate: §A5 (57–67), §A2 (29–36), §A14 (129–135).

## Scope
- New pure leaf (proposed `girpr-chunk-cost`). Sources: product
  `src/repair/phases/plan.rs` (`est_file_download_bytes`, no-scan and scanned derivations),
  `src/repair/scan.rs`, and gircap `tools/capture/src/blobs/mod.rs` /
  `blobs/plan.rs` (`projected_download`). Locate before editing.

## Implementation plan
1. Validate §F6: confirm the `Σ compressed_size.max(0)` formula and its saturating
   rationale, and that the product derives it twice.
2. Define a pure fold taking identity + compressed size + a local-reuse membership set
   supplied by the caller; do not let the leaf choose the set or policy.
3. Migrate all callers directly (§A2); the projection stays an estimate/lower bound, not
   a promise. Move the crate through §A14 seams if it becomes a gircap dependency.
4. Commit leaf, migrations, doc seams separately.

## Constraints / validation
- Byte contracts (§A5): the figure is simultaneously a ceiling input, a published `PLAN`
  key, `SUMMARY plan_download_bytes`, and the artifact bundle ranking key — all must stay
  identical. `plan.rs` records it as a lower bound; preserve that framing.
- Do not absorb collision/selection policy (belongs to T01) or reuse decision (T04).

## Done when
The projection formula lives in the pure leaf, every published figure is byte-identical,
callers supply their own reuse sets, and doc seams are updated. If the figure cannot be
preserved, **stop** and record `skipped`.
