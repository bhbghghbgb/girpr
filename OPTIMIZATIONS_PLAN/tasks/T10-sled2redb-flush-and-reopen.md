# T10 — Drop sled2redb's read-only flush and redundant cache reopens

Priority P1 · crate sled2redb · checkpoint CP3

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L426–L433` (§6.4 2 and 3).
* Report priority row: `docs/08-test-runtime-optimization.md:L474` (§8, P1).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T10-sled2redb-flush-and-reopen.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

Two small wins: (1) `convert_into` calls `old.flush()` after a **read-only**
iteration — the fsync buys nothing because the lock is released on `drop` — so
remove it (product code, validate first). (2) Several tests reopen the converted
cache via `open_db(&dst, true, rw())` (~200 ms disk open each) just to inspect
rows; assert the round-trip once in a dedicated test and elsewhere read the
just-written rows through the converter's own handle or an in-memory read.

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/sled2redb/src/convert.rs:228` — `old.flush().ok();` (PRODUCT CODE).
* `tools/sled2redb/tests/convert.rs:209`, `:356`, `:402` —
  `open_db(&dst, true, rw())` reopens.

Possible commits:

1. Remove the redundant `old.flush()` in `convert.rs` after confirming the
   iteration is read-only and nothing depends on a flushed sled state. (If the
   assumption does not hold, leave it and note why — do not force it.)
2. Keep one dedicated round-trip/reopen test; convert the other reopen sites to
   inspect rows without reopening the file-backed cache (converter's handle or
   in-memory read).

## Acceptance criteria

* The product `flush()` is removed only if validation confirms it is a no-op
  for correctness; sled2redb tests still pass.
* Only one test pays for a file-backed reopen of the converted cache; the
  round-trip contract remains asserted.

## Guardrails

* **Product code is involved** in the first part — validate the read-only
  assumption before editing; if uncertain, skip that part with a note and still
  do the test-side reopen reductions.
* Validate against current code first; if these sites have changed, mark
  `skipped` with a note.
* Do not change the converter's output format or durability semantics. Run the
  sled2redb suite; report follow-up ideas without implementing them.
