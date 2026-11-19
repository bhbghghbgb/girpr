# T17 — gircap retry/backoff → `ExponentialBackoff`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A2 (lines 70–108), gircap
  side (74–76, 94–98, 102, 106), §F4 (lines 658–663).
- Assumption to validate: §I.3 (lines 822–824).

## Scope
- Package: `tools/capture`, `src/http/transport.rs` (`RetryPolicy::delay_before`,
  `should_retry_status`, `Sleep`) and `src/http/fetcher.rs` (retry loop).

## Implementation plan
1. Replace `RetryPolicy::delay_before` with `ExponentialBackoff` configured
   `jitter: None`; keep the retry loop and the `Sleep` trait injection.
2. Keep per-attempt `seq` logging, `bytes_in` accounting on failed bodies, and
   `HashingFile::restart` — a full `reqwest-middleware`/`reqwest-retry` adoption
   cannot express these, so do not switch frameworks (§A2 restructure).

## Constraints / validation
- `jitter: None` is mandatory: `api/requests.ndjson` is intentionally diffable
  (§A2 expected change).
- Tests named in §A2 (lines 101–102): backoff-cap and retryable-status tests.

## Done when
The clamped exponential schedule is delegated, no-jitter behavior is preserved,
and the named tests pass.
