# T13 — Add a gircap zero-backoff retry hook

Priority P1 · crate gircap · checkpoint CP3

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L351–L357` (§6.2 4).
* Report priority row: `docs/08-test-runtime-optimization.md:L475` (§8, P1).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T13-gircap-retry-backoff-hook.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

`capture_layout.rs`'s `a_retried_request_is_recorded_as_retried` arms a 503 and the
child uses `RealSleep` with `RetryPolicy.base_delay = 500 ms`, so the test pays a
real 500 ms backoff to prove a retry is recorded. Add a test-only zero-backoff
hook mirroring the `RecordingSleep` already used by the HTTP unit tests, so the
retry is still exercised with no wall-clock delay.

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/capture/tests/capture_layout.rs:1194–1266` — the retry test (report
  cited `1198–1266`).
* `tools/capture/src/http/transport.rs:7–37` — `RetryPolicy`, `base_delay =
  500 ms` at `:21`; `:79` `RealSleep`; `:32` already handles zero delay.
* `tools/capture/src/http/tests.rs:960+` — `RecordingSleep` pattern to mirror.
* `tools/capture/src/http/fetcher.rs` — where the sleeper is chosen
  (`with_sleeper` around `:506`).

Possible commits:

1. Add a test-only way for the spawned capture child to select a zero-delay
   sleeper (env hook or CLI flag consistent with the crate's test hooks), default
   off in production.
2. Set that hook in the retry test; keep the 503/retry assertion and the request
   log check intact.

## Acceptance criteria

* The retry test still proves the 503 was retried and recorded, with ~0 ms
  backoff.
* Production retry backoff remains 500 ms; the hook is test-only and default-off.
* No other tests that depend on real backoff timing are affected.

## Guardrails

* Validate against current code first; if the retry test no longer uses a real
  backoff, stop and mark `skipped` with a note.
* Product code is involved — add only a test-only, default-off hook; do not
  change the production `RetryPolicy` default.
* Do not "fix" retry/backoff in unrelated normal tests (report §9 says those
  comments describe production and never cost test time). Run the gircap
  `capture_layout` suite; report follow-up ideas without implementing them.
