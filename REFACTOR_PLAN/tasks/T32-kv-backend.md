# T32 — Pluggable KV backend with in-memory mirror

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F37 (lines 528–535) — the in-memory
  cache backend blocks a generic backend cut; candidate boundary is a pluggable KV backend
  with an in-memory mirror and capture. **High-risk** because a shared global registry must
  preserve "path-keyed, per-process, forgettable" semantics; generalizing key or lifetime
  reintroduces cross-test leakage. Confidence medium.
- Assumptions to validate: §A10 (98–105), §A5 (57–67), §A2 (29–36), §A14 (129–135), §N15 (758–761).

## Scope
- `tools/sync/src/effective/side.rs` (`TestCacheBackend` resolved in `CommonOpts::try_from`,
  path-keyed/shared), `crates/sync-cache/src/lib.rs` (backend registry),
  `crates/sync-test-support/src/lib.rs` (process-global row capture). Locate before editing.

## Implementation plan
1. Validate §F37: confirm the backend is resolved once, path-keyed/shared, and row capture
   is process-global.
2. If attempted, define a pluggable backend trait with an in-memory mirror and capture while
   keeping "path-keyed, per-process, forgettable" semantics (`forget_memory_cache`,
   `with_observed_rows`).
3. Migrate callers directly (§A2); move through §A14 seams where relevant.
4. If a shared registry would leak across tests, stop — nextest isolation is why it is
   process-local (§A10).

## Constraints / validation
- High-risk (§F37): generalizing key/lifetime reintroduces cross-test leakage nextest
   isolation prevents (§A10).
- Do not reopen cache codec/schema/publisher or `FileRec.kind` (N15).
- No byte/contract drift (§A5).

## Done when
A pluggable backend is shared with path-keyed/per-process/forgettable semantics intact, or
a validated skip with the one-line reason is recorded.
