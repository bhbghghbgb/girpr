# T13 — Collapse duplicated root integration fixtures

## Goal / pointers
- `docs/10-test-optimization-research.md`:
  - §F19 (lines 308–311) — `cli_contract` JSON/text plane pairs use the same
    `seed_one_damaged_one_clean` fixture and differ only in `--output`; one test per pair
    running the binary twice against one mock/install.
  - §F20 (lines 313–318) — `process.rs` repeats `start_standard_install` across five
    phase-duration cases plus two base-precedence cases; one table-driven phase test and
    a two-row precedence table.
  - §F23 (lines 334–337) — repeated `build_fixture()` inside single tests (41 in
    `repair_writer.rs`, 28 in `repair_offline.rs`, 2 in `process.rs`); pass `&GameFixture`.
- Assumptions to validate: §A1 (46–52), §A5 (75–80), §A10 (108–110), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- `tests/cli_contract.rs`, `tests/process.rs`, `tests/repair_writer.rs`,
  `tests/repair_offline.rs`, and the fixture helpers they use. Locate current call sites
  before editing; account for the new binary grouping from T01 if it landed.

## Implementation plan
1. Validate each sub-item: confirm the paired cases truly share setup and differ only in
   the named flag, and that merging does not drop a process-level assertion.
2. Collapse the three JSON/text pairs into one test each that runs the binary twice
   against one mock/install (F19).
3. Convert the five phase cases and two precedence cases in `process.rs` to table-driven
   tests, keeping the same number of process-level assertions with fewer
   mocks/installs (F20).
4. Replace in-test `build_fixture()` calls with a borrowed `&GameFixture` where the test
   does not need independent fixtures (F23). Do not expect measurable wall savings
   (tiny fixtures, §A5) — do it for source clarity only.
5. Update nextest filters if a named case moves (§A10); deliver in commits per sub-item.

## Constraints / validation
- Keep process-level coverage: a binary smoke must remain where the subject is the
  process boundary (§A12, F22). Do not convert all spawns to in-process.
- Preserve assertion messages and result bytes (§6); fixtures are already tiny, do not
  resize them (§A5).
- Do not merge tests named in nextest filters or host-dependent skips (§A10).

## Done when
The duplicated mocks/installs and fixture rebuilds are collapsed, process-level
assertions are preserved, nextest filters match, and the integration suite passes.
