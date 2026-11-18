# T02 — Root-unit low-risk merges: `plan.rs` layout + `clock_support`

Priority P0 · root product unit tests · checkpoint CP1 · report findings C3, C4

## Pointers (give these to the implementing session)

* Report finding C3: `docs/09-test-scale-reduction-research.md:L393–L408`.
* Report finding C4: `docs/09-test-scale-reduction-research.md:L410–L423`.
* Report synthesis: `docs/09-test-scale-reduction-research.md:L803–L805`
  (pattern 4), `L812–L814` (Wave 1).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T02-root-unit-low-risk.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Two independent, low-risk test reductions in root product unit tests:

1. **`src/repair/phases/plan.rs` layout verdict cluster (C3).** Six near-identical
   `file_with(...).layout()` tests (:565, :574, :584, :592, :600, :608) plus
   `the_cheap_tiling_check_agrees_with_the_layout` (:692, which rebuilds the same
   six cases) → one table asserting all three fields per case plus
   `is_tiled() == layout.tiled`.
2. **`src/lib.rs` `clock_support` meta-tests (C4).** Delete the nested `mod tests`
   at :357–374 (`a_pre_epoch_fixture_skips_without_the_switch` :362 and
   `a_pre_epoch_fixture_fails_with_the_switch` :369). They test a `cfg(test)`
   skip helper, not product behavior.

## Implementation plan

Code anchors (validate before editing):

* `src/repair/phases/plan.rs:564–614`, `:688–719`, and the `file_with` helper.
* `src/lib.rs:346–355` (helper `skip_pre_epoch_decision`), `:357–374` (the module
  to delete). Real clock behavior is covered at `lib.rs:246/301/312` and
  `inplace.rs:1950+`.

Possible commits: one per bullet (two small commits).

For C3, build one table of `(&str, PlannedFile, bool /* tiled */, gaps, overlaps)`
over all six cases; assert every field per row plus the `is_tiled` agreement.
Keep a two-line order-independence assertion (or a reversed-chunks row) so the
order-independence property is not lost.

## Acceptance criteria

* C3: all six exact verdicts retained; `is_tiled` agreement checked on every
  case (strictly stronger than before); 7 tests → 1–2, 12 case constructions → 6;
  order-independence still asserted.
* C4: the two meta-tests and their module are gone; real clock coverage
  (`lib.rs`/`inplace.rs`) untouched.
* No product logic changed. Affected unit suites pass.

## Guardrails

* Do not touch the protected md5 RFC-1321 vectors or throughput benches (report
  §C non-findings, L461–L466).
* Validate each anchor; skip a bullet that no longer matches rather than forcing
  it. Test-only change. Report follow-up ideas without implementing them.
