# T10 — Retry policy/schedule leaf (`girpr-retry`)

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F15 (lines 241–249) — two attempt
  loops plus two zero-wait test modes.
- Assumptions to validate: §A6 (52–60), §A2 (21–27), §A1 (13–19).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (§A6 jitter divergence).

## Scope
- Candidate shared crate `crates/retry` (`girpr-retry`).
- Report names: `src/retry.rs` (`BackoffPolicy` real/none, ±50 % jitter, 30 s
  cap, `Retry`/`RetryState`, manual attempt cap),
  `tools/capture/src/http/transport.rs` (`RetryPolicy`, `Sleep` /
  `RealSleep` / `ZeroSleep` / `RecordingSleep`), `tools/capture/src/http/fetcher.rs`
  (hand-rolled attempt loop). Locate before editing.
- Both already use `backoff` 0.4 (product with `tokio`, capture without).

## Implementation plan
1. Validate §F15 and §A6: girpr's ±50 % jitter is intentional; gircap uses
   `randomization_factor = 0.0` for byte-stable artifacts. The shared leaf may
   own the policy/schedule factory and the sleep hook, **not** the choice of
   jitter.
2. Expose a schedule factory parameterised by base delay, factor, jitter and
   cap, plus a hook-based sleep driver (real / zero / recording) that both tools
   can inject.
3. Migrate the schedule construction and sleep driving in both tools. Keep the
   side-effectful gircap loop local — it logs every attempt into `RequestLog`
   and keeps error bodies (§F15). Keep girpr's `Retry`/`RetryState` transient/
   permanent mapping at its call site.
4. If gircap gains a new leaf dep, update §A2 doc tables.

## Constraints / validation
- Jitter must remain ±50 % for girpr and zero for gircap (§A6); artifact
  byte-stability must survive (§A3).
- Do not merge the two tools' logging/error-body policies into the leaf.
- Zero-wait test modes must keep working for both tools.

## Done when
Schedule construction and sleep driving come from the shared leaf, each tool
keeps its jitter and side-effect policies, and all retry/determinism tests pass.
