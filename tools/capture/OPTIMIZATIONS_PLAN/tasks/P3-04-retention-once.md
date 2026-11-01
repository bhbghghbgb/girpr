# P3-04 — Compute retention once per pair

Report: `../../OPTIMIZATIONS_REPORT.md` lines 505-517 (C2).

Dependencies: none.

## Goal

Stop computing `retention(pair)` twice per pair. `blobs::plan::plan` calls it
for the A12 check and logging, then `blobs::chunks::wants` calls it again for
every pair to build the reusable set. Each call rebuilds a `HashSet` and clones
old/new chunk ids.

## Where to start

- `src/blobs/plan.rs`: `plan` (around line 100), the loop over
  `selection.admitted.iter().zip(&pairs)` that computes `retention`.
- `src/blobs/chunks.rs`: `retention` (around line 303), `wants` (around line
  355), `Retention`, tests.
- `src/run/mod.rs` call site of `blobs::plan::plan`.

## Steps

1. Add `wants_with(pairs: &[FilePair], retentions: &[Retention]) -> Vec<Want>`
   (or change `wants` to take an optional cache) that consumes the precomputed
   retentions. Keep a `wants(pairs)` wrapper that computes them for tests and
   other callers, or update all callers if there is only one production path.
2. In `plan`, collect `Vec<Retention>` while running the A12 check, then pass
   it to `wants_with`. Do not compute a Retention twice.
3. Keep the deterministic order of `Want`s identical. `wants` iterates pairs in
   order and merges by id; the precomputed vector must be index-aligned.
4. Add a debug assertion that `retentions.len() == pairs.len()`.
5. Do not change `Retention`'s fields, the A12 rule, or the projected byte
   calculation.

## Tests

- Unit: `wants_with` produces byte-for-byte the same `Want` list as `wants` for
  a representative pair set (including shared chunks across builds).
- Unit: a mismatched length is caught by the debug assertion or returns the
  existing error shape.
- Existing `wants` order tests still pass.
- A plan-level test asserts `retention` is only computed once (if the code has
  a cheap counter/seam; otherwise the equality test is enough).

## Acceptance

- One retention computation per pair on the planning path.
- Artifact and plan output unchanged.
- `cargo test -p gircap` passes.

## Commit plan

1. `refactor(blobs): pass precomputed retentions into the want builder`
2. `test(blobs): prove cached and uncached wants are identical`

## Notes

- This is internal API only; do not change the `chunks/index.json` shape.
- Do not reference the report or plan in code comments.
