# T06 — sled2redb removes + usage-error parametrization

Priority P0 · crate sled2redb · checkpoint CP3

## Pointers (give these to the implementing session)

* Report findings: `docs/08-test-suite-reduction-audit.md:L208–L209` (§F.1),
  `L210–L214` (§F.2), `L215–L217` (§F.3).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T06-sled2redb-removes.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

Three low / low–medium risk reductions in `tools/sled2redb`:

1. **REMOVE:** `tests/convert.rs:677`
   `the_help_exits_zero_and_documents_the_flags`, subsumed by the strict pin
   `the_help_output_is_pinned` (805). **1 saved.**
2. **PARAMETRIZE:** usage-error cases `a_usage_error_exits_three` (613),
   `a_missing_positional_exits_three` (653),
   `an_unknown_output_format_exits_three_and_names_the_accepted_values` (668),
   optionally `a_version_request_is_refused...` (633) → one parametrized test.
   **2–3 saved.**
3. **REMOVE:** `tests/convert.rs:201` temp-name test, strictly weaker than
   `src/convert.rs:410` (which checks retry spellings n=1..3 and `is_cache_rel`).
   **1 saved.**

Expected ~4–5 tests removed.

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/sled2redb/tests/convert.rs:677`, `:805`, `:613/653/668` (`:633`).
* `tools/sled2redb/tests/convert.rs:201`, `tools/sled2redb/src/convert.rs:410`.

Possible commits:

1. Remove the two subsumed tests after confirming their stronger counterparts.
2. Parametrize the usage-error cases, keeping each case's exit-code/name
   assertions as rows.

## Acceptance criteria

* The strict help output pin (805) and the retry-spelling/`is_cache_rel` test
  (src/convert.rs:410) remain.
* Each usage-error case still asserted; convert suite passes.

## Guardrails

* Keep at least one end-to-end help and usage-error path; do not remove the
  strict pins.
* The redundant full conversions at `convert.rs:515` save runtime only — leave
  them for follow-up F03, not this task.
* Test-only change; validate anchors before editing. Report follow-up ideas
  without implementing them.
