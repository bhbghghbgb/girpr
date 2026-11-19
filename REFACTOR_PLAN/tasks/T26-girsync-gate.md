# T26 — girsync gate/concurrency simplification

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §E3 (lines 569–583).
- Assumption to validate: §I.8 (lines 837–839).

## Scope
- Package: `tools/sync`, `src/rw/gate.rs` (`Gate`, `RwLimiter`, `RwPermit`),
  `src/rw/mod.rs` (custom rayon `spawn_handler`).

## Implementation plan
1. Preferred: remove the `Gate` and bound concurrency with the rayon pool,
   which already sizes to `limits.pool_size()`; keep two pools only for
   `RwLimits::Split` (§E3 note). This is the largest simplification.
2. If a gate is genuinely still required, use `tokio::sync::Semaphore` or a small
   semaphore crate — `std::sync::Semaphore` is still unstable (§I.8).

## Constraints / validation
- Preserve the two-gate `acquire_copy` ordering (`gate.rs` lines 126–146) for
   deadlock freedom; any replacement supports `try_acquire` and returns permits
   on panic.
- Keep `in_flight()` for diagnostics.
- Tests named in §E3 (`rw/gate.rs`, `tests/rw_threads/*`, limit-resolution
   integration tests).

## Done when
The hand-rolled gate is removed or replaced and the concurrency/limit tests
pass.
