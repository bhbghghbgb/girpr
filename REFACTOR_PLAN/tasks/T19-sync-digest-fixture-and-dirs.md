# T19 — Shrink the sync-cache digest fixture and add `dirs()`

## Goal / pointers
- `docs/10-test-optimization-research.md` §F32 (lines 413–418) — one `sync-cache` unit
  test builds 300,000 bytes and calls `by_byte.update` 900,000 times in an unoptimized
  profile; the comment only requires the body to be under `CHUNK` (512 KiB) and to cross
  digest block boundaries. Fix: reduce to 4–8 KB. Benefit Med (one test).
- `docs/10-test-optimization-research.md` §F33 (lines 420–424) — ~50 repeated
  `temp_root`/`src`/`dst` sequences in `sync.rs`; add a thin `dirs(tag)` helper where
  semantics permit. Benefit Low (source/compile only).
- Assumptions to validate: §A5 (75–80), §A10 (108–110), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- `crates/sync-cache/src/hash/tests.rs` (the byte-at-a-time test), `tools/sync/tests/sync.rs`
  (the repeated setup sequences) and the existing `pair` helper in sync-test-support
  as precedent. Locate sites in the current tree before editing.

## Implementation plan
1. Validate §F32: confirm the body size requirement from the test's comment and the
   digest block size; choose 4–8 KB that still crosses block boundaries. Preserve the
   test's assertion about the streaming API.
2. Validate §F33: identify sequences where all three temp paths can be produced by a
   `dirs(tag)` helper without changing mtime pinning or one-sided shapes; leave
   heterogeneous cases alone.
3. Add the helper and replace only semantically identical sequences.
4. Update nextest filters if a named case moves (§A10).

## Constraints / validation
- §A5 says fixtures are already tiny globally, but §F32 is a specific exception — a
  CPU loop, not a byte-scale claim. Do not resize other fixtures.
- Preserve the digest test's purpose: crossing block boundaries and staying under
  `CHUNK`. A smaller body that no longer crosses a boundary is wrong.
- The `dirs()` helper is source hygiene only; do not claim wall savings. No sleeps (§A6).

## Done when
The byte-at-a-time test uses a 4–8 KB body that still crosses digest block boundaries
with its assertion intact, `dirs()` is applied to homogeneous setup sequences, and the
sync suites pass.
