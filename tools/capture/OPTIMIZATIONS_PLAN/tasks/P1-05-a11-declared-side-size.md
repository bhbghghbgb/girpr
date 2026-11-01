# P1-05 — A11 checks the declared side size

Report: `../../OPTIMIZATIONS_REPORT.md` lines 168-187 (A7).

Dependencies: P0-01.

## Goal

Make A11's size half real. `verify::materialize::rebuild` currently derives
`declared_size` as the sum of chunk lengths and passes that same sum to
`Layout::of`, so the layout's `total != declared_size` check is tautological.
Pass the patchmap side's declared `size` instead.

## Where to start

- `src/verify/materialize.rs`: `rebuild` (around line 187), `rebuild_side`
  (around line 315), `rebuild_all`; `assemble::Layout::of` and `assemble::tile`.
- `src/assemble/types.rs`: `Side { tag, size, md5, nchunks }` (around line 115)
  and A11 row construction in `selfcheck_rows`.
- `src/verify/checks.rs`: `check_pair` (around line 141), which already compares
  rebuilt length to `d.size` under A7/A8.
- `tests/offline_bundle.rs` and `src/verify/materialize.rs` tests that call
  `rebuild` directly.

## Steps

1. Add a `declared_size: i64` parameter to `rebuild` (or add a
   `rebuild_with_declared_size` and keep `rebuild` for callers that genuinely
   only have the sum). Prefer one public function with the explicit parameter;
   the tautology is easier to see gone.
2. Validate the parameter: negative declared size is a typed error naming the
   side. Do not let it reach `Layout::of` as a panic.
3. Pass `side.size` from `rebuild_side`. `rebuild_all` already has the
   `PatchMap` side.
4. Keep the chunk-length sum for the `tile` output sizing only if `tile`
   requires it; otherwise pass the declared size through. `Layout::of` must see
   the declared side size and report a mismatch/gap/overlap as it already does.
5. Update direct `rebuild` callers: tests and `tests/offline_bundle.rs` should
   pass the side's declared size. Where a test wants the old sum, compute it
   explicitly in the test.
6. Keep the A11 row wording; it can now honestly state the declared size.

## Tests

- Unit: a side whose declared `size` is larger than the chunk sum fails A11
  with a message naming both numbers.
- Unit: a side whose declared `size` is smaller than the chunk sum fails A11.
- Unit: a matching side still rebuilds and passes.
- Integration/verify: if P0-01 has a characterization test with a disagreeing
  side size, flip it to the intended failure and assert `verify` exits non-zero
  (or reports A11) rather than panicking.
- Existing assembly tests must keep their output bytes unchanged.

## Acceptance

- A11's size claim is independently falsifiable.
- No panic on a hostile declared size.
- Valid bundles rebuild byte-identically.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(verify): pass the declared side size into the layout check`
2. `test(verify): cover side sizes above and below the chunk sum`

## Notes

- This can turn a previously green bundle red; that is the intended
  strengthening. Do not weaken A7/A8 to compensate.
- Do not reference the report or plan in code comments.
