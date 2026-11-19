# T16 — girpr retry/backoff → `backoff`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A2 (lines 70–108), girpr side
  (72–78, 91–94, 100, 104–108).
- Assumption to validate: §I.3 (lines 822–824).

## Scope
- Package: root `girpr`, `src/retry.rs` (`Retry`, `RetryState`, `retry`,
  `retry_state`, salt-based linear backoff, jitter) and call sites in
  `src/config.rs`, `src/hyp.rs`, `src/repair/file/mod.rs`, `src/sophon/stream.rs`,
  plus the `NoSleep` test constructors.

## Implementation plan
1. Verify the current maintained `backoff` release line before adopting (§A2
   caveat, §I.3).
2. Map `Retry::Retry` → `Transient`, `Retry::Fatal` → `Permanent`; replace
   `Arc<dyn Sleep>` with a configured `Backoff` (`ExponentialBackoff`,
   `randomization_factor`, `Clock`).
3. Because `backoff::future::retry` does not carry state forward, add a
   captured-mutation adapter for the `retry_state` shape.
4. Update all named call sites and test constructors.

## Constraints / validation
- girpr's backoff becomes exponential and jitter becomes RNG-based — the
  deterministic de-synchronization is lost (§A2 expected change). Confirm that
  is acceptable (§I.3).
- Tests pinning the hand-rolled math and jitter injectivity (§A2, lines 100–102)
   disappear with the code they test.

## Done when
The hand-rolled retry driver and jitter scheme are replaced and call sites
compile and pass tests.
