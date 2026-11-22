# T18 — Publish-target predicate into the fs-names leaf

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F23 (lines 386–393) — publish-target
  validation is duplicated while write/atomicity policy intentionally diverges; candidate
  boundary is to extract **only the predicate** into the fs-names leaf and leave each
  writer's atomicity policy local. Confidence medium.
- Assumptions to validate: §A4 (48–56), §A5 (57–67), §A9 (88–97), §N2 (701–703), §N9 (729–732).

## Scope
- Extend `crates/fs-names` (or the relevant fs-names leaf). Sources: capture
  `tools/capture/src/paths.rs` (trailing dot/space, device names, non-atomic small
  documents) and `crates/util-path/src/atomic.rs` (refuses a Windows trailing dot/space
  before `NamedTempFile::persist`). Locate before editing.

## Implementation plan
1. Validate §F23: confirm the duplicated predicate and the intentionally divergent
   atomicity (capture non-atomic documents + one blob rename vs util-path atomic persist).
2. Extract only the shareable predicate (trailing dot/space + device-name shape) into the
   fs-names leaf; leave each caller's atomicity policy where it is.
3. Migrate both callers directly (§A2); move through §A14 seams if gircap calls it.
4. Commit predicate extraction, callers, seams.

## Constraints / validation
- High-risk (§F23): the two callers use the predicate at different times with different
  consequences — an over-eager merge would delete the intentional non-atomic metadata
  writer or the blob rename (N9). Do not touch writer atomicity.
- Do not absorb `_tmp` resume/rollback or the audio move (N2).
- Preserve device-name/rejection wording bytes (§A5).

## Done when
The predicate lives in the fs-names leaf, both writers keep their own atomicity, and all
path/rejection bytes are unchanged. If atomicity would be forced to converge, **stop** and
record `skipped`.
