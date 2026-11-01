# T25 - P13: fast-path the unlimited meters

Report: `../../OPTIMIZATIONS_REPORT.md` lines 572-591 (P13).

Dependencies: T03 (meter underflow hardening). Build on the hardened
subtraction; do not undo it.

## Goal

`Meter::charge_live` enters a compare-exchange loop even when the limit is the
`u64::MAX` sentinel, and `settle` CASes to report "within limit". These run per
reserved chunk and per settled write, and the write meter is touched per copied
piece, so unlimited runs pay contention for an answer that is always "yes".

Add a branch to `fetch_add`/`fetch_add + true` when the ceiling is the
sentinel. No behavioural change.

## Where to start

- `src/repair/scope/ledger.rs`: `Meter` (around line 122), `charge_live`
  (around line 196), `refund_live`/`adjust_live` (around lines 220-237),
  `settle` (around line 250), the tests.
- `src/report/summary.rs` / `src/repair/scope/ledger.rs`: where the limits are
  installed before tasks start.

## Steps

1. In `charge_live`, load the limit once. If it is `u64::MAX`, do
   `self.live.fetch_add(n, Ordering::Relaxed)` and return `true`. Use a
   saturating add if the counter could overflow without a ceiling; the
   `AtomicU64::fetch_add` wraps, so either keep the saturating behavior with a
   CAS only in the overflow case, or document that a run cannot reach
   `u64::MAX` bytes. Prefer an explicit saturating helper: load, if
   `cur.checked_add(n)` is `None`, fall back to the CAS with
   `saturating_add`; otherwise `fetch_add(n)` is safe.
2. In `settle`, load the limit. If sentinel, do `self.settled.fetch_add(n)`
   (same overflow reasoning) and return `true`; otherwise keep the existing
   CAS loop that both charges and checks atomically.
3. Keep the ceiling path exactly as it is: all-or-nothing, boundary inclusive,
   one CAS for charge+check. Do not split the limited `settle` into
   `fetch_add` then compare; that is the race the CAS prevents.
4. Make the sentinel check race-safe: the limits are installed once before
   tasks start, so a limit cannot appear mid-run. Document that assumption
   next to the fast path. Do not add a lock.
5. Do not touch `refund_live`/`adjust_live`'s hardening except to share a
   saturating helper if that keeps the code smaller.

## Tests

- Unlimited meter: many concurrent `charge_live` calls all return true and the
  live figure is the exact sum. The existing unlimited test can be extended
  with threads.
- Unlimited `settle`: always true, settled figure exact.
- Limited meter: all existing boundary/all-or-nothing/concurrency tests still
  pass; the fast path must not be taken when a limit is installed.
- A meter with a limit installed after construction (the only supported order
  is before tasks, but the setter allows it) must not skip the check.
- `cargo test -p girpr` passes.

## Acceptance

- The no-ceiling `charge_live` and `settle` paths take no CAS loop.
- Limited behaviour, boundary, and concurrency are unchanged.
- The overflow/sentinel assumptions are documented.
- `cargo test -p girpr` passes.

## Commit plan

1. `perf(ledger): fast-path the unlimited charge and settle`
2. `test(ledger): pin unlimited concurrency and the sentinel branch`

## Notes

- `u64::MAX` is a sentinel, not a real byte ceiling. Do not change the public
  `Option<u64>` API.
- If an unlimited run can actually reach `u64::MAX` bytes, keep the saturating
  fallback; a wrapped meter is worse than a slow one.
