# T18 — API DTO leaf (keep both decoding strategies)

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F26 (lines 354–364) — API DTO
  field sets match, decoding strategies deliberately do not.
- Assumptions to validate: §A7 (62–67), §A3 (29–35), §A2 (21–27).
- Related: T17 (`girpr-model`), T01 (digest mechanics).

## Scope
- Candidate shared crate `crates/model` (`girpr-model`), module `hyp` / dto, or
  a dedicated `girpr-api` leaf — choose the boundary that stays logic-free.
- Report names: `src/hyp.rs` (`Wrapper<T>`, `GameConfig`, `GameBranch`,
  `ChunkBuild`, `ChunkManifestMeta`, `ManifestFile`, `ManifestUrl`, deprecated
  types, `num_from_string` coercing Visitor), `tools/capture/src/hyp/types.rs`
  (`Wrapper<T>`, `GameConfigs`, `Branch`, `GetBuild`, `BuildManifest`,
  `ManifestRef`, `Download`, deprecated types, `IntOrStr` untagged
  wire-preserving, format-prefix URL join). Locate before editing.

## Implementation plan
1. Validate §F26: the JSON field sets match, but girpr coerces sizes-as-strings
   with `num_from_string` while capture preserves them with `IntOrStr`, and URL
   joining differs deliberately (`Url::join` with trailing slash vs prefix + id
   + suffix). A shared DTO crate must keep **both** strategies side by side.
2. Create the leaf with the shared DTO definitions, parameterised so each tool
   supplies its own number-decoding adapter and URL-join strategy (or keep the
   two adapters as separate public types in the leaf).
3. Migrate both crates' DTOs to the shared definitions, keeping their decoding
   and URL strategies and all field names/types byte-identical.
4. Update §A2 doc tables because gircap gains a new leaf dep.

## Constraints / validation
- Do not collapse `num_from_string` and `IntOrStr` into one decoder (§F26, §A7);
   capture's wire-preserving re-serialization is byte-critical (§A3).
- Keep both URL-join strategies; they are deliberately distinct (§F26, §N9).
- No program logic in the leaf (§A1, §A2).

## Done when
Both crates use the shared DTO definitions, both decoding and URL strategies
survive, all DTO/wire byte tests pass, and §A2 doc tables are updated.
