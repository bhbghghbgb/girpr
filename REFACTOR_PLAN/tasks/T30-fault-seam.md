# T30 — Generic injected-fault seam

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F35 (lines 510–518) — embedded test
  seams block extracting apply/copy and planner subsystems; candidate boundary is a generic
  injected-fault seam crate. **High-risk** because hook fields name girsync paths/stages and
  their env spellings are pinned by `.config/nextest-test-env`. Confidence medium.
- Assumptions to validate: §A10 (98–105), §A2 (29–36), §A14 (129–135).

## Scope
- `tools/sync/src/config/common.rs` (`TestHooks`), `apply/copy.rs` (`flush_file`),
  `tools/sync/src/scan.rs` (heartbeat), `crates/sync-cache/src/write.rs` (commit policy),
  `sync-cache/src/open.rs` (lock-failure injection, `#[cfg(test)]` re-exports). Locate before editing.

## Implementation plan
1. Validate §F35: confirm the typed fault hooks, env reads and `#[cfg(test)] pub(super)` seams.
2. If attempted, define a generic injected-fault seam while keeping hook field names and env
   spellings wired to production (`fail_rename_step1/step2`, `gate_copy`, `heartbeat_entries`).
3. Migrate callers directly (§A2); move through §A14 seams where relevant.
4. Generic names that disconnect tests from production are a blocker — stop in that case.

## Constraints / validation
- High-risk (§F35): generic names would silently disconnect tests from production; env
   spellings pinned by `.config/nextest-test-env` (§A10). Do not rename hooks/env vars.
- Do not absorb the in-memory backend (T32) or the test-support split (T27).
- No byte/contract drift (§A5).

## Done when
A generic seam is shared with all field/env spellings intact, or a validated skip with the
one-line reason is recorded.
