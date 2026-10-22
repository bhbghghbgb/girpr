# The test suite's time cost — where the minutes go

**Status: measured, 2026-10-22.** A profile of the workspace's test suite on
the machine it was run on, the seven groups that account for the time, and
what can be done about each.

**Scope:** run time only. Build time is out of scope by instruction, and the
one recommendation that touches it (§4, R6) says so rather than hiding the
trade.

**Nothing here changes what a test asserts.** Every recommendation in §4 is a
change to *how* a test spends its time, not to *what* it proves. Where a
recommendation would weaken a claim, that is said in the recommendation.

---

## 0. The headline

| | |
|---|---|
| tests measured | **1 227** across 39 test binaries (+1 doc test) |
| wall time, one test at a time | **461.7 s** |
| CPU time | **25.8 s** |
| **time spent waiting, not computing** | **435.9 s — 94.4 %** |
| `cargo test --workspace`, as a developer runs it | **421 s ≈ 7 min** (383 s of it inside the test binaries) |
| speed-up from running tests in parallel (4 cores) | **1.26×** |

> **Run-to-run variance on this box is real.** Repeating the same binary four
> times gave `girsync`/`rw_threads` at 32.6 s, 33.1 s, 33.9 s and 38.8 s — a
> spread of about 17 % on a suite whose wall time is dominated by a shared
> disk. The tables below are single runs, and the *shape* (the Pareto, the
> groups, the ratios) is stable even where the individual seconds are not.

**The suite is not slow because there is a lot to compute.** It is slow because
it spends nine-tenths of its time blocked: on `tokio::time::sleep`, on a
redb cache file, on a loopback connect that is refused after two seconds, and
on process round-trips. The 905 fastest tests (74 % of the suite) cost **4.0 s
in total** — 4.4 ms each. The 148 slowest (12 %) cost **397 s — 86 % of the
total**.

That shape is good news: it means the cost is concentrated, diagnosable, and
mostly removable. It also means that **adding tests is nearly free** and
**adding one slow pattern is not**.

One group below (G7) is different from the other six: it is not slow because
of what the test *does*, but because of what the *machine accumulates*. It is
the only one that gets worse every time the suite is run.

---

## 1. What was measured, and how

### 1.1 The machine

| | |
|---|---|
| CPU | Intel Pentium Silver N6000 @ 1.10 GHz, **4 logical processors** |
| RAM | 12 GB |
| OS | Windows (win32) |
| toolchain | `rustc 1.98.1` **stable** — no `cargo-nextest`, no nightly |
| temp dir | `E:\Symlinks\Temp` (the `TEMP`/`TMP` the tests' scratch dirs use) |
| test profile | **default** — there is no `[profile.*]` section in any of the four `Cargo.toml`s, so the crate *and every dependency* are compiled at `opt-level = 0` |

The absolute numbers are this box's. The **ratios** — wait vs CPU, file cache
vs in-memory cache, refused connect vs accepted connect — are the part that
travels.

### 1.2 Why not `cargo nextest --report-time`

The usual answer is not available here, and it is worth recording why:

- `cargo-nextest` is not installed, and installing it is a network operation
  this offline-by-construction repository does not assume.
- `--report-time` and `--format json` are **nightly-only** libtest features.
  The toolchain is stable.

So the per-test numbers below were produced by a harness (§1.3) rather than by
the toolchain.

### 1.3 The harness

Each test binary was run twice:

1. **serially** (`--test-threads=1`), for attribution;
2. **at the default thread count**, for the realistic total.

In the serial run, every `test <name> ... ok` line is a clean boundary
between two tests. The harness

- timestamps each boundary,
- samples the child process's **CPU time** (`Process.TotalProcessorTime`,
  user + kernel) at the same instant,
- samples the child process's **I/O counters**
  (`GetProcessIoCounters`: read/write operations and bytes),

and reports the **delta** between two boundaries as that test's wall time, CPU
time, and I/O volume. `wall − CPU` is the time the test spent **blocked**:
sleeping, in a file lock, in an fsync, in a connect, or waiting for a child
process.

The harness is in [`tools/testprof/`](../tools/testprof/) and is described in
Appendix B: three PowerShell scripts, one of which carries a small C#
`Add-Type` block for the I/O counters.

### 1.4 Three caveats that matter when reading the tables

- **I/O counters are the test process's.** Work done in a *spawned child* — a
  `gircap` capture, a `girsync` run — does not appear in them. A test that
  spawns the binary shows the child's cost in its *wall* time and none of it
  in its I/O columns.
- **The parallel numbers are per binary.** `cargo test` runs one test binary
  at a time; the threads are *inside* a binary. So the suite's wall time is
  the **sum** of the per-binary parallel times, and the parallel speed-up is
  only available *within* a binary. That is why the speed-up is 1.26× and not
  4×: the work is not parallelisable across binaries, and most of it is not
  parallelisable within one either.
- **Single runs, on a shared disk.** The variance note in §0 applies to every
  table here.

---

## 2. The whole picture

### 2.1 Per crate

| crate | tests | wall (serial) | CPU | **wait** |
|---|---:|---:|---:|---:|
| `girsync` (`tools/sync`) | 323 | **334.0 s** | 10.2 s | **323.8 s** |
| `girpr` (repair binary) | 268 | 65.8 s | 3.6 s | 62.2 s |
| `gircap` (capture tool) | 619 | 61.8 s | 12.0 s | 49.8 s |
| `girpr_log` (`crates/log`) | 17 | 0.06 s | 0.02 s | 0.04 s |

`girsync` is **72 % of the suite's time** and **16 % of its tests**.

### 2.2 Per test binary (serial)

| binary | tests | wall | CPU | wait | written |
|---|---:|---:|---:|---:|---:|
| `girpr`/`repair_offline` | 74 | 58.0 s | 2.4 s | 55.6 s | 0 MB |
| `girsync`/`lazy` | 27 | 43.4 s | 1.1 s | 42.2 s | 18.1 MB |
| `girsync`/`rw_threads` | 15 | 38.8 s | 2.4 s | 36.5 s | 2.8 MB |
| `girsync`/`why` | 25 | 36.3 s | 0.7 s | 35.7 s | 2.0 MB |
| `girsync`/`stat_trust` | 19 | 33.7 s | 0.7 s | 33.0 s | 15.1 MB |
| `girsync`/`sync` | 13 | 27.5 s | 0.9 s | 26.6 s | 8.4 MB |
| `girsync`/`migration_lock` | 11 | 25.9 s | 0.8 s | 25.1 s | 11.9 MB |
| `gircap`/`cli_contract` | 26 | 25.0 s | 0.3 s | 24.7 s | 0 MB |
| `girsync`/`compare` | 12 | 20.6 s | 0.7 s | 19.9 s | 11.6 MB |
| `girsync`/`output` | 14 | 20.4 s | 0.4 s | 20.1 s | 2.1 MB |
| `girsync`/`compare_self` | 12 | 20.3 s | 0.5 s | 19.8 s | 2.6 MB |
| `girsync`/`sync_plan` | 19 | 18.1 s | 0.4 s | 17.7 s | 52.8 MB |
| `girsync`/`hash_mode` | 18 | 13.4 s | 0.3 s | 13.1 s | 10.4 MB |
| `gircap`/`capture_layout` | 29 | 11.9 s | 1.9 s | 10.0 s | 0 MB |
| `girsync`/`helpers` | 10 | 9.9 s | 0.3 s | 9.6 s | 1.1 MB |
| `girsync`/`update` | 3 | 8.8 s | 0.2 s | 8.5 s | 3.3 MB |
| `gircap` lib (unit) | 491 | 8.2 s | 2.0 s | 6.2 s | 2.4 MB |
| `girsync`/`cli_dispatch` | 1 | 6.9 s | 0.1 s | 6.8 s | 4.0 MB |
| `girsync`/`convert` | 4 | 6.5 s | 0.2 s | 6.2 s | 1.7 MB |
| `girpr`/`repair_machinery` | 25 | 5.7 s | 0.6 s | 5.1 s | 0 MB |
| `gircap`/`matrix` | 8 | 4.5 s | 0.7 s | 3.8 s | 0 MB |
| `girsync` lib (unit) | 120 | 3.6 s | 0.6 s | 3.0 s | 5.5 MB |
| `gircap`/`measured_fixture` | 28 | 3.5 s | **3.4 s** | 0.1 s | 0 MB |
| `gircap`/`offline_bundle` | 9 | 3.0 s | **2.8 s** | 0.2 s | 0.1 MB |
| `gircap`/`case_coverage` | 2 | 2.6 s | 0.2 s | 2.4 s | 0 MB |
| `gircap`/`bundle_consistency` | 3 | 1.3 s | 0.2 s | 1.1 s | 0 MB |
| `girpr`/`cli_contract` | 14 | 1.0 s | 0.3 s | 0.7 s | 0 MB |
| `gircap`/`bundle_diff` | 8 | 1.0 s | 0.2 s | 0.8 s | 0 MB |
| `gircap`/`schema` | 15 | 0.9 s | 0.2 s | 0.7 s | 0.3 MB |
| `girpr` lib (unit) | 148 | 0.9 s | 0.2 s | 0.7 s | 0.5 MB |
| `girpr`/`artifact_server` | 7 | 0.2 s | 0.2 s | 0.0 s | 0.1 MB |
| `girpr_log` lib (unit) | 17 | 0.06 s | 0.02 s | 0.04 s | 0 MB |

Three of the 39 test binaries contribute no rows here: `girpr`/`artifact_bridge`
and `girpr`/`peak_heap` are `#[ignore]`d (they need a real bundle, or they are
a measurement rather than an assertion), and `gircap`/`tempdir` is a
helper module with no test of its own. The three `bin` harnesses
(`girpr`, `gircap`, `girsync`/`main.rs`) hold no unit tests either.

One row in this table moves as the machine accumulates state:
`girpr`/`cli_contract` was measured at 1.0 s, and §3 G7 explains why it can
be 0.4 s or 17 s on the same code depending on how many log files
`target/debug/logs/` holds.

### 2.3 The Pareto

| bucket | tests | share of tests | wall | CPU |
|---|---:|---:|---:|---:|
| ≥ 10 s | 6 | 0.5 % | 83.5 s | 2.0 s |
| 3–10 s | 28 | 2.3 % | 110.3 s | 2.6 s |
| 1–3 s | 114 | 9.3 % | 203.4 s | 7.7 s |
| 0.25–1 s | 92 | 7.5 % | 51.0 s | 8.7 s |
| 0.05–0.25 s | 82 | 6.7 % | 9.5 s | 2.5 s |
| **< 0.05 s** | **905** | **73.8 %** | **4.0 s** | 2.5 s |

**74 % of the tests cost 0.9 % of the time. 12 % of the tests cost 86 %.**

### 2.4 Parallel vs serial

Over the 31 integration binaries: **459.9 s serial → 363.8 s parallel**
(1.26×) on 4 cores. The best speed-ups are exactly the suites whose cost is
*sleeping*, because sleeps overlap:

| binary | serial | parallel | ratio |
|---|---:|---:|---:|
| `gircap`/`cli_contract` | 24.4 s | 6.4 s | 0.26× |
| `girpr`/`repair_offline` | 58.8 s | 16.8 s | 0.29× |
| `gircap`/`measured_fixture` | 4.2 s | 1.4 s | 0.34× |

And the suites whose cost is *disk* get **worse** in parallel, because they
contend for the same device:

| binary | serial | parallel | ratio |
|---|---:|---:|---:|
| `girsync`/`rw_threads` | 33.9 s | **40.2 s** | **1.19×** |
| `girsync`/`sync_plan` | 20.4 s | 20.8 s | 1.02× |
| `girsync`/`update` | 8.9 s | 9.0 s | 1.01× |
| `girsync`/`why` | 37.7 s | 37.6 s | 1.00× |

**A suite that is 94 % waiting cannot be parallelised, and making it parallel
can make it slower.** That is the argument for fixing the waits rather than
for adding threads.

---

## 3. The seven groups

### G1 — `girsync`: the redb cache file — **330.4 s (72 %)**

203 tests in 15 integration binaries. **330.4 s wall, 9.7 s CPU, 147.8 MB
written.** 97 % of the time is blocked.

The cause is one thing: **the cache is a redb file on disk, and every command
opens it, writes it, copies it aside and closes it.** Measured directly on
this box, same fixture, same command, only the backend differing:

| command | file-backed cache | in-memory cache |
|---|---:|---:|
| `girsync update --dir <2 files>` | **665–798 ms** | **34 ms** |
| `girsync compare --src --dst` (2 caches) | **727 ms** | **41 ms** (`--dry-run`) |
| `girsync update --dir --hash-none` (nothing to write) | 16 ms | — |

A ~20× difference, for a cache holding three file records. The file is
**288 KB** even for three records (redb preallocates), and `sync` additionally
calls `backup_db` (a copy aside) and `snapshot_old` (a second copy) before it
opens anything — see `tools/sync/src/commands/sync/mod.rs:135-157`.

The tests multiply that by the number of command invocations per test:

- `why/table.rs::every_pair_state_has_one_classification_and_one_tag` builds a
  fresh fixture and runs the pipeline **once per row** — 11 rows, **14.0 s**.
- `rw_threads/through_the_binary.rs::sync_reports_the_same_actions_under_every_rw_setting`
  spawns the binary **once per rw spelling** — 6 spellings, **13.2 s**.
- `rw_threads/through_the_binary.rs::update_hashes_the_same_set_at_every_limit`
  spawns it 6 times over a 40-file fixture, **6.6 s**.

**There is already an in-memory backend in this crate.** `CacheDb::open_temp`
(`tools/sync/src/cache/db.rs:152`) builds a redb database over
`InMemoryBackend::new()`, and `open_folder_cache` already uses it for
`--dry-run` (`tools/sync/src/effective/side.rs:160`). The tests just never
reach it, because they go through `open_db(path, …)`.

### G2 — `girpr`: the retry backoff — **45.3 s (10 %)**

Three tests, **15.1 s each**, with **0.12 s of CPU between them**:

| test | wall | CPU |
|---|---:|---:|
| `a_404_is_asked_once_where_a_500_is_asked_five_times` | 15 137.6 ms | 93.8 ms |
| `fail_fast_reaches_its_summary_in_seconds_against_a_500_origin` | 15 100.3 ms | 15.6 ms |
| `a_manifest_that_does_not_match_its_checksum_is_refused` | 15 078.0 ms | 15.6 ms |

All three provoke a retryable failure and then wait for the backoff to
elapse. The backoff is `sleep(attempt)` for `attempt` in `1..=ATTEMPTS` with
`ATTEMPTS = 5` — `src/sophon.rs:118-135` for manifests and `src/sophon.rs:394`
for chunks — so one failing fetch costs

> 1 + 2 + 3 + 4 + **5** = **15 s** of `tokio::time::sleep`.

Note the fifth sleep: it happens **after the last attempt**, so five seconds
are spent waiting for an attempt that will never be made. The capture tool
does not do this — `tools/capture/src/http/mod.rs:2527` asserts *"no wait
after the last attempt"* — and `girpr` has no equivalent.

The three tests assert what the retry policy *does*, not how long it takes:
`a_404_is_asked_once_where_a_500_is_asked_five_times` asserts the origin was
asked once versus five times, and separately that the run finished inside a
budget (`elapsed < 30 s`). **The elapsed-time assertions are the only reason
the wall clock is in the loop at all** — with a paused clock they would be
vacuous, and what they really mean is "this must not take 125 s", which is a
statement about the *policy*, not about the machine.

### G3 — `gircap`: one full capture per test — **22.2 s (5 %)**

A capture is a process against a listener: **≈ 0.45 s** on average (0.4–1.5 s
each). Counting the call sites that start one:

| suite | tests | captures started | wall |
|---|---:|---:|---:|
| `tools/capture/tests/capture_layout.rs` | 29 | **30** | 11.9 s |
| `tools/capture/tests/matrix.rs` | 8 | **11** | 4.5 s |
| `tools/capture/tests/bundle_consistency.rs` | 3 | **3** | 1.3 s |
| `tools/capture/tests/bundle_diff.rs` | 8 | **2** | 1.0 s |
| `tools/capture/tests/schema.rs` | 15 | **2** | 0.9 s |

48 captures in all, for 63 tests. `capture_layout.rs` is the extreme: 29
tests, 30 captures — **one per test, on average** — because
`capture_ok()` (`tools/capture/tests/capture_layout.rs:156`) is called from
almost every case.

**The last row is the answer.** `tools/capture/tests/schema.rs` already shares
one capture across its tests, and says why in the source:

> *"One capture, written once and shared by the tests below. `OnceLock` rather
> than one capture per test: each capture is a process against a listener, and
> five tests paying for five captures makes the suite slow for no extra
> coverage. The bundle is read-only to every test that uses it."*
> — `tools/capture/tests/schema.rs:79-88`

The other three suites that still capture per test predate that pattern or
did not adopt it. Adopting it is a copy of an existing, working pattern, not
a new idea.

### G4 — `gircap`: the refused loopback connect — **24.2 s (5 %)**

11 tests in `tools/capture/tests/cli_contract.rs` point the offline seam at
`http://127.0.0.1:1`, where nothing listens. Each waits **2.05 s**.

Measured on this box, directly:

| connect target | time to surface the refusal |
|---|---:|
| `127.0.0.1:1` (closed) | **2 177 ms** |
| `127.0.0.1:9` (closed) | 2 051 ms |
| a random closed high port | 2 046 ms |
| a bound-then-closed listener | 2 051 ms |
| **a live listener that accepts and closes** | **25 ms** |

The refusal is immediate; the *reporting* of it is not, on this machine's
loopback stack. The comment at `tools/capture/tests/cli_contract.rs:11` — *"the
request is refused immediately"* — is true of the refusal and false of the
timing.

The 11 tests are about the **log-file policy**, not about the network: they
assert that the log file is written, that `RUST_LOG` does not reach it, that
the file's level is its own. Every one of them pays 2 s to learn that a port
is closed.

### G5 — `gircap` lib: the real-sleeper retry tests — **5.6 s (1 %)**

Three unit tests in `tools/capture/src/http/mod.rs` use the real `RealSleep`:

| test | wall | CPU |
|---|---:|---:|
| `a_transport_failure_is_retried_and_reported_as_transport` | 4 088.5 ms | 0.0 ms |
| `the_write_happens_with_the_log_lock_released` | 1 203.9 ms | 0.0 ms |
| `waiting_for_a_retry_yields_the_runtime_rather_than_blocking_it` | 311.8 ms | 0.0 ms |

The same file already has the seam that fixes this: a `Sleep` trait
(`tools/capture/src/http/mod.rs:281`), a `RecordingSleep` that records delays
instead of taking them, and a `with_sleeper` constructor
(`tools/capture/src/http/mod.rs:1033`). **The retry-policy tests below it
already use the recording sleeper** (`:2413`, `:2445`, `:2486`, `:2515`,
`:2538`). The three slow ones use the real one.

### G6 — the debug profile — the remaining 12 s of CPU

Two binaries are almost purely CPU-bound, and both are the *cheap* ones:

| binary | wall | CPU | CPU share |
|---|---:|---:|---:|
| `gircap`/`measured_fixture` | 3.5 s | **3.4 s** | 97 % |
| `gircap`/`offline_bundle` | 3.0 s | **2.8 s** | 95 % |

Together they are 6.2 s of the suite's 25.8 s of CPU — 24 %. They are zstd,
md5 and protobuf arithmetic over a synthetic corpus.

Because no `[profile.*]` section exists anywhere in the workspace, **every
dependency is compiled at `opt-level = 0`** for the test profile: `zstd`,
`md5`, `sha2`, `serde_json`, `prost`. Those are exactly the crates whose
work the two binaries above are made of. This is the only recommendation in
this report with a build-time cost, and it is the smallest win — see R6.

### G7 — the accumulating `logs/` directory — **unbounded, and growing**

This one is not a fixed cost. It is a cost that **grows every time the suite
runs**, and it is the only finding here that a developer's machine gets
worse on over time without any code changing.

`girpr`'s `a_log_file_is_always_written_beside_the_executable`
(`tests/cli_contract.rs:350`) asserts that the log file lands beside the
executable. It finds its own file by **reading every file in
`target/debug/logs/` and searching each one's whole content** for a marker
string:

```rust
let mine: Vec<_> = std::fs::read_dir(&logs)
    .filter_map(|e| e.ok())
    .map(|e| e.path())
    .filter(|p| {
        std::fs::read_to_string(p)
            .map(|b| b.contains(&format!("unknown audio lang '{marker}'")))
            .unwrap_or(false)
    })
    .collect();
```

That directory is where **every** run of `girpr`, `gircap` and `girsync`
writes its log — by policy, "a log file is always written"
(`crates/log/src/plan.rs`), and the default path is `<exe-dir>/logs/`. Nothing
ever removes them.

Measured, same test, same machine, only the directory's contents differing:

| `target/debug/logs/` | the test takes |
|---|---:|
| empty (1 file) | **48 ms** |
| 1 410 files, 45 MB | **426 ms** (and **16.9 s** on a cold run, with the files evicted from the page cache) |

The test's cost is **O(the number of log files ever written)**, and the
denominator only goes up. The directory on this machine held **1 409 files
and 45 MB** by the end of the profiling runs — every spawned `girsync`,
`gircap` and `girpr` in this repository adds one, and none of them is ever
removed.

`gircap`'s equivalent test (`tools/capture/tests/cli_contract.rs:436`) has
the same shape and, on top of it, pays G4's dead-port wait: **2.23 s** with
only two files in the directory.

**This is the one finding whose cost is invisible in a fresh checkout** — it
appears only after the directory has accumulated, which is why it is easy to
miss and why it is worth a line in the report rather than a footnote.

---

## 4. What to do, ranked

Estimated savings are against the **461.7 s serial** baseline, and are
**upper bounds**: they assume the affected group goes to zero, which it will
not.

### R1 — Give the `girsync` tests an in-memory cache — **≈ −290 s**

The cache file costs **20×** what the in-memory backend costs for the same
command (§3, G1). The backend already exists: `CacheDb::open_temp`
(`tools/sync/src/cache/db.rs:152`).

What it needs is a seam so a *test* can ask for it without changing the
product. Two shapes, both small:

- **A `CacheOpen` variant.** `CacheOpen` is already the crate's "how is this
  cache opened" enum (`tools/sync/src/cache/open.rs:28`); a
  `CacheOpen::Memory` arm, or a `backend:` field on `ReadWrite`, is one
  match-site in `open_db`.
- **An environment variable.** `GIRSYNC_CACHE_BACKEND=memory`, read in
  `open_folder_cache`. Zero API change, and it cannot be set by accident in
  production.

Which tests may use it is the real question, and the answer is already in the
crate: **a test whose subject is *what the cache records* may use memory; a
test whose subject is *that the cache survives a process* may not.** The
suite's own docs make this distinction already (`migration_lock.rs` asserts
"backend-agnostic behaviour"; `hash_mode` counts reads; `rw_threads` compares
answers across settings). Most of the 203 tests are in the first camp.

*Risk:* a test that silently stops exercising the file path would lose the
coverage the file path gives. The migration to memory must therefore be
**per test and explicit**, not a global default, and the file-backed cases
must stay file-backed.

*Also worth doing, and smaller:* `why/table.rs` rebuilds the whole fixture and
runs the whole pipeline **per row** (11 rows, 14.0 s). The rows need three
different algorithm sets, one needs a record side, and one needs stat trust
switched off — the source says so (`tools/sync/tests/why/table.rs:28-33`) —
so they fall into about five configurations, not eleven. Building one fixture
per *configuration* and asserting several rows against it would remove most of
that 14 s without touching the cache backend at all.

### R2 — Pause the clock in the `girpr` retry tests — **≈ −45 s**

`tokio`'s `test-util` feature is **not** enabled: the workspace asks for
`tokio = { version = "1", features = ["full"] }`, and `full` does not include
`test-util` (verified against tokio 1.53.1's `Cargo.toml`). Adding

```toml
[dev-dependencies]
tokio = { version = "1", features = ["test-util"] }
```

enables `#[tokio::test(start_paused = true)]`, under which the runtime
auto-advances the clock whenever it is idle. The 15 s of backoff becomes
zero **wall** time, while the *number and order of the retries* — the thing
these tests actually assert — stay exactly as they are.

The elapsed-time assertions (`elapsed < 30 s`, `elapsed < 5 s`) must go, or be
restated as assertions about the policy. A paused clock makes "the run
finished in under 30 s" vacuously true; what the test should say is "the run
made five attempts and did not sleep after the last one". That is a *stronger*
claim than the wall-clock bound, which was really asserting that the machine
was not pathologically slow.

`tokio::time::pause()` also has to be checked against the mock server: the
tests run a real listener on loopback, so the I/O stays real while only the
sleeps are virtual. That is exactly what is wanted.

*Risk:* low. It changes a timing assertion into a counting assertion, which
is a stronger claim, but it is a different assertion and must be reviewed as
one.

### R3 — Adopt `schema.rs`'s shared capture in the three other `gircap` suites — **≈ −18 s**

`schema.rs` already does it (`tools/capture/tests/schema.rs:85`). Copying
that pattern to `capture_layout.rs` (30 captures → 2), `matrix.rs` (11 → 3)
and `bundle_consistency.rs` (3 → 1) is the single largest change per line of
code in this report: 48 captures become about 8, at ~0.45 s each.

Two caveats, both already handled in `schema.rs`:

- A capture that a test *writes to* cannot be shared. `schema.rs:229` copies
  the bundle for exactly that reason ("A copy, because `bundle()` is a
  `OnceLock` shared by every test in this binary").
- The shared tree outlives the run, because a `OnceLock` is never dropped.
  `schema.rs:40-52` explains the cost (two directories per run) and accepts
  it.

### R4 — Stop using a dead port as the offline seam — **≈ −24 s**

Replace `http://127.0.0.1:1` with a **live listener** that accepts and closes
(or answers with an error status). Measured: 25 ms versus 2 051 ms.

The seam must still produce the *same observable outcome* — the capture must
fail with exit `2` ("network or API failure") — so the listener has to make
the fetch fail rather than succeed. A listener that accepts and closes
immediately does that, and the assertion does not change.

This one is also **machine-specific in a good way**: the 2 s is a property of
this box's loopback stack, so on a different machine the same tests would
cost 0 s and the finding would be invisible. Replacing the dead port makes
the cost *deterministic everywhere*, which is the point.

### R5 — Use the `Sleep` seam `gircap` already has — **≈ −5 s**

`tools/capture/src/http/mod.rs` has a `Sleep` trait, a `RecordingSleep`, and
a `with_sleeper` constructor. Three tests use `RealSleep` where the
recording one would do. Swapping them is a one-line change per test and
removes the last real-clock dependency in the `gircap` unit tests.

The same seam is what R2 needs for `girpr`: `girpr` has no `Sleep` trait
today, and adding one (`src/sophon.rs`, `src/hyp.rs`, `src/repair/file.rs` —
six `tokio::time::sleep` call sites) would let the backoff *policy* be
asserted directly rather than through elapsed time.

### R6 — Optimise the dependencies in the test profile — **≈ −4 s, one-time build cost**

```toml
[profile.test.package."*"]
opt-level = 3
```

The two CPU-bound binaries (§3, G6) are made of `zstd`, `md5`, `sha2`,
`serde_json` and `prost` arithmetic, and all of them are at `opt-level = 0`
today. This is the only recommendation here that changes build time, and
**build time is out of scope** — so it is stated, not optimised: the saving is
~4 s of a 462 s suite, and the cost is a one-time rebuild of the dependency
graph. On this box, with `opt-level = 0`, that rebuild is minutes. **The
recommendation is only worth taking if the dependency rebuild is a one-off.**

### R7 — Split the suite into a fast tier and a full tier

Not a speed-up: an *ergonomics* change. The suite has a clean natural seam:

| tier | what | measured |
|---|---|---:|
| **fast** | every `#[cfg(test)]` unit test — 776 tests in 4 binaries | **12.7 s** serial |
| **full** | everything, including the binary-spawning suites | **421 s** for the whole `cargo test` |

`cargo test --lib --bins` is a **13-second** loop; the rest is the thing to
run before a release. There is an intermediate tier between the two — the
integration suites that call the library in-process rather than spawning the
binary — and it was not separately measured here, but it is where the next
split belongs.

The `#[ignore]`d suites already model this shape (`docs/06`'s "expensive and
run by hand"), and `girsync`/`stat_trust`, `girsync`/`why` and
`gircap`/`cli_contract` are the natural candidates for a `--ignored`-style
split if the fast tier is to stay under a minute.

### R8 — Stop scanning a directory that grows forever — **≈ −0.4 s now, unbounded later**

G7's test does not need to search the whole directory. It needs to find *its
own* file among the files its own run wrote, and it can know which those are
by **snapshotting the directory's contents before the spawn** and diffing:

```rust
let before: HashSet<_> = std::fs::read_dir(&logs)?.filter_map(|e| e.ok()).collect();
let (code, _, _) = run(&[...]);
let mine: Vec<_> = std::fs::read_dir(&logs)?
    .filter(|e| before.contains(&e.path()))
    .filter(|p| std::fs::read_to_string(p).map(|b| b.contains(&marker)).unwrap_or(false))
    .collect();
```

The test's own comment explains why it reads the directory at all — *"the
tests in this file run in parallel and every one of them writes a log file of
its own"* — and the snapshot handles that: a parallel sibling's file is in
`before`, so it is excluded, while this run's file is not. The assertion gets
**stricter**, not looser: today a sibling's file that happened to contain the
marker would be found and counted.

The same applies to `gircap`'s copy of the test.

Two smaller things in the same area:

- The log directory itself could be **cleaned by the suite** (or by a
  `cargo clean`-adjacent step). 45 MB of accumulated logs in `target/` is
  its own kind of waste, and `target/debug/logs/` is inside a gitignored
  tree, so nothing about it is precious.
- The default log path is `<exe-dir>/logs/`, which for a *test* is inside
  `target/debug/`. Nothing needs it to be there.

---

## 5. Scale reductions

The user asked which groups have a *scale* that can be reduced, not only
which are slow. The distinction matters: some of the cost is the number of
times a fixture is built, and some is the size of the fixture. **On this
suite, the size of the fixture is almost never the lever — the number of
times it is built is.**

| what | current scale | what it costs | reduced scale | lever |
|---|---|---|---|---|
| `girsync` cache cycles per test | 1–10 redb file lifecycles | 665 ms each | in-memory backend | **R1** |
| `girsync` `why/table.rs` | 11 rows × a full fixture + pipeline | 14.0 s | ≈ 5 configurations | group rows by configuration |
| `girsync` `rw_threads/through_the_binary.rs` | 6 binary spawns per test | 13.2 s + 6.6 s | 1 spawn per *distinct answer*, not per spelling | assert the settings are equivalent at the library level, then one spawn |
| `gircap` `capture_layout.rs` | 30 captures | 11.9 s | 2 | **R3** |
| `gircap` `matrix.rs` | 11 captures | 4.5 s | 3 | **R3** |
| `gircap` `bundle_consistency.rs` | 3 captures | 1.3 s | 1 | **R3** |
| `gircap` `bundle_diff.rs` | 2 captures, in one test | 1.0 s | 2 | already shared |
| `gircap` `cli_contract.rs` | 26 spawns, 11 of them to a dead port | 25.0 s | 26 fast spawns | **R4** |
| `girpr` `repair_offline.rs` retry cases | 5 attempts × 15 s of backoff | 45.3 s | 5 attempts, 0 s of backoff | **R2** |
| `target/debug/logs/` | one file per program run, never cleaned | 48 ms → 426 ms as it fills | bounded | **R8** |
| `gircap` `common::World::all_shapes()` | 8 game files + 1 audio file per build; 256 B – 32 KB | 6–39 KB per build | **unchanged** | see below |
| `gircap` `measured_fixture.rs` corpus | synthetic, ~2 744 real entries reduced to a handful | 3.4 s of CPU | **unchanged** | see below |

**Two scale reductions that are *not* worth making, stated so they are not
made:**

- **The capture fixture's byte sizes.** `World::all_shapes()` uses files of
  256 B, 512 B, 1 KB, 2 KB and 32 KB — 6 KB for the new side, 39 KB for the
  old one. A capture's cost is driven by the *number of chunks and requests*,
  not the bytes: the 32 KB `old.blk` costs the same as the 512 B
  `browser_assets` to fetch, verify and assemble. Shrinking the bytes changes
  nothing measurable.
- **The `measured_fixture` corpus.** It is already the reduced form — the file
  says so: *"A 2 744-entry fixture would be a megabyte of checked-in JSON to
  re-derive the same handful of identities."* Its 3.4 s of CPU is the price of
  the corpus being the *real* one, and R6 (not shrinking) is the right lever.

**One scale reduction that is worth making and is not in the table above:**
`gircap`/`cli_contract.rs` has 26 tests and 26 process spawns. Several of them
assert on the *same* run's output — the log-file policy tests all read one
capture's log file. One spawn whose log file is then read by several
assertions would remove ~20 spawns. That is a test-shape change, not a speed
hack: it makes the log-file policy one fact observed several ways rather than
eleven near-identical runs.

---

## 6. A note on the product side (not a test change)

`girpr`'s retry loop sleeps `attempt` seconds after **every** failed attempt,
including the last:

```rust
// src/sophon.rs:118-135
for attempt in 1..=ATTEMPTS {          // ATTEMPTS = 5
    match try_fetch_manifest(...).await { ... }
    tokio::time::sleep(Duration::from_secs(attempt)).await;
}
```

So a fetch that fails five times spends **15 s**, of which the last **5 s**
buy nothing — the loop is about to return the error. The capture tool already
avoids this (`tools/capture/src/http/mod.rs:2527` asserts it).

That is a **product** behaviour change, not a test change, and it is out of
scope for this report. It is recorded here because it is the same 15 s the
tests are paying: fixing it in the product would make R2 unnecessary for the
*duration* of the backoff, and would make a real failing repair 5 s faster.

---

## 7. What this report does not claim

- **Build time is out of scope.** R6 is the only recommendation with a build
  cost; it is stated, not optimised, and its saving (~4 s) is the smallest
  here.
- **The absolute numbers are this box's.** A machine with a fast loopback
  stack would not show G4 at all; a machine with a fast disk would not show
  G1. The *ratios* are the transferable part, and every recommendation in §4
  is aimed at a ratio rather than at a number.
- **I/O counters are the test process's** (§1.4). Child-process work shows up
  in wall time only.
- **No CI configuration exists in this repository**, so "the CI cost" is this
  box's number. There is no `.github/` and no pipeline file.
- **The suite was measured with `cargo test`'s own build**, i.e. the `test`
  profile with `opt-level = 0` for all dependencies. A release-profile
  measurement would show a different CPU column and the same wait column.
- **No test was changed.** This is a measurement, a set of proposals, and the
  instrument that produced the measurement. The two things added to the tree
  are this document and `tools/testprof/`.

---

## Appendix A — the 25 slowest tests

| test | binary | wall | CPU |
|---|---|---:|---:|
| `a_404_is_asked_once_where_a_500_is_asked_five_times` | `girpr`/`repair_offline` | 15 137.6 ms | 93.8 ms |
| `fail_fast_reaches_its_summary_in_seconds_against_a_500_origin` | `girpr`/`repair_offline` | 15 100.3 ms | 15.6 ms |
| `a_manifest_that_does_not_match_its_checksum_is_refused` | `girpr`/`repair_offline` | 15 078.0 ms | 15.6 ms |
| `table::every_pair_state_has_one_classification_and_one_tag` | `girsync`/`why` | 14 040.4 ms | 359.4 ms |
| `through_the_binary::sync_reports_the_same_actions_under_every_rw_setting` | `girsync`/`rw_threads` | 13 234.1 ms | 671.9 ms |
| `limiter::every_rw_setting_answers_the_same_question` | `girsync`/`rw_threads` | 10 900.2 ms | 796.9 ms |
| `run_cli_dispatch_update_compare_sync` | `girsync`/`cli_dispatch` | 6 907.3 ms | 140.6 ms |
| `through_the_binary::update_hashes_the_same_set_at_every_limit` | `girsync`/`rw_threads` | 6 607.9 ms | 187.5 ms |
| `lock_no_trust_cached_hashes_is_per_side` | `girsync`/`migration_lock` | 5 329.2 ms | 156.2 ms |
| `as_done::the_rw_limit_does_not_change_what_is_reported` | `girsync`/`sync_plan` | 5 057.6 ms | 0.0 ms |
| `a_lazy_sync_can_leave_a_record_that_cannot_answer_and_that_is_now_an_error` | `girsync`/`compare_self` | 4 480.3 ms | 93.8 ms |
| `as_done::a_multi_file_parallel_copy_reports_the_same_set_whatever_the_order` | `girsync`/`sync_plan` | 4 426.0 ms | 46.9 ms |
| `cache_repair::no_trust_size_rehashes_a_stale_row_that_the_short_circuit_would_leave_stat_only` | `girsync`/`stat_trust` | 4 316.2 ms | 234.4 ms |
| `flags::why_does_not_change_the_exit_code` | `girsync`/`why` | 4 276.6 ms | 15.6 ms |
| `read_count::any_of_avoids_backfilling_an_algorithm_the_pair_already_shares` | `girsync`/`hash_mode` | 4 094.7 ms | 140.6 ms |
| `http::tests::a_transport_failure_is_retried_and_reported_as_transport` | `gircap` lib | 4 088.5 ms | 0.0 ms |
| `a_cache_row_for_a_path_deleted_after_the_run_is_dropped` | `girsync`/`sync` | 4 076.5 ms | 93.8 ms |
| `read_count::any_of_still_hashes_when_the_pair_shares_no_algorithm` | `girsync`/`hash_mode` | 3 960.6 ms | 109.4 ms |
| `run_update_prunes_and_excludes` | `girsync`/`update` | 3 845.4 ms | 78.1 ms |
| `verdict::no_trust_mtime_clears_a_touched_file_whose_bytes_never_changed` | `girsync`/`stat_trust` | 3 668.4 ms | 62.5 ms |
| `flags::show_identical_is_default_off_and_independent_of_why` | `girsync`/`why` | 3 649.2 ms | 15.6 ms |
| `the_converter_reports_json_records_on_stdout` | `girsync`/`convert` | 3 635.6 ms | 15.6 ms |
| `sync::a_folder_prunes_its_own_cache_before_the_planner_sees_it` | `girsync`/`lazy` | 3 459.5 ms | 62.5 ms |
| `compare::stat_equal_pair_costs_only_the_side_missing_the_algorithm` | `girsync`/`lazy` | 3 313.5 ms | 15.6 ms |
| `run_sync_keep_extra_spares_extra_dirs_too` | `girsync`/`sync` | 3 276.6 ms | 93.8 ms |

---

## Appendix B — reproducing the measurement

The harness is in [`tools/testprof/`](../tools/testprof/): three PowerShell
scripts and a README that says what each one does and what it cannot do.

| script | what it does |
|---|---|
| `tools/testprof/measure.ps1` | runs **one** test binary serially and writes one row per test: wall ms, CPU ms, read/write ops and bytes |
| `tools/testprof/drive.ps1` | runs `measure.ps1` over every test binary in the workspace, serially and at the default thread count |
| `tools/testprof/aggregate.ps1` | turns the per-test CSVs into the tables this report quotes |

The core of `measure.ps1`:

```powershell
$p = New-Object System.Diagnostics.Process
$p.StartInfo = $psi
$null = $p.Start()
$stdout = $p.StandardOutput

while ($true) {
  $line = $stdout.ReadLine()
  if ($null -eq $line) { break }
  $t = $sw.Elapsed.TotalMilliseconds
  if ($line -match '^test (.+?) \.\.\. (ok|FAILED|ignored)$') {
    $cpu = $p.TotalProcessorTime.TotalMilliseconds
    $io  = [IoCounters]::Read($p)     # GetProcessIoCounters
    # ... record the deltas against the previous boundary ...
  }
}
```

Three Windows-specific details the harness had to get right, recorded here
because they are easy to get wrong — and because each one produces a
*plausible but wrong* number rather than an error:

1. **`$p.StandardOutput` is null until `Start()` has been called**, and
   **enabling `BeginOutputReadLine` detaches it again**. The reader must be
   captured *between* `Start()` and the attach.
2. **stderr must be drained asynchronously**, or a chatty test deadlocks
   against a full pipe. The `DataReceivedEventHandler` has to be attached from
   C# (`Add-Type`), because a PowerShell script-block delegate needs a
   runspace and fails on the thread-pool thread the event arrives on.
3. **The child must be the test binary itself**, not `cmd.exe` wrapping it —
   `cmd.exe`'s `TotalProcessorTime` does not include its grandchild's CPU,
   which would silently halve every CPU figure.

To re-run the whole profile:

```powershell
cargo test --workspace --no-run            # build the harnesses
.\tools\testprof\drive.ps1                 # per-test rows  -> target\testprof\*.csv
.\tools\testprof\aggregate.ps1             # the tables     -> target\testprof\agg\
```

`drive.ps1` takes about 15 minutes on the machine this report was measured on
(serial plus parallel over every binary); `aggregate.ps1` is instant.
