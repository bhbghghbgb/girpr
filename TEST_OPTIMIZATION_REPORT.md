# Test Optimization Report — girpr workspace (final status)

**Scope:** All 5 workspace packages (`girpr`, `girpr-log`, `girpr-tempdir`, `girsync`, `gircap`)
**Runner:** `cargo nextest run --workspace` (nextest 0.9.149, test profile — unoptimized + debuginfo)
**Result:** 1,251 tests run, 1,251 passed, 8 skipped, ~113 s total wall time (down from ~222 s)

`girsync` alone: 325 tests, 1 skipped, ~79 s (down from ~200 s).

---

## Strategy

Cache-focused suites were moved off the on-disk redb backend where the cache *file* is
not the subject under test:

- A path-keyed, in-process memory cache (`CacheDb::open_memory_cache`,
  `memory_cache_at`, `forget_memory_cache`) is shared across commands in one test
  process, so a later command sees rows an earlier command wrote.
- `CacheDb::Handle::Writable` holds an `Arc<Database>`, letting the registry and a
  command share one database.
- `cmd_compare_self` gained a test-only branch: when a memory record exists at the
  folder's cache path it reads that, instead of requiring the file to exist. Its
  production path is unchanged.
- `common::opts_mem`, `pair_mem`, `update_mem`, `compare_mem`, `sync_mem`, and
  `with_algos_mem` provide the memory-backed options; `recs_of` and `edit_row`
  transparently prefer a memory row when one exists, so cache-content assertions
  stay identical.

`GIRSYNC_TEST_CACHE=memory` switches `open_folder_cache` for whole-process suites
(e.g. `cli_dispatch`). Since nextest runs every test in its own process, the
registry is effectively per-test.

---

## Converted (representative impact)

| Area | Change | Impact |
|---|---|---|
| `lazy` suite | all 27 safe cases moved to the shared memory cache | ~16 s → ~1.5 s |
| `stat_trust cache_repair` | memory `update_mem` + memory `distrusting` | ~12 s → ~0.2 s |
| `stat_trust` compare-self cases | `update_mem` + memory `distrusting` | ~3 s → ~0.4 s |
| `update` suite | all three cases on `update_mem`/`sync_mem` | ~23 s → ~1 s |
| `compare effective_maps` | memory-backed map cases and dry runs | ~80 s → ~9 s |
| `compare_self` suite | memory records; `cli_dispatch` via `GIRSYNC_TEST_CACHE=memory` | ~17 s → <2 s |
| `sync`/`rw_threads`/`hash_mode` | prune, cache-row and read-count cases on memory | several seconds each |
| `migration_lock` | 7 of 10 cases on memory | ~18 s → ~4 s |

---

## Intentionally left on the File backend / real process

These tests observe the cache file, the process boundary, or real redb read-only
semantics; replacing the backend would weaken the assertion:

- `cache::db` read-only tests: a real redb file is required
  (`read_only_open_tolerates_a_case_mode_mismatch`,
  `read_only_cache_refuses_every_write`,
  `read_only_open_requires_a_readable_current_version_cache`).
- Cache byte-identity and no-cache-created assertions (`compare_dry_run_*`,
  `sync_dry_run_creates_no_cache`, `compare_self_accepts_dry_run_with_a_warning`,
  `migration_lock::lock_dry_run_changes_nothing`).
- Backup/snapshot-created tests (`migration_lock::lock_backups_and_snapshots_created`,
  `lock_update_records_files_dirs_and_hashes`).
- Record-path comparisons and `ensure_distinct_sides` cases in `compare` and
  `migration_lock::lock_record_side_is_read_only`.
- `convert`, `output`, `why`, `cli_dispatch` end-to-end/process tests, and
  `rw_threads::through_the_binary` smoke tests.
- `compare_self::a_case_only_pair_with_differing_content_*` and
  `run_compare_self_rejects_a_missing_cache_and_ignore_cache`: both spawn the
  binary, so the child cannot see the parent's memory registry.
