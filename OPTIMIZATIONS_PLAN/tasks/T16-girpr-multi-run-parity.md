# T16 — Consolidate girpr multi-run parity tests

Priority P1 · crate girpr · checkpoint CP4

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L396–L400` (§6.3 3).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T16-girpr-multi-run-parity.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

`tests/repair_scan.rs` spawns 2–4 binaries per test to compare modes, overlapping
with parity checks in `repair_machinery.rs` and `repair_writer.rs`. Keep one
comprehensive parity site and make the others single-mode, asserting `--plan-out`
JSON from stdout where it is already available. This cuts binary spawns while
keeping each mode-vs-mode parity assertion in exactly one place.

## Implementation plan

Code anchors from the report (validate before editing):

* `tests/repair_scan.rs:228–744` — multi-spawn parity tests.
* Overlaps: `repair_machinery.rs:791/1145`, `repair_writer.rs:143/309/354`.
* `--plan-out` JSON from stdout around `repair_scan.rs:312–332`.

Possible commits:

1. Map which parity relations are asserted where; choose one comprehensive site
   per relation (prefer one that already captures `--plan-out`).
2. Convert the duplicate sites to single-mode tests, or remove the redundant
   assertions, without dropping a parity relation.
3. Ensure the kept site still compares all intended modes and their outputs.

## Acceptance criteria

* Each mode-vs-mode parity relation is asserted in exactly one place.
* The number of binary spawns across these files drops.
* No parity coverage is lost; kept assertions are as strong or stronger.

## Guardrails

* Validate against current code first; if the overlap no longer exists, stop and
  mark `skipped` with a note.
* This is a consolidation, not a behavior change — do not alter repair modes or
  product code. Be conservative: if two tests only look similar but assert
  different relations, keep both and note it.
* Run the girpr `repair_scan`, `repair_machinery` and `repair_writer` suites;
  report follow-up ideas without implementing them.
