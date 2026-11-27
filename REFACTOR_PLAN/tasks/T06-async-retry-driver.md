# T06 — Generic async retry driver

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Share the attempt loop, first/last-attempt wait rule, injected sleep,
fatal-stops-immediately behavior and last-error reporting between fetch and
capture, in a policy-free driver above `girpr-retry`.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:472-479` — APP-13, two async retry drivers.
- `docs/09-shared-code-refactor-findings.md:897-898` — non-findings note: retry
  drivers stay local because `crates/retry/src/lib.rs:4-8` leaves classification
  and side effects to callers.

## Boundary / approach

Add a generic driver in a leaf that depends on `girpr-retry`, parameterized by
schedule, sleep, and carried work, returning the last error to the caller. Do
not move classification into `girpr-retry` itself.

## Guardrails

- Jitter choice (`app/fetch/src/retry.rs` vs `tools/capture/http/src/transport.rs`),
  per-attempt journal logging and typed failures stay local.
- Every HTTP attempt in both products is affected: verify attempt counts and
  fatal paths explicitly.
- Do not change retry status semantics from T05.

## Commit breakdown

1. Add the driver leaf with injected schedule/sleep and unit tests.
2. Adopt `app/fetch/src/retry.rs`.
3. Adopt `tools/capture/http/src/fetcher.rs`.

## Validation

- Attempt-count and fatal-stop tests in both products.
- `cargo nextest` for fetch and capture-http; recorded request goldens unchanged.

## Done when

Both drivers delegate the loop mechanics to one implementation while keeping
their own jitter, logging and error types.
