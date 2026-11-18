# T20 — `repair_scan.rs` estimate/mode downshift + merge

Priority P2 · root product integration tests · checkpoint CP10 · report finding A2

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L96–L118` (§A2).
* Report synthesis: `docs/09-test-scale-reduction-research.md:L790–L795`
  (pattern 1), `L818–L820` (Wave 3).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T20-repair-scan-downshift.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Remove ~12–15 process spawns and mock servers by running estimate/mode cases
in-process, and merge 3 tests → 2.

`tests/repair_scan.rs` :194–259, :262–356, :359–429, :512–580, :583–652,
:655–760 (~14 binary spawns across six tests) spawn the binary and parse stdout
with `field(...)`, though the asserted values (`RunExit`,
`summary.download.settled()`, `plan_download_bytes`, the `--plan-out` JSON written
by the library, chunk hit sets) are all obtainable from `common::run(ctx)` plus
`summary_record`. The comments at :225–229 and :394–397 already justify avoiding
a separate `--print-plan` spawn — the same reasoning applies one level down.

## Implementation plan

Code anchors (validate before editing): `tests/repair_scan.rs:194–259`,
`:262–356`, `:359–429`, `:512–580`, `:583–652`, `:655–760`,
`tests/common/mod.rs` (`ctx_for`, `run`, `field`, `summary_record`), and the
library harness introduced by T09 (reuse `Rig::run_lib` pattern if available).

Steps (one or two commits):

1. Convert read-only/estimate cases to the library harness.
2. Merge the two `scanned estimate` tests into one table over
   `{inplace, temp}` with a shared estimate ↔ document ↔ real-download parity
   assertion.
3. Keep one binary run per test only where the assertion is genuinely
   process-level: stdout negatives (`no PLAN`, `no CHECK`) and the stderr retry
   wording at :728–744.

## Acceptance criteria

* Exit-code gradations, disk immutability, plan-out content and fetch sets
  unchanged; the process-only stdout/stderr pins stay on the binary.
* ~12–15 fewer spawns and mock servers; 3 tests → 2.
* Affected suite passes. No production behavior change.

## Guardrails

* Do not downshift the stdout-negative or stderr-retry assertions — they are
  process-level by design. Validate anchors; skip a case that asserts a binary
  fact. Report follow-up ideas without implementing them.
