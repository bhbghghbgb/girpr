# T02 — Fetch and decode chunks within one file in a bounded window

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §2 **S-03** (lines 143–157) — chunks write to disjoint
  offsets but are fetched, decoded and written one at a time; `--jobs` only
  parallelizes across files. For one large file with many small chunks this prevents
  network/decompression overlap — important for a low-disk patcher.
- §11 **assumption 2** (lines 841–842) — chunk ranges must be disjoint for the tiers
  where concurrent fetch is proposed; overlapping/salvage layouts must stay ordered,
  and the write-ceiling ledger must accept out-of-order settlement.
- Design note at `file/mod.rs:56–57`; consumers in `inplace.rs:498–531` and
  `temp.rs:126–174`.
- Keep items: §12 (lines 890–905), deterministic output where required,
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/repair/file/mod.rs` (design note ~56–57, bounded-window orchestration).
- `src/repair/file/inplace.rs` (~498–531), `src/repair/file/temp.rs` (~126–174).
- The write-ceiling reservation/settlement code those paths use.
- Locate current structures before editing; the list above is a hint.

## Implementation plan
1. Validate assumption 2: confirm which layouts have disjoint chunk ranges and
   whether the ceiling ledger tolerates out-of-order settlement. If not, limit the
   change to the tiers that are provably independent, or stop.
2. For non-overlapping layouts and the staged/discard tiers, fetch and zstd-decode a
   bounded window with `buffer_unordered`, writing at precomputed offsets via
   positional I/O.
3. Keep the strict ordered tier wherever salvage dependencies require it; keep the
   per-chunk write-ceiling reservations exact.
4. Deliver in several commits if useful: window plumbing, per-tier switch, tests.

## Constraints / validation
- Chunk writes must target disjoint ranges; never overlap concurrent writes.
- Overlapping/salvage layouts stay strictly ordered.
- The ceiling ledger must remain exact under out-of-order settlement; add a test if
  none exists.
- Do not change the repair result bytes or the data-plane output.

## Done when
Independent chunks within a file are fetched/decoded in a bounded overlapping
window, ordered tiers and the ceiling ledger remain correct, and tests pass. If
disjointness or ledger tolerance cannot be confirmed, **stop** and record `skipped`
with a one-line reason.
