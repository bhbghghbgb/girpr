# T04 — Small-crates table folds: `crates/log` + `crates/tempdir`

Priority P0 · `crates/log`, `crates/tempdir` · checkpoint CP1 · report findings D5, D6, D7

## Pointers (give these to the implementing session)

* Report finding D5: `docs/09-test-scale-reduction-research.md:L551–L563`.
* Report finding D6: `docs/09-test-scale-reduction-research.md:L565–L574`.
* Report finding D7: `docs/09-test-scale-reduction-research.md:L576–L581`.
* Report constraint (KEEP the release-build test):
  `docs/09-test-scale-reduction-research.md:L826–L831`.
* Report non-findings: `docs/09-test-scale-reduction-research.md:L593–L607`.
* This plan: `OPTIMIZATIONS_PLAN/tasks/T04-small-crates-table-folds.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Table-fold near-identical pure-function tests in the small crates:

1. **D5 — `crates/log/src/plan.rs:384–613`.** File resolution (:449, :468,
   :492, :509) and level/filter resolution (:483, :543, :521, :550, :570, :588)
   → two tables `file_resolution_precedence` and `console_and_file_filters`, with
   a per-row comment preserving each policy rationale. 13 tests → ~6 (net −7).
2. **D6 — `crates/log/src/filter.rs:70–83`.** `a_valid_spec_is_used_as_it_is` +
   `a_broken_spec_falls_back_to_the_projects_own_level` → one 2-row table (−1).
3. **D7 — `crates/tempdir/src/lib.rs:364–374`, `:379–387`.**
   `a_chosen_path_starts_clear` + `a_file_at_the_chosen_path_is_cleared_too` →
   one test covering both, keeping all five assertions (−1).

## Implementation plan

Code anchors (validate before editing): `crates/log/src/plan.rs:384–613`,
`crates/log/src/filter.rs:70–83` (and the optional `Targets` harness at :36–68 /
upstream tests at :92–111), `crates/tempdir/src/lib.rs:364–387`.

Possible commits: one per crate (log tables, tempdir merge), or one per bullet
(three small commits).

D6 optional hygiene (no count effect): the `Targets` `Layer`/`Arc<Mutex<Vec>>`
harness at `filter.rs:36–68` exists only for two upstream-`EnvFilter` tests
(:92–111); it could downshift to `EnvFilter::enabled` with constructed metadata,
removing ~30 lines. Only do this if it stays clearly in scope.

## Acceptance criteria

* All per-case assertions and policy rationales survive in the table rows.
* **Must keep** named and intact:
  `the_default_file_is_written_in_a_release_build_and_not_in_a_debug_one`
  (`plan.rs:426–443`) and `a_bad_file_level_is_a_warning_and_not_a_failure`
  (`plan.rs:600–612`). `crates/log/src/level.rs` tests are already table-driven —
  leave them.
* The two `no_run` doctests mirrored by tests in `crates/log`/`crates/tempdir`
  are kept (nextest cannot run doctests).
* No production logic changed. Affected suites pass.

## Guardrails

* Do not fold away the release-build default-file test — the documented gate
  (`README.md:446–450`) runs `--release -p girpr-log` solely for it.
* Pure functions, no IO change. Report follow-up ideas without implementing them.
