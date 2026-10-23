# Test-suite I/O optimisation plan

Derived from `test-io-report.md` (measured 2026-10-23, HDD `E:` vs RAM disk `Z:`).
Baseline: parallel wall **353 s** on HDD, **73.8 s** on RAM disk; serial **491 s** vs **152 s** RAM disk.
Storage-attributable wall: **339 s serial** / **~279 s parallel**. Retry backoff: **79.7 s serial**.

## Goal

Cut the storage-latency and backoff wall time the tests pay on a real disk, without
changing what the tests assert. Every change is test-only or test-seam-only except
R3/R4, which touch `tools/sync` (the binary under test) but are behaviour-preserving
for real runs (the verify re-read still runs; meta still commits when it changes).

## 1. Feasibility determination — every recommendation in the report

| Rec | What | Feasibility | Decision | Why |
|-----|------|-------------|----------|-----|
| **R1** | In-memory redb cache for semantic tests (skip the 1 MB file create + fsync per command) | **HIGH** | **SELECT** | `CacheDb::open_temp` (`tools/sync/src/cache/db.rs:152`, `InMemoryBackend`) already exists and is used by `--dry-run`. `open_folder_cache` (`tools/sync/src/effective/side.rs:143-166`) is the single chokepoint that decides how a folder cache is opened — one place to add a test-only switch. Subprocess tests (`girsync.exe`/`gircap.exe`) propagate an env var through `Command`; lib-driven tests can thread an explicit argument. Keep the real file for `migration_lock`, `convert`, cache tests, anything asserting `girpr-cache*` on disk. |
| **R2** | Deduplicate fixtures / scenario loops (build tree once, reuse caches) | **MEDIUM** | **SELECT — partial** | `rw_threads`: 6 `flag_spellings()` variants each rebuild a fresh `TempDir` + 2 fresh caches — build once, reset changed files, reuse caches. `capture_layout`: 18 tests each re-capture the same healthy mock — capture once into a bundle, read it; keep one re-capture test. `sync_plan`: reuse the copied tree across `--rw` limits. `why::table`: **defer** — its 11 rows deliberately use different configs (algos, record side, stat trust); the code comment says one fixture satisfies none of them faithfully, so deduplicating risks changing assertions. |
| **R3** | Drop / batch per-file `sync_all()` in `copy_one` | **HIGH** | **SELECT** | One line (`tools/sync/src/commands/sync/apply.rs:536`, `fout.sync_all().ok();`). Measured A/B: −11.5 s on 130 `girsync` tests (−4 %), −40 % on the copy-heavy test. Verify re-read still sees bytes via page cache; only the "reached the disk" claim weakens (run-level, not file-level). |
| **R4** | Commit meta only when it changed | **HIGH** | **SELECT** | `init_meta` (`tools/sync/src/cache/db.rs:83-133`) calls `txn.commit()` on every writable open even when nothing changed. Track inserted/rewritten; skip commit otherwise. Measured A/B: −12.4 s on 130 tests, zero failures. Cheap and correct. |
| **R5** | Remove fixed retry sleeps from tests | **MEDIUM** | **SELECT** | 79.7 s serial backoff, unchanged on RAM disk. `gircap` already has the `Sleep`/`RecordingSleep` seam (`tools/capture/src/http/mod.rs:260-322`). `girpr` has **no** seam: `src/repair/file.rs:402`, `src/hyp.rs:307/329/362`, `src/sophon.rs:134/394` — add an injectable `Sleep` seam mirroring `gircap`. Subprocess `cli_contract` tests point at `127.0.0.1:1` — env knob `GIRPR_RETRY_BASE_MS=0` where the policy is built. |
| **R6** | Bound cache backups / snapshots | **MEDIUM** | **DEFER (low priority)** | Removing them was inside run-to-run noise (285.8 s vs 275.0 s) and broke `migration_lock::lock_backups_and_snapshots_created`. Write-volume / production-hygiene work, not a wall-time fix on this HDD. Revisit after R1–R5. |
| **R7** | Fast scratch root as opt-in dev/CI mode | **HIGH** (ops, not code) | **DEFER** | `GIRPR_TEST_TEMP`/`TMPDIR` contract + documentation. Not a code fix; serves as the verification harness (if a fix is real, `E:` time approaches `Z:` time). Document after R1–R5 land. |

## 2. Selected fixes, mapped to scenarios

### Scenario A — `girsync` suites pay ~0.8–1.4 s per command to open/create fsync the cache (R1 + R2 + R3 + R4)

Suites: `sync`, `lazy`, `why`, `compare`, `compare_self`, `hash_mode`, `stat_trust`, `output`, `sync_plan` — all assert *sync results*, not the cache file.

- **R1**: run these against an in-memory cache via a test-only switch at `open_folder_cache`.
- **R2**: build the fixture once per test, reset only what a scenario changes, reuse caches; `capture_layout` captures once into a bundle.
- **R3**: drop/batch `sync_all()` in `copy_one`.
- **R4**: skip the no-op `meta` commit.

Keep the real on-disk cache for `migration_lock`, `convert`, and any test asserting `girpr-cache*` on disk.

### Scenario B — `girsync::rw_threads` / `girsync::why::table` rebuild per setting/row (R2)

- `rw_threads`: 6 variants × fresh `TempDir` + 2 fresh caches → fixture once, reset changed files, reuse cache pair.
- `why::table`: deferred (11 rows, differing configs; not safely deduplicable without changing assertions).

### Scenario C — `gircap::capture_layout` re-captures the same mock 18 times (R2)

- Capture once into a fixture bundle; the 18 layout tests read it; keep one test that re-captures and compares.

### Scenario D — `girpr` / `gircap` retry backoff sleeps (R5)

- `girpr`: add injectable `Sleep` seam; apply at `src/repair/file.rs:402`, `src/hyp.rs:307/329/362`, `src/sophon.rs:134/394`; tests pass a no-sleep implementation.
- `gircap` `cli_contract` subprocess tests: env knob `GIRPR_RETRY_BASE_MS=0`.
- `gircap` HTTP unit test: `tokio::time::pause()`.

## 3. Commit plan

Commits are ordered quick-wins first (measured, zero-risk), then the larger semantic changes, each verified before the next. Run the serial sweep (`tests/scripts/io-profile/run-sweep.ps1`) on HDD and RAM disk after each commit to confirm the `E:`/`Z:` gap shrinks as expected.

### Commit 1 — R3: drop the per-file `sync_all()` in `copy_one`

**Message:** `girsync: drop per-file sync_all() in copy_one (R3)`

**Files:**
- `tools/sync/src/commands/sync/apply.rs`

**Changes:**
- Replace the unconditional `fout.sync_all().ok();` at `apply.rs:536` with a run-level flush policy: flush every N files and at end of run (default N large enough that the common test copy counts never reach it), or simply remove the per-file flush and keep the existing end-of-run/pre-drop commit as the durability point.
- Update the module doc comment and `copy_one` doc comment: the "bytes that actually reached the disk" claim becomes run-level, not per-file; the size/mtime/hash verification is unchanged.
- Add a `#[cfg(test)]` unit test or extend an existing one asserting a copy still verifies (size + mtime + hash) without the per-file flush.

**Verification:** `cargo test -p girsync sync::` and `copy_one` unit tests pass; re-run the 130 `girsync` tests ≥ 1 s on HDD — expect ~−4 % (the copy-heavy `rw_threads` test ~−40 %).

**Estimated savings:** 10–30 s serial (measured −11.5 s on 130 tests).

---

### Commit 2 — R4: commit cache meta only when it changed

**Message:** `girsync: skip the no-op meta commit on cache open (R4)`

**Files:**
- `tools/sync/src/cache/db.rs`

**Changes:**
- In `init_meta` (`db.rs:83-133`), track whether the meta row was inserted or rewritten.
- Skip `txn.commit()` when the row already matches (version and `case_sensitive` both unchanged). Keep the write txn so the entries table still gets touched on a fresh file.
- No public API change; `init_meta` stays `pub(super)`.

**Verification:** existing `db.rs` unit tests (`temp_db_put_get_remove_iter`, `read_only_*`) pass; `cargo test -p girsync` green; A/B-style re-run of the 130 `girsync` tests — expect ~−4 % (measured −12.4 s), zero failures.

**Estimated savings:** 5–15 s serial (measured −12.4 s on 130 tests; close to noise, landed because cheap and correct).

---

### Commit 3 — R1: in-memory cache for semantic tests (test-only switch)

**Message:** `girsync: test-only in-memory cache for semantic suites (R1)`

**Files:**
- `tools/sync/src/effective/side.rs` (`open_folder_cache`)
- `tools/sync/src/cache/db.rs` (possibly a small helper)
- Test helper(s) in `tools/sync/tests/common/mod.rs` and/or per-suite test files

**Changes:**
- Add a test-only switch at the `open_folder_cache` boundary: read an env var once (e.g. `GIRSYNC_TEST_CACHE=memory`) or thread an explicit `CacheBackend` argument from the test helpers. When set and the suite is *not* about the cache file, `open_folder_cache` serves `CacheDb::open_temp()` (already exists, `InMemoryBackend`) instead of opening `<root>/girpr-cache`.
- Apply to suites that assert *sync results*: `sync`, `lazy`, `why`, `compare`, `compare_self`, `hash_mode`, `stat_trust`, `output`, `sync_plan`.
- Keep the real file for `migration_lock`, `convert`, cache tests, and anything asserting `girpr-cache*` on disk (those tests open the cache path directly).
- For subprocess-driven tests, set the env var on the `Command` in the test helper so the spawned `girsync.exe` inherits it; rebuild once per suite run.

**Risk / containment:** The switch is gated behind an env var / explicit argument, so production code paths are untouched. A test that accidentally needs on-disk cache behaviour (backup/snapshot assertions) must keep using the real path — review each migrated test.

**Verification:** full `cargo test -p girsync` green (including `migration_lock`, `convert`); re-run the serial sweep — expect the girsync storage delta to collapse toward its `Z:` floor. Conservatively 150–250 s serial.

**Estimated savings:** 150–250 s serial (largest single fix; only one addressing the 0.8–1.4 s per-command fixed cost).

---

### Commit 4 — R2: deduplicate fixtures and scenario loops

**Message:** `tests: build fixtures once, reuse caches and captured bundles (R2)`

**Files:**
- `tools/sync/tests/rw_threads/main.rs` (+ `through_the_binary.rs`, `limiter.rs` as needed)
- `tools/capture/tests/capture_layout.rs`
- `tools/sync/tests/sync_plan/` (as applicable)

**Changes:**
- `rw_threads`: build the 102-file fixture once per test binary invocation, reset only the files a scenario changes, reuse the cache pair between the 6 `--rw` settings instead of a fresh `TempDir` per setting. Preserve the per-setting equivalence assertion (same effective maps, same `ScanStats`).
- `capture_layout`: do the live `gircap capture` once into a fixture bundle; the 18 layout tests read that bundle; keep exactly one test that re-captures and compares.
- `sync_plan`: reuse the copied tree for the different `--rw` limits (the 105 MB copy is already cheap; the repeated scans are not).
- `why::table`: left unchanged (11 rows, differing configs — not safely deduplicable).

**Verification:** all migrated tests pass with identical assertions; `cargo test -p girsync` and `cargo test -p gircap` green; serial sweep shows `rw_threads` 38.8 s → under 10 s, `why::table` unchanged, `capture_layout` 31.4 s → ~10 s.

**Estimated savings:** 60–100 s serial on its own; overlaps with R1, count once.

---

### Commit 5 — R5: remove fixed retry sleeps from tests

**Message:** `tests: injectable Sleep seam, drop fixed retry backoff in tests (R5)`

**Files:**
- `src/repair/file.rs` (`file.rs:402`)
- `src/hyp.rs` (`hyp.rs:307/329/362`)
- `src/sophon.rs` (`sophon.rs:134/394`)
- `tools/capture/src/http/mod.rs` (already has `Sleep`/`RecordingSleep`; confirm callers use it)
- `tools/capture/tests/cli_contract.rs` (env knob)
- gircap HTTP unit test (`tokio::time::pause()`)

**Changes:**
- Add to `girpr` an injectable `Sleep` trait mirroring `gircap`'s (`RealSleep` for production, `RecordingSleep`/no-sleep for tests), threaded through the retry call sites in `repair/file.rs`, `hyp.rs`, `sophon.rs`. Tests pass a no-sleep implementation.
- For the subprocess-based `cli_contract` tests, add a test-only env knob `GIRPR_RETRY_BASE_MS=0` read where `RetryPolicy` is built, so the dead-loopback `127.0.0.1:1` cases fail immediately instead of backing off.
- For the `gircap` HTTP unit test, use `tokio::time::pause()` instead of waiting 4 s.

**Verification:** `cargo test -p girpr` and `cargo test -p gircap` green; `repair_offline` three 15.1 s tests drop to near-instant; `cli_contract` 12 × ~2.1 s tests drop to near-instant; serial sweep storage+backoff wall falls accordingly.

**Estimated savings:** up to 80 s serial; 20–30 s of parallel wall (a 15 s sleeping test still occupies one of the four workers).

---

### Commit 6 (optional, after R1–R5) — R6/R7: bound backups + fast-scratch documentation

**Message:** `docs: bound cache backups; document fast scratch root (R6/R7)`

**Files:**
- `tools/sync/src/cache/open.rs` (`backup_db`/`snapshot_old`)
- `docs/` (fast scratch root contract)
- `tests/scripts/io-profile/README.md` (verification harness note)

**Changes:**
- Retain one backup per side per run, skip the copy when the cache is unchanged, don't create both a backup and an old-state snapshot of the same bytes. (Low priority; only if R1–R5 leave headroom.)
- Document the `GIRPR_TEST_TEMP`/`TMPDIR` contract honoured by `TempDir` (it already uses `std::env::temp_dir()`), plus a nextest profile running I/O-heavy suites serially on the fast root.

**Verification:** `migration_lock::lock_backups_and_snapshots_created` still passes; sweep on RAM disk confirms fixes are real (`E:` time approaches `Z:` time).

**Estimated savings:** write-volume / hygiene, not wall-time on this HDD.

## 4. Verification harness (re-used, not new)

The report's reproduction scripts already live under `tests/scripts/io-profile/` and are git-ignored outputs under `target/`. Use them at every commit:

- Serial sweep HDD vs RAM disk: `run-sweep.ps1` with `%TEMP%` on `E:` and `-TempRoot Z:\tmp`.
- Confirm each fix is real: after a commit, `wall(E:) − wall(Z:)` for the affected suites should shrink toward the `Z:` floor.
- Parallel baseline: `cargo nextest run --workspace` (353 s → target well below).

## 5. Expected combined effect

| Stage | Serial wall | Parallel wall |
|-------|-------------|---------------|
| today (`E:`) | 491 s | 353 s |
| after commit 1 + 2 (R3 + R4, measured) | ~465 s | ~335 s |
| after commit 3 (R1) | ~180–220 s | ~150–180 s |
| after commit 4 (R2) | ~140–180 s | ~120–150 s |
| after commit 5 (R5) | ~110–150 s | ~110–140 s |
| floor (`Z:` measurement) | 152 s | 74 s |

The floor is not zero: the suite does real CPU work (66.7 s serial) and some tests legitimately exercise cache/backup paths. R1's exact share is the largest uncertainty (the A/B in §2.5 of the report could not test it without changing cache semantics); it is estimated from the 0.8–1.4 s per-command fixed cost times the number of commands per test.

## 6. Rollback / risk notes

- R3 and R4 are behaviour-preserving for real runs (verify still runs; meta still commits when it changes). If either causes a flaky failure, revert that single commit.
- R1 changes cache semantics for tests only, behind an env var / explicit argument. If a migrated test needs on-disk cache behaviour, move it back to the real path rather than weakening the switch.
- R2 rewrites test fixtures; preserve every assertion, only remove redundant setup. `why::table` is deliberately left alone.
- R5 removes sleeps that *are* the thing some tests wait on; only tests that assert *how many* requests were made or what the summary says should get no-sleep. Tests that genuinely need timing (none identified) keep real sleeps.
- The HDD has ±10–20 % run-to-run variance on individual tests; treat only aggregate or multi-second deltas as real. Confirm on the serial sweep, not a single parallel run.
