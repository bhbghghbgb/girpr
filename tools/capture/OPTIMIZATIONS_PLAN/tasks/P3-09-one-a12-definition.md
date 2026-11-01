# P3-09 — One definition of A12

Report: `../../OPTIMIZATIONS_REPORT.md` lines 576-588 (C7).

Dependencies: none.

## Goal

Make `assemble::types::Assembled::a12_holds` linear and share the A12
disjointness rule with `blobs::chunks::Retention`. Today it collects fetched and
reused ids into `Vec`s and asks `!reused.contains(f)` for each fetched id, which
is O(fetch × reuse) and a second spelling of the rule.

## Where to start

- `src/assemble/types.rs`: `Assembled::a12_holds` (around line 409).
- `src/blobs/chunks.rs`: `Retention::a12_failure` (around line 270),
  `a12_holds`.
- `src/assemble/mod.rs` / `src/blobs/mod.rs` module dependencies:
  `assemble` already depends on `blobs`, so putting the shared helper in
  `blobs::chunks` is allowed.

## Steps

1. Add a small helper in `blobs::chunks`, e.g.
   `pub fn fetch_is_disjoint_from_reuse<'a>(fetch: impl Iterator<Item = &'a str>,
   reuse: impl Iterator<Item = &'a str>) -> bool`, implemented with one
   `HashSet` over the reuse side.
2. Use it in `Retention::a12_failure` (or keep `a12_failure` as the richer
   diagnostic and call the helper for the overlap half).
3. Use it in `Assembled::a12_holds` with the plan's fetch/reuse entries.
   `Assembled::a12_holds` only checks disjointness today; keep that exact
   boolean, including the `plan: None -> true` case.
4. Do not change `Retention`'s subset check or its error strings.
5. Do not couple `assemble` to `blobs::chunks` beyond the helper; avoid moving
   types across modules.

## Tests

- Existing A12 tests pass.
- A test with a large fetch/reuse set asserts the same boolean as the old
  quadratic form (use a test-only reference implementation if useful).
- A plan with an overlapping id returns false; a disjoint plan returns true.
- `plan: None` still returns true.

## Acceptance

- Linear A12 check in assembly.
- One definition of "fetch and reuse are disjoint".
- No artifact change.
- `cargo test -p gircap` passes.

## Commit plan

1. `refactor(blobs): share the A12 disjointness predicate`
2. `refactor(assemble): use the shared A12 check`
3. `test(assemble,blobs): cover overlapping and disjoint plans`

## Notes

- Keep the two checks distinct where they are distinct: `Retention` also checks
  `patch_fetch` is a subset of `materialize_new`; `Assembled` does not.
- Do not reference the report or plan in code comments.
