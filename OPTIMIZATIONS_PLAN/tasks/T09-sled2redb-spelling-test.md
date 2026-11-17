# T09 — Cut the sled2redb argument-spelling test from 8 conversions to 2–3

Priority P1 · crate sled2redb · checkpoint CP3

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L420–L425` (§6.4 1).
* Report priority row: `docs/08-test-runtime-optimization.md:L474` (§8, P1).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T09-sled2redb-spelling-test.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

`the_flags_are_accepted_on_either_side_of_the_paths` is the single worst test in
the suite (7.25 s): it runs 8 full converter subprocesses (6 argument spellings
plus 2 equality checks), each a full sled→redb conversion. Reduce to 2–3
representative spellings (flag-before / flag-after / `--output=json`), or test
clap's parsing in-process and keep one full conversion. Expected ~7 s → ~2 s.
The thing under test is argument *acceptance*, not conversion correctness —
that is covered by the rest of the suite.

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/sled2redb/tests/convert.rs:494–573` — the test; eight `run_converter`
  calls around `:534–571`; `--output=json` spellings at `:537–538`, `:570`.
* `tools/sled2redb/src/main.rs:9` — the documented "flags before/after paths,
  `--output=json` == `--output json`" contract.

Possible commits:

1. Reduce the `run_converter` invocations to a representative subset covering
   flag-before, flag-after and the equals form; keep the equality assertion.
2. Optionally move pure clap spellings to an in-process parse assertion, leaving
   one full conversion subprocess to prove end-to-end acceptance.

## Acceptance criteria

* The documented flag-position and `--output=json` contract is still covered.
* The number of full conversions in this test drops to ~2–3; expected runtime
  falls accordingly.
* No conversion-correctness coverage is lost (it lives in other tests).

## Guardrails

* Validate against current code first; if the test no longer runs 8 conversions,
  stop and mark `skipped` with a note.
* Do not weaken the CLI contract — keep at least the before/after and equals
  forms.
* Test-only change (unless an in-process parse test requires exposing a parser —
  validate before any product change). Run the sled2redb `convert` suite; report
  follow-up ideas without implementing them.
