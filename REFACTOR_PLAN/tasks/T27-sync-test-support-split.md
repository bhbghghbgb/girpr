# T27 — Split sync-test-support fixtures into a generic runner

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F32 (lines 479–487) — the relocated
  inversion is still a high-risk cut; candidate boundary is to split fixtures into a generic
  CLI-runner/option-builder crate parameterized by product types so neither test crate needs
  a normal edge on a program. Confidence high, risk high.
- Inversion context: §F2 (156–163). Assumptions: §A10 (98–105), §A4 (48–56), §A14 (129–135).

## Scope
- `crates/sync-test-support/src/lib.rs` (builds girsync `CommonOpts`, `CacheDb`, `HashPlan`,
  `Report`, in-process runners; seams call `girsync::run_captured_with_hooks` and
  `subscriber_for` directly). Locate before editing.

## Implementation plan
1. Validate §F32/§F2: confirm the test-only-inversion-as-normal-dependency and the direct
   seams; confirm this is a test-graph change, not a mechanical extraction.
2. If attempted, split a generic CLI-runner/option-builder layer parameterized by product
   types; keep env pinning (`GIRSYNC_TEST_CACHE=file/memory`) and nextest isolation exactly.
3. Migrate callers directly (§A2); move through §A14 seams.
4. Given high risk, a validated skip is legitimate if env pinning/isolation cannot be
   reproduced. Do not break the existing in-process suites.

## Constraints / validation
- High-risk (§F32): env pinning and nextest process isolation must survive or every
   in-process suite reverts to spawning children (§A10, §F2).
- gircap independence (§A4) and doc seams (§A14) still apply if the generic crate is shared.
- Do not absorb the in-memory cache backend (T32) or embedded fault seams (T30).

## Done when
The generic runner is split with env pinning and isolation intact, or a validated skip with
the one-line reason is recorded.
