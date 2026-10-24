# Test I/O Analysis Report

**Repository:** E:\hoccode\hocrust\girpr
**Date:** 2026-10-24
**Runner:** cargo nextest 0.9.149
**Disks:** E: = HDD (temp dir symlinked, high IO penalty), Z: = RamDisk (comparison baseline)
**Method:** Serial sweep with job-object I/O accounting (`tests/scripts/io-profile/run-sweep.ps1`), each test in its own process, concurrency=1

---

## 1. Executive Summary

| Metric | E: (HDD) | Z: (RAM) | IO-attributable |
|--------|----------|----------|-----------------|
| Tests | 1260 | 1260 | — |
| Wall time | 413.1 s | 106.6 s | **306.5 s (74%)** |
| CPU time | 58.6 s | 66.9 s | — (unchanged) |
| Wait time | 357.1 s | 45.9 s | **311.2 s** |
| Write MB | 255.9 | 256.4 | — |
| Read MB | 284.4 | 284.9 | — |

CPU time is essentially identical on both disks (~59–67 s). The ~306 s gap is pure storage latency — every test pays 20–220× more per fsync/DB-open on the HDD than on the RamDisk (see §5 microbenchmarks). Build/CPU time is out of scope; this report addresses only IO.

**girsync is the dominant IO consumer:** 327 tests, 314 s on E: vs 28.8 s on Z: → **285 s IO penalty (91% of girsync wall time)**. Nearly every girsync test is 90%+ IO wait.

---

## 2. Tests Running Over 10 s on E: (HDD)

Only **one** test still exceeds 10 s wall on the HDD, and it is entirely IO-bound:

| # | Test | E: Wall | Z: Wall | IO Saved | Verdict |
|---|------|---------|---------|----------|---------|
| 1 | `girsync::why::table::every_pair_state_has_one_classification_and_one_tag` | 16.16 s | 0.28 s | **15.88 s** | **IO-bound** — fixable |

The three ~15 s `repair_offline` tests that were slow before the recent fix pass are now down to ~12 s total for 74 tests (R5 partial: injectable Sleep seam). They are sleep/retry-bound, not IO-bound, and their remaining time is mostly CPU/sleep on both disks.

### Tests over 5 s on E: that are IO-bound

| Test | E: Wall | Z: Wall | IO Saved |
|------|---------|---------|----------|
| `girsync::why::table::every_pair_state_has_one_classification_and_one_tag` | 16.16 s | 0.28 s | 15.88 s |
| `girsync::rw_threads::limiter::every_rw_setting_answers_the_same_question` | 9.47 s | 0.99 s | 8.48 s |
| `girsync::rw_threads::through_the_binary::update_hashes_the_same_set_at_every_limit` | 6.14 s | 0.86 s | 5.28 s |

---

## 3. IO vs Sleep Classification by Suite

Aggregate differential (E: wall − Z: wall = IO saved):

### Highly IO-bound (DeltaS > 10 s, Z: wall < 5 s)

| Suite | Tests | E: Wall | Z: Wall | IO Saved | % IO |
|-------|-------|---------|---------|----------|------|
| girsync::lazy | 27 | 48.4 s | 1.9 s | 46.4 s | 96% |
| girsync::why | 25 | 40.8 s | 1.7 s | 39.0 s | 96% |
| girsync::stat_trust | 19 | 34.6 s | 2.1 s | 32.5 s | 94% |
| girsync::migration_lock | 11 | 26.1 s | 1.1 s | 24.9 s | 95% |
| girsync::compare | 12 | 21.6 s | 0.9 s | 20.6 s | 95% |
| girsync::rw_threads | 15 | 26.2 s | 5.7 s | 20.5 s | 78% |
| girsync::sync | 14 | 21.4 s | 1.5 s | 20.0 s | 93% |
| girsync::compare_self | 12 | 15.2 s | 0.8 s | 14.4 s | 95% |
| girsync::hash_mode | 18 | 17.6 s | 3.4 s | 14.2 s | 81% |
| girsync::sync_plan | 19 | 14.7 s | 3.6 s | 11.1 s | 76% |
| girsync::helpers | 10 | 11.0 s | 0.5 s | 10.4 s | 95% |
| girsync::update | 3 | 9.4 s | 0.5 s | 8.9 s | 95% |
| girsync::output | 16 | 9.1 s | 1.5 s | 7.6 s | 84% |
| girsync::convert | 4 | 6.8 s | 0.7 s | 6.2 s | 91% |
| girsync::cli_dispatch | 1 | 4.8 s | 0.2 s | 4.7 s | 98% |

### Sleep-bound (DeltaS ≈ 0 — NOT IO, out of scope for this fix)

| Suite | Tests | E: Wall | Z: Wall | IO Saved | Reason |
|-------|-------|---------|---------|----------|--------|
| gircap::cli_contract | 28 | 25.7 s | 25.5 s | 0.2 s | Process spawn + HTTP timeout, not disk IO |
| gircap::http::a_transport_failure_is_retried | 1 | 4.12 s | 4.10 s | 0.02 s | Retry backoff sleep |
| girpr::repair_offline | 74 | 12.0 s | 4.1 s | 7.9 s | Mix of sleep + IO; sleep portion already cut by R5 |

### Mostly CPU-bound (low IO saved)

| Suite | E: Wall | Z: Wall | IO Saved | Note |
|-------|---------|---------|----------|------|
| gircap | 15.9 s | 13.2 s | 2.3 s | CPU-heavy capture work |
| gircap::capture_layout | 10.0 s | 9.0 s | 0.9 s | 7.9 s CPU on both disks |
| gircap::schema | 6.9 s | 5.8 s | 1.1 s | 5.4 s CPU on Z: |
| gircap::matrix | 4.6 s | 4.1 s | 0.5 s | 3.9 s CPU on Z: |

---

## 4. Root Causes of the IO

Microbenchmarks (iot-bench, 10 ops each) show the per-operation cost on HDD vs RAM:

| Operation | E: (HDD) | Z: (RAM) | Ratio |
|-----------|----------|----------|-------|
| redb fresh create (durable=true) | 494.7 ms | 2.6 ms | **189×** |
| file create 1 MB + sync_all | 81.6 ms | 0.37 ms | **220×** |
| create+write512+sync_all | 64.1 ms | 0.47 ms | **135×** |
| redb open+meta commit | 206.5 ms | 10.1 ms | **20×** |
| redb open (no commit) | 205.3 ms | 8.6 ms | **24×** |
| redb 10 commits × 10 rows | 43.3 ms | 0.7 ms | **62×** |
| plain file write 512 B (no sync) | 0.26 ms | 0.27 ms | 1× |
| plain file copy 1 MB | 1.30 ms | 1.30 ms | 1× |

**Key insight:** Operations involving `sync_all` (fsync), file creation with sync, and redb database open/create are **20–220× slower on HDD**. Plain reads/writes/copies without sync are the same speed on both disks. Therefore the IO problem is not "how many bytes" but "how many syncing metadata operations".

Five IO sources in the tests:

1. **Redb cache DB open/create (~200 ms/open on HDD).** Every girsync test that opens a folder cache pays this. `open_folder_cache()` (`tools/sync/src/effective/side.rs:143`) already has an in-memory switch (`TestCacheBackend::Memory` or `GIRSYNC_TEST_CACHE=memory`), but only `sync`, `compare`, `output`, and `rw_threads/through_the_binary` use it.

2. **Per-test temp directory lifecycle.** Each test calls `TempDir::new` → `create_dir_all` on `<temp>/girpr-test/` (HDD) and `remove_dir_all` on drop. For 327 girsync tests that is 327 create+remove cycles of directory trees.

3. **Fixture file writes.** Each test writes multiple fixture files before running the code under test.

4. **Binary subprocess spawns.** Tests running `girsync.exe` pay ~81 ms per command for the 1 MB cache file create + fsync, plus file copy IO.

5. **No fixture deduplication.** Tests that iterate over configurations (flag spellings, table rows) rebuild fixtures and recaches every iteration instead of building once and resetting.

---

## 5. Already-Applied Fixes (from git history)

| Commit | Fix | Scope | Measured effect |
|--------|-----|-------|-----------------|
| `75db39b` (R3) | Drop per-file `sync_all()` in `copy_one` | `tools/sync/src/commands/sync/apply.rs` | −11.5 s on 130 girsync tests |
| `1b5e2e8` (R4) | Skip no-op meta commit on cache open | `tools/sync/src/cache/db.rs` | −12.4 s on 130 tests |
| `989b618` (R1 partial) | Test-only in-memory cache seam | `tools/sync/src/effective/side.rs` | Enables Memory backend |
| `2f049bc` (R2 partial) | `rw_threads` subprocess sync vs in-memory cache | `tools/sync/tests/rw_threads/` | `sync_reports_*` test: 17 s → ~2.5 s on E: |
| `36d025a` (R5 partial) | Injectable Sleep seam in girpr | `src/sleep.rs`, `repair/file.rs`, `hyp.rs`, `sophon.rs` | `repair_offline`: 57.8 s → 12.0 s total |

These fixes cut total serial wall from 491 s to 413 s (−78 s), but the remaining IO-bound tests are still 10–16 s each because the in-memory cache seam was not applied to all suites, and fixtures are still rebuilt per iteration.

---

## 6. Recommended Fixes

### Fix 1 — R1 full: in-memory cache for all girsync semantic suites (largest single win)

**What:** Set `test_cache = TestCacheBackend::Memory` in every girsync lib-driven test suite that asserts *sync results* (not cache file contents), so `open_folder_cache()` serves `CacheDb::open_temp()` (in-memory redb) instead of opening `<root>/girpr-cache` on the HDD.

**Where:** `tools/sync/tests/common/mod.rs` `opts()` default, or per-suite overrides. The seam already exists at `tools/sync/src/effective/side.rs:143–166`.

**Suites to migrate** (assert results, not cache file):
- `lazy` (46.4 s IO saved)
- `why` (39.0 s IO saved) — includes the 16.16 s table test
- `stat_trust` (32.5 s IO saved)
- `compare_self` (14.4 s IO saved)
- `hash_mode` (14.2 s IO saved)
- `sync_plan` (11.1 s IO saved)
- `helpers` (10.4 s IO saved)
- `update` (8.9 s IO saved)
- `cli_dispatch` (4.7 s IO saved)

**Keep real on-disk cache (do NOT migrate):**
- `migration_lock` — asserts backup/snapshot file creation on disk
- `convert` — converts sled→redb, operates on real cache files
- `cache` unit tests — assert cache file behavior directly

**Subprocess tests** (spawn `girsync.exe`): ensure `GIRSYNC_TEST_CACHE=memory` is set on the `Command`, as `sync.rs`, `output.rs`, and `rw_threads/through_the_binary.rs` already do.

**Estimated savings:** ~150–200 s serial (largest single fix; addresses the ~0.8–1.4 s per-command fixed cost).

### Fix 2 — R2 full: deduplicate fixtures across iterations

**What:** Build the fixture tree once per test and reuse it across scenario iterations, resetting only what each scenario changes. Avoids repeated `create_dir_all` + file writes + cache open per iteration.

**Where:**
- `tools/sync/tests/rw_threads/main.rs` — `flag_spellings()` runs 6 variants, each rebuilding `temp_root` + 102-file fixture + 2 caches. Build once, reset changed files, reuse cache pair.
- `tools/sync/tests/sync_plan/` — reuses copied tree across rw limits already partially; verify no redundant rebuild.
- `tools/capture/tests/capture_layout.rs` — 18 tests each re-capture the same healthy mock. Capture once into a bundle fixture; the 18 layout tests read it; keep exactly one re-capture test.

**Estimated savings:** ~60–100 s serial (overlaps with Fix 1; count once in aggregate).

### Fix 3 — R6: bound cache backups/snapshots

**What:** `backup_db()` and `snapshot_old()` in `tools/sync/src/cache/open.rs` copy the entire cache file per run. On HDD this is extra write IO. Retain one backup per side per run, skip when cache unchanged, and avoid creating both a backup and an old-state snapshot of identical bytes.

**Where:** `tools/sync/src/cache/open.rs:131` (`backup_db`) and :152 (`snapshot_old`).

**Estimated savings:** write-volume / hygiene; low wall-time impact on this HDD but reduces total IO ops (255 MB written on E:).

## 7. Tests That Should NOT Be Optimized for IO

| Test/Suite | E: Wall | Z: Wall | IO Saved | Why not IO |
|------------|---------|---------|----------|------------|
| `gircap::cli_contract` (28 tests) | 25.7 s | 25.5 s | 0.2 s | Process spawn + HTTP timeout to 127.0.0.1:1; not disk IO |
| `gircap::http::a_transport_failure_is_retried` | 4.12 s | 4.10 s | 0.02 s | Retry backoff sleep |
| `girpr::repair_offline` 500/404 retry tests | ~15 s each | ~15 s each | ~0 s | Retry backoff sleep (R5 already cut 45 s of backoff) |

These are slow on *both* disks. Optimizing IO will not help them. The `repair_offline` sleep portion was already addressed by R5; any remaining sleep is inherent to the retry policy being tested.

---

## 8. Expected Effect After Fixes 1–2

| Stage | Serial Wall (E:) | Serial Wall (Z:) | IO Gap Closed |
|-------|------------------|------------------|---------------|
| Today (after R1–R5 partial) | 413.1 s | 106.6 s | 306.5 s |
| After Fix 1 (R1 full: in-memory cache all suites) | ~220–260 s | ~100–110 s | ~150–200 s |
| After Fix 2 (R2 full: fixture dedup) | ~160–200 s | ~90–100 s | ~60–100 s additional |
| Z: floor (cannot go below without code change) | — | 106.6 s | — |

The Z: floor is not zero: the suite does real CPU work (~67 s serial) and some tests legitimately exercise cache/backup paths on disk.

---

## 9. Verification

Run after each fix:
```pwsh
# Serial sweep on HDD
pwsh -File tests/scripts/io-profile/run-sweep.ps1 -Out target/io-profile/e-current
# Serial sweep on RAM disk
pwsh -File tests/scripts/io-profile/run-sweep.ps1 -TempRoot Z:\tmp -Out target/io-profile/z-current
# Differential analysis
pwsh -File tests/scripts/io-profile/analyze.ps1 `
    -Tests target/io-profile/e-current/tests.json `
    -Csv   target/io-profile/e-current/tasks.tsv.csv `
    -CompareCsv target/io-profile/z-current/tasks.tsv.csv `
    -Out   target/io-profile/e-current/analysis
```

Confirm: for each affected suite, `wall(E:) − wall(Z:)` shrinks toward the Z: floor, and no test assertion changes. The HDD has ±10–20% run-to-run variance; treat only aggregate or multi-second deltas as real.

---

**Data files:** `target/io-profile/e-current/` (HDD sweep), `target/io-profile/z-current/` (RAM sweep), `target/io-profile/e-current/analysis/compare-tests.csv` (per-test differential), `target/io-profile/e-current/analysis/compare-suites.csv` (per-suite differential). All git-ignored.
