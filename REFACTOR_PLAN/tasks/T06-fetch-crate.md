# T06 — HTTP + Sophon fetch crate

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F17 (lines 253–255) —
  `src/hyp.rs` (782) + `src/sophon/{mod,filter,map,proto,stream}.rs` (1529) ≈
  2,311 lines, sole home of reqwest/zstd/prost-facing repair code; blocker is
  `src/report/emit.rs:25` (`ReportProvenance` naming `GameConfig` /
  `GameBranchPackage` / `ChunkBuild`).
- Assumptions to validate: §A2 (27–33), §A6 (55–61), §A4 (40–46), §A5 (48–53).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/hyp.rs`, `src/sophon/*.rs`.
- Blocker: `src/report/emit.rs:25` and the provenance types it names.
- Proto wire types already live in `girpr-proto` (§A3); do not re-create them.
- Locate all edges in the current tree before editing.

## Implementation plan
1. Validate §F17: confirm the fetch code is the only reqwest/zstd/prost user in
   the product and map the `ReportProvenance` dependency. Decide whether the
   provenance type can move down below the new crate or whether an interface
   seam is needed.
2. Create the fetch crate and make the high-level repair entry points depend on
   it directly (§A2), so a change elsewhere leaves the heavy reqwest/zstd/prost
   code cached.
3. Keep the byte-critical protobuf spelling (`chuncks`, `unknown_field6`) and
   report bytes untouched (§A6); reuse `girpr-proto` rather than duplicating.
4. Preserve the four fake HTTP servers / test HTTP seam as kept duplication
   (§F40, §A13); do not merge them into a shared fixture.
5. If gircap gains a leaf dependency, update the §A5/§F39 doc tables.

## Constraints / validation
- Do not cross into gircap's http subsystem (T14) or couple the two programs
  (§A5). The product and capture mocks deliberately stay separate (§F40).
- No byte/behaviour change in fetch or report provenance (§A6).
- If the provenance blocker forces the report plane into the crate, stop and
  raise a follow-up rather than merging the planes.

## Done when
Fetch code is a crate that high-level repair code depends on directly, proto
bytes and provenance contracts are unchanged, and all fetch/sophon tests pass.
