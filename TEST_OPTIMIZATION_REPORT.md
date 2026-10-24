# Test Optimization Report — girpr workspace (remaining work)

**Scope:** All 5 workspace packages (`girpr`, `girpr-log`, `girpr-tempdir`, `girsync`, `gircap`)
**Runner:** `cargo nextest run --workspace` (nextest 0.9.149, test profile — unoptimized + debuginfo)
**Environment:** `TEMP`/`TMP` = `E:\Symlinks\Temp` (HDD); `Z:\tmp` = RamDisk (~2 GB free)
**Result:** 1,252 tests run, 1,252 passed, 8 skipped, ~222 s total wall time (down from ~251 s)

---

## Changes already applied (committed)

These are **done** and removed from this report:

| Fix | What changed | Impact |
|---|---|---|
| Group A (Memory cache) | `helpers`: `cold_then_warm` and `phase_a_never_hashes` use `CacheDb::open_temp` instead of redb File DBs | `every_file_lands_in_exactly_one_bucket`: 10.3 s → 0.4 s |
| Group A (Memory cache) | `rw_threads/limiter`: `every_rw_setting_...` and `an_overlapping_resolve_...` use `opts_mem()` | `every_rw_setting...`: 33.4 s → 2.8 s; `an_overlapping...`: 10.5 s → 1.1 s |
| Group B (process spawn) | `rw_threads/through_the_binary`: `update_hashes_the_same_set_at_every_limit` and `a_dry_run_is_unaffected_by_the_rw_limit` converted from `girsync.exe` spawns to in-process `cmd_update`/`cmd_sync` | `update_hashes...`: 23.4 s → 6.7 s; `a_dry_run...`: 7.0 s → 4.5 s |
| Group B (process spawn) | `stat_trust/flag_surface`: `the_two_flags_are_independent...` and `the_flags_compose_with_both_hash_modes` converted from binary spawns to in-process `cmd_update` | both: ~5–6 s → ~0.1 s |
| Group B (process spawn) | `stat_trust/refusal`: `the_refusal_leaves_every_supported_combination_alone` converted to in-process `cmd_compare` | ~10 s → 2.5 s |
| Group E (log file writes) | `common::log()` changed from `file: Some("girsync.log")` to `file: None` | ~5–15 s across suite |

**girsync suite: ~795 s → ~200 s. Total workspace: ~251 s → ~222 s.**

---

## Remaining root causes

### Group A — Heavy redb File-cache IO on HDD (not yet fixed)

**Cause:** Tests that assert `recs_of()` contents, backup siblings, or cache-file layout need `TestCacheBackend::File` and open redb DBs on disk. These were not switched to Memory because doing so would break the cache-content assertions.

**Still slow:**
- `migration_lock` — all 7 tests assert `recs_of()` cache contents; each opens redb DBs per `cmd_update`/`cmd_compare` (17.9 s, 11.1 s, 9.6 s, 8.8 s, 9.6 s, 5.0 s, 5.0 s)
- `stat_trust cache_repair::no_trust_size_rehashes...` — 16.3 s (uses `TestCacheBackend::File`, asserts `recs_of()`)
- `sync` tests asserting cache rows — `a_cache_row_for_a_path_deleted_after_the_run_is_dropped` (12.7 s), `files_copied_into_a_directory_this_run_created_keep_their_cache_rows` (~9 s), `rows_for_a_subtree_a_copy_wiped_are_dropped` (~9 s)
- `update run_update_prunes_and_excludes` — 13.1 s (asserts `recs_of()` after `cmd_update`)
- `hash_mode read_count` tests — `pair(..., warmed: true)` calls `cmd_update` with File backend; Memory cache wouldn't persist warmed data across separate `cmd_update` calls (14.4 s, 13.8 s)
- `compare.rs effective_maps()` — tried switching to `opts_mem()` but reverted: tests create orphan rows directly in File DBs via `open_db()` and expect `effective_maps` to see them; a fresh Memory DB per call wouldn't contain those rows

### Group B — Process spawning (`girsync.exe`) (partially fixed)

**Still spawns the binary:**
- `rw_threads through_the_binary::sync_reports_the_same_actions_under_every_rw_setting` — kept as a smoke test; spawns binary 6 times (4.8 s)
- `stat_trust refusal::no_digest_available_with_a_distrusted_field_is_refused` — reverted to binary spawn; refusal is a clap-level validation that returns exit code (not an error) at the library level (0.2 s)
- `stat_trust flag_surface::update_warns_that_the_flags_do_not_apply_and_still_populates` — asserts on stderr message content; still spawns binary twice (~3.8 s)
- `cli_dispatch run_cli_dispatch_update_compare_sync` — 6 in-process `run()` calls; each opens File cache DBs. Setting `GIRSYNC_TEST_CACHE=memory` was tried but reverted: `cmd_update` writes to a temp DB that's dropped, so subsequent `compare-self`/`compare` commands can't read the cache (~13 s)
- `why` tests — assert on CLI text output rendering (`--why`, `--show-identical`, etc.); require the real binary (~11 s each)

### Group C — Repeated cold cache resolves (not yet fixed)

**Cause:** `every_rw_setting_answers_the_same_question` still calls `drop_caches()` before each of the 6 `resolve_overlapping` calls. Each resolve scans 96 pad files + stats trees on HDD. Now uses Memory cache (`opts_mem()`) so redb IO is eliminated, but the 6 cold stat-scans + hashes of ~102 files per side still cost ~2.8 s.

**Further optimization possible:** one cold baseline + warm re-resolves per setting, or reduce fixture from 96 pad files to ~32.

### Group D — Hash-heavy tests (not yet fixed)

**Still slow:**
- `hash_mode read_count::any_of_avoids_backfilling...` — 14.4 s
- `hash_mode read_count::any_of_still_hashes...` — 13.8 s
- `hash_mode end_to_end::any_of_answers_a_record...` — 7.1 s
- `hash_mode end_to_end::update_populates_every_algorithm...` — 4.3 s
- `hash_mode end_to_end::the_two_modes_cannot_be_asked_for_at_once` — 5.7 s (spawns binary)

### Group F — Sync/copy operations on HDD (not yet fixed)

**Still slow:**
- `sync run_update_prunes_and_excludes` — 13.1 s (also Group A)
- `sync a_cache_row_for_a_path_deleted_after_the_run_is_dropped` — 12.7 s (also Group A)
- `sync run_sync_dry_run_with_ignore_cache_leaves_caches_intact` — 10.1 s
- `sync run_sync_keep_extra_and_missing_only` — 7.8 s
- `lazy sync::a_folder_prunes_its_own_cache_before_the_planner_sees_it` — 12.3 s (also Group A)
- `lazy sync::update_still_hashes_every_file` — 3.7 s

---

## Suggested next steps (in order of impact)

1. **`migration_lock` tests → in-process with Memory backend where possible.** Tests that assert `recs_of()` contents need File, but some (e.g. `lock_update_records_files_dirs_and_hashes`, `lock_update_is_idempotent`, `lock_empty_dirs_and_nesting_sync`) only check row existence/kind/size/mtime — could use `CacheDb::open_temp` + `load_all_records` on the temp DB instead of `open_db` on disk.

2. **`hash_mode read_count` → reduce `pair()` warming overhead.** The warmed `cmd_update` calls create File cache DBs. Consider constructing `CommonOpts` with `test_cache: Memory` directly and calling `cmd_update` through `open_folder_cache` (which checks `GIRSYNC_TEST_CACHE=memory`), or restructure so both modes share one warmed fixture.

3. **`why` tests → in-process `run(Cli, &plan)` where possible.** Tests that only check exit codes (not text output) could use `run()` directly. Text-output assertions need the binary or a custom subscriber.

4. **`every_rw_setting_answers_the_same_question` → warm re-resolves.** Keep one cold `drop_caches` baseline, then resolve warm (no `drop_caches`) for the other 5 settings. The limiter gate still fires on every file read/write.

5. **`sync` cache-row tests → keep File backend.** These genuinely assert on-disk cache layout; no safe Memory alternative without restructuring the assertions.