# T17 — Retry sleep hook → async-trait

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §LEAF-6 (lines 481–494) — the
  `Pin<Box<dyn Future + Send + 'static>>` plumbing for the injectable sleep hook exists
  only because a dyn-compatible trait cannot use `async fn`; `#[async_trait]` removes
  the hand-written boxing.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A5 (26–28). Keep items:
  `guidance/DO-NOT-TOUCH.md` — `backoff 0.4.0` stays the schedule engine (line 493).

## Scope
- `crates/retry/src/lib.rs` — `SleepFuture`, `Sleep`, `RealSleep`, `ZeroSleep`,
  `RecordingSleep` (`:106–181`), the `Box::pin` sites, and the tests at `:190–258` and
  `:260–276`. `crates/retry/Cargo.toml`.

## Implementation plan
1. Validate how callers use `Sleep`: whether `RecordingSleep` is expected to record at
   *call* time or first poll. If call-time recording is load-bearing, account for the
   macro's poll-time behavior or stop (§LEAF-6 lines 492–493).
2. Change the trait to `#[async_trait] pub trait Sleep { async fn sleep(&self, d: Duration); }`,
   delete `SleepFuture` and every manual `Box::pin`. Add `async-trait` (not currently
   locked).
3. Keep `backoff` as the schedule engine — do not adopt `backoff::future::retry`, which
   would absorb caller-owned retry policy.
4. Keep `schedule_contracts` and `zero_and_recording_sleepers_return_immediately`,
   adapting only timing-sensitive assertions if the macro changes poll behavior.

## Constraints / validation
- The macro still boxes internally; the win is removing hand-written type plumbing, not
  allocation changes. Do not claim a performance improvement.
- `girpr-retry` is shared; all consumers must still build.

## Done when
The boxed-future alias and manual `Box::pin` sites are gone, the retry tests pass, and
backoff still owns the schedule math. If a premise is stale or a keeper would change,
**stop** and record `skipped`.
