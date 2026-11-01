# P1-08 — Cross-check a patchmap plan against the index

Report: `../../OPTIMIZATIONS_REPORT.md` lines 229-246 (A10).

Dependencies: P0-03.

## Goal

Make `verify` prove that a patchmap's `plan[]` and the blob closure in
`chunks/index.json` describe the same patch. Today
`check_patchmap_arithmetic` treats `plan[]` as the source of truth and only
checks its internal arithmetic.

## Where to start

- `src/verify/checks.rs`: `check_patchmap_arithmetic` (around line 247) and
  `check_pair` (around line 141); A20 is the derived arithmetic row.
- `src/verify/materialize.rs`: `ChunksByFile`, `chunks_for`, `rebuild`,
  `rebuild_side`, `rebuild_all`; `Ref.key()` shows the canonical
  `(tag, capture_id, path, role)` grouping.
- `src/assemble/types.rs`: `PatchMap`, `Side`, `PlanEntry` (`i`, `decision`,
  `target_range`, `source_range`, `chunk_id`, `url`, sizes).
- `src/blobs/chunks.rs`: `Set::MaterializeOld`, `MaterializeNew`,
  `PatchFetch`, and the A12 retention sets.
- The bridge in `tests/artifact_bridge/bundle.rs` trusts `plan[]`; no test
  change is required there, but its cross-tool assertions will benefit.

## Canonical mapping

For one capture:

- New side index references: `ChunksByFile::get(tag, capture_id, path,
  Set::MaterializeNew)` -> ordered `(chunk_id, offset)`.
- Old side index references: same key with `Set::MaterializeOld`.
- Plan entries: `i` is the new-side chunk index; `target_range.start` is the
  target offset; `chunk_id` is the new chunk.
- A `Reuse` entry must have a `source_range` whose start/end and `chunk_id`
  match an old-side reference. A `Fetch` entry must have no `source_range`.
- Duplicate placements (same chunk id at multiple offsets) are legal; match by
  offset, not by id alone. If the plan has duplicate target offsets, fail.

## Steps

1. Add a new check function (same A20 row family or a sibling id if the schema
   already reserves one; do not invent an id if A20 is the documented
   "patchmap arithmetic" row). It should consume the `PatchMap` and the
   capture's index references from `ChunksByFile`.
2. Compare, for each plan entry:
   - target offset and chunk id against the new-side reference at that offset;
   - decision against the index role sets (`PatchFetch` vs reuse) where the
     index records them;
   - `source_range` against an old-side reference for `Reuse`, and absence for
     `Fetch`;
   - `compressed_size`/`uncompressed_size` against the index blob when those
     are available.
   - URL: at minimum require that a `Fetch` entry's URL is present and ends in
     the chunk id (the suffix may follow); do not re-fetch anything.
3. Check the converse: every new-side index reference has exactly one plan
   entry, and every old-side reference used by a `Reuse` is actually
   represented. A plan that omits a chunk is as wrong as one that invents one.
4. Return `Assertion::fail` rows naming the capture, plan index, field, and
   both values. Return a single passing row when everything agrees.
5. Wire it into `verify::report` after `rebuild_all` (where the index and
   patchmaps are both available) or into `check_pair` if that is where the
   `ChunksByFile` is naturally in scope. Do not duplicate the grouping work.
6. If the existing A20 row already reports arithmetic, append these rows under
   the same id and wording, or use a clearly documented sibling id; keep
   `check_patchmap_arithmetic`'s current tests passing.

## Tests

- A self-consistent plan with a different chunk id than the index fails.
- A plan with a target offset that disagrees with the index fails.
- A `Fetch` entry with a `source_range`, or a `Reuse` entry without one, fails.
- A plan that omits an index reference fails; one that invents a chunk fails.
- A correct plan passes (existing fixtures).
- Flip the P0-03 characterization test so it now expects failure.

## Acceptance

- `verify` proves the plan and the blob closure describe the same patch.
- A stale/tampered but internally consistent plan no longer passes.
- Valid bundles are unchanged.
- `cargo test -p gircap` passes.

## Commit plan

1. `feat(verify): cross-check patchmap plans against the chunk index`
2. `test(verify): cover plan/index divergence in ids, offsets, and decisions`

## Notes

- Prefer one canonical mapping helper over ad-hoc scans; duplicate matching is
  the source of subtle false failures.
- Do not change the bridge's assertions in this task.
- Do not reference the report or plan in code comments.
