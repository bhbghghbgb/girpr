# T05 — Shared retryable-status predicate

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Replace the two "keep in sync" spellings of the retryable HTTP status rule with
one logic-free predicate in `girpr-retry`.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:116-124` — X4, duplicated rule.
- `docs/09-shared-code-refactor-findings.md:403-405` — APP-03, fetch side.
- `docs/09-shared-code-refactor-findings.md:748-750` — CAP-03, capture side.

## Boundary / approach

Add `should_retry_status(u16)` to `crates/retry` (classification of `408`,
`429`, `5xx`) and call it from `app/fetch/src/sophon/stream.rs` and
`tools/capture/http/src/transport.rs`.

## Guardrails

- Jitter, attempt counts, HYP `retcode` classification and per-attempt logging
  stay at call sites.
- Taking `u16` deliberately resolves the existing discrepancy on out-of-range
  values; do not silently widen it back to a numeric range.
- Retry counts are published in both tools; they must not change.

## Commit breakdown

1. Add predicate to `girpr-retry` with boundary tests (407/408/429/5xx/600).
2. Adopt in fetch's `retryable`.
3. Adopt in capture's `RetryPolicy::should_retry_status`; remove the stale
   "keep in sync" note.

## Validation

- `cargo nextest` for retry, fetch and capture-http.
- Compare published retry counts on recorded cases.

## Done when

Both programs call the shared predicate and no duplicated status rule remains.
