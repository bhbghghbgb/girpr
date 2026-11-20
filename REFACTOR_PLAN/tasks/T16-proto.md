# T16 — Sophon protobuf wire types leaf (`girpr-proto`)

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F24 (lines 331–341) — wire-identical
  duplicate protobuf messages.
- Assumptions to validate: §A2 (21–27), §A3 (29–35), §A1 (13–19).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (§A2 on shared contracts).

## Scope
- Candidate shared crate `crates/proto` (`girpr-proto`).
- Report names: `src/sophon/proto.rs`, `tools/capture/src/manifest/proto.rs`
  (three types with same tags/types, misspelled `chuncks`, opaque field 6,
  `unknown` vs `unknown_field6`, capture's added methods and `Serialize` derive,
  dump tests pinning exact key set). Locate before editing.

## Implementation plan
1. Validate §F24 side by side in the current tree: same messages, tags and
   types; note that field-name and `Serialize` key differences are load-bearing
   (§A3). Confirm a leaf can hold the wire types without holding program logic
   (§A1, §A2) — a shared wire type is not a shared *contract between the two
   programs*, since the artifact is the interface.
2. Create `girpr-proto` with the three wire types and their `prost` attributes,
   preserving the misspelling `chuncks`, field 6 and the exact `Serialize` key
   names.
3. Migrate both crates to the shared types. Keep capture's added methods
   (`is_folder_entry`, `chunk_bytes`, `covered_end`, `validate_arithmetic`,
   `identity`, `normalise`) in capture as extension impls or local wrappers; do
   not move them into the leaf if they encode capture logic.
4. Update §A2 doc tables because gircap gains a new leaf dep.

## Constraints / validation
- Wire tags, field names and JSON key names must be byte-identical (§A3); dump
   tests pin the exact key set.
- The leaf must not become a shared program contract or gain program logic
   (§A2, §A1).
- If capture cannot keep its methods without moving logic into the leaf, stop
   and mark `skipped` with that reason.

## Done when
Both crates use `girpr-proto` wire types, capture keeps its methods locally,
dump/wire byte tests pass, and §A2 doc tables are updated.
