# P1-14 — Reserve the error-body budget before reading

Report: `../../OPTIMIZATIONS_REPORT.md` lines 278-294 (A13).

Dependencies: P0-02.

## Goal

Bound peak memory when many error responses arrive concurrently. Today each
request reads `error_bodies.remaining()` and buffers up to that amount; all
concurrent requests can observe the same remainder, so peak is roughly
`jobs × min(max_response_bytes, remaining)`.

## Where to start

- `src/http/error_bodies.rs`: `ErrorBodies`, `State { used, rows }`,
  `remaining`, `push`, `snapshot`, tests.
- `src/http/fetcher.rs`: `drain_error_body` (around line 238), the error path
  around lines 904-973 that calls `remaining()` and then `push`.
- `src/http/tests.rs` for fetcher-level error-body tests.

## Design

Add a reservation API:

- `ErrorBodies::reserve(max: u64) -> Reservation` locks `state`, computes
  `room = budget - used`, grants `min(room, max)`, adds `granted` to `used`
  immediately, and returns a token carrying `granted`.
- The request reads at most `reservation.granted()` bytes.
- `ErrorBodies::push_reserved(row, reservation)` truncates the body to the
  granted amount if needed, refunds unused granted bytes (`used -= granted -
  retained`), and pushes the row. A body larger than granted is marked
  `truncated`.
- Keep `push(row)` as a convenience that reserves `row.body.len()` and calls
  `push_reserved`, so existing tests and callers keep their behavior.
- The total retained is still bounded, and the sum of concurrent grants is at
  most the budget.

## Steps

1. Implement `Reservation` (private to the module or `pub(crate)`) with
   `granted()` and a `Drop` that refunds any unspent bytes if the caller never
   pushes. Drop-refund makes the reservation panic/early-return safe.
2. Implement `reserve` and `push_reserved`. Make `push` delegate.
3. In the fetcher, reserve before `drain_error_body`:
   `let reservation = self.error_bodies.reserve(self.max_response_bytes);`
   then pass `reservation.granted().min(cap)` as the retain limit. After the
   read, call `push_reserved(row, reservation)`.
4. If `drain_error_body` can return early on an I/O error, ensure the
   reservation is still consumed or dropped so the budget is not leaked.
5. Keep `remaining()` public if used elsewhere, but document that it is
   advisory and that readers should reserve. Prefer migrating the fetcher away
   from `remaining()`.
6. Do not change the document shape or the final `total_retained_bytes`
   semantics for a sequential run.

## Tests

- Unit: two concurrent reservations of a 10-byte budget with a 6-byte max grant
  6 and 4; pushing 5 and 5 retains 5 and 4 (second truncated), total 9.
- Unit: an unused reservation refunds its grant on drop.
- Unit: `push` still truncates exactly as before.
- Unit: a body smaller than its grant refunds the difference.
- Sequential behavior tests in `error_bodies.rs` still pass.

## Acceptance

- Peak memory for error bodies is bounded by the run budget, not by
  `jobs × remaining`.
- Final rows and totals are unchanged when responses arrive sequentially.
- Under concurrency, a later row may be truncated more than before; that is the
  intended tradeoff and must still be marked `truncated`.
- `cargo test -p gircap` passes.

## Commit plan

1. `feat(http): reserve error-body budget before reading a body`
2. `test(http): cover concurrent reservations and refunds`

## Notes

- Do not hold the mutex while reading the network body.
- Do not reduce `--max-response-bytes`; the reservation is the fix.
- Do not reference the report or plan in code comments.
