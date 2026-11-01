# P2-02 — Reconcile `budget_cost` semantics

Report: `../../OPTIMIZATIONS_REPORT.md` lines 412-429 (B2), 876-895 (Phase 2).

Dependencies: P0-02, P2-01.

## Goal

Make `--max-download-bytes` admission match its name for `added` paths. Today
`SizedCandidate::budget_cost` charges `self.size` for both `added` and
`removed`; for `added`, `PathRow.size` is the declared uncompressed size, while
the documented rule and download semantics are compressed bytes.

## Decision

Charge `added` its **compressed** size: the sum of its chunks'
`compressed_size` (the bytes that must be downloaded). Charge `removed` the old
size (the bytes it occupies in `game/`), as documented. Charge a common path
its existing `total_cost`.

This changes admission under a byte ceiling for `added` paths; that is the
intended correction. Do not rename the flag.

## Where to start

- `src/select/mod.rs`: `SizedCandidate` (around line 76), `budget_cost`
  (around line 99), `candidates` (around line 393).
- `src/manifest/diff.rs`: `PathRow` (around line 125), where `size` is set in
  `diff_field`; `SophonChunkFile.chunks[].compressed_size`.
- `src/blobs/chunks.rs` / `src/assemble` for the existing compressed-size
  conventions.
- `docs/01-artifact-schema.md` / `docs/00-spec.md` where the budget is
  described; update the prose, not the flag.

## Steps

1. Add a `download_bytes` (or `compressed_size`) field to `PathRow`:
   - for an `added` entry, saturating sum of `chunk.compressed_size.max(0)` over
     its chunks;
   - for a common entry, `None` or the sum, whichever is cheap (the score's
     `total_cost` is already the budget cost for common paths);
   - for a removed entry, `None` (its budget cost is the old `size`).
2. Carry that into `SizedCandidate` (or compute it in `candidates` from the
   row) and use it in `budget_cost` for `State::Added`.
3. Keep `budget_cost` documented with the exact rule: common -> `total_cost`,
   added -> compressed bytes downloaded, removed -> bytes removed from `game/`.
4. Handle a missing/negative compressed size with the existing saturating
   convention; a zero-chunk added file costs 0.
5. Update any record that exposes the budget charge (selection rejected rows,
   matrix, candidates document) so the number shown is the number charged.
6. Do not change `total_cost` for common paths or the `removed` charge.

## Tests

- Unit: an `added` path with chunks totaling 100 compressed bytes and a
  declared size of 1000 has `budget_cost() == 100`.
- Unit: a `removed` path still charges its old size.
- Unit: a common path still charges `total_cost`.
- Selection: under a ceiling that admits the compressed cost but not the
  uncompressed size, an `added` path is admitted; reverse the numbers and it is
  rejected.
- Existing budget tests updated only where the old expectation encoded the
  uncompressed charge.

## Acceptance

- The budget charge matches the documented download semantics.
- `removed` and common behavior are unchanged.
- The selection documents show the charged number.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(select): charge an added path its compressed download size`
2. `test(select): cover compressed and uncompressed added-path costs`
3. `docs(select): state the budget charge for each class`

## Notes

- This is artifact-visible admission; do not bundle it with comparator changes.
- If a manifest entry has no chunks, decide explicitly between 0 and the
  declared size; the download cost is 0 and that is the defensible choice.
- Do not reference the report or plan in code comments.
