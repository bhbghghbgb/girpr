# Test-suite I/O profile and optimisation report

Measured on this machine (4 CPUs, 11.8 GB RAM, no admin rights) on 2026-10-23.
Scope: the whole workspace, 1,259 test cases in 39 suites / 293 test binaries.
Build time and CPU time are out of scope; the target is file-system work the
tests do and do not need.

**Headline.** The suite is not I/O-volume bound, it is I/O-*latency* bound.
A serial pass over every test writes 259 MB and reads 282 MB, yet spends
339 s waiting on the disk — an effective 1.6 MB/s. The same tests with the
scratch tree on a RAM disk take 74 s instead of 353 s in the normal parallel
run: **79 % of the suite's wall time is one hard disk's latency**. Almost all
of it is the per-test `girpr-cache` (a 1 MB redb file) being created, opened
and re-opened on every command, plus one `fsync` per copied file.

---

## 1. Summary

| measurement | value |
|---|---:|
| test cases (`cargo nextest`, 8 `#[ignore]`d skipped) | 1,251 run / 1,259 listed |
| parallel wall, scratch on `E:` (HDD) | **353.0 s** |
| parallel wall, same binaries, scratch on `Z:` (RAM disk) | **73.8 s** |
| serial per-test sweep, scratch on `E:` | **491.1 s** |
| serial per-test sweep, scratch on `Z:` | **152.2 s** |
| CPU time, serial sweep | 66.7 s |
| bytes written / read, serial sweep | 259 MB / 282 MB |
| I/O operations, serial sweep | 665,042 |
| storage-attributable wall (serial) | **338.9 s (69 %)** |
| deliberate retry sleeps (serial, measured on `Z:` too) | **79.7 s (16 %)** |
| CPU (serial) | 66.7 s (14 %) |

The parallel run is the workflow number; the serial sweep exists only to
attribute time to individual tests without cross-test disk contention.

Buckets:

- **Storage latency** — `E:` wall minus `Z:` wall per test. 338.9 s over the
  serial sweep, and 279 s of the 353 s parallel wall.
- **Retry backoff** — tests that take the same time on a RAM disk: 79.7 s,
  concentrated in `girpr::repair_offline` (49.6 s) and `gircap::cli_contract`
  (24.7 s). This is *not* I/O, but it is the second-largest wall item.
- **CPU** — 66.7 s serial, and the floor of the `Z:` run (73.8 s parallel).

The distribution is extremely skewed: **the 25 slowest tests (2 % of the
suite) account for 113 s of the 339 s storage delta (33 %)**, all of them in
`girsync` and `gircap`, and all of them small (0.3–4.5 MB written).

---

## 2. Method

### 2.1 Runners

- `cargo nextest run --workspace` for the normal parallel baseline (one
  process per test, `num-cpus` concurrency).
- A serial sweep invoking every test binary with `--exact <name>` one at a
  time, to attribute wall, CPU and I/O counters per test without neighbours.

### 2.2 Per-test I/O counters

`iotime` (source: `tests/scripts/io-profile/iotime/`) puts each
test process in a Windows **job object** and reads
`JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION` after the tree exits. The
counters cover the test process *and every binary it spawns* (the suites shell
out to `girsync.exe` / `gircap.exe` a lot), so a test is charged for the I/O
its children do.

Columns: wall, exit latency, CPU (kernel+user, in 15.6 ms ticks), read/write/
other operation counts, bytes, process count. The 1,259 rows joined with
nextest's suite metadata give per-suite and per-test tables.

### 2.3 Separating disk from sleep and CPU

The same 1,259 rows were collected twice: once with `TEMP`/`TMP` on the HDD
(`E:`), once on a 2 GB RAM disk (`Z:`). Everything else — binaries, CPU,
sleeps, assertions — is identical, so

```
storage-attributable time ≈ wall(E:) - wall(Z:)
```

is a direct differential measurement, not an estimate. It also cleanly
separates the retry-sleep tests, whose two walls are equal.

### 2.4 Microbenchmarks

`iot-bench` (source: `tests/scripts/io-profile/iot-bench/`) is pinned to redb
4.3.0, the version `tools/sync` resolves, and isolates the exact syscalls the
tests make: `sync_all` of a copied file, redb open/commit/create, file delete,
1 MB copy.

### 2.5 A/B patches

Three speculative fixes were implemented behind environment variables,
rebuilt, and re-run against the 130 `girsync` tests that take ≥ 1 s on `E:`
(298.9 s baseline), then **reverted**:

| variant | change | serial total | vs baseline | note |
|---|---|---:|---:|---|
| V0 | none | 298.9 s | — | |
| V1 | skip `sync_all()` in `copy_one` | 287.4 s | −11.5 s (−4 %) | worst test 17.2 → 10.3 s (−40 %) |
| V2 | V1 + skip no-op meta commit | 275.0 s | −23.9 s (−8 %) | 66 tests faster, 45 slower; ±0.1 s noise |
| V3 | V2 + no cache backups | 285.8 s | −13.1 s | within run-to-run noise; breaks one test |

The HDD has ±10–20 % run-to-run variance on individual tests, so only
aggregate or multi-second deltas are treated as real.

### 2.6 Caveats

- Windows `CopyFileEx` (used by `std::fs::copy`) does not always show up in
  the process's write counters; cache backup/snapshot bytes are therefore
  under-counted even though the kernel does the I/O.
- "Other" operations (≈564 k of the 665 k) are metadata/control operations;
  they are the NTFS directory opens, `stat`s, renames and deletes.
- The RAM-disk run is a *diagnostic*, not a proposed fix: it measures the
  share of the problem that is storage latency.
- The serial sweep total is not the parallel run's wall; do not compare 491 s
  with 353 s as a speed-up.

---

## 3. Where the time goes

### 3.1 By suite (serial, sorted by storage delta)

`serial E` / `serial Z:` are sums of per-test wall; `delta` is what the disk
adds. Only suites ≥ 1 s are shown.

| suite | tests | serial E (s) | serial Z: (s) | storage delta (s) | written MB | I/O ops |
|---|---:|---:|---:|---:|---:|---:|
| girpr::repair_offline | 74 | 57.8 | 53.3 | 4.4 | 0.1 | 29,457 |
| girsync::lazy | 27 | 40.5 | 2.4 | 38.1 | 18.1 | 15,255 |
| girsync::rw_threads | 15 | 38.8 | 4.1 | 34.7 | 5.2 | 70,284 |
| girsync::why | 25 | 35.2 | 1.5 | 33.6 | 16.3 | 13,626 |
| gircap::capture_layout | 29 | 31.4 | 9.5 | 21.9 | 7.5 | 154,672 |
| girsync::stat_trust | 19 | 30.7 | 1.6 | 29.0 | 20.8 | 11,203 |
| girsync::sync | 14 | 30.4 | 1.1 | 29.3 | 8.6 | 8,006 |
| girsync::migration_lock | 11 | 26.3 | 1.4 | 24.9 | 11.9 | 7,444 |
| gircap::cli_contract | 28 | 25.7 | 25.5 | 0.2 | 0.1 | 6,745 |
| girsync::compare | 12 | 20.6 | 1.3 | 19.3 | 11.5 | 7,830 |
| gircap (lib) | 492 | 19.8 | 15.5 | 3.6 | 2.4 | 43,345 |
| girsync::output | 16 | 17.1 | 1.4 | 15.7 | 6.4 | 9,909 |
| girsync::sync_plan | 19 | 15.8 | 2.5 | 13.4 | 107.7 | 20,310 |
| girsync::hash_mode | 18 | 15.4 | 2.0 | 13.4 | 11.5 | 4,881 |
| girsync::compare_self | 12 | 15.0 | 1.0 | 14.0 | 2.6 | 4,676 |
| girsync::helpers | 10 | 9.5 | 0.7 | 8.9 | 1.1 | 3,045 |
| girsync::update | 3 | 8.7 | 0.4 | 8.3 | 3.3 | 2,554 |
| gircap::matrix | 8 | 8.2 | 3.8 | 4.4 | 3.2 | 60,493 |
| gircap::schema | 15 | 6.5 | 5.1 | 1.4 | 4.4 | 80,677 |
| girpr::repair_machinery | 25 | 6.1 | 2.9 | 3.2 | 0.8 | 21,048 |
| girsync::convert | 4 | 6.0 | 0.7 | 5.3 | 3.4 | 1,776 |
| girsync (lib) | 120 | 5.2 | 2.4 | 2.7 | 5.5 | 7,487 |
| girsync::cli_dispatch | 1 | 4.8 | 0.2 | 4.6 | 4.0 | 1,609 |
| girpr (lib) | 148 | 4.6 | 2.3 | 2.1 | 0.5 | 11,018 |

`repair_offline` and `gircap::cli_contract` are at the top of the wall table
but have no storage delta: they sleep.

### 3.2 The 25 worst tests

| # | suite | test | E (s) | Z: (s) | delta (s) | written MB | I/O ops |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | girsync::rw_threads | `through_the_binary::sync_reports_the_same_actions_under_every_rw_setting` | 17.18 | 1.66 | 15.52 | 2.24 | 2,062 |
| 2 | girsync::why | `table::every_pair_state_has_one_classification_and_one_tag` | 11.16 | 0.30 | 10.86 | 1.38 | 627 |
| 3 | girsync::rw_threads | `limiter::every_rw_setting_answers_the_same_question` | 9.10 | 0.72 | 8.37 | 1.18 | 644 |
| 4 | girsync::rw_threads | `through_the_binary::update_hashes_the_same_set_at_every_limit` | 6.02 | 0.56 | 5.46 | 0.76 | 586 |
| 5 | girsync::migration_lock | `lock_no_trust_cached_hashes_is_per_side` | 5.02 | 0.27 | 4.76 | 2.11 | 262 |
| 6 | girsync::cli_dispatch | `run_cli_dispatch_update_compare_sync` | 4.84 | 0.22 | 4.62 | 3.96 | 231 |
| 7 | girsync::sync | `run_sync_keep_extra_spares_extra_dirs_too` | 4.29 | 0.08 | 4.21 | 0.35 | 174 |
| 8 | girsync::stat_trust | `cache_repair::no_trust_size_rehashes_a_stale_row…` | 4.29 | 0.16 | 4.14 | 3.58 | 240 |
| 9 | girsync::update | `run_update_prunes_and_excludes` | 4.10 | 0.17 | 3.93 | 1.63 | 207 |
| 10 | girsync::why | `flags::why_does_not_change_the_exit_code` | 4.02 | 0.19 | 3.83 | 4.48 | 280 |
| 11 | girsync::why | `flags::show_identical_is_default_off_and_independent_of_why` | 3.85 | 0.19 | 3.66 | 2.56 | 298 |
| 12 | girsync::stat_trust | `verdict::no_trust_mtime_clears_a_touched_file_whose_bytes_never_changed` | 3.71 | 0.16 | 3.55 | 4.45 | 208 |
| 13 | girsync::sync | `run_sync_resolves_type_conflicts` | 3.59 | 0.09 | 3.50 | 1.33 | 137 |
| 14 | girsync::hash_mode | `read_count::any_of_still_hashes_when_the_pair_shares_no_algorithm` | 3.63 | 0.14 | 3.49 | 4.43 | 190 |
| 15 | girsync::sync_plan | `as_done::the_rw_limit_does_not_change_what_is_reported` | 3.67 | 0.19 | 3.48 | 0.64 | 378 |
| 16 | girsync::sync | `run_sync_keep_extra_and_missing_only` | 3.47 | 0.09 | 3.38 | 1.35 | 138 |
| 17 | girsync::hash_mode | `read_count::any_of_avoids_backfilling_an_algorithm_the_pair_already_shares` | 3.41 | 0.15 | 3.26 | 4.45 | 198 |
| 18 | girsync::lazy | `sync::a_folder_prunes_its_own_cache_before_the_planner_sees_it` | 3.37 | 0.20 | 3.16 | 1.41 | 175 |
| 19 | girsync::migration_lock | `lock_hash_none_tracks_without_digests` | 3.15 | 0.13 | 3.02 | 2.31 | 122 |
| 20 | girsync::sync | `a_cache_row_for_a_path_deleted_after_the_run_is_dropped` | 3.08 | 0.10 | 2.98 | 1.37 | 147 |
| 21 | girsync::sync_plan | `as_done::a_multi_file_parallel_copy_reports_the_same_set_whatever_the_order` | 3.86 | 0.94 | 2.93 | 105.49 | 290 |
| 22 | girsync::helpers | `every_file_lands_in_exactly_one_bucket` | 3.00 | 0.14 | 2.86 | 0.28 | 142 |
| 23 | girsync::migration_lock | `lock_cache_writes_merge_unless_stat_changed` | 2.98 | 0.21 | 2.77 | 0.44 | 153 |
| 24 | girsync::update | `update_fills_the_gap_that_makes_an_audit_of_the_same_folder_fail` | 2.89 | 0.15 | 2.75 | 1.37 | 153 |
| 25 | girsync::lazy | `compare::stat_equal_pair_costs_only_the_side_missing_the_algorithm` | 2.86 | 0.16 | 2.70 | 1.60 | 147 |

Note test 21: it writes **105 MB** and costs 2.9 s. The others write under
5 MB and cost 3–17 s. Bytes are not the problem; the 100–2,000 small
operations each test performs are.

### 3.3 The sleep tests (no storage delta)

| suite | tests | serial E (s) | serial Z: (s) |
|---|---:|---:|---:|
| girpr::repair_offline — three 15.1 s tests | 3 | 45.3 | 45.3 |
| girpr::repair_offline — other waits | 4 | 5.8 | 4.3 |
| gircap::cli_contract — 12 × ~2.1 s | 12 | 24.8 | 24.7 |
| gircap — `http::tests::a_transport_failure_is_retried…` | 1 | 4.1 | 4.1 |

---

## 4. What the tests actually do

### M1. Every command creates or opens a 1 MB redb cache — the dominant cost

`open_db` (`tools/sync/src/cache/open.rs:49`) opens `<root>/girpr-cache` for
both sides of a run (`tools/sync/src/commands/sync/mod.rs:136-198`), and
`CacheDb::init_meta` (`tools/sync/src/cache/db.rs:83`) writes a meta row and
commits even when the row already exists.

End-to-end, on empty trees, with the real binary:

| operation | `E:` HDD | `Z:` RAM disk |
|---|---:|---:|
| `girsync sync` on two empty dirs, **first** run (creates both caches) | **1,421 ms** | 38 ms |
| same, **second** run (opens both caches, takes backups) | **821 ms** | 38 ms |
| same, `--dry-run` (writes no cache) | **38 ms** | 31 ms |
| 50 × 1 KB files, `sync` that copies them | 3,530 ms | 190 ms |
| the same tree, second `sync` that copies nothing | 760 ms | 60 ms |

Microbench per operation:

| operation | `E:` | `Z:` | ratio |
|---|---:|---:|---:|
| `create` + write 512 B + `sync_all` | 60–75 ms | 0.5 ms | ~140× |
| `create` + write 512 B, no fsync | 0.3 ms | 0.3 ms | — |
| redb open, no commit | 160–235 ms | 9 ms | ~20× |
| redb open + empty commit | 220–260 ms | 9.5 ms | ~25× |
| redb open + meta insert + commit | 230–275 ms | 10 ms | ~25× |
| redb **fresh create** + meta commit | 410–500 ms | 3 ms | ~150× |
| redb 10-row commit, `Durability::Immediate` | 40–90 ms | 0.6 ms | ~100× |
| redb 10-row commit, `Durability::None` | 0.04 ms | 0.04 ms | — |
| copy a 1 MB file | 1.3 ms | 1.3 ms | — |
| delete a file | 0.5 ms | 0.2 ms | 2× |
| allocate 1 MB + `sync_all` | 60–85 ms | 0.35 ms | ~200× |

Ranges are across three runs of `iot-bench` (pinned to redb 4.3.0, the version
`tools/sync` resolves), each with a flush and pause between phases. The
device's writeback state still moved the open/commit figures by up to 2×
between runs. Treat the table as orders of magnitude; the end-to-end and A/B
figures are the primary evidence.

Two conclusions:

1. A test that runs `sync`/`update`/`compare` pays **~0.8–1.4 s per command**
   on this HDD before doing any work of its own, and pays it per side.
   `girsync` has 326 tests; the suite's 320 s serial is almost exactly this
   fixed cost multiplied by the number of commands and scenarios.
2. The cost is in the *open/create path and its fsyncs*, not in the bytes:
   a cold fresh DB is 0.4–0.5 s while allocating and fsyncing a 1 MB file is
   60–85 ms, and copying that file is 1.3 ms. `Durability::None` makes commits
   free but does not touch the open cost, so tuning commit durability alone is
   not enough.

The redb file's initial region is 1 MB (a 3-row cache is 1,056,768 bytes), so
every backup/snapshot copy is a 1 MB copy even when the cache has three rows.

### M2. `init_meta` commits unconditionally

`db.rs:131` — `txn.commit()` runs on every writable open. On the HDD a commit
that changed nothing added **10–100 ms** across settled runs (and up to
160 ms when the device still had writeback queued): the cost is the flush, not
the row. The A/B run is at the upper end of that: skipping the commit when
nothing changed saved **12.4 s over 130 `girsync` tests (4 %)**, with no test
failures. This is a 2-line fix (track whether the row was inserted/rewritten;
commit only then).

### M3. One `sync_all()` per copied file, then a reopen and a re-read

`copy_one` (`tools/sync/src/commands/sync/apply.rs:536`) does:
stream copy → `fout.sync_all()` → `drop` → reopen for `set_modified` →
`metadata` → re-read the whole destination to hash it.

- The fsync is **~70 ms per file** on this HDD (0.5 ms on `Z:`). It is the
  difference between the 3.53 s copy run and the 0.19 s copy run for 50 files.
- The reopen/set_modified is cheap (0.07 ms) — not worth changing.
- The verify re-read is deliberate (`apply.rs:472-505`: it hashes what is on
  disk, after the flush). It does not need the flush to be correct — a read
  through the page cache returns the bytes just written — but removing the
  flush weakens the "actually reached the disk" claim.
- A/B measurement: skipping this fsync saved 11.5 s over 130 tests (4 %), but
  **40 % on the copy-heavy test** (17.18 → 10.30 s). The suite copies a few
  hundred files per test at most, so the aggregate is modest; the per-test
  effect on `rw_threads` is not.

### M4. Cache backup and snapshot copies on every run

Before a non-dry `sync`, `tools/sync/src/commands/sync/mod.rs:136-149` calls
`backup_db` for both sides and `snapshot_old` for dst; `open_db` also backs up
by default. `backup_db`/`snapshot_old` (`open.rs:131-168`) are whole-file
`std::fs::copy` calls with timestamped names, and nothing prunes them during a
run. A 4-run experiment left 4 backups per side, plus 4 old-state snapshots.

A/B: removing them did **not** reduce wall time here (285.8 s vs 275.0 s,
within noise), because a 1 MB copy is ~1.3 ms. They matter for write volume and
for production hygiene, not for this HDD's latency. One test
(`migration_lock::lock_backups_and_snapshots_created`) asserts they exist and
must keep doing so.

### M5. Repeated fixtures and per-setting loops

Many tests rebuild the same tree and the same caches once per flag/row:

- `rw_threads`: `flag_spellings()` has 6 variants
  (`tools/sync/tests/rw_threads/main.rs:181`), and each iteration calls
  `fixture(&t)` (`main.rs:117`) on a fresh `TempDir`, then runs the command.
  6 × (fixture + 2 fresh caches) = the 17.2 s worst test and the 9.1 s limiter
  test.
- `why::table`: `for row in rows()` (`tools/sync/tests/why/table.rs:218`),
  one fresh pair and cache set per row (11 rows); 11.2 s for 627 I/O
  operations.
- `capture_layout`: 18 tests call `capture_ok()`
  (`tools/capture/tests/capture_layout.rs:157`), each doing a full capture of
  the same healthy mock (`World::all_shapes()`) into a fresh scratch tree;
  31.4 s serial, 21.9 s of it storage.
- `sync_plan`: several tests re-run the same multi-file copy under different
  `--rw` limits.

This is the "deduplicate work between tests" part: the assertion is about the
result, but the test pays for a full fixture and a full cache each time.

### M6. Deliberate retry backoff

- `src/repair/file.rs:348-402`: up to 5 attempts with
  `sleep(Duration::from_secs(attempt))`, and each attempt has its own 5× chunk
  retry inside `stream_chunk_to` (`tests/repair_offline.rs:2299` documents the
  ~125 s worst case). Three tests spend 15.1 s each doing nothing but waiting.
- `src/hyp.rs:269-362`: 5 attempts with `sleep(secs = attempt)`.
- `tools/capture/src/http/mod.rs:219-229`: 500 ms base, doubling; the
  `cli_contract` tests point at `127.0.0.1:1` with the default retries, so each
  pays ~1.5 s of real backoff plus startup → 12 tests × 2.1 s.

`gircap` already has the right seam — `Sleep` / `RecordingSleep`
(`http/mod.rs:260-322`) — and its unit tests use it. The subprocess tests do
not, and `girpr` has no seam at all.

### M7. Scratch-tree churn

`TempDir` (`crates/tempdir/src/lib.rs`) creates and removes a fresh tree per
test (`<temp>/girpr-test/<prefix>-<tag>-<pid>-<nanos>-<seq>`), and `Drop`
does `remove_dir_all`. That is the right design; the cost is only
0.54 ms/file on this disk and is included in the numbers above. Not a target.

---

## 5. Recommendations, ranked

The ranks are by measured wall-time saved, with the evidence and the exact
place to change. Savings are serial-wall unless stated; the parallel share is
roughly the same fraction of the 279 s of storage latency in the parallel run.

### R1 — Stop creating a real redb cache per test case (largest)

**What:** give tests a cache that does not need a 1 MB file created, fsynced
and re-opened for every command, unless the test is *about* the cache file.

**Options, in order of preference:**

1. **In-memory cache for semantic tests.** `CacheDb::open_temp` already exists
   (`tools/sync/src/cache/db.rs:152`, `InMemoryBackend`). Add a test-only
   switch at the `open_folder_cache` boundary (env var read once, or an
   explicit `CacheBackend` argument threaded from the test helpers) so suites
   that assert *sync results* — `sync`, `lazy`, `why`, `compare`,
   `compare_self`, `hash_mode`, `stat_trust`, `output`, `sync_plan` — run
   against memory. Keep the real file for `migration_lock`, `convert`, the
   cache tests and anything that asserts `girpr-cache*` on disk. The dry-run
   experiment (38 ms vs 1,421 ms) is the size of the prize.
2. **Seed from a template.** If the file must exist, check in (or build once
   at suite start) an empty current-version `girpr-cache` and `fs::copy` it
   into the scratch tree: ~2 ms instead of a 400–500 ms fresh create, and it
   skips the creation fsyncs. The open that follows still costs the usual
   ~200 ms; the cache is then a normal file for the rest of the test.
3. **Reuse one cache pair across loop iterations** (see R2): reset rows or
   point the next scenario at the same roots instead of rebuilding.

**Expected:** most of `girsync`'s 300 s storage delta — conservatively
**150–250 s serial**, and a large share of the 279 s parallel storage cost.
This is the only recommendation that addresses the measured fixed cost of
0.8–1.4 s per command.

### R2 — Deduplicate fixtures and scenario loops

**What:** build the tree once per test, not once per setting/row; capture one
healthy bundle and let layout tests read it.

- `rw_threads`: build the 102-file fixture once, reset only the files a
  scenario changes, and reuse the caches; or assert the per-`--rw` equivalence
  on a shared pair with the cache reset between runs rather than a new
  `TempDir` each time. Target: 38.8 s → under 10 s.
- `why::table`: same for the 11 rows; the loop is a pure lookup table, and
  each row's caches are fresh. Target: 35.2 s → under 8 s.
- `capture_layout`: do the live capture **once** into a fixture bundle, then
  have the 18 layout tests read that bundle; keep one test that re-captures
  and compares. Target: 31.4 s → ~10 s (the `Z:` floor is 9.5 s).
- `sync_plan`: reuse the copied tree for the different `--rw` limits; the
  105 MB copy is already cheap, the repeated scans are not.

**Expected:** 60–100 s serial on its own; it overlaps with R1, so count it
once. It also removes the most disk operations (fixture creation, cache
creation and `remove_dir_all`).

### R3 — Drop or batch the per-file `sync_all()` in `copy_one`

**What:** `tools/sync/src/commands/sync/apply.rs:536`. Either remove the
flush (the verify re-read still sees the bytes through the page cache), or
make it a policy: flush every N files / at end of run, or only when a
`--durable` flag is set. Keep a documented note that the "reached the disk"
claim then applies to the run, not the file.

**Evidence:** ~70 ms per file on `E:` vs 0.5 ms on `Z:`; A/B −11.5 s over
130 tests (−4 %), −6.9 s (−40 %) on the copy-heavy test.

**Expected:** 10–30 s serial, up to 40 % on copy-heavy tests; no correctness
change (the size/mtime/hash verification is unchanged).

### R4 — Commit meta only when it changed

**What:** `tools/sync/src/cache/db.rs:83-133`. Track whether the meta row was
inserted or rewritten; skip `txn.commit()` when it was not. (A write-free
commit buys nothing: the row did not change.)

**Evidence:** 10–100 ms per no-op commit on `E:`; A/B −12.4 s over 130 tests
(−4 %); zero test failures in the A/B run.

**Expected:** 5–15 s serial, a few seconds parallel. The A/B delta is close to
the run-to-run noise, so the per-commit microbenchmark is the stronger
evidence; land it because it is cheap and correct, not for a large win.

### R5 — Remove the fixed retry sleeps from tests

**What:** 79.7 s of the serial wall is backoff, unchanged on a RAM disk.

- Give `girpr` the same injectable `Sleep` seam `gircap` has
  (`tools/capture/src/http/mod.rs:260-322`), and pass a no-sleep
  implementation from the tests that assert *how many* requests were made or
  what the summary says. `src/repair/file.rs:402`, `src/hyp.rs:307/362`,
  `src/sophon.rs` are the call sites.
- For the subprocess-based `cli_contract` tests, a test-only environment knob
  (`GIRPR_RETRY_BASE_MS=0`, read where the policy is built) is enough; the
  binary already accepts configuration through the environment.
- For the `gircap` HTTP unit test, `tokio::time::pause()` advances the clock
  instead of waiting 4 s.

**Expected:** up to 80 s serial; in parallel, 20–30 s of wall, because a 15 s
sleeping test still occupies one of the four workers. These tests also have
the worst signal-to-noise in CI (a slow machine looks like a policy change).

### R6 — Cache backups: keep, but bound them (low priority)

**What:** `backup_db`/`snapshot_old` copy the whole 1 MB cache on every run
and never prune within a run. Keep the feature (one test asserts it), but
retain one backup per side per run, skip the copy when the cache is unchanged,
and do not create both a backup and an old-state snapshot of the same bytes.

**Evidence:** removing them was inside noise here (285.8 s vs 275.0 s) and
broke `lock_backups_and_snapshots_created`. This is write-volume and
production-hygiene work, not a wall-time fix on this machine.

### R7 — A fast scratch root as an opt-in developer/CI mode (not a fix)

The differential (353 s → 74 s) shows that a RAM-backed `TEMP` is the single
biggest local speed-up available today. Since "change disk" is not a code fix,
the recommendation is only to make it **possible and documented**: a
`GIRPR_TEST_TEMP`/`TMPDIR` contract honoured by `TempDir` (it already uses
`std::env::temp_dir()`, so this is mostly documentation and a guard that
refuses a path on a slow volume unless asked), plus a nextest profile that
runs the I/O-heavy suites serially on the fast root and the CPU suites in
parallel. It also serves as the verification harness for R1–R5: if a fix is
real, the `E:` time approaches the `Z:` time.

---

## 6. Estimated combined effect

Applying R1–R5, with the measured deltas and the `Z:` floors as bounds:

| stage | serial wall | parallel wall |
|---|---:|---:|
| today (`E:`) | 491 s | 353 s |
| after R3 + R4 (measured: −24 s on 130 tests) | ~465 s | ~335 s |
| after R1 + R2 (cache and fixture work removed from girsync/gircap) | ~180–220 s | ~150–180 s |
| after R5 (sleeps removed) | ~110–150 s | ~110–140 s |
| floor if storage were free (`Z:` measurement) | 152 s | 74 s |

The floor is not zero because the suite does real CPU work (67 s serial) and
some tests legitimately exercise the cache/backup paths. The largest single
uncertainty is R1's exact share; the A/B in §2.5 could not test it without
changing cache semantics, so it is estimated from the 0.8–1.4 s per-command
fixed cost and the number of commands per test.

---

## 7. Reproducing

The scripts live in `tests/scripts/io-profile/` (see its `README.md`), and the
recorded outputs are under `target/io-profile/`. `target/` is git-ignored, so
the outputs survive a temp-directory sweep without being committed.

- Serial sweep and analysis: `tests/scripts/io-profile/run-sweep.ps1`. It
  lists the tests with `cargo nextest list`, builds `iotime/`, runs every test
  serially inside a job object, and writes the tables. Run it once with
  `%TEMP%` on the disk under test and once with `-TempRoot Z:\tmp`.
- Per-test accountant: `tests/scripts/io-profile/iotime/`
  (`iotime <tasks.tsv> [concurrency]`; output `<tasks.tsv>.csv`).
- Microbenchmarks: `tests/scripts/io-profile/iot-bench/`
  (`iot-bench <scratch-dir>`). Recorded runs:
  `target/io-profile/iot-bench/{e,z}-{1,2,3}.txt`.
- Recorded per-test tables:
  `target/io-profile/analysis/{tests-ranked,by-suite,by-crate}.csv` and
  `target/io-profile/analysis/summary.txt`.
- Recorded differential (HDD minus RAM disk):
  `target/io-profile/analysis/{z-vs-e-tests,z-vs-e-suites}.csv`. A rerun via
  `analyze.ps1 -CompareCsv` writes the same data as
  `compare-tests.csv`/`compare-suites.csv`.
- Raw `Z:` sweep: `target/io-profile/tasks-z.csv` (1,259 rows).
- Baselines: `target/io-profile/baselines/baseline-elapsed.txt` (352.95 s
  parallel), `sweep-elapsed.txt` (491.74 s serial), `sweep-z-elapsed.txt`
  (152.74 s serial RAM disk), `nextest-z.txt` (73.83 s parallel RAM disk,
  1,251 passed, 8 skipped).

The temporary A/B patches were reverted. The only changes are this report and
the reproduction scripts under `tests/scripts/io-profile/`; the recorded
outputs are ignored because they live under `target/`.
