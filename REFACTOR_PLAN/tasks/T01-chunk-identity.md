# T01 — `girpr-chunk-identity` leaf (with digest-normalization placement)

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F4 (lines 177–190) — chunk identity
  `(md5, size)` is one rule in two representations; candidate boundary is a
  program-free leaf owning the digest-bytes/length key, a borrowed view and the
  case-insensitive-hex parse rule, with selection/collision policy left to callers.
- §F8 (lines 232–238) — digest normalization placement is byte-load-bearing and belongs
  with F4, **not** in `girpr-proto`; fold it into this task's constraints.
- Assumptions to validate: §A2 (29–36), §A4 (48–56), §A5 (57–67), §A8 (81–87),
  §A14 (129–135). Keep item: §N4 (709–711).

## Scope
- New leaf (proposed `girpr-chunk-identity`) built on `girpr-digest`/`girpr-proto`.
  Callers: product reuse index/plan/write paths (`crates/fetch/src/sophon/map.rs`,
  `src/repair/reuse.rs`, `src/repair/phases/plan.rs`) and gircap
  (`tools/capture/src/blobs/chunks.rs`, `tools/capture/src/manifest/proto.rs`).
  Locate in the current tree before editing; line numbers may have drifted.

## Implementation plan
1. Validate §F4/§F8: confirm the two representations and that lowercase normalization is
   done at gircap's single decode boundary "to match girpr"; confirm `girpr-proto` must
   stay logic-free (§A4, N10).
2. Define the leaf's owned surface: identity key (digest bytes + length), a borrowed view
   avoiding per-comparison allocation, and the case-insensitive-hex parse/normalize rule.
   Keep selection and collision policy in callers.
3. Migrate product and gircap call sites to consume the leaf directly (§A2); do not keep a
   re-exporting facade.
4. Move the new crate through every §A14 seam: workspace, README table/prose, gircap §2
   table + manifest comment (if gircap depends on it), bridge doc, nextest filters.
5. Deliver in several commits: leaf, product migration, gircap migration, doc seams.

## Constraints / validation
- Byte contracts (§A5): collision handling (`(hash,size) → Vec<usize>` with confirmation
  vs gircap's `BTreeMap` sets) and first-match ordering decide which bytes are fetched and
  therefore `PLAN`/`SUMMARY` bytes — preserve exactly.
- Do not collapse the three digest semantics (§A8); `DigestSet`/`AlgoSet`/`HashAlg`
  stay raw/allocation-free (N4). Do not move normalization into the wire leaf (F8).
- Divergent policies are keepers (§A9): if a representation difference turns out to be
  intentional, stop and raise a follow-up.
- gircap independence (§A4) and synchronized doc tables (§A14) are mandatory.

## Done when
Identity logic and normalization sit in the leaf, both programs consume it directly, all
byte-level identity tests still pin the same figures, and doc seams are updated. If a safe
boundary cannot be found or a keeper would change, **stop** and record `skipped` with a
one-line reason.
