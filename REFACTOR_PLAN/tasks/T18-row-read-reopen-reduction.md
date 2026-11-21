# T18 — Reduce row-reading redb reopens

## Goal / pointers
- `docs/10-test-optimization-research.md` §F31 (lines 405–411) — `md5_in`/`md5_in_file`
  in `sync.rs` and `recs_of`/`cache_at`/`edit_row`/`strip_algo`/`poison_digest` in
  `sync-test-support` reopen redb after every command. `migration_lock.rs` documents the
  cost and has a metadata-only `cache_snapshot`. Fix: largely falls out of T02's memory
  seam; where a real file is needed, return rows on a test-only `RunOutcome` instead of
  reopening.
- Assumptions to validate: §A7 (91–95), §A10 (108–110), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/sync/tests/sync.rs`, `crates/sync-test-support/src/lib.rs`,
  `tools/sync/tests/migration_lock.rs`. Locate the reopen sites in the current tree
  before editing.

## Implementation plan
1. Check whether T02 already removed most reopens (memory entries are found by
   `cache_at`). If so, record a skip with that reason rather than duplicating work.
2. For remaining cases that genuinely need a file-backed cache, return rows on a
   test-only `RunOutcome` (or extend `cache_snapshot`) so tests read rows without
   reopening redb.
3. Preserve the semantics of each helper (digest, algorithm stripping, poisoning) and
   keep their assertions unchanged.
4. Re-measure cache opens for the affected sync tests; update nextest filters if a
   named test moves (§A10).

## Constraints / validation
- Do not change production cache behavior; the seam is test-only.
- Keep file-backed cases whose subject is the cache file (§F30); do not downshift them
   to memory just to avoid an open.
- Preserve row semantics and assertions (§6); no fixture growth (§A5).

## Done when
Remaining row reads no longer reopen redb per command, file-backed cache tests still
pass, and the reopen count/cumulative time for affected tests is lower.
