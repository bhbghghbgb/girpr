# T02 — `ChunkIdentity` (md5, size) value type

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F27 (lines 366–371) — chunk
  identity `(md5, size)` implemented in four places.
- Assumptions to validate: §A7 (62–67), §A3 (29–35), §A2 (21–27).
- Follows T01; reuse its digest mechanics if T01 completed.

## Scope
- Report names: `tools/capture/src/manifest/proto.rs` (`SophonChunk::identity`),
  `tools/capture/src/manifest/diff.rs`, `tools/capture/src/assemble/assembly.rs`,
  `src/sophon/map.rs` (`ReuseEntry::matches`, `FileReuse`),
  `src/repair/reuse.rs` (`sources_for`). Locate in the current tree before editing.
- Candidate home: a value type in `girpr-digest` (T01) or a small `girpr-model`
  leaf; choose whichever keeps it logic-free (§A1).

## Implementation plan
1. Confirm the one rule described in §F27: first-occurrence-wins,
   case-insensitive MD5 comparison, size part of identity. Verify each of the
   four sites actually obeys it in the current code before unifying.
2. Introduce a `ChunkIdentity` value type that owns the equality/hash rule and
   expose it from the chosen shared crate.
3. Migrate the four sites to it, preserving each caller's surrounding policy
   (diff vs assembly vs reuse); do not move the callers' selection logic into
   the shared crate.
4. If any site's rule differs on inspection, keep that site local and record
   the divergence in the task's `note` rather than forcing a merge.

## Constraints / validation
- Case-insensitive MD5 and size-as-part-of-identity are load-bearing (§F27);
  byte-level manifest/diff tests must stay green (§A3).
- Do not pull in girsync's `DigestSet` or collapse §A7 semantics.
- If a new gircap leaf dep is added, update §A2 doc tables (see T01).

## Done when
All four chunk-identity checks use the shared value type and the diff,
assembly and reuse tests pass unchanged.
