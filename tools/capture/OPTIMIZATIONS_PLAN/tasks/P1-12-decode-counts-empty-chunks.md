# P1-12 — `decode_counts` reports empty-chunk size mismatches

Report: `../../OPTIMIZATIONS_REPORT.md` lines 431-444 (B3).

Dependencies: P0-02.

## Goal

A non-folder entry whose `chunks` list is empty but whose declared `size` is
nonzero must count as a `size_mismatch_files` violation. Today the
`e.is_folder || e.chunks.is_empty()` guard suppresses it.

## Where to start

- `src/manifest/mod.rs`: `decode_counts` (around line 666), the guard near the
  end of the per-entry loop, and the comment that explains it.
- `src/manifest/tests.rs` for the counter tests.

## Steps

1. Change the guard to skip byte-level checks only for `e.is_folder`. For a
   non-folder with no chunks, run the existing checks:
   - `ascending` and `gapless` remain true (no chunks to violate them);
   - `chunk_bytes` is `0`, so `size != 0` increments `size_mismatch_files`.
2. Decide whether to also increment `non_gapless_files` for this shape. The
   report allows either, but says reporting both is consistent with the
   "independent violations are separate" rule. Prefer only `size_mismatch_files`
   unless an existing test or doc says an empty non-folder is non-gapless; a
   file with no chunks has no tiling to violate. Record the choice in the
   comment.
3. Update the comment so it no longer says `chunks.is_empty()` is justified by
   folders.
4. Do not change the folder path, totals, or `files`/`folders` counts.

## Tests

- Non-folder, `size > 0`, `chunks = []` -> `size_mismatch_files == 1`.
- Non-folder, `size == 0`, `chunks = []` -> no size mismatch (a legitimate
  empty file).
- Folder with `size > 0` and no chunks -> still no byte-level violation.
- Existing gapless/ascending tests unchanged.
- Flip the P0-02 characterization test if one was added.

## Acceptance

- `size_mismatch_files` means what its name says.
- Only malformed manifests newly report a violation.
- Zero-byte empty files remain clean.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(manifest): report a size mismatch for empty-chunk non-folders`
2. `test(manifest): cover empty files with and without a declared size`

## Notes

- Do not fold this into the folder policy task; the guards are related but the
  decisions are different.
- Do not reference the report or plan in code comments.
