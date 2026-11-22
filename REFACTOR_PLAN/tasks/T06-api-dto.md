# T06 — `girpr-api-dto` leaf (revisit of skipped T18)

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F10 (lines 249–258) — API DTO
  decoders are still duplicated; candidate boundary holds only shared wire shapes with
  both scalar adapters side by side — the boundary the skipped T18 specified. Most
  concretely revisit-able of the skipped tasks, but high-risk.
- Premise change: §F3 (lines 165–173) — DTO decoders now sit in two isolated crates.
- Assumptions to validate: §A4 (48–56), §A5 (57–67), §A14 (129–135), §N10 (734–737).

## Scope
- New leaf (proposed `girpr-api-dto`). Sources: `crates/fetch/src/hyp.rs`
  (`Wrapper<T>`, coercing `num_from_string`) and `tools/capture/src/hyp/types.rs`
  (`IntOrStr`, wire-preserving, forward-compatible). Locate before editing.

## Implementation plan
1. Validate §F10/§F3: confirm both DTO sets are isolated and that `girpr-model` only
   exposes `region`; confirm the old "shared DTO already exists" premise is false.
2. Define the leaf holding only shared wire shapes, with **both** scalar adapters side by
   side (coercing and wire-preserving) — do not collapse them into one.
3. Migrate both decoders directly (§A2). Move through every §A14 seam; adding a gircap
   leaf dependency forces two doc-table updates (§A4).
4. Deliver in commits: leaf, fetch migration, gircap migration, doc seams.

## Constraints / validation
- Capture re-serializes `IntOrStr` to the same JSON form and captures bodies byte-exact
  (§A5); no key/format/order drift.
- URL-join strategies are a recorded divergence — keep them local and raise a follow-up if
  a shared rule seems tempting (§A9).
- gircap stays independent with synchronized doc tables (§A4, N10).

## Done when
Shared wire shapes live in the leaf with both adapters intact, captured bodies stay
byte-identical, and doc seams are updated. If byte-exactness or the adapter divergence
cannot be preserved, **stop** and record `skipped`.
