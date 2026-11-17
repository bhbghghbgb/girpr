# T09 — Consolidate `run_lock.rs` case clusters

Priority P1 · root product integration tests · checkpoint CP4

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-suite-reduction-audit.md:L101–L106` (§B.2).
* Report context on where product lock behavior is actually covered:
  `docs/08-test-suite-reduction-audit.md:L79–L83` (§B intro).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T09-run-lock-clusters.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

`tests/run_lock.rs` (15 tests) guards the test-support helper in
`tests/common/run_lock.rs`, not product lock behavior (which is covered in
`repair_offline.rs:348/408/441`). Consolidate the case clusters:

* `game_path_arg` (108, 131, 156, 175, 203, 238) → 1
* mode/args mirrors (63, 86) → 1
* `lock_key_path` (356, 364) → 1
* cleanup (376, 390, 403) → 1 (**keep one end-to-end cleanup case**)
* CLI bad-game-path (319, 346) → 1

15 → ~5, ~9–10 saved, **medium risk**.

## Implementation plan

Code anchors from the report (validate before editing):

* `tests/run_lock.rs:63/86`, `:108–238`, `:319/346`, `:356/364`, `:376–403`.
* `tests/common/run_lock.rs` — the test-support helper under test.

Possible commits:

1. Merge the `game_path_arg` and mode/args clusters into tables.
2. Merge `lock_key_path`, cleanup and CLI bad-game-path clusters, keeping one
   end-to-end cleanup test.

## Acceptance criteria

* All argument/lock-key/cleanup dimensions still asserted.
* One end-to-end cleanup case retained.
* `run_lock` suite passes; product lock code untouched.

## Guardrails

* Validate first: confirm the file still guards a test-support helper; if it now
  covers product lock behavior, stop and mark `skipped`.
* Keep one end-to-end cleanup case — do not reduce to pure tables.
* Test-only change. Report follow-up ideas without implementing them.
