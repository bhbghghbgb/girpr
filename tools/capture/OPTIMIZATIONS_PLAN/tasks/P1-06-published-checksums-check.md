# P1-06 — Independently check the published checksums

Report: `../../OPTIMIZATIONS_REPORT.md` lines 189-205 (A8) and 481-503 (C1
context).

Dependencies: P0-01.

## Goal

Ensure the `verify/checksums.json` that is finally published is the document
that was checked. Today A16 validates the first checksums document, then a
second document is written by `write_checksums_reusing` and published without
another A16 pass.

The task must preserve the cost saving: do not simply run a second full `a16`
over the whole bundle. Use a scoped check that is exact given the first pass.

## Where to start

- `src/emit/closeout.rs`: `close` (around line 127), especially the second
  `write_checksums_reusing` and the A16 row construction.
- `src/emit/mod.rs`: `write_checksums_reusing` (around line 256),
  `write_manifest_files`.
- `src/emit/index.rs`: `Checksums::scan_reusing`, `a16`, `findings`, `walk`.
- `src/emit/closeout.rs` tests, including the existing
  `a16_is_recorded_exactly_once_despite_two_passes`.

## Design

A scoped A16 for the second document has two halves:

1. **Membership:** walk the tree and compare it to the second document's
   entries, excluding `verify/checksums.json` itself. This catches files that
   appeared or vanished between passes.
2. **Rewritten entries:** re-read and re-digest exactly the paths named in the
   second `write_checksums_reusing` call (`verify/selfcheck.json`, `README.md`)
   and compare to their entries.

Every other entry was re-hashed by the first A16 and was not written between
the passes, so it is covered transitively. State that scope in the A16 detail
text; do not claim a full re-hash.

## Steps

1. Add a `Checksums` method (for example `a16_scoped`) that takes the root and
   the list of rewritten paths, performs the two halves above, and returns
   `(bool, String)` like `a16`. Keep `a16` for callers that need the full
   independent check (`verify`).
2. Ensure the membership check uses the same exclusion (`checksums.json`) and
   sorted walk as `findings`, and that it also verifies `file_count` and
   `total_bytes` against the entries if those are cheap to check from the
   document alone.
3. In `close`, keep the first full `a16` on the first document. After the
   second `write_checksums_reusing`, call `a16_scoped` on the returned
   `checksums` with the rewritten list.
4. Build the A16 row from the scoped result, not the first result. If the
   scoped check fails, the selfcheck must record it and the run must fail
   (exit 3) as A16 failures already do.
5. Handle the failure write order carefully. The selfcheck is written before
   the second checksums document. If the scoped check fails, either rewrite the
   selfcheck with the failing A16 row and recompute/publish checksums once
   more, or ensure the already-published selfcheck already records the failure.
   Do not leave a published selfcheck claiming A16 passed when the scoped check
   failed. A second rewrite in the failure path is acceptable; the success path
   must not gain a third full pass.
6. Keep the "two passes converge" property and its test. Update the test to
   assert the published document is the checked one.

## Tests

- Success path: A16 row is still recorded exactly once and says the published
  document was checked; `a16_scoped` passes.
- Failure path: make one rewritten file differ from the reused entry (inject
  via a test seam or call `a16_scoped` directly on a crafted document) and
  assert it fails, naming the path. If an end-to-end seam is unavailable, a
  unit test on `a16_scoped` plus a close-out test that the row uses its result
  is enough.
- Membership failure: an extra file on disk after the known list is reported.
- Existing A16 tests continue to pass.

## Acceptance

- The published `checksums.json` is independently checked before the run can
  claim A16 passed.
- The success path performs no second full-tree re-hash.
- A late discrepancy produces a failing A16 row and exit 3, not a silent pass.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(closeout): scope A16 to the published checksums document`
2. `test(closeout): cover the published-document check and its failure path`

## Notes

- Do not add a second full `a16` call in the success path; the point is to keep
  the cost saving while making the claim exact.
- If the failure path proves to require a third pass, stop and return the
  design tradeoff rather than silently accepting an unchecked published
  document.
- Do not reference the report or plan in code comments.
