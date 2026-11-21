# T20 — Merge pure gircap test clusters

## Goal / pointers
- `docs/10-test-optimization-research.md` §F35 (lines 435–443) — gircap's lib suite has
  641 tests / 91.2 cumulative s (~0.14 s each, mostly process spawns). Named clusters:
  `tools/capture/tests/measured_fixture.rs` (29 tests each rebuilding the same fixture),
  `hash/src/tests.rs` (23), `src/config/tests.rs` (39), `src/select/tests.rs` (39).
  Fix: consolidate pure-function groups into single tests with labelled assertion
  blocks, accepting coarser granularity; the `capture_layout.rs` missing-document trio
  is one candidate table. Measure nextest per-test overhead before a broad rewrite.
- Assumptions to validate: §A1 (46–52), §A10 (108–110), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (merge caveats, capture independence).

## Scope
- The four named clusters plus the `capture_layout.rs` trio; locate their case tables
  in the current tree before editing. Check `.config/nextest.toml` for named paths.

## Implementation plan
1. Validate §F35: measure nextest per-test overhead for gircap before a broad rewrite;
   if a merge saves no wall time, skip the broad part.
2. For each validated cluster, consolidate pure cases into one test with labelled
   assertion blocks that preserve every original message and result-byte check.
3. Convert the missing-document trio into a single table-driven test if the shapes are
   genuinely identical.
4. Update nextest filters if a named test path changes (§A10). Deliver in commits per
   cluster, largest last.

## Constraints / validation
- Merging coarsens failure granularity (§A1) — keep labelled blocks and messages.
- gircap independence: do not make its tests depend on the product's fixtures or binary
  (§5, `DO-NOT-TOUCH.md`).
- Do not change bundle/report/dump bytes or pinned blobs (§6, §A5); do not add sleeps
  (§A6). Keep host-dependent skips and named tests intact (§A10).

## Done when
Validated pure clusters are merged with labelled assertions preserved, nextest filters
match, gircap stays independent, and the capture suite passes with fewer spawns.
