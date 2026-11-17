# T13 — Cross-binary log-file policy reduction

Priority P1 · all binaries (girpr, gircap, girsync) · checkpoint CP5

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-suite-reduction-audit.md:L50–L73` (§A).
* Report overlap notes: `docs/08-test-suite-reduction-audit.md:L95–L100` (§B.1
  bullets 4–5), `L152–L153` (§D.1), `L157–L158` (§D.3).
* Report "kept intentionally / kept" notes: `docs/08-test-suite-reduction-audit.md:L143–L148`,
  `L168–L173`, `L200–L204`.
* This plan: `OPTIMIZATIONS_PLAN/tasks/T13-cross-binary-log-policy.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

One shared policy in `crates/log` is asserted at three layers, three times over.
Reduce the duplication while keeping girpr's CLI set as the end-to-end
representative:

* **girpr (E2E representative):** table-drive the log-file process tests in
  `tests/cli_contract.rs:981–1331` (11 tests) → 4–5 process tests (keep them as
  process tests). Also remove the golden self-test
  `the_golden_rejects_a_reorder_a_duplicate_a_misplaced_progress_and_a_rename`
  (2130) — it tests the test-only `check_golden` helper on synthetic values;
  the real goldens (1936–2065) exercise it.
* **gircap:** reduce `tools/capture/tests/cli_contract.rs` log-file tests
  (529, 565, 586, 610, 698, 635, 665) to one smoke case; move the env-prefix
  check into the `logging` unit test.
* **girsync:** reduce `tools/sync/tests/output.rs` log-file tests
  (632, 670, 697, 728, 548, 483, 517) to one smoke case; move the env-prefix
  check into the `logging` unit test. Fold `output.rs:438/483` (console-level
  assertion into record-log assertion, §D.1) and parametrize `output.rs:517/548`
  (flag vs environment log-file level, §D.3).
* **Per-binary unit duplicates:** remove
  `the_default_file_is_named_for_the_project` in `src/logging.rs:359`,
  `tools/capture/src/logging/mod.rs:257`, `tools/sync/src/logging.rs:233`.

The policy's shape is already covered by ~23 unit tests in
`crates/log/src/{plan,filter,level}.rs`. Expected ~18–20 tests removed,
**medium risk**.

## Implementation plan

Code anchors from the report (validate all before editing):

* `tests/cli_contract.rs:981–1331`, `:2130`, `:1936–2065`.
* `tools/capture/tests/cli_contract.rs:529/565/586/610/698/635/665`;
  `tools/capture/src/logging/mod.rs:257`.
* `tools/sync/tests/output.rs:632/670/697/728/548/483/517/438`;
  `tools/sync/src/logging.rs:233`.
* `src/logging.rs:359`; `crates/log/src/{plan,filter,level}.rs` (policy coverage).

Possible commits (large task — multiple commits expected):

1. Table-drive girpr log-file process tests; remove the golden self-test.
2. Reduce gircap CLI log-file tests to one smoke; move env-prefix into its
   logging unit test; remove its unit duplicate.
3. Reduce girsync output.rs log-file tests to one smoke; fold/parametrize the
   named cases; move env-prefix into its logging unit test; remove its unit
   duplicate.
4. Remove the girpr logging unit duplicate.

## Acceptance criteria

* girpr retains an end-to-end log-file process set (4–5 cases); gircap and
  girsync each retain one smoke case plus an env-prefix unit assertion.
* The shared policy remains covered by `crates/log` unit tests; no CLI contract
  case (flag/env conflict, level, file-off) lost without a retained equivalent.
* All affected suites pass; no change to the log policy implementation.

## Guardrails

* **Count girpr's log-file tests once** (§A notes overlap with §B). This task
  owns all log-file-policy CLI tests; T04 (sync hash table) and T12 (capture
  `cli_contract.rs:214` + `closeout.rs:1000`) own their separate items — do not
  duplicate them here.
* Keep at least one end-to-end process case per binary; verify each removed case
  is covered by a `crates/log` unit test or a retained smoke before deleting.
* The env-prefix check moving into `logging` unit tests is test code; do not
  change the log policy implementation itself.
* Validate anchors first (line numbers are from the report snapshot). Report
  follow-up ideas without implementing them.
