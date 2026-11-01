# P1-16 — Blob diagnostic fixes

Report: `../../OPTIMIZATIONS_REPORT.md` lines 314-325 (A15) and 327-340 (A16).

Dependencies: none.

## Goal

Two small correctness/diagnostic fixes in `blobs::props`:

1. `why_unusable` must name a missing `stored_path` before falling through to
   the "verified" message.
2. `measure_file` must fill the four-byte magic with `read_upto`/`read_exact`
   semantics, not a single `read`, so a short read is not misclassified as
   "not a zstd frame".

## Where to start

- `src/blobs/props.rs`: `why_unusable` (around line 583), `measure_file`
  (around line 338), `not_a_frame`, `is_usable`, tests.
- `src/hash/mod.rs`: `read_upto` (around line 588, currently private),
  `is_zstd` (around line 325), `zstd_measure_from`.
- `src/blobs/tests.rs` for `why_unusable` tests.

## Steps

1. Make `hash::read_upto` `pub(crate)` (or add a small local equivalent). Use it
   in `measure_file` to fill `head`. It returns the number of bytes read and
   only returns `Ok(0)` at EOF.
2. If fewer than four bytes arrive, treat it as the malformed case: call
   `not_a_frame` with the actual bytes seen so the diagnostic says how many.
   Do not call `is_zstd` on a short slice and silently report "not a frame"
   without the count.
3. A full four-byte read that is not zstd keeps the existing undecodable row
   path.
4. In `why_unusable`, add an arm/check for `stored_path.is_none()` before the
   final verified arm. A row with all four checks green but no path should say
   `"chunk <id>: no stored blob"` (or the existing no-path wording used by
   `check_blobs`) rather than rendering `Verified` as if it were fine.
5. Keep `is_usable` unchanged: it still requires `stored_path`.
6. Do not change `measure` (the in-memory path); it already receives a full
   slice.

## Tests

- `why_unusable`: a `BlobProps` with a green `verified`, no errors, and
  `stored_path: None` returns a no-path message, not the verified text.
- `measure_file`: a file shorter than four bytes returns an undecodable row
  whose message includes the byte count (or the existing `not_a_frame` text
  with the count).
- `measure_file`: a valid zstd frame with a `BufReader` still decodes
  (existing tests).
- A short-read test can use a custom reader only if `measure_file` is
  generalized; otherwise a 0-3 byte file is enough to exercise EOF.

## Acceptance

- The diagnostic names the actual defect.
- A valid frame is not turned into `undecodable` by a short read.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(blobs): name a missing stored path in why_unusable`
2. `fix(blobs): fill the zstd magic with a short-read-safe read`
3. `test(blobs): cover the pathless reason and a truncated magic`

## Notes

- Keep the `UnexpectedEof` diagnostic informative; do not collapse it into a
  generic I/O error.
- Do not reference the report or plan in code comments.
