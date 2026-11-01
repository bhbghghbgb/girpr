# T12 - P6: verify in place with one whole-file digest

Report: `../../OPTIMIZATIONS_REPORT.md` lines 410-434 (P6).

Dependencies: none.

## Goal

`repair::file::inplace` calls `util::scan_reader` after patching. `scan_reader`
always builds a digest context for the whole file **and** for every destination
range (and every source range). The first verification only needs the whole-file
MD5 to decide success; the per-range verdicts are consulted only when that MD5
fails, to name the chunks to redo. The final scan after a redo passes
`dest_ranges` but reads only `file_md5`, so its per-range digests are computed
and thrown away. That is roughly a second MD5 over the file's bytes on the
success path.

## Where to start

- `src/repair/file/inplace.rs`: `attempt_inplace_inner` (around line 69), the
  first verify around line 317, the redo around lines 325-347, `final_scan`
  around line 347, and `repaired_in_place` around line 384.
- `src/util/md5.rs`: `scan_reader` (around line 207), `md5_file_digest` (around
  line 345), `MD5_BUF`.
- Existing in-place tests at the bottom of `inplace.rs`; the `Reused`/`Redo`
  fixtures.

## Steps

1. Add a `pub(crate) fn md5_reader_digest(f: &mut File) -> Result<Md5>` to
   `src/util/md5.rs`. It seeks to the start, reads with the same `MD5_BUF`, and
   returns `Md5` directly. Share the buffer/loop with `md5_file_digest` if that
   is clean; do not duplicate the read policy.
2. In `attempt_inplace_inner`, replace the first post-write
   `scan_reader(&mut handle, &dest_ranges, &[])` with
   `md5_reader_digest(&mut handle)`. Compare it to the manifest MD5 with
   `Md5::parse_hex(&file.md5).is_ok_and(|e| e == after)`.
3. On success, return `repaired_in_place(stats, after)`; change the helper to
   take `Md5` (or keep `&str` and pass `after.to_hex()`; prefer `Md5` so the
   value is not re-parsed).
4. On mismatch, run `scan_reader(&mut handle, &dest_ranges, &[])` once to get
   the per-chunk verdicts needed to name `still_bad`. This is a second read,
   but only on the rare mismatch path; keep the diagnostic accurate.
5. After the redo, replace `final_scan`/`scan_reader` with `md5_reader_digest`
   and compare the digest. Do not compute per-range verdicts just to discard
   them.
6. Do not change the write order, the gap zero-fill, the set_len, or the
   salvage-order logic. This task is the verification read only.
7. If a caller elsewhere immediately re-parses a hex string returned by a scan,
   leave it for T31/D5 unless the change is local and obviously safe.

## Tests

- Add a unit test that `md5_reader_digest` equals `md5_file_digest` for an
  empty file, a small file, and a file larger than `MD5_BUF`; also assert it
  seeks to the start when the handle is at EOF.
- Keep the existing in-place success and mismatch tests. Add a mismatch test if
  one is missing: a corrupted byte outside the patched chunks must still name
  the right still-bad chunks after the extra read.
- Do not assert the number of reads in a timing test; if a read counter seam
  exists, assert the success path does not call `scan_reader` (a test-only
  counter is acceptable if local).

## Acceptance

- The success path computes only the whole-file MD5 for verification.
- The final redo verification computes only the whole-file MD5.
- A mismatch still reports the correct still-bad chunks.
- `cargo test -p girpr` passes.

## Commit plan

1. `perf(md5): add a whole-file digest over an open handle`
2. `perf(inplace): verify with one digest and scan ranges only on mismatch`
3. `test(inplace): pin success digest and mismatch chunk reporting`

## Notes

- The read still happens once on success; only the hashing CPU is removed. Do
  not claim an I/O win in comments.
- `scan_reader` remains the right tool for the initial scan and for the
  mismatch diagnosis; do not delete or weaken it.
