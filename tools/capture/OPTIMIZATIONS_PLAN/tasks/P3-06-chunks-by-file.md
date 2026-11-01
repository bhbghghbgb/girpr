# P3-06 — Build `ChunksByFile` once and use `HashMap`

Report: `../../OPTIMIZATIONS_REPORT.md` lines 544-556 (C5).

Dependencies: none.

## Goal

Two related fixes in `verify::materialize::ChunksByFile`:

1. `sizes` is a `BTreeMap` keyed by random hex chunk ids; ordering is unused.
   Use `HashMap`.
2. The public `chunks_for` convenience builds a whole `ChunksByFile` on every
   call. `rebuild_all` builds once, but other callers can pay O(index) per
   file. Make the one-shot grouping explicit.

## Where to start

- `src/verify/materialize.rs`: `ChunksByFile` (around line 102),
  `build` (around line 110), `get`, `size`, `chunks_for` (around line 51),
  `rebuild_all` (around line 271), and tests.
- `src/verify/report.rs` and `tests/offline_bundle.rs` for callers.

## Steps

1. Change `sizes: BTreeMap<&str, i64>` to `HashMap<&str, i64>`. Lookup results
   are identical; iteration order is not used. Update any code that iterates
   `sizes` (there should be none; if there is, sort explicitly).
2. Change `chunks_for` to take `&ChunksByFile` instead of `&ChunkIndex`, or add
   a method `ChunksByFile::chunks_for(...)`. Keep a thin compatibility wrapper
   only if a non-test caller needs the old signature; otherwise update the
   tests. The goal is that no convenience function silently rebuilds the index.
3. Keep `rebuild_all` building exactly once per call and reusing the same
   `ChunksByFile` for both sides.
4. Do not change the sorted `rows` ordering or the lookup semantics.

## Tests

- Existing grouping/lookup tests pass with `HashMap`.
- A test calls the new API once and asserts the same `(id, offset)` list as
  before.
- If there are benchmark-style call-count tests, assert `build` is not called
  per file (or remove the old convenience).
- `rebuild_all` output is unchanged.

## Acceptance

- Lower allocation/CPU in verify/materialize.
- No per-file full-index rebuild remains in the public API.
- `cargo test -p gircap` passes.

## Commit plan

1. `perf(verify): use a hash map for chunk sizes`
2. `refactor(verify): make the file grouping explicit at the call site`
3. `test(verify): cover the one-build lookup path`

## Notes

- If iteration is ever added over `sizes`, sort explicitly; do not rely on
  `HashMap` order.
- Do not reference the report or plan in code comments.
