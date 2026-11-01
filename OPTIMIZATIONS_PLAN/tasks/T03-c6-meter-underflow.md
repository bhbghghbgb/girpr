# T03 - C6: harden the byte meters against underflow

Report: `../../OPTIMIZATIONS_REPORT.md` lines 185-203 (C6).

Dependencies: none.

## Goal

`scope::ledger::Meter` uses unchecked `fetch_sub` for `refund_live`, and
`adjust_live` can reach it with an arbitrary negative delta. A future caller
that refunds twice, or refunds a reservation that was refused, wraps the live
figure near `u64::MAX` and disables the ceiling for the rest of the run.

Make the invariant explicit in the code: a refund never takes the live figure
below zero. Either saturate and warn, or assert in debug builds; the report
allows either. Choose saturation plus a warning so a release run cannot panic
and the logic error is still visible.

## Where to start

- `src/repair/scope/ledger.rs`: `Meter::charge_live`, `refund_live`,
  `adjust_live`, `settle`; the `Meter` tests at the bottom.
- Callers: `Summary::reserve_download_bytes`, `adjust_download_bytes`,
  `reserve_write_bytes`, `note_write_bytes`; `file::temp` and `file::inplace`
  reserve/settle sites. Do not change their accounting policy.

## Steps

1. Implement `refund_live` with a compare/update loop (or
   `AtomicU64::fetch_update`) that computes `cur.saturating_sub(n)`. If the
   previous value was smaller than `n`, emit one `warn!` naming the attempted
   refund and the current value, then store zero (the saturating result).
   Avoid a `debug_assert!` that panics in tests unless the task also makes the
   test expect the panic; a warning keeps the common failure observable.
2. Do the same for the positive branch of `adjust_live` if `fetch_add` could
   overflow: use a saturating add. `adjust_live`'s negative branch must call
   the hardened `refund_live`.
3. Leave `charge_live`'s all-or-nothing compare-exchange alone. It already
   refuses a charge that would cross the ceiling and uses `saturating_add`.
4. `settle` already uses `saturating_add`; do not change its return contract
   here. The unlimited fast path is T25's job.
5. Keep the atomic ordering `Relaxed`; these are counters, not
   synchronization.

## Tests

- Add a meter test: charge 100, refund 30 twice, refund 1000; the live figure
  ends at 0, never a huge number. The warning need not be asserted unless the
  existing `test_support::logs_of` helper makes it trivial.
- Add an `adjust_live` test with a negative delta larger than the live figure;
  it saturates at 0.
- Keep the existing all-or-nothing and concurrent-reservation tests passing.

## Acceptance

- No refund path can wrap the live figure.
- A refused reservation that is refunded anyway cannot disable the ceiling.
- Normal accounting is byte-identical to today.
- `cargo test -p girpr` passes.

## Commit plan

1. `fix(ledger): saturate refunds and report an underflow`
2. `test(ledger): pin refund saturation and adjustment bounds`

## Notes

- Do not "fix" this by changing callers to refund less. The point is that the
  invariant must live in the meter, where the arithmetic is.
- The live figure is deliberately monotone except for a refund of a resource
  that delivered nothing; do not add refunds for consumed-but-failed bytes.
