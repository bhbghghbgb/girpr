# P4-04 — Allocation cleanups

Report: `../../OPTIMIZATIONS_REPORT.md` lines 637-648 (C10), 650-662 (C11),
664-676 (C12), and 744-750 (D6 context).

Dependencies: none.

## Goal

Three independent, low-risk allocation/complexity cleanups:

1. `paths::escape_path` always allocates a `Vec` and a `String` even when no
   component needs escaping.
2. `verify::checks::check_agreement` is O(recorded × recomputed) because
   `recomputed(id)` scans the whole row vector.
3. `blobs::ChunkIndex::build` clones each want's role set twice
   (`indexed_from` returns `want.sets.clone()` and keeps a clone on the blob).

## Where to start

- `src/paths.rs`: `escape_path` (around line 190), `escape_component`,
  `is_windows_device_name`, tests.
- `src/verify/checks.rs`: `check_agreement` (around line 431) and `REDERIVED`.
- `src/blobs/mod.rs`: `indexed_from` (around line 66), `ChunkIndex::build`
  (around line 95), `IndexedBlob { props, referenced_by, sets }`; call sites.
- `src/emit/index.rs`, `src/verify/bundle.rs`, A14, and readers that call
  `escape_path`.

## Steps

### C10: escape_path fast path

1. Add a predicate that scans `/`-separated components for `%` or a Windows
   device name.
2. If no component needs escaping, return `path.to_string()` (one allocation,
   same as the final join today but no `Vec`/`Cow` churn). Keep the `String`
   return type so callers do not change.
3. Otherwise keep the existing per-component mapping.
4. Do not change the escaped output for any input; add a table test including
   `%`, `%%`, device names, nested paths, and already-plain paths.

### C11: check_agreement

1. Build `HashMap<&str, bool>` once from `rows`, where the value is "all rows
   with this id are green". Decide the missing-id default: the current closure
   `rows.iter().filter(...).all(...)` returns true for a missing id, so the map
   lookup must default to true.
2. Iterate recorded assertions and consult the map. Preserve the current
   one-direction rule (only recorded-green rows that recompute false are
   disagreements).
3. Keep the tracing message unchanged.

### C12: ChunkIndex role clones

1. Make `ChunkIndex::build` take `Vec<IndexedBlob>` and derive role lists from
   each blob's own `sets` field. Since `IndexedBlob` already carries `sets`,
   no second clone is needed.
2. Update `indexed_from` to return just `IndexedBlob`.
3. Update the three call sites and tests.
4. Keep the index's `referenced_by`/`sets` invariants: the role lists must
   still be the union of the blobs' sets.

## Tests

- `escape_path` table tests above; all outputs unchanged.
- `check_agreement`: green/green, green/false, missing id, multiple rows per id;
  compare to the old semantics.
- `ChunkIndex::build` output is identical for a representative store
  (including a blob in two roles); a test asserts the role list is derived
  from `sets`.
- `cargo test -p gircap` passes.

## Acceptance

- Common path escapes without a `Vec`/join.
- Agreement check is linear in the row count.
- One fewer role-set clone per blob.
- No artifact change.
- `cargo test -p gircap` passes.

## Commit plan

1. `perf(paths): skip the escape allocation when nothing needs escaping`
2. `perf(verify): index recomputed rows for agreement`
3. `perf(blobs): derive chunk role lists without a second clone`
4. `test(paths,verify,blobs): cover unchanged outputs and linear checks`

## Notes

- These are independent; keep separate commits so each can be reverted alone.
- Do not change the escape format or the agreement semantics.
- Do not reference the report or plan in code comments.
