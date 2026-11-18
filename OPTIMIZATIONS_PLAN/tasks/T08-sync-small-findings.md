# T08 — Sync smaller findings: helper meta-tests + fixture trims

Priority P0 · `tools/sync` (girsync) · checkpoint CP2 · report finding E6

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L764–L784` (§E6).
* Report synthesis: `docs/09-test-scale-reduction-research.md:L803–L805`
  (pattern 4), `L812–L814` (Wave 1).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T08-sync-small-findings.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

A few small, low-value scale trims in `tools/sync`:

1. **Helper/meta-tests (−2 tests).**
   * `tests/sync_plan/table.rs:209–240`
     (`the_fixture_covers_every_apply_phase`) reads only the test's own `cases()`
     table and `APPLY_ORDER` — fold into a `debug_assert` in `cases()`
     (`sync_plan/main.rs:342`).
   * `tests/lazy/sync.rs:208–218` (`fixtures_classify_as_folders`) asserts a
     fixture helper — fold into `pair()`/`fixture()` as a `debug_assert`.
2. **`src/scan.rs:240–248`** half-asserts that a test env var is unset — keep the
   constant pin, drop the harness pin (marginal; skip if unclear).
3. **Fixture-size trims (low value).**
   * `tests/rw_threads/main.rs:116–140`: `PAD_COUNT = 16` only needs to exceed the
     widest pool (8+1) — derive it from the limit instead of a fixed constant
     (saves ~42 file writes).
   * `tests/rw_threads/through_the_binary.rs:177–180`: 40 files × 6 spellings
     (=240); derive from `limits_of_flags()` (e.g. `2 × pool + 4`) → ~96
     (saves ~144 writes).

## Implementation plan

Code anchors (validate before editing): `tests/sync_plan/table.rs:209–240`,
`tests/sync_plan/main.rs:342`, `tests/lazy/sync.rs:208–218`, `src/scan.rs:240–248`,
`tests/rw_threads/main.rs:116–140`,
`tests/rw_threads/through_the_binary.rs:177–180`.

Possible commits: one for meta-test folds, one for fixture trims, or a single
commit for the whole (small) task.

## Acceptance criteria

* The two meta-tests become `debug_assert`s with the same invariant; no
  assertion about production behavior is lost.
* Fixture counts are derived from the actual concurrency limits and still exceed
  the widest pool.
* `benches/tree_shapes.rs` is **not** touched (its `TINY_FILES/LARGE_FILES/
  DEEP_LEVELS` are load-bearing for timing, not a gate — report §E6 L779–L784).
* Affected suites pass. No production behavior change.

## Guardrails

* These are low-value trims; if a trim turns out to be already done or not worth
  the churn, skip that bullet and report it rather than forcing it.
* Test-only change (the `scan.rs` constant pin stays). Report follow-up ideas
  without implementing them.
