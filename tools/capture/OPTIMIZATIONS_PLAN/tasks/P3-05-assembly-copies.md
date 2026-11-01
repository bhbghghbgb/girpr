# P3-05 — Avoid assembly copy-through

Report: `../../OPTIMIZATIONS_REPORT.md` lines 558-574 (C6).

Dependencies: none.

## Goal

Reduce allocations and one copy per chunk in `assemble::assembly::patch_flow`:
REUSE currently copies the old file slice into a temporary `Vec`, then copies
that into `out`; FETCH produces an owned `Vec`, then copies it into `out`. The
old-identity map also clones the md5 for every old chunk.

## Where to start

- `src/assemble/assembly.rs`: `patch_flow` (around line 261), the
  `old_by_identity` `BTreeMap<(String, i64), &SophonChunk>` and the
  `(decision, source_range, note, body)` match; `out.extend_from_slice(&body)`.
- `src/assemble/types.rs`: `PlanEntry`, `PatchOutcome`; `assemble` compares
  `patch_flow` output to the direct assembly.
- `src/assemble/tests.rs` for byte-equality tests.

## Steps

1. Pre-size `out` to `new_entry.size` (guarded for negative/oversized values;
   use the existing `max(0)` and checked arithmetic, and return a typed error if
   the size does not fit `usize`). Initialize with zeros if direct indexing is
   used.
2. Replace the `body` temp with direct writes:
   - REUSE: after md5 verification, copy the old slice straight into
     `out[target.start as usize..target.end as usize]`.
   - FETCH: decode into an owned `Vec` (the decode API returns one), then copy
     it into the target range. This is one copy instead of two; further
     zero-copy would require a decode-into API and is out of scope.
   - Keep the error when a range is outside `out` or the old file.
3. Change `old_by_identity` to a `HashMap<(&str, i64), &SophonChunk>` keyed by
   borrowed old md5 slices. Look up with
   `get(&(c.uncompressed_md5.as_str(), c.uncompressed_size))`. The old entry
   outlives the map.
4. Keep `or_insert` (first identity wins) and the deterministic choice.
5. Do not change the plan entry construction, counts, notes, or rewrites.
6. If the new code needs a final length check, keep the existing
   `out.len() != new_entry.size` error.

## Tests

- All assembly byte-equality tests pass, including moved/reused/fetched chunks.
- A test with a negative or too-large `new_entry.size` returns an error rather
  than panicking.
- A test with duplicate old identities still chooses the first offset.

## Acceptance

- Assembly output is byte-identical.
- One fewer copy per chunk on the reuse path and one fewer temporary `Vec`.
- No md5 clone per old chunk in the identity map.
- `cargo test -p gircap` passes.

## Commit plan

1. `perf(assemble): write chunks directly into the output buffer`
2. `perf(assemble): borrow old chunk identities in the reuse map`
3. `test(assemble): cover direct writes and invalid target sizes`

## Notes

- Do not change the measured byte movement through `assemble::tile` or the
  `ChunkSource` memoization; those are deliberate.
- Do not reference the report or plan in code comments.
