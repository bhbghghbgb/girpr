# T14 — Group `artifact_server.rs` bundle tests

## Goal / pointers
- `docs/10-test-optimization-research.md` §F21 (lines 320–325) — eight tests each build
  a mini-bundle and start four listeners (32 listeners plus real HTTP GETs). Fix: group
  by bundle shape into ~4 tests, keeping one full TCP end-to-end case — the TCP path is
  the contract under test. Do not expect cross-test fixture sharing (§A1).
- Assumptions to validate: §A1 (46–52), §A5 (75–80), §A10 (108–110), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- `tests/artifact_server.rs` (`MiniBundle::build`, `Bridge::start` sites, the eight
  tests) and any doc that counts them. Locate current sites before editing.

## Implementation plan
1. Validate §F21: map the eight tests to bundle shapes and identify which one provides
   the full TCP end-to-end coverage that must be kept.
2. Group tests sharing a bundle shape into single tests with labelled subcase blocks,
   starting one bundle/bridge per group rather than per test.
3. Keep one full TCP case unchanged; do not replace it with a mock shortcut.
4. Preserve every assertion, including HTTP status/body checks and byte contracts (§6).
5. Update nextest filters if a named case moves (§A10).

## Constraints / validation
- The TCP path is the contract under test — one full end-to-end case stays (F21).
- No cross-test fixture sharing (§A1); a `OnceLock` bundle would be useless across
  nextest tests and could introduce ordering dependencies.
- Do not grow fixtures (§A5) or add sleeps (§A6). Keep host-dependent skips intact.

## Done when
Bundle shapes are grouped into ~4 tests with one full TCP case retained, listeners and
bundle builds per run are reduced, assertions are unchanged, and the suite passes.
