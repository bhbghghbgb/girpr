# T13 - P7: reuse the temp source handle and track the temp length

Report: `../../OPTIMIZATIONS_REPORT.md` lines 436-457 (P7).

Dependencies: none.

## Goal

On the temp path, every chunk calls `out.metadata()` to read the resumed length,
and every reuse candidate opens `File::open(src)`. A file with thousands of
chunks pays thousands of stats and opens. The temp length only grows
monotonically within an attempt, and the source is the same file for every
chunk, so track the length and keep one lazy source handle per file.

## Where to start

- `src/repair/file/temp.rs`: `repair_attempt_inner` (around line 87), the
  per-chunk loop around lines 123-154, `try_reuse_slice` (around line 239),
  `rollback_tmp` (around line 190).
- `src/repair/file/inplace.rs` also uses `reuse::sources_for`, but this task is
  only the temp path.
- Existing temp tests: `rollback_tmp` and the streamed-chunk tests; add a
  reuse-focused test if one is missing.

## Steps

1. In `repair_attempt_inner`, read the temp length once after opening and
   truncating to the target size. Keep it in `let mut tmp_len: u64`. Log the
   resume point from this value, not a second `metadata()`.
2. Change `try_reuse_slice` to take `tmp_len: &mut u64` instead of statting
   `out` per chunk. Its resume decision uses `*tmp_len >= end`. The caller
   updates `tmp_len` only after a successful write:
   - `Resumed`: no change (the prefix is already at least `end`);
   - `Reused`: `*tmp_len = (*tmp_len).max(end)`;
   - `Download` success: `*tmp_len = (*tmp_len).max(end)`;
   - a failed download with a successful rollback: set `*tmp_len = c.offset`
     (the rollback truncated to that point);
   - a failed rollback: return the error; no later chunk uses the counter.
3. Add a lazy source handle, for example `let mut src_handle: Option<File> =
   None;`, passed as `&mut Option<File>` to `try_reuse_slice`. On the first
   reuse, open `src`; on subsequent chunks, reuse the handle and seek to
   `entry.offset` before the slice digest. Put the handle back after a
   successful read/copy.
4. If opening or reading the source fails, set the handle to `None` and fall
   back to `Download` for that chunk. The caller's download re-seeks to
   `c.offset` and overwrites any partial copy, which is the existing contract.
   Do not fail the file because a reuse source vanished.
5. Keep a fresh-open fallback if a held handle becomes unusable: after a read
   error, the next chunk may try `File::open(src)` once more. Do not add an
   unbounded retry loop.
6. Do not change `rollback_tmp`'s on-disk behavior. It may keep its own
   metadata call, because it is the failure path and it must observe the real
   file length. Update the in-memory `tmp_len` at the call site as above.
7. The write/settle accounting is unchanged: each copied piece still calls
   `reserve_write` and `note_write`.

## Tests

- A reuse-heavy temp repair (multiple chunks with valid local slices) produces
  the same bytes and `ChunkStats` as before. The existing end-to-end repair
  fixtures may cover this; if not, add a small one.
- A source deleted after the handle is opened (or replaced) falls back to
  download and still completes; on Unix the open handle may keep reading, so
  assert only the final bytes, not which path won.
- A failed chunk rollback sets the resume point back to the chunk offset; the
  next attempt does not treat the failed chunk as complete. Extend the existing
  rollback test with the counter update if it is cheap.
- Do not assert syscall counts; the gain is a code-review property.

## Acceptance

- The temp loop stats the temp once per attempt, not once per chunk.
- A reuse source is opened at most once per file while the handle stays valid.
- The resumed-prefix decision and rollback behavior are unchanged.
- `cargo test -p girpr` passes.

## Commit plan

1. `perf(temp): track the temp length instead of statting per chunk`
2. `perf(temp): reuse the source handle across chunks`
3. `test(temp): pin reuse fallback and rollback resume points`

## Notes

- The temp mode resumes by length; a wrong counter is a data-integrity bug.
  Update it only after the corresponding write/rollback actually happened.
- T35 later turns this parameter bag into a struct; keep the behavior change
  and the signature cleanup in separate tasks.
