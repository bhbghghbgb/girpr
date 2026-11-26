# T02 — Repair futures combinators → futures-util

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §R5 (lines 198–210) — the
  "settle primary, retain sibling" `tokio::select!` loop is `future::select` / `Either`.
- §R7 (lines 229–242) — the hand-rolled `OrderedResults` buffer over
  `buffer_unordered` is `futures-util`'s `FuturesOrdered`.
- Validate assumptions first: §A1 (15–16), §A4 (22–23), §A5 (26–28). Keep items:
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/repair/phases/metadata.rs` — `Settled`/`wait` (`:163–179`), `settle_primary`
  (`:190–210`), call sites `:261`, `:399`, and the three named tests (`:650`, `:692`, `:726`).
- `src/repair/phases/manifest.rs` — `OrderedResults` (`:100–144`),
  `ManifestFetches`/`ManifestStream`/`from_stream`/`next_ordered` (`:154–232`) and
  the tests at `:447`, `:478`, `:605`, `:874`.
- Root `Cargo.toml` (futures-util is already a direct dependency with `std`).

## Implementation plan
1. Validate both premises in the current code: confirm the "never cancel the primary"
   rule and that `OrderedResults` exists solely to restore selection order and report
   the first failure.
2. Replace the select loop with `future::select`; `Either::Right` must still await the
   primary to completion — keep the sibling-cancellation and sibling-error behavior.
   `Settled` may remain or become `Either`; boxing the futures is acceptable.
3. Replace `OrderedResults` with `FuturesOrdered` while keeping results in selection
   order and cancelling later fetches after the first failure. Because
   `FuturesOrdered` does not bound concurrency, acquire the existing `ManifestBudget`
   semaphore **inside** each future so peak memory does not regress. The `from_stream`
   test seam changes type; adapt it.
4. Run the five named tests, adapting the reorder-mechanics test (`:447`) and keeping
   the failure-ordering and reuse-map assertions.

## Constraints / validation
- The never-cancel-primary rule is load-bearing (§R5 lines 205–206). Sibling errors kept
  when the primary succeeds, and authoritative config failures, must survive.
- Bounded concurrency must be preserved explicitly (§R7 lines 241–242); do not rely on
  `FuturesOrdered` for it.
- Tests of intended function stay; only the mechanics-pinning test goes (§A5, 26–28).

## Done when
Both combinators use futures-util, the first-failure-in-selection-order contract and
bounded concurrency hold, and the adapted tests pass. If either premise is stale or a
keeper would change, **stop** and record `skipped` with a one-line reason.
