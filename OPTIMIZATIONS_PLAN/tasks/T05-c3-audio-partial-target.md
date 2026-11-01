# T05 - C3: never publish a partial audio target

Report: `../../OPTIMIZATIONS_REPORT.md` lines 121-142 (C3).

Dependencies: none.

## Goal

`phases::post::move_audio_cache` tries `rename` first and falls back to
`std::fs::copy` + `remove_file(src)`. A copy that fails partway leaves a
truncated target, keeps the source, and only warns. The post-phase then bumps
`config.ini`, so the launcher is told the install is current while an audio
pack may be truncated.

Make the copy fallback publish only a complete file: copy to a temp sibling in
the destination directory, rename the temp over the target, then remove the
source. On any failure clean up the temp, keep the source, and report the
existing `kept` outcome. Do not claim `moved` for a partial target.

## Where to start

- `src/repair/phases/post.rs`: `move_audio_cache` (around line 279); the
  `rename`/`copy`/`remove_file` block around lines 358-405.
- The `moved`/`kept` summary line at the end of the function.
- Existing audio-move tests in the same file (the failed-copy test around line
  445); keep their intent and add the partial-copy case.

## Steps

1. Extract a helper that performs the copy fallback for one `(src, target)`:
   - create the destination parent (the caller already does this; keep it);
   - compute a temp path beside `target` (append a `.girpr_tmp` suffix to the
     whole file name; do not use `with_extension`, which drops the real
     extension);
   - copy `src` to the temp path;
   - rename the temp path to `target`;
   - on any error, remove the temp path if it exists, leave `target` as it was,
     and return the error;
   - only after the rename succeeds does the caller remove `src`.
2. Keep the existing `rename(src, target)` attempt first. It is the cheap
   same-volume path and is already atomic.
3. In the copy fallback, count `moved` only after the source is removed.
   If the target was published but the source removal fails, count `kept` and
   keep the existing "copied, but the source could not be removed" warning;
   the target is complete, so do not delete it.
4. Do not change the function's return type or make a failed move fatal. The
   report accepts a best-effort outcome; the important property is that a
   truncated file is never published.
5. Do not touch `config.ini` bump ordering in this task. A kept source is a
   best-effort miss, not a reason to skip the post-phase; the bytes premise is
   preserved because no partial target exists.

## Tests

- Add a unit test for the copy helper with an injected copy operation that
  writes a partial temp file and then returns an error. Assert:
  - the target does not exist (or is the original file, if one existed);
  - the temp sibling does not exist;
  - the source still exists.
  A small generic helper `fn copy_publish_with(src, target, copy: impl
  Fn(&Path, &Path) -> io::Result<u64>)` is enough; production passes
  `std::fs::copy`.
- Keep the existing test that a failed copy keeps the source.
- Add a successful-copy test asserting the target bytes equal the source bytes
  and the source is gone (the existing tests may already cover this; extend,
  do not duplicate).

## Acceptance

- A failed copy cannot leave a partial target.
- A failed copy leaves no temp file behind.
- The source is removed only after a complete target exists.
- `moved` and `kept` still describe the real outcome.
- `cargo test -p girpr` passes.

## Commit plan

1. `fix(post): publish audio cache copies through a temp sibling`
2. `test(post): pin partial-copy cleanup and source retention`

## Notes

- The temp sibling lives in the destination directory so the rename is on one
  volume. Do not copy to the system temp directory.
- This is a cold path; one extra path manipulation is acceptable. Do not add a
  retry loop around the copy.
