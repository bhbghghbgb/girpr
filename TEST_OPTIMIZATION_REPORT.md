# Test Optimization Report — girpr workspace

**Scope:** All 5 workspace packages (`girpr`, `girpr-log`, `girpr-tempdir`, `girsync`, `gircap`)
**Runner:** `cargo nextest run --workspace` (nextest 0.9.149, test profile — unoptimized + debuginfo)
**Environment:** `TEMP`/`TMP` = `E:\Symlinks\Temp` (HDD); `Z:\tmp` = RamDisk (~2 GB free)
**Result:** 1,252 tests run, 1,252 passed, 8 skipped, ~251 s total wall time

---

## 1. Executive summary

One package dominates test time: **`girsync`** — 205 tests, **795.6 s** (~63 % of all test time), with individual tests up to 33.4 s.

Every other package combined is ~210 s. The `girpr` main package (the thing this repo is actually about — repair_offline, repair_machinery, cli_contract) is entirely fast: 122 tests, 37.3 s, slowest single test 1.7 s.

**Root cause (verified):** HDD disk IO, not compute. Running the slowest girsync test (`every_rw_setting_answers_the_same_question`) on the same machine:

| Temp location | Time for one test |
|---|---|
| `E:\Symlinks\Temp` (HDD) | 16.5 s |
| `Z:\tmp` (RamDisk) | 0.8 s |

→ **~20× speedup purely from moving temp files off the HDD.** Per the task constraints, "change disk" is not an acceptable fix — the fix must reduce or eliminate the IO the tests perform. The good news: most of that IO is avoidable (in-memory cache backends exist, process spawning can be reduced, repeated cold resolves can be trimmed).

---

## 2. Test time by package/binary

| Test binary | Tests | Total time | Max single test | Notes |
|---|---|---|---|---|
| `girsync` | 205 | 795.6 s | 33.40 s | **63 % of suite time** |
| `gircap` | 130 | 120.5 s | 4.23 s | |
| `girpr` (root integration) | 122 | 37.3 s | 1.74 s | repair_offline / repair_machinery / cli_contract |
| `girpr repair` (unit tests) | 93 | 8.0 s | 1.19 s | |
| `gircap http` | 59 | 7.9 s | 4.23 s | |
| `gircap emit` | 60 | 3.3 s | 0.15 s | |
| `girpr util` | 18 | 3.2 s | 0.68 s | |
| `girsync cache` | 17 | 5.6 s | 1.80 s | |
| all others (~20 binaries) | ~328 | ~43 s | ≤1.0 s | |

The top 80 slowest tests in the entire workspace are **all** `girsync` tests. The slowest `girpr` test (1.74 s) would not even make the girsync top 80.

---

## 3. The 80 slowest tests (all girsync)

Sorted by time. Full list available in `E:\Symlinks\Temp\nextest_full_run.txt`.

| # | Time | Test |
|---|------|------|
| 1 | 33.40 s | `girsync::rw_threads limiter::every_rw_setting_answers_the_same_question` |
| 2 | 21.47 s | `girsync::rw_threads through_the_binary::update_hashes_the_same_set_at_every_limit` |
| 3 | 17.91 s | `girsync::migration_lock lock_no_trust_cached_hashes_is_per_side` |
| 4 | 16.32 s | `girsync::stat_trust cache_repair::no_trust_size_rehashes_a_stale_row...` |
| 5 | 15.87 s | `girsync::cli_dispatch run_cli_dispatch_update_compare_sync` |
| 6 | 14.38 s | `girsync::hash_mode read_count::any_of_avoids_backfilling_an_algorithm_the_pair_already_shares` |
| 7 | 13.78 s | `girsync::hash_mode read_count::any_of_still_hashes_when_the_pair_shares_no_algorithm` |
| 8 | 13.08 s | `girsync::update run_update_prunes_and_excludes` |
| 9 | 12.66 s | `girsync::sync a_cache_row_for_a_path_deleted_after_the_run_is_dropped` |
| 10 | 12.28 s | `girsync::lazy sync::a_folder_prunes_its_own_cache_before_the_planner_sees_it` |
| 11 | 11.27 s | `girsync::why flags::why_does_not_change_the_exit_code` |
| 12 | 11.14 s | `girsync::migration_lock lock_cache_writes_merge_unless_stat_changed` |
| 13 | 10.82 s | `girsync::stat_trust refusal::the_refusal_leaves_every_supported_combination_alone` |
| 14 | 10.75 s | `girsync::why flags::show_identical_is_default_off_and_independent_of_why` |
| 15 | 10.47 s | `girsync::rw_threads limiter::an_overlapping_resolve_agrees_with_a_sequential_one` |
| 16 | 10.28 s | `girsync::helpers every_file_lands_in_exactly_one_bucket` |
| 17–80 | 10.13 → 4.27 s | 64 more girsync tests (sync, compare, lazy, migration_lock, stat_trust, hash_mode, convert, output, compare_self, sync_plan, why) |

Every one of these is in `girsync`. No `girpr`, `gircap`, `girpr-log`, or `girpr-tempdir` test appears in the top 80.

---

## 4. Grouping by root cause ("reason")

The slow girsync tests fall into six overlapping groups. Each group's cause can be removed or reduced without changing what the test asserts.

### Group A — Heavy redb File-cache IO on HDD (`opts()` / `TestCacheBackend::File`)

**Cause:** `girsync` tests' default options (`common::opts()`) use `TestCacheBackend::File`, which opens a **redb database on disk** for every side of every fixture. Redb performs a ~1 MB file create + `fsync` per cache open/write. On an HDD this is the dominant cost. The helper comment at `tools/sync/tests/common/mod.rs:130-133` already documents this:

> "Cache-focused suites (compare_self, update, sync/cache, compare) keep `opts()` and its `File` backend."  
> "`opts_mem()` ... Skips the per-command 1 MB create + fsync on the HDD."

**Affected tests (representative):**
- `migration_lock lock_no_trust_cached_hashes_is_per_side` — 17.91 s (multiple `cmd_update` + `cmd_compare`, each opening redb DBs)
- `migration_lock lock_cache_writes_merge_unless_stat_changed` — 11.14 s
- `migration_lock lock_backups_and_snapshots_created` — 9.64 s (also creates backup/snapshot files)
- `migration_lock lock_sha256_sync_converges` — 8.76 s
- `migration_lock lock_hash_none_tracks_without_digests` — 9.61 s
- `migration_lock lock_multi_algo_backfill` — 5.04 s
- `migration_lock lock_update_is_idempotent` — 5.02 s
- `stat_trust cache_repair::no_trust_size_rehashes...` — 16.32 s (uses `TestCacheBackend::File`)
- `stat_trust counters::no_trust_mtime_costs...` — uses `distrusting()` (Memory) already — **already optimized**
- `stat_trust verdict/*`, `other_commands/*` — check cache backends
- `helpers every_file_lands_in_exactly_one_bucket` — 10.28 s (uses `cold_then_warm` → `opts()` File backend)
- `helpers phase_a_*` tests — 3.8–4.1 s each (open redb DBs repeatedly)

**Why it's avoidable:** Tests that assert on **counters, verdicts, and effective maps** (not the on-disk cache file format) do not need the File backend. `distrusting()` in `stat_trust/main.rs:105-112` already builds `CommonOpts` with `TestCacheBackend::Memory` — the same pattern can be applied to `migration_lock`, `helpers`, `sync`, `compare`, `compare_self`, and `lazy` tests that don't inspect `recs_of()` on-disk rows.

**Recommended fix:** Switch non-cache-file-asserting tests from `opts()`/`log()` (File backend) to `opts_mem()`/`distrusting()` (Memory backend). Only tests that explicitly assert `recs_of()` contents, backup siblings, or cache-file layout need the File backend. Estimated saving: **most of the 10–18 s migration_lock and helpers tests**, likely 30–60 s total.

### Group B — Process spawning (`girsync.exe`)

**Cause:** Tests that spawn the real `girsync.exe` binary via `std::process::Command` (`bin()`, `run()`, `run_env()`, `run_binary()`). Process creation on Windows is expensive, and each spawned process repeats HDD cache IO (redb create + fsync, temp dir creation, log file writes).

**Affected tests:**
- `rw_threads through_the_binary::update_hashes_the_same_set_at_every_limit` — 21.47 s (**spawns the binary 6 times**, once per `--rw-*` flag spelling, each hashing 40 files and writing a cache)
- `rw_threads through_the_binary::sync_reports_the_same_actions_under_every_rw_setting` — 7.77 s (spawns 6 times)
- `rw_threads through_the_binary::a_dry_run_is_unaffected_by_the_rw_limit` — 7.01 s (spawns twice)
- `cli_dispatch run_cli_dispatch_update_compare_sync` — 15.87 s (6 in-process `run()` calls; each is a full update/compare/sync pipeline)
- `stat_trust refusal::no_digest_available_with_a_distrusted_field_is_refused` — spawns binary 3 times
- `stat_trust refusal::the_refusal_leaves_every_supported_combination_alone` — spawns ~7 times
- `stat_trust flag_surface::the_two_flags_are_independent_on_the_command_line` — 5.88 s (spawns 3 times)
- `stat_trust flag_surface::update_warns_that_the_flags_do_not_apply_and_still_populates` — 5.07 s (spawns twice)
- `stat_trust flag_surface::the_flags_compose_with_both_hash_modes` — 4.25 s (spawns 6 times)

**Recommended fix:**
1. Where the test asserts **CLI flag parsing / exit codes / stderr messages** (refusal, flag_surface), the binary spawn is necessary — keep, but **share one fixture across flag variants** instead of `temp_root()` per variant (each variant currently creates a fresh temp dir + cache).
2. Where the test asserts **command behavior** (sync_reports_the_same_actions, update_hashes_the_same_set), replace binary spawns with in-process `cmd_sync`/`cmd_update` calls (as `sync.rs`'s `run_binary` already does via `GIRSYNC_TEST_CACHE=memory`). The `rw_threads/through_the_binary` tests exist to pin end-to-end behavior; a hybrid — one binary-spawn test as a smoke test, the rest in-process — keeps coverage while cutting ~5 process spawns per test.
3. `cli_dispatch` runs 6 full pipeline commands in-process; each rebuilds caches from scratch. The fixture is tiny (2 files) but each call opens redb DBs. Switching to Memory cache (Group A) helps here too.

Estimated saving: **~40–60 s** across Group B tests.

### Group C — Repeated cold cache resolves (`rw_threads` limiter)

**Cause:** `every_rw_setting_answers_the_same_question` (33.40 s — the single slowest test in the workspace) resolves the **same fixture cold, 6 times** (once per rw setting: serial, shared(2), shared(4), split(1,1), split(4,4), split(4,1)). Each resolve:
- drops both redb cache files (`drop_caches`)
- scans both directory trees (stat every file on HDD)
- hashes files in phase C (md5 of every file)
- writes redb cache rows for every file

The fixture has ~102 files per side (96 pad files + 6 special), so ~204 files hashed × 6 settings = ~1,224 file hashes + 12 redb DB open/write cycles, all on HDD.

The test's rationale is sound (map equality must hold across settings), but **six full cold resolves is more than needed** to prove that scheduling changes nothing. The assertion is that the *limiter changes scheduling and nothing else* — one cold baseline + one warm re-resolve per setting would prove the same property with far less IO.

**Recommended fix:**
- Keep one full cold resolve as the baseline (serial).
- For each of the other 5 settings, resolve **once warm** (do not `drop_caches`) and compare maps/stats against the baseline. The limiter cannot change *what* is read, only *when*; a warm resolve still exercises the limiter gate on every file read/write.
- Alternatively, reduce the fixture from 96 pad files to ~32 (one window) and keep 6 cold resolves — the test already notes "more files than one resolve window" is deliberate, but 3× the minimum is enough to exercise window boundaries.

Estimated saving: **~20–25 s** on this one test (from 33.4 s down to ~8–10 s).

### Group D — Hash-heavy tests (full file reads + md5/sha256)

**Cause:** Tests that hash file contents (`hash_file` reads the entire file in 512 KB chunks, computing md5 and/or sha256). On an HDD, reading many small files has high per-file open/seek overhead.

**Affected tests:**
- `hash_mode read_count::any_of_avoids_backfilling...` — 14.38 s (warms two pairs with md5, then all-of backfills sha256 = 4 file reads + 2 redb cache writes)
- `hash_mode read_count::any_of_still_hashes...` — 13.78 s (two fresh pairs, cold, all-of reads both algorithms)
- `hash_mode read_count::any_of_reads_one_algorithm...` — 5.15 s
- `hash_mode end_to_end::any_of_answers_a_record...` — 7.11 s
- `hash_mode end_to_end::update_populates_every_algorithm...` — 4.33 s
- `hash_mode end_to_end::the_two_modes_cannot_be_asked_for_at_once` — 5.71 s
- `migration_lock` tests — each calls `hash_file` multiple times (poison, verify)

**Recommended fix:**
- The `read_count` tests use `pair()` with `warmed: true`, which calls `cmd_update` on both sides (redb File backend, Group A). Switching to Memory backend cuts their time substantially.
- `hash_mode end_to_end` tests spawn the binary (Group B). Converting to in-process `cmd_compare` calls with Memory cache cuts both spawn + IO cost.
- Fixture sizes are tiny (a few small files), so the hashing itself isn't the issue — it's the **per-file redb cache write on HDD** that accompanies each hash. Group A fix covers most of this.

Estimated saving: **~20–30 s** (mostly via Group A + Group B fixes).

### Group E — Per-test log file writes (`log()` writes `girsync.log`)

**Cause:** `common::log()` (`tools/sync/tests/common/mod.rs:155-162`) sets `file: Some(PathBuf::from("girsync.log"))`. Every test that calls `log()` writes a `girsync.log` file to the current working directory (the HDD temp). This is a small file but created on every test, and the tracing subscriber opens/writes/closes it. Multiplied by 205 girsync tests this is non-trivial HDD metadata IO.

**Recommended fix:** Change `log()` to use `file: None` (log to console/stderr only, which `tracing` discards without a subscriber in tests), or write to a path inside the per-test `TempDir` (already on HDD but at least not in the shared cwd). Tests that assert on log file contents can opt into a named file.

Estimated saving: **~5–15 s** across the suite (metadata IO on HDD).

### Group F — Sync/copy operations on HDD

**Cause:** Tests that copy files between directories (`cmd_sync`), delete files, create directories — all HDD file operations.

**Affected tests:**
- `sync run_update_prunes_and_excludes` — 13.08 s
- `sync a_cache_row_for_a_path_deleted_after_the_run_is_dropped` — 12.66 s
- `sync run_sync_dry_run_with_ignore_cache_leaves_caches_intact` — 10.05 s
- `sync run_sync_keep_extra_and_missing_only` — 7.83 s
- `sync run_sync_resolves_type_conflicts` — 3.54 s
- `lazy sync::a_folder_prunes_its_own_cache...` — 12.28 s
- `lazy sync::update_still_hashes_every_file` — 3.65 s

**Recommended fix:** These tests genuinely need filesystem operations (they test sync/copy behavior). The fix is Group A (Memory cache backends so the cache side of sync doesn't hit the HDD) plus ensuring fixtures are small. The actual file copies are small (a few files), so most of the time is the cache IO. Estimated saving: **~15–25 s** via Group A.

---

## 5. Packages that are already fast (no action needed)

| Package | Tests | Total | Slowest | Assessment |
|---|---|---|---|---|
| `girpr` (repair_offline, cli_contract, repair_machinery) | 122 | 37.3 s | 1.74 s | All fast. Offline mock API, small fixtures (2–3 files, tens of bytes), in-process `repair::run` calls, no network, no redb cache. Six tests spawn the binary for flag-surface contract — appropriate and fast. |
| `girpr repair` (unit tests) | 93 | 8.0 s | 1.19 s | Pure unit tests next to code. Fast. |
| `gircap` | 130 | 120.5 s | 4.23 s | Max test 4.23 s. Mostly pure unit tests on in-memory structures. No slow outliers. |
| `girpr-log`, `girpr-tempdir` | ~35 | ~1.1 s | ≤0.4 s | Trivially fast. |
| `peak_heap.rs` | 1 (ignored) | — | — | Already `#[ignore]`d; runs only on demand. Not in the regular suite. |
| `artifact_bridge.rs` | (ignored) | — | — | Already `#[ignore]`d; needs a real bundle, run by hand. Not in the regular suite. |

The `girpr` main package tests do not need optimization. Any further gains there would be marginal (<1 s per test).

---

## 6. Consolidated recommendations (with estimated impact)

| Priority | Group | Fix | Tests affected | Est. saving |
|---|---|---|---|---|
| 1 | A | Switch non-cache-file-asserting tests from `opts()` (File/redb on HDD) to `opts_mem()` / `distrusting()` (Memory). The `distrusting()` helper in `stat_trust/main.rs:105` is the model. | migration_lock (all), helpers, sync, compare, compare_self, lazy, stat_trust/cache_repair | 40–70 s |
| 2 | C | Reduce repeated cold resolves in `every_rw_setting_answers_the_same_question`: one cold baseline + warm re-resolves per setting, or smaller fixture. | rw_threads/limiter | 20–25 s |
| 3 | B | Replace binary spawns with in-process `cmd_*` calls where CLI boundary isn't under test; share fixtures across flag variants. | rw_threads/through_the_binary, stat_trust/refusal, stat_trust/flag_surface, cli_dispatch | 30–50 s |
| 4 | E | Stop writing `girsync.log` to cwd on every test (`log()` → `file: None` or per-test TempDir path). | All girsync tests using `log()` | 5–15 s |
| 5 | D/F | Covered by 1–4 (Memory cache removes the redb IO that dominates hash/sync tests). | hash_mode, sync | (included above) |

**Total estimated saving: ~100–160 s** (roughly 12–20 % of the current 251 s, and ~13–20 % of girsync's 795 s). The single biggest lever is Group A (Memory cache) because redb `fsync` on HDD is the dominant per-test cost, as proven by the 20× RamDisk speedup.

---

## 7. How to verify IO vs. compute with the Z: RamDisk

Do **not** ship tests that require a RamDisk — the fix must work on HDD. Use Z: only to confirm a test is IO-bound:

```powershell
# Baseline on HDD
$env:TEMP="E:\Symlinks\Temp"; $env:TMP="E:\Symlinks\Temp"
cargo nextest run -p girsync --test rw_threads limiter   # ~33 s for the limiter module

# Same test on RamDisk — if time collapses, the test is IO-bound
$env:TEMP="Z:\tmp"; $env:TMP="Z:\tmp"
cargo nextest run -p girsync --test rw_threads limiter   # expect ~2–3 s

# Reset back to HDD for normal runs
$env:TEMP="E:\Symlinks\Temp"; $env:TMP="E:\Symlinks\Temp"
```

If a test's time drops by >5× on RamDisk, its slowness is disk IO and it is a candidate for Group A/B/C fixes above. If time is unchanged on RamDisk, the test is compute-bound and should be profiled differently.

---

## 8. Tests that could be reduced in scope (integration → unit)

| Test | Current scope | Lower-scope alternative | Rationale |
|---|---|---|---|
| `rw_threads through_the_binary::*` | Spawns `girsync.exe`, end-to-end via CLI | In-process `cmd_sync`/`cmd_update` with `RwRuntime` | The limiter invariant (same map at every limit) is already pinned at unit level in `limiter.rs`. Binary spawn adds process startup + HDD cache IO for marginal extra coverage. Keep one smoke test, convert the rest. |
| `stat_trust refusal::*` / `flag_surface::*` | Spawns binary per flag variant | In-process `run(Cli, &plan)` (as `cli_dispatch` already does) | These test flag parsing and dispatch, not the binary boundary. `cli_dispatch` proves in-process dispatch works; refusal/flag_surface can use the same path. |
| `hash_mode end_to_end::*` | Spawns binary per mode | In-process `cmd_compare` with `CommonOpts` | Same planner/code path; binary spawn adds no assertion value. |

No test in the workspace appeared useless (all assert meaningful behavior). The optimization opportunities are purely about **removing avoidable IO and process spawn**, not deleting assertions.

---

## 9. Summary

- **1,252 tests pass; ~251 s total.**
- **`girsync` = 63 % of suite time (795.6 s / 205 tests).** All top-80 slowest tests are girsync tests.
- **Verified root cause: HDD IO.** Same test: 16.5 s on HDD vs 0.8 s on Z: RamDisk (20×).
- **Fixes must reduce IO, not change disks.** The `girsync` test helpers already provide `TestCacheBackend::Memory` (`opts_mem()`, `distrusting()`) precisely to skip redb create+fsync on HDD — most slow tests simply don't use it.
- **Biggest single win:** `every_rw_setting_answers_the_same_question` (33.4 s → est. 8–10 s) by reducing repeated cold resolves.
- **girpr main package tests are already fast** (max 1.74 s) — no action needed there.
