# T13 — `girpr-install-provenance` value crate (near-forbidden)

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F18 (lines 334–341) — report
  provenance is a value-shape duplication; candidate boundary is a **format-free** value
  crate holding only scalar identity fields and their extraction policy. **High-risk /
  near-forbidden**: the moment it emits a key or fixes a field order it becomes a shared
  contract. Confidence low-medium.
- Assumptions to validate: §A4 (48–56), §A5 (57–67), §A6 (68–75), §A14 (129–135).

## Scope
- New value crate (proposed `girpr-install-provenance`). Sources: product
  `src/repair/mod.rs` / `crates/report/src/emit.rs` (`ReportProvenance` from `Meta`) and
  gircap `tools/capture/src/hyp/mod.rs` / `emit/selfcheck.rs` (bundle provenance).
  Locate before editing.

## Implementation plan
1. Validate §F18: confirm both provenance shapes are built from the same API answers and
   that no emitter/order is shared today.
2. If attempted, define a strictly format-free value crate: scalar identity fields plus
   extraction policy only, with **no** serialization, key emission or field order.
3. Migrate both builders directly (§A2); move through §A14 seams if gircap calls it.
4. Given the near-forbidden risk, a validated skip with a one-line reason is a legitimate
   and likely outcome; do not create a shared format.

## Constraints / validation
- The moment the crate emits a key or fixes field order it becomes a shared contract that
  §A4 reserves to each program and §A6 forbids merging — stop if this is unavoidable.
- All provenance bytes and field orders must stay identical (§A5).
- Do not absorb the report plane (§A6) or exit contracts (§A7).

## Done when
A strictly format-free value crate is shared with zero emitted keys — or, more likely, a
validated skip is recorded with the reason.
