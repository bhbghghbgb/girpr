# T14 — `girpr-path-shape` validator leaf

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F19 (lines 345–354) — the "legal
  relative path" rule is written twice with opposite normalization; candidate boundary is
  a parameterized path-shape validator with policy fields (normalize vs reject, reserved
  set, trim flag, colon rule) and a caller-supplied stable-rule-ID mapping. Confidence high,
  risk high.
- Assumptions to validate: §A3 (37–47), §A5 (57–67), §A4 (48–56), §A14 (129–135), §A9 (88–97).

## Scope
- New leaf (proposed `girpr-path-shape`). Sources: gircap `tools/capture/src/paths.rs`
  (`path_rule`, stable rule IDs) and product `crates/util-path/src/path.rs` plus
  `tools/capture/src/rejections.rs` (rule IDs feed `rejections.json`). Locate before editing.

## Implementation plan
1. Validate §F19: confirm the two opposite normalizations and that rule IDs feed a
   published file (`rejections.json`).
2. Define the validator with explicit policy fields; keep the caller-supplied stable-rule-ID
   mapping in the caller (do not mint a shared ID scheme).
3. Migrate both callers directly (§A2); move through §A14 seams if gircap calls it.
4. Commit leaf, callers, seams separately.

## Constraints / validation
- High-risk (§F19): policies are semantic — a wrongly kept `./` would make purge delete a
  just-repaired file. Rule IDs feed `diagnostics/rejections.json`, so ID values must stay
  byte-stable (§A5).
- Product lint policy is stricter than the tools' (§A3); code promoted to the leaf must
  satisfy it. gircap independence and doc tables (§A4, §A14) mandatory.
- Do not absorb the refusal ledger (T15) — that is a separate, A4-forbidden boundary.

## Done when
The validator is shared with both policies intact, all rule-ID/rejection bytes are
unchanged, and doc seams are updated. If the semantic divergence cannot be parameterized
safely, **stop** and record `skipped`.
