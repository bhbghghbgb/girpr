# T31 - D5: one per-range to per-chunk verdict remap

Report: `../../OPTIMIZATIONS_REPORT.md` lines 708-723 (D5).

Dependencies: T16 (scan reuse), T27 (scan keying). Adapt to the types those
tasks leave behind.

## Goal

The mapping "scan verdicts are per range; a chunk with no source has no intact
source" is duplicated in `repair::file::inplace::source_ok_by_chunk` and in
the scan's own estimate path. The mapping is small but subtle: a chunk with no
source must stay `false`, and the discard path substitutes an absent scan. Two
copies mean the next change to source collection has to find both.

Keep the helper in the scan/range module and let both call it. Do not force a
shared struct.

## Where to start

- `src/repair/file/inplace.rs`: `source_ok_by_chunk` (around line 367), its
  call (around line 143), its tests (around line 694).
- `src/repair/scan.rs`: `scan_ranges` (around line 290), `scan_one`
  (around line 323), the source_ok loop (around lines 363-368),
  `est_file_download_bytes` (around line 148).
- `src/util/md5.rs`: `Scan` (around line 117) and `Scan::absent`.

## Steps

1. Move `source_ok_by_chunk` into `src/repair/scan.rs` as
   `pub(crate) fn source_ok_by_chunk(chunks_len: usize, source_owner: &[usize],
   scan_source_ok: &[bool]) -> Vec<bool>`.
2. Use it in `scan_one` instead of the hand-written loop. The result is the
   per-chunk `Verdict.source_ok`, which is what the estimator consumes.
3. Use it in `inplace.rs` instead of the local copy. Remove the local function
   and update the tests to call the shared one (or keep a `#[cfg(test)]`
   wrapper only if the test paths need it).
4. Keep the invariant: source ranges are ordered by the chunk they belong to;
   `source_owner[k]` is that chunk; a chunk without a source stays `false`.
   Add a `debug_assert_eq!(source_owner.len(), scan_source_ok.len())` (or a
   short-circuit) so a future mismatch is visible in tests. `get(k)` with
   `unwrap_or(false)` remains the production-safe behavior.
5. The discard path still replaces the scan with `util::Scan::absent` and then
   remaps; do not special-case it in the helper.
6. The scan estimator uses per-chunk `source_ok` directly; keep that, but make
   sure it is produced by the shared helper, not a second loop. If T16 changed
   the scan verdict shape, adapt the helper signature to the new type rather
   than reintroducing a rel-keyed map.

## Tests

- A chunk list with sources for chunks 0 and 2 (not 1) maps
  `[true, false, true]` correctly; a source verdict shorter than
  `source_owner` yields `false`, not a panic.
- A no-source chunk stays `false` even if the scan's source list has a stray
  entry.
- The in-place and scan estimates agree for the same chunks/sources/verdict.
- The existing scan and in-place tests pass.
- `cargo test -p girpr` passes.

## Acceptance

- One remap implementation, used by both the scan and the in-place writer.
- A chunk with no source is always `false`.
- The discard/absent path is unchanged.
- `cargo test -p girpr` passes.

## Commit plan

1. `refactor(scan): share the per-range to per-chunk remap`
2. `test(scan): pin source ownership and the no-source case`

## Notes

- Keep the helper in the scan/range module; the two consumers use the ranges
  differently and a shared struct would over-couple them.
- Do not change `reuse::sources_for` or `salvage_order`; this is a remap only.
