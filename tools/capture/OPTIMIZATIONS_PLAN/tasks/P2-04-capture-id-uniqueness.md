# P2-04 — Capture-id uniqueness invariant

Report: `../../OPTIMIZATIONS_REPORT.md` lines 459-475 (B5).

Dependencies: P2-02.

## Goal

Assert that capture ids are unique across admitted and rejected records, and
that no two records claim the same `patchmaps/<id>.json` stem. Today the flow
usually avoids a collision, but only de-duplicates rejections by path; a future
budget/order change could publish two records with one id.

## Where to start

- `src/select/mod.rs`: `select` (around line 525), `Admission`, `Rejection`,
  `Candidate::capture_id`, the forced-ordinal loop, phase 1/2 ordinals.
- `src/emit/mod.rs`: `write_patchmap` uses `capture_id` as the file stem.
- `src/assemble` / `src/run/mod.rs` for where admitted captures become
  patchmaps.
- Existing selection tests.

## Decision

Add a debug assertion and a test, not a release-mode refusal. A hard refusal
could reject a degenerate manifest that still produces a usable bundle; the
report recommends catching the bug without changing release behavior.

## Steps

1. Add a private `debug_validate_unique_ids(selection: &Selection)` (or a
   method) in `select` that:
   - collects admitted `capture_id`s and rejects duplicates;
   - collects rejected `capture_id`s and rejects duplicates;
   - checks no admitted id equals a rejected id;
   - checks each id is a valid single file stem (no `/`, `\`, `..`, empty).
2. Call it at the end of `select` behind `debug_assert!` (or
   `if cfg!(debug_assertions)`), so tests and debug builds catch a collision.
3. Do not change the ordinal-assignment logic in this task. If the assertion
   fires on an existing test, investigate and fix the ordinal logic only if the
   fix is local and clearly correct; otherwise return the finding.
4. Add a unit test that constructs a `Selection` with duplicate ids and invokes
   the validator directly, asserting it reports the duplicate. Add a positive
   test for the normal selection.
5. Make sure the validator does not allocate a huge set on the normal path in
   release; a debug-only guard is fine.

## Tests

- Positive: a normal selection with one capture per class passes.
- Negative: duplicate admitted ids fail the validator.
- Negative: an admitted id colliding with a rejected id fails.
- The existing selection tests still pass.

## Acceptance

- Debug/test builds catch a capture-id collision.
- Release behavior and artifacts are unchanged.
- The invariant covers both admitted and rejected records.
- `cargo test -p gircap` passes.

## Commit plan

1. `test(select): assert capture ids are unique across admitted and rejected`
2. `chore(select): add a debug-only capture-id invariant`

## Notes

- Do not make this a user-facing refusal.
- Do not reference the report or plan in code comments.
