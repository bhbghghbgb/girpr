# T15 — Parameterize the sync heartbeat period

Priority P1 · `tools/sync` (girsync) / production code · checkpoint CP6 ·
report finding E4

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L721–L739` (§E4).
* Report synthesis: `docs/09-test-scale-reduction-research.md:L797–L799`
  (pattern 2), `L815–L816` (Wave 2).
* Report note on dependency: `docs/09-test-scale-reduction-research.md:L821–L824`
  (E2 / T17 rayon caveat).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T15-sync-heartbeat-period.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Remove ~100 file creates and one spawn by making the heartbeat period injectable
and asserting the progress counters directly.

`tests/output.rs:703–902` uses fixture :710–719
(`TEST_HEARTBEAT_ENTRIES = 101`, a loop creating 100 files + one excluded) and
the child-process-only env seam at :739–742 (`GIRSYNC_TEST_HEARTBEAT_ENTRIES`).
The comment at :704–709 admits the number comes from a phase-A progress period
of 100, not from the phenomenon under test. Production `src/scan.rs:21` has
`HEARTBEAT_ENTRIES = 2000` and pure `heartbeat_entries_from` at :31–47.

## Implementation plan

Code anchors (validate before editing): `tests/output.rs:703–902`, `:710–719`,
`:739–742`, `src/scan.rs:21`, `:31–47` (`heartbeat_entries_from`), and
`scan_stat_only` / `open_folder_cache` / `ScanMode`.

Steps (one or two commits):

1. Add `walk_live_with(root, max_depth, heartbeat_entries)` and thread a period
   into `scan_stat_only`/`open_folder_cache` via `ScanMode`, defaulting to
   `HEARTBEAT_ENTRIES`.
2. Expose the two progress counters on a library result instead of only log
   events.
3. Rewrite the test as an integration test over 2–3 files with period 2,
   asserting the same heartbeat/phase fields directly.

## Acceptance criteria

* ~100 file creates removed, one spawn removed, ~10 log-parse assertions replaced
  with direct values.
* The same heartbeat/phase fields are asserted (no coverage loss).
* Affected suites pass; production behavior with the default period unchanged.

## Guardrails

* If the counters must stay log-only, this task alone cannot remove the spawn —
  either coordinate with T17 (E2 scoped subscriber) or stop and report the
  limitation rather than weakening the test.
* The env seam must keep working as a fallback for `.config/nextest-test-env`
  where applicable. Report follow-up ideas without implementing them.
