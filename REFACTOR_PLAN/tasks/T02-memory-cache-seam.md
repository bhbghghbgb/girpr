# T02 — Memory cache seam for girsync record/FS cases

## Goal / pointers
- `docs/10-test-optimization-research.md` §F30 (lines 386–403) — `run_cli_with_hooks`
  unconditionally sets `GIRSYNC_TEST_CACHE=file`; ~44 in-process CLI cases force a
  file-backed redb cache with ~90 expensive opens (~0.5 s each), the dominant source
  of sync-side test-seconds. Fix: add `run_args_mem`/`run_cli_mem` setting
  `GIRSYNC_TEST_CACHE=memory` for record/FS-only cases; keep the file backend where the
  cache file is the subject.
- Related: §F31 (405–411) may largely fall out of this task — see T18.
- Assumptions to validate: §A1 (46–52), §A7 (91–95), §A10 (108–110), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` and `CONVENTIONS.md`.

## Scope
- `crates/sync-test-support/src/lib.rs` (`run_cli_with_hooks` and the path-keyed
  in-memory registry + forget helper), the `run_args`/`run_text`/`run_error_text`
  helpers that funnel through it, and every call site named in §F30 (symlink.rs,
  sync.rs, compare.rs, compare_self.rs, sync_plan/*, rw_threads/main.rs, why/flags.rs).
- Keepers named in §F30: backup/retain cases in `sync.rs`, `migration_lock.rs`, and
  cache-copy checks. Locate all call sites in the current tree before editing.

## Implementation plan
1. Validate §F30 against current code: confirm the env var is set unconditionally and
   that a memory backend exists; measure current sync-side cumulative test time.
2. Add a narrow seam (`run_args_mem`/`run_cli_mem`) that sets
   `GIRSYNC_TEST_CACHE=memory`, leaving the existing helpers' default unchanged.
3. Migrate record/FS-asserting call sites to the memory seam. Audit every dry-then-real
   pair named in §F30; call the existing forget helper where a cold cache is required
   so the second command does not inherit the first's registry entry.
4. Leave file-backend cases that test the cache file itself untouched.
5. Re-measure sync-side wall time and I/O (cache opens, temp trees); deliver in
   separate commits for the seam, the migration, and any forget-helper fixes if large.

## Constraints / validation
- The in-memory registry is path-keyed and shared within a process (§F30); a migration
  that silently shares state between two commands changes behavior and must be caught.
- No cache file asserted by a test may be downshifted; keep the file backend for
  `migration_lock.rs` and backup/retain/copy cases.
- Do not lower `GIRSYNC_CACHE_SIZE_MIB` (F34) — redb treats it as a budget.
- Do not change nextest-named tests or their filters (§A10). Preserve each test's
  semantic assertion.

## Done when
Record/FS-only CLI cases run against the memory cache, file-cache cases still pass,
dry-then-real pairs explicitly forget cache entries where needed, and sync-side
cumulative test time and cache opens are measurably lower with all tests passing.
