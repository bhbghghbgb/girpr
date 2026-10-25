# Test-performance report

**Scope:** every test in the workspace, measured once per test body, on the current tree.

**Question:** which tests are slow, why, and what can be changed without weakening what they
prove?

**Answer in one line:** the wall time is highly concentrated — 63 of 1259 tests are 58% of it —
and roughly 30 s of the serial total is self-inflicted waiting (a dead loopback port, three
`sleep`s) that has nothing to do with the code under test; most of the remaining HDD penalty is
one `fsync` per copied file, one redb open per command, and one `fsync` per redb commit — each
costing ~0.04–0.5 s on E: and ~0.0004–0.01 s on Z:.

No source or test file in this tree was modified for this report. The P1.5 A/B probe ran in a
throwaway worktree (§6.8); the tree under measurement is unchanged.

---

## 1. Method

- **Inventory.** `cargo nextest list --workspace` enumerated **1259 test bodies in 39 suites**
  across five crates (`gircap`, `girsync`, `girpr`, `girpr-log`, `girpr-tempdir`).
- **Serial sweep.** Each test was run in its own process, one at a time, with a job object
  measuring wall, CPU, wait, read/write bytes and operation counts. A serial sweep was chosen
  deliberately: a parallel `cargo nextest run` measures scheduler contention, not per-test cost,
  and the question here is which tests are expensive.
- **Two media.** The same sweep ran twice: `target/io-profile/fresh-e` on the HDD (`E:`) and
  `target/io-profile/fresh-z` on the RAM disk (`Z:`). `Z:` exists only to attribute time to
  storage; it is not a proposed fix. The HDD environment is the one this project targets.
- **Attribution.** "Storage delta" is HDD wall minus RAM wall for the same test. It is an
  **upper bound** on storage time: the HDD sweep ran first and the RAM sweep benefited from a
  warm page cache and less contention.
- **Microbenchmarks.** `tests/scripts/io-profile/iot-bench` isolated the primitives the suites
  actually use: per-file `fsync`, redb open/create/commit, and raw copy.
- **Raw data:** `target/io-profile/analysis-fresh/` (`summary.txt`, `tests-ranked.csv`,
  `compare-tests.csv`, `compare-suites.csv`, `by-crate.csv`).

All 1259 rows exited zero; no test failed in either sweep.

---

## 2. Where the time goes

### 2.1 Totals

| Metric | HDD (`E:`) | RAM disk (`Z:`) | Delta |
|---|---:|---:|---:|
| Tests | 1259 | 1259 | — |
| Wall | **199.8 s** | **120.2 s** | **79.9 s** |
| CPU | 66.6 s | 73.7 s | −7.1 s |
| Wait | 136.5 s | 50.4 s | 86.1 s |
| Read | 195.2 MB | 195.7 MB | — |
| Write | 154.5 MB | 155.0 MB | — |
| Ops | 628,849 | 629,302 | — |

On the HDD, **68% of wall time is wait**, not CPU. On the RAM disk wait falls to 42%. The CPU
figures differ by 10.7% between the two runs, which is the size of the noise floor for
cross-run CPU comparisons; treat them as approximate.

### 2.2 Concentration

| Slice | Tests | Wall (HDD) | Share |
|---|---:|---:|---:|
| All | 1259 | 199.8 s | 100% |
| `< 0.1 s` | 1016 | 26.0 s | 13% |
| `≥ 0.5 s` | 97 | 148.0 s | 74% |
| `≥ 1.0 s` | 63 | 116.1 s | 58% |
| `≥ 2.0 s` | 28 | 68.5 s | 34% |

Median test: **0.02 s**. p90: 0.36 s. p99: 2.17 s. There is no long tail to chase; a small
set of suites owns the total.

### 2.3 By crate

| Crate | Tests | HDD wall | RAM wall | Delta | Share of delta |
|---|---:|---:|---:|---:|---:|
| `girsync` | 326 | 93.7 s | 28.4 s | **65.3 s** | 82% |
| `gircap` | 622 | 82.2 s | 77.5 s | 4.7 s | 6% |
| `girpr` | 277 | 23.5 s | 13.8 s | **9.7 s** | 12% |
| `girpr-log` | 23 | 0.3 s | 0.2 s | 0.1 s | — |
| `girpr-tempdir` | 11 | 0.1 s | 0.1 s | 0.0 s | — |

`girsync` is the storage story. `gircap` is the delay story: it is almost as slow on the RAM
disk as on the HDD, so its cost is not I/O.

### 2.4 By suite (top 12 by HDD wall)

| Suite | Tests | HDD | RAM | Delta |
|---|---:|---:|---:|---:|
| `gircap::cli_contract` | 28 | 25.7 | 25.8 | −0.1 |
| `gircap` (unit) | 492 | 19.7 | 17.7 | 2.2 |
| `gircap::capture_layout` | 29 | 12.2 | 13.8 | −1.6 |
| `girsync::compare` | 12 | 11.0 | 1.1 | **9.9** |
| `girsync::rw_threads` | 15 | 10.8 | 7.5 | 3.3 |
| `girpr::repair_offline` | 74 | 10.7 | 6.1 | **4.6** |
| `girsync::sync_plan` | 19 | 10.6 | 2.8 | **7.8** |
| `gircap::schema` | 15 | 9.2 | 6.6 | 2.7 |
| `girsync::migration_lock` | 11 | 8.6 | 0.7 | **7.8** |
| `girsync::output` | 16 | 7.5 | 1.7 | **5.7** |
| `girpr::repair_machinery` | 25 | 7.0 | 3.4 | 3.6 |
| `girsync::sync` | 15 | 6.3 | 1.3 | **4.9** |

Those twelve suites are 126.0 s, **63% of the serial total**. The full 39-suite table is in
`compare-suites.csv`.

---

## 3. The causes, with evidence

### G1 — A dead loopback port costs 2 s per connect on Windows (≈24.5 s)

`gircap::cli_contract` defines its offline seam as `http://127.0.0.1:1/...`
(`tools/capture/tests/cli_contract.rs:36-45`) and twelve tests use it. Each takes
2.04–2.25 s and **the same on the RAM disk** (delta ≈ 0), so it is not storage.

Direct measurement with the current binary, same arguments the tests use:

```text
port 1 → exit 2, 2053 ms
port 0 → exit 2,   25 ms
```

The 2 s is the Windows TCP connect to a closed port (`SYN` retry), not the `--timeout 2` flag:
the error is `transport failed after 1 attempt(s)` and the log timestamps are 2.018 s apart.
Port 0 fails immediately, produces the identical `exit_code=2` /
`exit_code_slug=network` / `artifact_trustworthy=true` record, and `validate_base` already
permits it (it only requires loopback).

**Cost: 12 × ~2.03 s ≈ 24.5 s, on both media.**

### G2 — Real sleeps inside `gircap` tests (≈1.5 s)

| Test | Wall | Cause |
|---|---:|---|
| `http::tests::the_write_happens_with_the_log_lock_released` | 1.22 s | `SlowWrite::write` sleeps **150 ms per call** (`tools/capture/src/http/mod.rs:1909`), and `write_ndjson_to` calls it for each of four records plus flushes |
| `http::tests::waiting_for_a_retry_yields_the_runtime_rather_than_blocking_it` | 0.32 s | 300 ms + 30 ms real sleeps (`mod.rs:2386`) |

Both are identical on the RAM disk. These are test fixtures, not behaviour under test.

### G3 — One `fsync` per copied file (≈5 s of `girsync`)

`copy_one` ends every copy with `fout.sync_all().ok()`
(`tools/sync/src/commands/sync/apply.rs:570`). The microbenchmark isolates the cost:

| Primitive | HDD | RAM disk |
|---|---:|---:|
| create + write 512 B + `sync_all` | **86.05 ms** | 0.36 ms |
| create + write 512 B, no sync | 0.29 ms | 0.25 ms |
| copy 1 MB | 1.39 ms | 1.32 ms |

Raw copy bandwidth is identical on both media. The 86 ms is the flush. A test that copies
*n* files pays ~86 ms × *n* on E: and ~0.4 ms × *n* on Z:.

Affected: `rw_threads` (six spawns × four copied files), `sync_plan::as_done` (6 and 24 files),
and the copy half of `girsync::sync`.

### G4 — redb open costs ~0.5 s on E: (≈55–60 s of `girsync`)

| Primitive | HDD | RAM disk |
|---|---:|---:|
| `Database::create` on an existing cache, no commit | **498 ms** | 9.5 ms |
| open + meta commit | 326 ms | 9.7 ms |
| fresh create + commit | 480 ms | 3.2 ms |
| 10 commits × 10 rows, `Durability::Immediate` | 40 ms/commit | 0.65 ms/commit |

The commit row is the other component of G4, and it is not covered by the open number. A commit that
inserts ten rows costs **40 ms** on E: with the default `Durability::Immediate` and **0.18 ms**
with `Durability::None`; on Z: the same pair is 0.65 ms and 0.035 ms. A `sync` run commits roughly
ten times (both phase A scans, the resolve commits, five unconditional apply commits, and `meta`),
and a `compare` run four to six times. So a file-backed command pays ~0.2–0.4 s in commit flushes
on the HDD *on top of* its opens.
P1.5 below targets exactly that term.

Every `cmd_update`, `cmd_compare`, `cmd_sync` and `recs_of` opens the cache again. The
file-backed suites therefore pay a fixed ~0.3–0.5 s per command on the HDD, and ~0.01 s on the
RAM disk. That fixed cost, multiplied across the ~100–150 opens these suites perform, is the
largest single component of the 65 s `girsync` delta — not the bytes copied.

This is also why the same test can be 2.6 s on E: and 0.13 s on Z: while doing almost no I/O
(`compare::an_uncovered_record_fails_before_the_folder_side_is_resolved`: three commands, five
opens).

### G5 — Subprocess tests that omit the memory-cache switch (≈9 s)

`girsync` already has the right tool: `TestCacheBackend::Memory` for in-process calls, and
`GIRSYNC_TEST_CACHE=memory` for spawned binaries
(`tools/sync/src/effective/side.rs:161-169`). `common::recs_of` even prefers the memory entry
(`tools/sync/tests/common/mod.rs:346-359`). Several helpers use it; several direct spawns do
not.

| Test | HDD | RAM | Why |
|---|---:|---:|---|
| `output::an_unrelated_rust_log_adds_to_the_console_without_silencing_it` | 2.87 | 0.15 | two direct spawns (`output.rs:527,544`); asserts stderr only |
| `output::the_environment_sets_the_log_files_level` | 1.78 | 0.14 | direct spawn (`output.rs:495`); asserts the log file |
| `output::the_environment_names_the_log_file_when_the_flag_does_not` | 1.08 | 0.13 | direct spawn (`output.rs:670`) |
| `output::no_log_file_turns_the_file_off_even_when_the_environment_names_one` | 0.93 | 0.09 | direct spawn (`output.rs:613`) |
| `sync_plan::as_done::parallel_copies_actually_land_on_disk` | 1.99 | 0.10 | direct spawn (`as_done.rs:100`) while every `run()` in the suite sets the env |
| `stat_trust::flag_surface::update_warns_that_the_flags_do_not_apply_and_still_populates` | 3.41 | 0.18 | two spawns via `bin()` (`stat_trust/main.rs:62`), which sets no env |

The sibling of the last one, `the_two_flags_are_independent_on_the_command_line`, was already
converted to in-process `cmd_update` for exactly this reason (see its doc comment); the slow one
was left behind.

### G6 — `girpr` writes and fsyncs `config.ini` once per real repair (≈8 s)

The `girpr` delta is not diffuse. It is one line:

```rust
// src/util.rs:515
f.sync_all().ok();
```

`post::run` calls `write_config_ini(..., false)` on every non-readonly run
(`src/repair/post.rs:96`), after the bytes are repaired. There is no other `fsync` in the repair
file path (`src/repair/file.rs` only `flush`es). The arithmetic matches the measurement:

- `repair_offline`: 74 tests, 87 `repair::run` calls → ~60 real runs.
- `repair_machinery`: 25 tests, several multi-mode loops → ~35 real runs.
- ~95 runs × 34–86 ms = 3.2–8.2 s; measured delta for those two suites: **4.6 + 3.6 = 8.2 s**.
  The per-fsync cost in the benchmark ranges from 34 ms (a 1 MB file already streaming) to 86 ms
  (a lone 512 B file); a repair run's single small-file `config.ini` flush sits at the slow end.

That is essentially the whole `girpr` delta. `girpr::artifact_bridge` and `girpr::peak_heap`
are `#[ignore]`d and contribute only process-startup time (see §6).

### G7 — Fixture scale and repeated setup (≈3–5 s)

| Where | What | Size |
|---|---|---|
| `sync_plan/as_done.rs:149` | 24 files sized `i × 200 KB` | **55 MB** |
| `rw_threads/main.rs:127` | 96 pad files × 2 sides × 6 spawns | 576 files |
| `why/table.rs:219` | 22 rows, each with a fresh temp root and fixture | 22 trees |
| `compare.rs` fixtures | 2–4 small trees per test | — |

The 55 MB fixture exists to make completion order diverge under `--rw-threads 4`. The assertions
only need 24 distinct paths with *increasing* sizes; ~10 MB preserves the property.

### G8 — `gircap` capture suites are CPU-bound, not I/O-bound (27.5 s)

`capture_layout` (29 tests, 12.2 s), `schema` (15, 9.2 s) and `matrix` (8, 6.1 s) each run a
real `gircap capture` through a shared helper. CPU is 9.2/6.0/3.8 s respectively, and
`capture_layout` is **1.6 s slower on the RAM disk**. These are not storage targets; the only
lever is running fewer captures per suite.

### Not causes

- **Raw copy throughput.** 1 MB copy: 1.39 ms on E:, 1.32 ms on Z:.
- **`girpr` backoff.** `repair_offline` uses `NoSleep`.
- **Ignored tests.** `artifact_bridge` (6), `peak_heap` (1) and one `girsync::sync` test are
  `#[ignore]`d; running them by name without `--ignored` exits immediately. Their rows are
  executable paging, not test bodies.
- **`girpr-log` / `girpr-tempdir`.** 0.4 s combined.

---

## 4. Recommendations, in priority order

Estimates are serial-sweep savings. "Both" means the fix helps HDD and RAM; "HDD" means it
targets the storage delta.

### P0 — Remove test-induced delays (~30 s, both media, no coverage change)

| # | Change | File | Now | After | Save |
|---|---|---|---:|---:|---:|
| 1 | `OFFLINE` port `1` → `0` | `tools/capture/tests/cli_contract.rs:38,40` | 24.9 s | 0.4 s | **24.5 s** |
| 2 | Transport-retry test: use `127.0.0.1:0` instead of a dropped ephemeral port | `tools/capture/src/http/mod.rs:2534-2537,2565` | 4.07 s | 0.05 s | **4.0 s** |
| 3 | Replace `SlowWrite`'s 150 ms sleep with a rendezvous channel (write blocks until the test releases it after `try_lock`) | `mod.rs:1898-1912,1996` | 1.22 s | 0.05 s | **1.2 s** |
| 4 | Shrink the yield test to 100 ms / 10 ms and assert `< 60 ms` | `mod.rs:2386` | 0.32 s | 0.12 s | 0.2 s |

Notes:

- For #1 and #2, port 0 is a *transport* failure like port 1; both produce exit 2 and the
  `network` slug. The tests assert transport semantics, not `ECONNREFUSED` specifically.
  Add a one-line comment saying why port 0 is used, so nobody "fixes" it back.
- For #3, a channel barrier is stronger than a sleep: the test knows the writer is inside
  `write`, and no timing assumption is involved.
- #4 is a timing test; if it is ever flaky, the right move is to delete it and assert the
  `Sleep` trait is async by construction, not to widen the margin.

### P1 — Use the memory cache where the test does not assert the cache file (~9.5 s, HDD)

| # | Change | File | Now | After | Save |
|---|---|---|---:|---:|---:|
| 5 | Add `.env("GIRSYNC_TEST_CACHE","memory")` to the five direct spawns in `output.rs` | `output.rs:495,527,544,613,670` | 6.7 s | 0.6 s | **6.1 s** |
| 6 | Add a `bin_mem` helper and use it for the **second** spawn (the "unflagged is silent" half); keep the first file-backed because it asserts `recs_of` | `stat_trust/flag_surface.rs:19-49`, `main.rs:62` | 3.41 s | 2.0 s | **1.4 s** |
| 7 | Add the same env to the direct spawn in `parallel_copies_actually_land_on_disk` | `sync_plan/as_done.rs:100` | 1.99 s | 0.6 s | **1.4 s** |
| 8 | Shrink the 24-file fixture from `i × 200 KB` to `i × 40 KB` (~11 MB) | `sync_plan/as_done.rs:149` | 2.13 s | 1.4 s | 0.7 s |

Why these are safe: the memory backend is path-keyed and shared *within one process*, so
in-process command sequences still see each other's rows. Subprocess tests that assert cache
*contents* afterwards cannot use it (the child's memory dies with the child) — that is exactly
why #6 keeps its first spawn file-backed. Tests that assert `girpr-cache` bytes, backups,
record-vs-folder identity, or `--ignore-cache` must stay file-backed; see §5.

### P1.5 — Make the cache commits non-durable in tests (~14 s, HDD; ~0 on RAM)

This is the other half of the file-cache story. P1 removes the file where the test does not assert
it; P1.5 cheapens the commits in the tests that **must** keep it. It is the same shape as the
existing `GIRSYNC_TEST_CACHE` switch: a test-only environment variable, default off, read when a
write transaction is created.

| # | Change | File | Save |
|---|---|---|---:|
| 8a | Add `GIRSYNC_TEST_DURABILITY=none`; set `WriteTransaction::set_durability(Durability::None)` in `CacheWrite::with_limits`, `CacheWrite::flush_txn` and `CacheDb::init_meta` | `tools/sync/src/cache/write.rs:116,203`, `tools/sync/src/cache/db.rs:110` | **~14 s** |

All command write transactions already funnel through `CacheWrite` (`apply`, `rename`, `convert`
and both phases), so this is **three call sites, not one per command**. The probe patch was one
helper plus those three calls: 26 added lines, most of them the helper's doc comment. The switch
has to be an environment variable rather than `cfg(test)`: the slowest tests run the real binary
as a subprocess, and a `cfg(test)` build would not reach it.

**Evidence (throwaway worktree, same warm E:, 326-test `girsync` suite, serial):**

| Run | Wall |
|---|---:|
| `Durability::Immediate` (3 runs) | 75.9 / 78.6 / 78.9 s — mean **77.8 s** |
| `Durability::None` (2 runs) | 65.2 / 62.3 s — mean **63.8 s** |
| Saving | **14.0 s (18%)** |

The reverse-order pair rules out warm-cache bias: `none` first was 62.3 s and `immediate` after it
was 78.9 s. On the same runs the job object counted **13,170 → 12,550 write operations (−4.7%)**
and **139.9 → 127.6 MB written (−8.9%)**: a durable commit also writes allocator/free-page
metadata that a non-durable commit keeps in memory until the drop. Per-test examples from the
isolated A/B (warm, five to seven runs each):

| Test | Immediate | None |
|---|---:|---:|
| `migration_lock::lock_dry_run_changes_nothing` | 2.0 s | 1.6 s |
| `compare::an_uncovered_record_fails_before_the_folder_side_is_resolved` | 2.3 s | 1.8 s |
| `compare::run_update_then_compare_equal` | 1.8 s | 1.3 s |
| `stat_trust::flag_surface::update_warns_that_the_flags_do_not_apply_and_still_populates` | 1.2 s | 0.9 s |

The same suite on Z: is 21.4 s immediate vs 20.8 s none — within noise. Commit fsync is ~0.65 ms
on Z: and ~40 ms on E:, so **this is an HDD-only change** and should not be expected to help a
RAM-disk run.

**Why the flush requirement is already met.** redb 4.3.0 does a durable close on drop:
`Database::drop` (`db.rs:2012`) calls `close_database` (`db.rs:1963`), which runs
`ensure_allocator_state_table_and_trim` (`db.rs:1922`) — a commit with the **default
`Durability::Immediate`** — and then `mem.close()` → `flush_shutdown_header`
(`page_manager.rs:2449,2466`), which calls `storage.flush()`. A pending non-durable commit is
therefore flushed when the `CacheDb`/`Database` is dropped, and the next command's reopen sees
every row. That is the property the tests assert; a crash *between* commit and drop is the only
thing `Durability::None` loses, and no test does that.

Two details keep the drop path honest:

- `girsync`'s `main` returns `ExitCode`, not `process::exit`, specifically so destructors run
  (`tools/sync/src/main.rs:7-13`). The only `process::exit` calls in the crate are `sled2redb`'s
  usage and error paths (`tools/sync/src/bin/sled2redb.rs:15,72`). The usage path exits before opening a
  cache, but the error path can skip the drop flush; either fix that path or leave the gate off
  for that binary.
- `Durability::None` is refused in redb's multiprocess modes. The crate uses the default
  single-writer mode, and no test sets a shared mode, so the gate is valid here.

**Is it worth it? Yes, with conditions.** It is ~14 s on the HDD for ~26 lines, it makes the
file-backed suites (the ones P1 cannot move to memory) faster, and it removes no assertion: the
cache file is still created, written, reopened, copied for backups and inspected byte-for-byte.
It does not change which rows are written; it removes the per-commit flush (and some allocator
metadata), so the same logical cache is produced with fewer writes. The conditions are:

- Default **off**; CI opts in explicitly, and the variable is documented next to
  `GIRSYNC_TEST_CACHE`.
- If you want the "commit survives a process kill" contract exercised, keep one explicit test on
  `Immediate`. Today no test exercises it — the only commit-timing test
  (`sync::the_copy_phase_commits_its_cache_rows_while_it_copies`) asserts log ordering, not
  durability, and passes with the gate — so the gate removes no existing coverage.
- Count it as **~11–15 s incremental** if P1 is already applied: P1 converts some of the same
  subprocess tests to the memory cache, so ~3 s of the 14 s overlaps.
- It does not reduce the open cost, which remains the largest per-command term. P2's fewer opens
  still matter.

### P2 — Reduce repeated opens and temp trees (~2.5 s, HDD)

| # | Change | File | Now | After | Save |
|---|---|---|---:|---:|---:|
| 9 | `lock_dry_run_changes_nothing`: replace the two pre-run `recs()` opens with a cache-file size/mtime snapshot, and compare it after the dry run | `migration_lock.rs:422-446` | 2.44 s | 1.0 s | **1.4 s** |
| 10 | `every_pair_state...`: build all 22 rows under one temp root (per-row subdirectory, cleaned between rows) instead of 22 temp roots | `why/table.rs:218-246` | 2.17 s | 1.3 s | **0.9 s** |

#9 also makes the assertion stronger: "same row count" passes on a rewrite that produces the
same count, while "same bytes and mtime" does not.

### P3 — Test-only file-`sync_all` gate (~11.5 s, HDD; needs a design decision)

After P1.5 removes the redb commit flushes, two `sync_all`s in our own code are the largest
remaining flush costs, and neither is asserted by any test:

| # | Change | File | Save |
|---|---|---|---:|
| 11 | Gate `copy_one`'s per-file `sync_all()` behind `GIRSYNC_TEST_NO_FSYNC` (default off) | `apply.rs:570` | **~3–4 s** |
| 12 | Gate `write_config_ini`'s `sync_all()` behind `GIRPR_TEST_NO_FSYNC` (default off) | `src/util.rs:515` | **~7.5 s** |

The pattern already exists in the tree (`GIRSYNC_TEST_CACHE`, plus P1.5's cache-durability
switch), and the switch is read once per process. The trade-off is honest: tests stop exercising
the durability path. Mitigate by keeping
one explicit test per binary that runs **with** the fsync on and asserts the file survives a
handle close, and by documenting the env var next to the cache switch. If a production-code test
hook is unacceptable, drop #12 and accept ~8 s of `girpr` wall time on the HDD; #11 is the same
decision for `girsync`.

### P4 — Consolidate capture suites (5–8 s, both media, high churn; optional)

`capture_layout`, `schema` and `matrix` each run one full capture per test. Several tests in a
suite inspect different properties of the *same* artifact. Merging 2–3 assertions per capture
would cut process startup and fixture build, but the work is CPU-bound and parallelises well, so
the CI-wall payoff is smaller than the serial number suggests. Do this last, and only if the
suites are being touched anyway.

### Deliberately not recommended

- **Switch the workspace to SSD/RAM disk.** The HDD is the target environment; `Z:` was a
  measurement instrument.
- **Delete tests.** None were found to be useless. The closest thing to redundancy is the twelve
  offline `cli_contract` tests, which share one capture but assert different output/logging
  contracts; after the port fix they cost ~0.03 s each, so merging them buys nothing.
- **Move the file-cache contract tests to the memory backend.** `compare`, `migration_lock`,
  `convert`, `sync` (cache half), `update`, `compare_self` and `stat_trust` assert bytes,
  backups, locks or record-vs-folder identity. Those assertions are the point of the suites.
- **Chase the `gircap` capture suites for storage.** They are CPU-bound and slightly *slower*
  on the RAM disk.

---

## 5. Tests that must stay file-backed

Keep the real redb file (and therefore the ~0.5 s open) where the assertion is about the file:

| Area | What it asserts |
|---|---|
| `girsync::compare` | `CACHE_PREFIX` exists, byte-identical dry runs, same-cache refusal, record-vs-folder identity |
| `girsync::migration_lock` | backups, snapshots, lock behaviour, "dry run changes nothing" on disk |
| `girsync::convert` | sled directory → redb file, source left untouched |
| `girsync::update` | cache creation and gap-filling |
| `girsync::compare_self` | case-only pairs and self-audit |
| `girsync::stat_trust` (cache repair) | cache poisoning and repair |
| `girsync::sync` (cache half) | `--ignore-cache` leaves caches intact, dry run creates none |

P1.5 does not move any of these off the file. It keeps the real redb file and every byte, backup,
snapshot and lock assertion; it only drops the per-commit fsync. The file is still written and
still read back by the next command, so the difference is visible only to a process killed
between commit and drop — which none of these tests do.

Within those, the only safe savings are fewer redundant opens (#9), the non-durable test commits
(P1.5), and a smaller redb file if it measures well. A smaller initial redb file is *not*
recommended without measuring it first: the open cost appears to be dominated by metadata
validation and the header flush, not the 1 MB extent.

---

## 6. Validity limits

1. **Serial, not CI-parallel.** 199.8 s is the sum of per-test walls, not `cargo nextest` wall
   time. Under parallelism, I/O-bound tests contend, so the HDD penalty can be worse than the
   serial delta suggests; CPU-bound suites (`gircap`) shrink instead.
2. **Order bias.** The HDD sweep ran first. The RAM sweep had a warm page cache and no competing
   disk activity, so 79.9 s is an **upper bound** on storage time.
3. **One sample per test.** No variance estimate. Tests in the 0.1–0.2 s range (process startup)
   are noise; the recommendations only touch tests ≥1 s or ≥0.3 s delta.
4. **Byte accounting is a lower bound.** The job object does not see `CopyFileEx`/
   `std::fs::copy`, so copy-heavy tests report less write volume than they produce. This does not
   affect wall-time attribution.
5. **Ignored tests.** `girpr::artifact_bridge` (6), `girpr::peak_heap` (1) and
   `girsync::sync::rows_for_a_directory_created_and_removed_in_one_run_are_dropped` (1) are
   `#[ignore]`d. Their rows measure executable load, not the test body. The first two are
   deliberately hand-run and are not candidates for optimisation.
6. **`girpr` attribution is arithmetic, not a direct A/B.** The config-fsync claim in G6 follows
   from one `sync_all` per run, the run count, and the measured per-fsync cost; it was not
   confirmed by running with the fsync disabled. If #12 is implemented, measure before/after to
   confirm the estimate.
7. **Four listed names produced no row.** The inventory listed 1263 names; the sweep recorded
   1259 distinct `(suite, test)` pairs. The four-name difference was not investigated. It is not
   a failure (every recorded row exited zero, and the per-suite sums reconcile with the totals),
   but read the headline as **1259 measured tests**, not 1263.
8. **P1.5 was measured in a throwaway worktree, not the main tree.** The 326-test `girsync`
   suite ran three times with the default durability and twice with `GIRSYNC_TEST_DURABILITY=none`
   on the same warm HDD; the main tree is unchanged. That sweep was isolated and warm, while the
   fresh-e sweep was colder and busier, so the ~14 s measured here is a **conservative** estimate
   of the sweep-scale saving (per-commit fsync is more expensive under contention), not an upper
   bound. The Z: A/B was neutral. The probe patch and the per-run CSVs are under
   `target/io-profile/` (`p15-probe.patch`, `probe-suite/`), which git ignores.

---

## 7. Expected effect

Applying P0, P1, P1.5, P2 and P3 (no assertion changes; test-only env switches only):

| | Now | After | Saved |
|---|---:|---:|---:|
| Serial HDD | 199.8 s | ~136 s | **~64 s (32%)** |
| Serial RAM | 120.2 s | ~90 s | **~30 s (25%)** |
| HDD − RAM delta | 79.9 s | ~46 s | **~34 s** |

The delta does not fall by the full 64 s because P0 (30 s) is delay that both media pay; only
P1, P1.5, P2 and P3 attack storage. P1.5 is counted at the conservative end of its incremental
range (~11 s, not the ~14 s standalone), because P1 converts some of the same subprocess tests to
the memory cache. Adding P4 would take another 5–8 s off the serial total but much less off a
parallel CI run.

P0 alone is the highest-value change per line of diff: **four edits, ~30 s, no behavioural
risk**, and it makes the `gircap` suite roughly 30 s faster on every machine regardless of disk.

---

## 8. Implementation status

This section is updated by the task that lands each change. A task is `done` only when the
targeted tests pass and the change is committed; `skipped` records a deliberate decision not to
apply a recommendation.

| Task | Scope | Status | Commit / note |
|---|---|---|---|
| T1 | P0 #1–#4: remove the dead loopback port and the real sleeps in `gircap` | done | `5d97c50` — `cli_contract` nextest summary 6.9 s → 0.8–2.8 s; the three `http::tests` 4.11 s → 0.13 s (4-core HDD). |
| T2 | P1 #5–#8: use the memory cache in the subprocess tests that do not assert it, and shrink the 24-file fixture | done | `59bad4b` — warm nextest summaries (E:): `output` 5.3–5.8 s → 0.6–1.5 s, `sync_plan` 7.6–7.9 s → 4.6 s, `stat_trust` 2.8–3.0 s → ~2.3 s (HDD noise). |
| T3 | P1.5 #8a: `GIRSYNC_TEST_DURABILITY=none` for test cache commits | pending | |
| T4 | P3 #11–#12: test-only file-`sync_all` gates for `girsync` copies and `girpr` config writes | pending | |
| T5 | P2 #9–#10: fewer cache opens in `migration_lock`, one temp root for the `why` table | pending | |
| T6 | Final workspace verification and measured before/after summary | pending | |
| — | P4: consolidate the capture suites | skipped | Optional, high churn; the suites are CPU-bound and parallelise well. |
