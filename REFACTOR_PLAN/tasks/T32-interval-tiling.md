# T32 — Chunk-range tiling → interval crate

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §B3 (lines 410–414).
- Assumption to validate: §I.5 (lines 828–830).

## Scope
- Package: root `girpr`, `src/repair/phases/plan.rs` (lines 44–138, 363+).

## Implementation plan
1. Measure the diff before adopting `nodit`, `rangemap` or `iset` for the
   ~90 LOC of chunk-range tiling/gap/overlap validation (§I.5).
2. Adopt only if it genuinely reduces hand-rolled code.

## Constraints / validation
- Keep the allocation-free `is_tiled()` fast path and the exact
   `(offset,len)` / `(earlier,later)` tuples consumed by zero-fill.
- If the crate adds more call-site code than it removes, **stop** and record as
   `skipped` (report calls this the weakest crate-adoption case).
- Tests named in §B3 (line 414): `plan.rs` lines 554+, 687, 1017, 1069.

## Done when
Either the interval logic is delegated with the fast path intact, or the task is
skipped with the diff-based reason recorded.
