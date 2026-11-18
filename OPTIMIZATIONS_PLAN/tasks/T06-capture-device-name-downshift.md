# T06 — Capture: device-name flag refusal → config unit test

Priority P0 · `tools/capture` (gircap) · checkpoint CP2 · report finding B2

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L236–L251` (§B2).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T06-capture-device-name-downshift.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Downshift a mock-and-process test to a config unit test.

`tests/capture_layout.rs:1739–1760`
(`a_device_name_from_a_flag_is_still_refused`) starts
`Mock::start(World::game_only())` and spawns
`gircap capture --files GenshinImpact_Data/Resources/CON` — but `--files` values
are rejected in `config/mod.rs:348` via `validate_paths` →
`paths::path_rule(..., allow_device = false)` (`paths.rs:67–140`) **before any
request is made**; the mock is never contacted. The rule is already unit-tested
at `src/paths.rs:383`, and `--files` refusal at `src/config/tests.rs:240–257`.

## Implementation plan

Code anchors (validate before editing):
`tools/capture/tests/capture_layout.rs:1739–1760`,
`tools/capture/src/config/mod.rs:348`,
`tools/capture/src/paths.rs:67–140, :383`,
`tools/capture/src/config/tests.rs:240–257`.

Steps (one commit):

1. Add the path `GenshinImpact_Data/Resources/CON` to the `bad[]` list in
   `config/tests.rs:242–249`.
2. Delete the integration test.

## Acceptance criteria

* The device-name refusal is still asserted, now at the unit level; −1 test,
  −1 process, −1 mock server.
* The contrasting `a_device_name_manifest_path_is_kept_and_escaped_on_disk`
  (`capture_layout.rs:1630`) is kept — it genuinely needs the full round-trip.
* Affected suites pass. No production behavior change.

## Guardrails

* Confirm at implementation time that the path is rejected before any HTTP
  request (report's premise); if the current code contacts the mock first,
  stop and report `skipped` rather than deleting a meaningful e2e test.
* Test-only change. Report follow-up ideas without implementing them.
