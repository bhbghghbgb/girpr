# T44 - T1: regression tests for the fixed failure paths

Report: `../../OPTIMIZATIONS_REPORT.md` lines 1082-1100 (T1), and the note on
tests at lines 1218-1226.

Dependencies: T01-T31 (the fixes these tests pin). If a fix was skipped, adapt
the test to the actual behavior and say so; do not test a change that was not
made.

## Goal

The suite is extensive on the happy and refusal paths, but the failure modes
this work fixed are exactly the ones a refactor can break silently. Add
regression tests for:

- a closed stdout pipe (C1);
- a partial audio-cache copy (C3);
- concurrent manifest failure ordering (P2);
- reuse of a stale scan verdict (P3);
- the lock's scope-independent acquisition and dead-owner recovery (C2/C9);
- dry-run post-phase output (A3) if not already pinned by T20.

## Where to start

- `tests/repair_offline.rs` (process-level fixtures; T47 will split it later,
  so add tests in the area they belong to, not a new monolith).
- `tests/artifact_server.rs`, `tests/artifact_bridge.rs` for HTTP/manifest
  ordering.
- `src/repair/phases/post.rs`, `src/repair/lock.rs`, `src/repair/scan.rs` unit
  modules for seams that are hard to drive end to end.
- `src/repair/file/temp.rs` / `post.rs` for the copy/rename seam.

## Steps

1. **Closed stdout (C1):** spawn `girpr` with a piped stdout, close the reader
   before/while the run emits, and assert:
   - the process exits with the run's documented code, not `101`;
   - no panic text on stderr;
   - the install state is what the run earned (use a tiny fixture).
   This is a process-level test; do not try to redirect the global stdout in a
   unit test.
2. **Partial audio copy (C3):** the post module needs an injectable copy or
   rename seam (the lock module already has one). Add the seam, then a test
   where `copy` writes a partial target and fails:
   - the partial target is removed or never published;
   - the source is kept;
   - `config.ini` is not bumped (or the outcome reports "not moved").
   If the seam is too invasive, test the helper that performs temp-copy+rename
   directly and record the gap.
3. **Concurrent manifest failure ordering (P2):** with a fixture server that
   fails one manifest and succeeds others, assert the run reports the error in
   selection order and does not silently drop a failure. Assert the final exit
   code and the `REPORT`/`SUMMARY` order, not the completion timing.
4. **Stale scan verdict (P3):** reuse T16's guard helper test, plus one
   integration case if reachable: plan with `--scan`, mutate the file, then
   repair; the result must match the mutated file, not the stale verdict.
5. **Lock (C2/C9):** a scoped run takes the lock (two scoped runs conflict); a
   dead owner is recovered per the chosen design. Use the lock path injection;
   do not sleep for a real stale window.
6. **Dry-run post (A3):** pin the "would …" lines and the no-`config.ini`-bump
   guarantee if T20 did not already add them.
7. Keep tests deterministic: `NoSleep`, temp directories, no wall-clock
   assertions, no reliance on completion order for correctness (only for the
   ordered report).

## Tests

- The new tests plus the full `cargo test -p girpr`.
- If a test cannot run on this host (for example a Windows-only broken pipe or
  a Unix-only symlink fixture), mark it with the repository's existing
  platform-gating pattern and say exactly why in the return.
- Do not add timing assertions.

## Acceptance

- Each fixed failure mode has at least one regression test that would fail on
  the old behavior.
- The tests are deterministic and use the existing seams.
- The full suite passes.
- Any untestable mode is named with the reason and a suggested follow-up.

## Commit plan

1. `test: pin the closed-stdout exit path`
2. `test(post): pin the partial audio-copy outcome`
3. `test(manifest): pin concurrent failure ordering`
4. `test(scan): pin the stale-verdict fallback`
5. `test(lock): pin scoped locking and dead-owner recovery`

## Notes

- T47 may move these tests into split files. Put each test beside its area so
  the move is mechanical.
- Do not preserve old behavior to keep a test green; the note on tests says to
  update tests with the change.
