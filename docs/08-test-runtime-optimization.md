# Test runtime optimization review (all workspace projects)

Date: 2026-11-17

Scope: every crate in the workspace — the main product (`girpr`), `girsync`
(`tools/sync`), `gircap` (`tools/capture`), plus the workspace member
`sled2redb` (`tools/sled2redb`) and the two support crates (`girpr-log`,
`girpr-tempdir`). Build time is **not** in scope. The review measures wall time,
CPU time and filesystem work, and recommends ways to **reduce or sidestep IO**
rather than relocate it. The existing test-only hooks
(`GIRSYNC_TEST_DURABILITY=none`, `GIRSYNC_TEST_NO_FSYNC=1`,
`GIRPR_TEST_NO_FSYNC=1`, installed by `.config/nextest.toml` setup scripts) were
active for every measurement below.

---

## 1. Executive summary

* The default `cargo nextest run --workspace` executes **2,256 tests in 177 s**
  of parallel wall clock (13 tests are `#[ignore]`d). The sum of the individual
  test durations is **692 s**, so the suite already gets roughly 3.9× from
  parallelism — but see the contention note below.
* By summed test time: **girsync 346 s**, **gircap 193 s**, **girpr 119 s**,
  **sled2redb 33 s** (only 16 convert tests), log+tempdir < 2 s combined.
  girsync is the clear priority.
* A serial, job-object-based sweep of the **120 slowest tests** (41% of all test
  time) took **116.4 s on the HDD** and **32.7 s on the unused 1 GB RAM disk**.
  **83.7 s — 72% — is storage-attributable.** In several girsync suites the
  figure is 90–97%.
* The microbenchmark explains why: on the HDD a `sync_all` costs **169 ms**
  (vs 0.42 ms on RAM), a fresh redb cache create **276–425 ms** (vs 2.2–2.5 ms),
  and a redb open + meta commit **211–545 ms** (vs 9 ms). The existing hooks
  removed the explicit `sync_all` calls, but **redb's open/create itself remains
  a ~200–500 ms disk-bound operation** and is repeated many times per test.
* Parallel IO queueing is severe: the 120 slow tests sum to 282 s under nextest
  but only 116 s run serially — a 2.4× inflation from contending on the same
  HDD. Reducing IO therefore improves both serial time *and* parallel scaling.
* The biggest, concrete wins are: (1) shrink/reuse the oversized girsync
  fixtures and stop opening file-backed caches for pure logic assertions;
  (2) use `GIRSYNC_TEST_CACHE=memory` consistently; (3) consolidate the
  capture-and-verify churn in `gircap::capture_layout`/`schema`/`matrix`;
  (4) reduce binary spawns and trace-log IO in girpr; (5) cut the 8-conversion
  sled2redb argument-spelling test. The ignored bench/bridge tests are large
  but cost nothing in the normal gate (see §7).

---

## 2. How this was measured

| Step | Tool / command | What it yields |
|---|---|---|
| Test inventory | `cargo nextest list --workspace --message-format json` | 2,269 listed tests, 13 ignored |
| All-test timing | `cargo nextest run --workspace --message-format libtest-json-plus` (`NEXTEST_EXPERIMENTAL_LIBTEST_JSON=1`) | `exec_time` per test, parallel wall 177 s |
| Per-test IO/CPU | `tests/scripts/io-profile/iotime` (Windows job object, aggregates children), serial sweep of the 120 slowest tests, with the GIR hooks exported into the job | wall, CPU, read/write ops and bytes per test |
| Differential | same sweep with `TEMP`/`TMP`/`TMPDIR` = `Z:\tmp`, compared via `tests/scripts/io-profile/analyze.ps1` | storage-attributable time per test/suite |
| Primitive costs | `tests/scripts/io-profile/iot-bench` run once with scratch on `E:\Symlinks\Temp` (HDD) and once on `Z:\` (RAM) | per-operation fsync / redb / copy costs |
| Bench/stress | `cargo nextest run --workspace --run-ignored only` | timing of the ignored bench/bridge tests |

Raw data is under `target/`: `nextest-list.json`, `nt-full.jsonl`, `times.csv`,
`top-tasks.tsv[.csv]`, `ram-top-tasks.tsv[.csv]`, `io-analysis/*.csv`,
`nt-ignored.jsonl`. Re-running `tests/scripts/io-profile/run-sweep.ps1` produces
the same tables for the whole suite.

Caveats: `iotime` counts `std::fs::copy` bytes unreliably (the job's write
counter is a lower bound), and `wait = wall − CPU` includes deliberate sleeps.
The differential sweep separates the two. The nextest `exec_time` figures were
collected under the default parallel load and are inflated by IO contention;
the sweep figures are serial.

---

## 3. IO baseline: why the HDD hurts even with the hooks enabled

Per-operation cost (ms), HDD `E:` vs RAM `Z:`:

| Operation | HDD | RAM | Ratio |
|---|---:|---:|---:|
| create + write 512 B + `sync_all` | 169.4 | 0.42 | ~400× |
| create + write 512 B (no fsync) | 0.60 | 0.16 | ~3.7× |
| reopen + `set_modified` | 0.29 | 0.13 | ~2.2× |
| `remove_file` | 0.47 | 0.14 | ~3.5× |
| redb fresh create, durable commit | 425.3 | 2.49 | ~171× |
| redb fresh create, `Durability::None` | 276.5 | 2.21 | ~125× |
| redb open + meta commit, Immediate | 545.4 | 9.61 | ~57× |
| redb open + meta commit, none | 211.3 | 9.17 | ~23× |
| redb open, no commit | 258.4 | 8.70 | ~30× |
| redb 10 commits × 10 rows, none | 0.11 | 0.03 | ~3× |
| file create 1 MB + `sync_all` | 51.1 | 0.44 | ~116× |
| `copy` 1 MB | 1.48 | 1.48 | 1× |

Interpretation:

1. The `*_NO_FSYNC` hooks are worth keeping — they remove ~169 ms per explicit
   `sync_all`. They do **not** remove redb's open/create, which is the new
   dominant cost (200–500 ms each, even with `Durability::None`).
2. Every `girsync::cache::open_db` call is expensive: it takes a lock sidecar,
   opens the redb file, runs `init_meta` (a commit), reads the folder and applies
   the copy bound. Tests that call it repeatedly pay this each time.
3. Bulk sequential `copy` is cheap (cached); the penalty is **many small
   metadata/create/open operations**, not bytes moved.

---

## 4. Overall test-time ranking

### By crate (nextest `exec_time`, parallel run)

| Crate | Tests | Sum | Average | Max |
|---|---:|---:|---:|---:|
| girsync | 631 | 346.4 s | 549 ms | 10.99 s |
| gircap | 883 | 193.2 s | 219 ms | 4.60 s |
| girpr | 691 | 118.7 s | 172 ms | 1.63 s |
| sled2redb | 17 | 32.8 s | 1,928 ms | 7.25 s |
| girpr-log | 23 | 1.0 s | 42 ms | 59 ms |
| girpr-tempdir | 11 | 0.5 s | 44 ms | 58 ms |

### By suite (top 15 by summed time)

| Suite | Crate | Tests | Sum | Avg |
|---|---|---:|---:|---:|
| girsync (lib) | girsync | 343 | 100.2 s | 292 ms |
| girsync::sync | girsync | 48 | 84.7 s | 1,764 ms |
| gircap::capture_layout | gircap | 37 | 53.1 s | 1,434 ms |
| gircap::schema | gircap | 21 | 53.0 s | 2,522 ms |
| girpr (lib) | girpr | 458 | 43.4 s | 95 ms |
| gircap (lib) | gircap | 725 | 34.5 s | 48 ms |
| sled2redb::convert | sled2redb | 16 | 32.7 s | 2,046 ms |
| girsync::compare | girsync | 14 | 31.2 s | 2,225 ms |
| girsync::rw_threads | girsync | 15 | 30.8 s | 2,055 ms |
| gircap::cli_contract | gircap | 80 | 21.7 s | 271 ms |
| gircap::matrix | gircap | 10 | 20.8 s | 2,080 ms |
| girpr::repair_machinery | girpr | 28 | 16.5 s | 590 ms |
| girsync::sync_plan | girsync | 19 | 15.3 s | 806 ms |
| girsync::symlink | girsync | 34 | 14.9 s | 437 ms |
| girsync::output | girsync | 21 | 13.4 s | 639 ms |

### 25 slowest individual tests (parallel `exec_time`)

| Test | Crate/suite | Time |
|---|---|---:|
| `sync_reports_the_same_actions_under_every_rw_setting` | girsync::rw_threads | 10.99 s |
| `the_walk_counts_what_it_saw_and_the_map_counts_what_it_kept` | girsync::output | 9.99 s |
| `run_sync_backup_retain_bounds_each_prefix` | girsync::sync | 7.72 s |
| `the_flags_are_accepted_on_either_side_of_the_paths` | sled2redb::convert | 7.25 s |
| `run_sync_copies_the_cache_before_ignore_cache_removes_it` | girsync::sync | 6.69 s |
| `run_sync_excludes_a_case_renamed_cache_directory` | girsync::sync | 6.39 s |
| `every_rw_setting_answers_the_same_question` | girsync::rw_threads | 5.91 s |
| `run_sync_with_no_backup_still_removes_the_cache` | girsync::sync | 5.83 s |
| `a_forced_conversion_replaces_the_destination_and_leaves_no_temporary` | sled2redb::convert | 5.55 s |
| `an_uncovered_record_fails_before_the_folder_side_is_resolved` | girsync::compare | 5.47 s |
| `a_failed_flush_aborts_before_the_copy_is_cached` | girsync::sync | 5.25 s |
| `a_failed_copy_discards_only_the_uncommitted_cache_tail` | girsync::sync | 5.22 s |
| `a_dry_run_is_unaffected_by_the_rw_limit` | girsync::rw_threads | 5.11 s |
| `a_backup_record_is_offered_no_repopulation` | girsync::compare | 4.88 s |
| `run_sync_dry_run_with_ignore_cache_leaves_caches_intact` | girsync::sync | 4.87 s |
| `a_failed_forced_conversion_leaves_the_previous_cache_intact` | sled2redb::convert | 4.85 s |
| `an_explicit_file_is_captured_and_the_matrix_is_unchanged` | gircap::matrix | 4.60 s |
| `a_nested_case_rename_rekeys_the_destination_map_and_cache` | girsync::sync | 4.41 s |
| `run_sync_with_no_backup_leaves_no_copy` | girsync::sync | 4.34 s |
| `a_manifest_without_the_gzip_threshold_does_not_open` | gircap::schema | 4.22 s |
| `run_sync_backs_up_src_and_snapshots_dst` | girsync::sync | 4.08 s |
| `checksums_cover_every_file_and_re_hash_to_what_they_record` | gircap::schema | 3.98 s |
| `the_converter_reports_json_records_on_stdout` | sled2redb::convert | 3.93 s |
| `a_budget_turns_a_satisfiable_class_into_over_budget` | gircap::matrix | 3.91 s |
| `run_update_then_compare_equal` | girsync::compare | 3.74 s |

---

## 5. IO attribution of the slow tests (HDD vs RAM)

Serial sweep of the 120 slowest tests, with the GIR hooks active:

* HDD: **116.4 s wall**, 38.1 s CPU, 78.5 s wait, 85.5 MB read, 59.5 MB write,
  **769,740 filesystem operations**.
* RAM: **32.7 s wall**.
* Delta: **83.7 s (72%) storage-attributable**.

| Crate | Tests | HDD wall | CPU | Wait | Storage delta |
|---|---:|---:|---:|---:|---:|
| girsync | 58 | 73.3 s | 11.1 s | 62.2 s | ~60 s (dominant) |
| gircap | 52 | 31.2 s | 25.1 s | 6.3 s | ~10 s |
| sled2redb | 8 | 11.5 s | 1.6 s | 9.9 s | 8.7 s |
| girpr | 2 | 0.3 s | 0.2 s | 0.2 s | ~0 |

By suite (top storage deltas):

| Suite | HDD wall | RAM wall | Delta | Wait % on HDD |
|---|---:|---:|---:|---:|
| girsync::sync | 24.9 s | 1.4 s | **23.5 s** | 95% |
| girsync::compare | 10.2 s | 0.7 s | **9.4 s** | 97% |
| sled2redb::convert | 11.5 s | 2.8 s | 8.7 s | 86% |
| girsync::helpers | 5.8 s | 0.4 s | 5.4 s | 96% |
| girsync (lib) | 5.3 s | 0.3 s | 5.0 s | 97% |
| gircap::capture_layout | 9.9 s | 5.4 s | 4.5 s | 23% |
| gircap::schema | 9.8 s | 5.6 s | 4.3 s | 28% |
| girsync::output | 7.1 s | 3.1 s | 4.0 s | 50% |
| girsync::rw_threads | 6.3 s | 2.6 s | 3.7 s | 40% |
| girsync::migration_lock | 3.3 s | 0.3 s | 3.0 s | 95% |

Largest individual deltas (HDD → RAM):

| Test | HDD | RAM | Delta |
|---|---:|---:|---:|
| `the_walk_counts_what_it_saw_and_the_map_counts_what_it_kept` | 7.12 s | 3.12 s | 4.00 s |
| `run_sync_backup_retain_bounds_each_prefix` | 2.72 s | 0.08 s | 2.64 s |
| `run_sync_backs_up_src_and_snapshots_dst` | 2.67 s | 0.07 s | 2.60 s |
| `sync_reports_the_same_actions_under_every_rw_setting` | 2.58 s | 0.30 s | 2.28 s |
| `the_flags_are_accepted_on_either_side_of_the_paths` | 3.60 s | 1.38 s | 2.22 s |
| `cache::open::tests::an_earlier_runs_copy_is_an_ordinary_copy_again` | 2.29 s | 0.09 s | 2.20 s |
| `an_uncovered_record_fails_before_the_folder_side_is_resolved` | 2.37 s | 0.18 s | 2.19 s |
| `run_sync_copies_the_cache_before_ignore_cache_removes_it` | 2.24 s | 0.10 s | 2.13 s |
| `lock_record_side_is_read_only` | 2.08 s | 0.06 s | 2.02 s |
| `as_done::a_multi_file_parallel_copy_reports_the_same_set_whatever_the_order` | 1.89 s | 0.12 s | 1.76 s |

Two distinct populations emerge:

* **girsync's file-backed cache and fixture IO**: near-total wall collapse on
  RAM, CPU < 0.15 s. These tests are almost purely waiting on the disk; the fix
  is to do less disk work, not to optimize code.
* **gircap's capture/verify tests**: 25 s CPU across 52 tests and 200k–400k
  `OtherOps` (metadata syscalls) per test, but only ~30–40% delta. These are
  CPU/syscall-bound; the fix is fewer captures, smaller trees and fewer verifies.

---

## 6. Project findings and recommendations

### 6.1 girsync (highest priority)

**A. Oversized fixtures that assert nothing about their content**

1. `tests/output.rs:840–1018` —
   `the_walk_counts_what_it_saw_and_the_map_counts_what_it_kept` creates
   `HEARTBEAT_ENTRIES − 1 = 1,999` one-byte files (`HEARTBEAT_ENTRIES = 2000` is
   a production constant at `src/scan.rs:26`) solely to trigger a walk heartbeat.
   Measured 7.12 s on HDD / 40,685 IO ops, only 3.55 s CPU.
   **Fix:** add a test-only env knob (e.g. `GIRSYNC_TEST_HEARTBEAT_ENTRIES=8`) to
   lower the period inside the spawned child, then build 8 files. ~99.6% of the
   fixture IO disappears; the "counts what it saw" assertions are unchanged.

2. `tests/sync_plan/as_done.rs:145–179` — 24 files of 1 KB…~920 KB (~11 MB),
   copied again by the binary (~22 MB total) only to assert the *set* of copy
   events. Content and size are never read back (166–178). Measured 21.1 MB
   written, 1.76 s storage delta.
   **Fix:** use 24 (or 8) files of 1–10 KB. With `--rw-threads 4` the completion
   order already diverges; the multiset assertion is size-independent.

3. `tests/rw_threads/main.rs:116–132` — a 96-file × 2-side, 512 B fixture is
   rebuilt ~12× (`limiter.rs:114,154,202`, `through_the_binary.rs:39` ×6).
   `sync_reports_the_same_actions_under_every_rw_setting` (10.99 s nextest,
   2.28 s storage delta) and `every_rw_setting_answers_the_same_question`
   (5.91 s) are the two biggest consumers; `limiter.rs:119` also requires
   `hashed > 50` per side, so each pass re-hashes >100 files.
   **Fix:** reduce `PAD` count from 96 to 16 (still > the widest 8-worker limit,
   so "more files than workers" holds), lower the `hashed > 50` guard
   proportionally, and build the fixture once where a test performs several
   passes. This also cuts the ~1,400 re-hashes.

**B. File-backed cache opened where semantics are asserted**

4. `tests/sync.rs:95–113` — `run_binary_raw()` does **not** set
   `GIRSYNC_TEST_CACHE=memory`, while `run_binary()` (67–88) does. ~20 callers
   therefore create a full 1 MB redb file + lock sidecar + meta commit per
   binary run on the HDD. With `Durability::None`, a fresh create still costs
   ~276 ms here (§3).
   **Fix:** default `run_binary_raw` to the memory backend; opt out only in the
   few tests that assert on-disk cache bytes (e.g.
   `a_nested_case_rename_rekeys_the_destination_map_and_cache` ~820, and the
   real-fsync case at ~1763 which already sets `GIRSYNC_TEST_NO_FSYNC=0`).
   Remove the redundant per-caller env entries (486, 536, 593, 728–733, …).

5. `tests/helpers.rs` uses `opts()` 20× and no `*_mem` variants; `hash_mode` has
   both `asking()` (file) and `asking_mem()`. The in-memory helpers
   (`opts_mem`, `pair_mem`, `sync_mem`, `compare_mem`, `recs_of`'s memory-first
   lookup at `common/mod.rs:510–516`) already exist precisely for tests that
   assert rows/verdicts rather than cache-file bytes.
   **Fix:** audit each suite and switch every test that only reads rows through
   `recs_of`/`hashed` to the memory backend. Keep `File` only where backup
   siblings or raw cache bytes are the assertion (cache-focused cases in
   `compare`, `compare_self`, `update`, `migration_lock`).

**C. Duplicated work inside a test**

6. `src/cache/open.rs` publish tests at 2208, 2367, 2456, 2502 set
   `PUBLISH_PAUSED` and `pause_publish()` (`1064–1068`) performs a fixed
   **500 ms sleep** before the test's `join()` — ~2 s of pure sleep across four
   tests. `src/cache/write.rs:586` has another explicit 150 ms sleep.
   **Fix:** replace the fixed sleep with a one-shot signal/condvar: block in
   `pause_publish()` until the main test says it has observed the claim, then
   release. Deterministic race window, zero dead time. Same pattern for the
   150 ms interval test.

7. `pair_with(..., warmed: true)` (`common/mod.rs:624–667`) runs `cmd_update`
   per side (two file-backed opens) even when a test only needs files on disk.
   **Fix:** prefer `warmed: false` plus targeted hashing, or `pair_mem`.

**D. Redundant / mergeable**

8. Dry-run trio in `tests/sync.rs:124–150,155–179,184–206`:
   `run_sync_dry_run_writes_nothing` and `…creates_no_cache` overlap almost
   entirely. Merge into one run with all checks; keep
   `…with_ignore_cache_leaves_caches_intact` (distinct).
9. Hard-link parity trio in `tests/sync.rs:477–517,527–574,583–623` each spawn
   the binary 2–4× for the same "hard links refused at plan time" invariant in
   three dimensions. Not strictly subsumed, but batchable into a shared fixture
   with fewer spawns.

**E. Bench / stress** — `benches/tree_shapes.rs` (2,000 files, 4 × 8 MB, 200
levels) is a standalone `main()` benchmark with no `#[test]`; nextest never runs
it. No action. `tests/symlink.rs` (34 tests) and `stat_trust` are mostly already
on the memory backend and small; low priority.

### 6.2 gircap (second priority: process + capture churn)

The fixtures are small (100 B–32 KB chunks, ~20–40 files). The cost is that
each of ~85 integration cases independently starts a mock server, spawns a
fresh `gircap capture` subprocess, writes a whole bundle tree to the HDD,
re-hashes it in close-out, then recursively deletes it in `TempDir::drop`.
The code's own estimate is ~0.5 s per capture (`tests/schema.rs:118`).

1. **Capture consolidation (HIGH).** `capture_layout.rs` (37 tests, ~38
   captures), `schema.rs` (21 tests, 22 captures) and `matrix.rs` (10 tests,
   14 captures) redo the same default capture many times. `schema.rs` alone
   calls `bundle()` 19× for a single document assertion each.
   **Fix:** group "read document X from the default bundle" cases into one test
   per capture variant (default / gzip threshold / ceiling); add a shared,
   file-locked read-only bundle fixture in `tests/common/mod.rs` so suites
   reuse one capture. Because nextest gives each test its own process, an
   in-process `OnceLock` cannot do this — the sharing must be file-based or the
   tests must be merged. Target: remove ~30 captures (~15–20 s on this HDD).

2. **IO performed but not asserted (HIGH/MEDIUM).**
   * `capture_layout.rs:387–517` — six "delete one required document → verify"
     tests capture an entire bundle just to remove one JSON file. `schema.rs:439–488`
     goes further: `copy_tree()` duplicates the whole bundle before deleting a
     key. **Fix:** build a minimal valid bundle with the crate writer (as
     `offline_bundle.rs::Bundle::build` does) and assert the error from that.
   * `common/mod.rs:308–424` — `World::all_shapes()` zstd-encodes ~25 blobs and
     computes ~60 MD5s per capture. Most tests assert metadata, not bytes.
     **Fix:** `OnceLock` the byte tables per process (helps multi-capture tests)
     and shrink the largest chunk (32 KB `old.blk` at 329) where bytes are not
     asserted.

3. **Repeated `gircap verify` (MEDIUM).** `verify::reverify`
   (`src/verify/report.rs:144–199`) re-hashes every file, re-decodes every blob
   and rebuilds both sides. Called from `capture_layout.rs` (6 sites),
   `matrix.rs`, and ~11 `offline_bundle.rs` tests. With small bundles this is
   modest but multiplicative. **Fix:** verify once per shared bundle, not once
   per test.

4. **One real sleep (MEDIUM).** `capture_layout.rs:1198–1266`
   (`a_retried_request_is_recorded_as_retried`) arms a 503 and the child uses
   `RealSleep` with `RetryPolicy.base_delay = 500 ms`
   (`src/http/transport.rs:21`). **Fix:** add a test-only zero-backoff hook
   mirroring the `RecordingSleep` used by the HTTP unit tests
   (`src/http/tests.rs:960+`). Other sleeps (`tasks.rs:185–200`, 5 s deadlock
   guards in `http/tests.rs`) are negligible or bug-only.

5. **Redundant tests (MEDIUM/LOW).** The three
   `every_command_that_reads_a_bundle_agrees_*` cases (`capture_layout.rs:414,
   470`, plus the rejections twin at 498) are subsumed by their
   `a_bundle_without_*_does_not_verify` partners plus the materialize variant —
   merge. `cli_contract.rs::the_binary_exists_where_this_test_expects_it`
   (102–104) is subsumed by every other spawning test in the file.

6. **Already fine:** `measured_fixture.rs` uses a synthetic in-memory struct,
   not the multi-MB production fixture its comments describe — no action. The
   CLI contract suite already uses port 0, `--retries 1`, `--timeout 2`.

### 6.3 girpr (third priority: binary spawns and log IO)

In-process tests are cheap (in-memory 2-file/3-chunk fixture, `NoSleep`). The
cost is the ~100 spawns of the real binary in `cli_contract`, `process`,
`repair_scan`, `repair_machinery`, `repair_plan`, `repair_offline`, `repair_post`,
each creating a `TempDir`, a run-lock file and a tracing log file.

1. **Trace-level logging (HIGH).** `tests/repair_machinery.rs:104–205` — each
   `Rig::run()` spawns the binary with `--log-file-level trace` (180–184),
   writes a trace log to TEMP, reads it back (203), then `snapshot()` walks and
   MD5s every file (239–262). Several tests call `run()` 2–3×
   (`the_three_tiers_produce_the_same_file:791`,
   `the_three_modes_produce_the_same_bytes:1145`,
   `no_mode_leaves_a_temp_file_behind:1176`). **Fix:** make the log level
   configurable per test and use `trace` only for the ~3 tests that assert
   chunk-level records (1535, 1210, 1278); default `debug`/`warn` elsewhere.

2. **`--help`/argument tests via subprocess (HIGH).** `cli_contract.rs` spawns
   the binary ~30×; every run initializes tracing and leaves
   `logs/girpr_<ts>.log` beside the binary (header at 9–14) whose contents are
   never read. **Fix:** convert pure text/help cases to in-process clap
   (`Args::command().render_help()`), keeping a couple of binary cases for the
   process exit-code contract; or add `GIRPR_TEST_NO_LOG=1` to skip file logging
   for argument-only runs. Same idea for the run-lock file (lock behavior has
   its own 15-test `run_lock.rs` suite).

3. **Multi-run parity tests (MEDIUM).** `repair_scan.rs:228–744` spawns 2–4
   binaries per test to compare modes; some overlaps with
   `repair_machinery.rs:791/1145` and `repair_writer.rs:143/309/354`.
   **Fix:** keep one comprehensive parity site, make the others single-mode;
   assert `--plan-out` JSON from stdout where already available (312–332).

4. **One deterministic 300 ms sleep (LOW but pure waste).**
   `src/repair/file/mod.rs:1009–1037` sleeps 300 ms in task 0 so task 1's panic is
   observed first. **Fix:** replace with a `Barrier`/channel signal. A 5 ms
   `ShortSleep` in `src/repair/phases/manifest.rs:609–616` is similarly
   removable in favour of asserting attempt counts with `NoSleep`.

5. **Ignored and out of the normal gate:** `artifact_bridge` (6 cases, ~38–48 s
   each when enabled — see §7) and `peak_heap` (1.95 s). If the bridge is ever
   gated on, verify the immutable bundle once per suite and share the manifest
   cache instead of re-verifying/re-binding four listeners per case.

### 6.4 sled2redb (small but surprisingly slow)

Only 16 tests, but 32.7 s — third-slowest suite by average (2.05 s/test), 86%
storage-attributable. Each conversion opens a sled DB (with `flush()`), creates
a fresh redb temp file, commits, opens the result again through
`girsync::cache::open_db`, and deletes sidecars; several tests do this 2–8×.

1. **`the_flags_are_accepted_on_either_side_of_the_paths`
   (`tests/convert.rs:504–573`) is the single worst case (7.25 s)**: it runs
   **8 full converter subprocesses** (6 argument spellings + 2 equality checks,
   534–571), each a full conversion. **Fix:** reduce to 2–3 representative
   spellings (flag-before / flag-after / `--output=json`), or test clap's
   parsing in-process and keep one full conversion. Expected ~7 s → ~2 s.
2. **Re-opening the converted cache** (`open_db(&dst, true, rw())` at 209, 356,
   402) is a ~200 ms disk open each, in addition to the converter's own opens.
   **Fix:** assert the round-trip once in a dedicated test; elsewhere inspect
   the just-written rows through the converter's own handle or an in-memory
   read.
3. `convert_into` calls `old.flush()` (`src/convert.rs:228`) after a **read-only**
   iteration. That fsync/flush buys nothing (the lock is released by `drop`);
   removing it is a small safe win. Note this is product code, so validate first.

### 6.5 Support crates

`girpr-log` (23 tests, 1.0 s) and `girpr-tempdir` (11 tests, 0.5 s) are already
negligible. `TempDir::drop` recursively deletes the tree on the HDD
(`crates/tempdir/src/lib.rs:215–228`); per-op `remove_file` is ~0.47 ms, so the
lever is **fewer/larger trees**, which the girsync and gircap recommendations
already deliver. No changes needed in these crates.

---

## 7. Bench, stress and ignored tests

`cargo nextest run --workspace --run-ignored only` (all 13 ignored tests; 12
passed, 1 failed):

| Test | Time | Note |
|---|---:|---|
| `girpr::util::md5::tests::scan_throughput` | **94.98 s** | throughput bench; single biggest ignored cost |
| 6 × `girpr::artifact_bridge::*` | **38.2–48.3 s each** (~252 s total) | each re-runs `gircap verify` over the whole bundle and binds 4 listeners; 12 binary runs, ~19 MiB manifests per run |
| `girpr::util::md5::tests::md5_throughput` | 17.91 s | throughput bench |
| `girpr::peak_heap::the_chunk_paths_peak_live_heap` | 1.95 s | 2.16 MB payload; ratio is scale-independent — can shrink to ~256 KiB |
| 3 × `gircap::bundle_shapes::*` | ~1.8 s each | full capture + fetch/assembly |
| `girsync::sync::rows_for_a_directory_created_and_removed_in_one_run_are_dropped` | 0.90 s | **currently FAILS** (`sync.rs:1431`: "a row survived for Sub") |

These are `#[ignore]`d, so they cost **zero** in the normal gate. If the project
starts running them, the bridge suite alone adds ~4 minutes; restructure it to
verify once and share a bridge before enabling it. The failing ignored test is
stale — fix or delete it rather than ever gating it on.

---

## 8. Prioritized action list

| P | Change | Where | Expected effect |
|---|---|---|---|
| P0 | Lower heartbeat entry count via test hook; 2,000 → 8 files | `girsync/output.rs:840`, `src/scan.rs:26` | −4.0 s storage on that test; −99% of its IO |
| P0 | Default `run_binary_raw` to `GIRSYNC_TEST_CACHE=memory`; audit `opts()` → `opts_mem()` across semantic suites | `girsync/tests/sync.rs:95`, `common/mod.rs:293` | removes hundreds of ~200–500 ms redb creates; biggest aggregate win |
| P0 | Shrink 96-file / 11 MB fixtures (rw_threads, as_done) | `girsync/rw_threads/main.rs:127`, `sync_plan/as_done.rs:153` | −2.3 s + −1.8 s storage, plus less parallel contention |
| P0 | Consolidate gircap captures; share one file-locked bundle fixture; hand-build minimal bundles for "delete a doc" tests | `gircap/capture_layout.rs`, `schema.rs`, `matrix.rs`, `common/mod.rs` | −15–20 s; fewer trees to write and delete |
| P1 | Cut sled2redb spelling test 8 → 2–3 conversions; drop read-only `old.flush()` | `sled2redb/tests/convert.rs:534`, `src/convert.rs:228` | −5 s |
| P1 | Replace fixed sleeps (4 × 500 ms publish, 300 ms panic-ordering, 500 ms retry backoff) with signals/hooks | `girsync/cache/open.rs:1064`, `girpr/file/mod.rs:1024`, `gircap/capture_layout.rs:1198` | −2.5 s pure sleep, deterministic |
| P1 | Reduce trace logging and `--help` subprocess spawns in girpr | `girpr/repair_machinery.rs:180`, `cli_contract.rs:42` | −5–10 s process + log IO |
| P1 | Merge redundant dry-run/hard-link and "delete a doc" test groups | `girsync/sync.rs:124,477`, `gircap/capture_layout.rs:414` | −3–5 s, less maintenance |
| P2 | Verify bundles once per suite; shrink `peak_heap` payload; fix/delete the failing ignored test | `girpr/artifact_bridge.rs`, `peak_heap.rs:268`, `girsync/sync.rs:1431` | only matters if ignored tests are ever enabled |
| P2 | Keep all existing `*_NO_FSYNC` / `DURABILITY=none` hooks; do **not** redirect temp to Z: as a "fix" | `.config/nextest.toml`, `.config/nextest-test-env` | preserves the ~169 ms/op fsync saving without relocated storage |

A realistic target is to remove **half to two-thirds of the storage-attributable
time** in the slow suites (≈40–55 s of the 83.7 s measured delta on the
120-test subset), while also improving parallel scaling because the tests stop
queuing on the same HDD.

---

## 9. What was deliberately *not* recommended

* **Moving the temp directory to `Z:`** — explicitly out of scope; the goal is
  to reduce IO, not hide it. The RAM disk was used only as a measurement
  instrument.
* **Shrinking `benches/tree_shapes.rs`** — nextest never runs it.
* **"Fixing" retry/backoff in normal tests** — binary-spawned tests use
  `RealSleep` only on success paths and never take a backoff; the 125 s comments
  in `repair_limits.rs`/`cli.rs`/`sophon/stream.rs` describe production, not
  test time.
* **Touching the hand-rolled JSON writer, A16 fixpoint, byte-movement through
  assembly, or other design choices documented as deliberate** in the previous
  gircap correctness review — those are product concerns, not test-runtime ones.
