# 09 — Test-scale reduction research (all workspace projects)

**Status:** research only — no source files changed.
**Date:** 2026-11-18
**Method:** five parallel read-only audits, one per area: root product integration
tests, root product unit tests + production code, `tools/sync`, `tools/capture`,
and the small crates (`crates/log`, `crates/tempdir`, `tools/sled2redb`) plus
cross-cutting concerns. Every auditor read the production code behind the tests
it assessed. The root-integration findings below were independently re-checked
against the source during aggregation.

## Scope and framing

The goal of this pass is to reduce the **scale** of the suite, not its runtime.
In scope: fewer test functions, fewer files/directories/HTTP responses created
to make a gate fire, fewer process spawns, test-level downshifting
(e2e → integration → unit), replacing stress/measurement proofs with scale-free
invariants, and merging/removing tests. Production code changes are explicitly
allowed and expected. An optimization does **not** need a measurable runtime
win. A separate upcoming session will look at delegating hand-rolled code to
third-party libraries; that is **out of scope** here — no recommendation in this
report depends on adding a crate purely to hand code off.

### Baseline (measured)

| Area | Test attributes (approx.) |
|---|---:|
| root crate (`src/**` + `tests/**`) | ~648 |
| `tools/capture` | ~859 |
| `tools/sync` | ~625 |
| `crates/log` | ~22 |
| `crates/tempdir` | ~9 |
| `tools/sled2redb` | ~12 |
| **Total** | **~2,175** (the earlier audit measured 2,260–2,268 including macro-generated cases) |

Twelve-plus `#[ignore]`d cases exist across `src/util/md5.rs`,
`tests/artifact_bridge.rs`, `tests/peak_heap.rs` and
`tools/capture/tests/bundle_shapes.rs`; they are skipped by the default gate.

### What was already done — excluded from this report

An earlier suite-reduction audit (`docs/08`, since removed) and its T01–T13
follow-up commits already applied the obvious merges/removals: CLI self-test
tables, duplicated fixture-helper meta-tests, `run_lock` clusters, hash dedup
tables, cross-binary log-policy smoke-only cases, sled2redb help/usage folding,
purge classifier consolidation, and the `girpr-tempdir` fixture sharing. None of
those are re-recommended here. The `nextest.toml` test-only hooks
(`GIRSYNC_TEST_DURABILITY`, `GIRSYNC_TEST_NO_FSYNC`, `GIRPR_TEST_NO_FSYNC`,
`GIRSYNC_TEST_HEARTBEAT_ENTRIES`, zero-backoff retry) are precedents for lowering
production constants behind a test seam and are treated as accepted practice.

## Findings by area

### A. Root product integration tests (`tests/**`)

#### A1 — DOWNSHIFT: `repair_machinery.rs` spawns the binary in cases that only read `Summary` fields

**Location:** `tests/repair_machinery.rs` — `Rig::run` / `Rig::run_at_level`
(:181–244), and the 28 tests in the file (notably :410, :450, :622, :693,
:789, :830, :1144, :1184, :1215, :1249, :1317, :1350, :1372, :1391, :1427,
:1471, :1502, :1548).

**Current state.** Every test goes through `Rig::run_at_level`, which spawns
`CARGO_BIN_EXE_girpr` (:218), forces `--output json` (:206–207) and writes a
per-run log file into a second `TempDir` (:194, :208–215), then parses NDJSON
and reads counters via `run.num("summary", …)`. The file header (:10–18) claims
the figures are "only visible on the data plane", but `girpr::run` returns a
public `Summary` (`src/lib.rs:71,74–76`) whose `download`/`write` meters,
`ram_bytes`, `ram_limit`, `flags.local_manifest`, `files_inplace`/`files_temp`
and other counters are all public (`src/report/summary.rs:49–120,215–236,299`).
`summary_record(&summary, code)` reproduces the same record in-process.

**Independently checked.** The file has 28 test attributes and 35 `run(...)`
invocations; `.log()` is called at 29 sites, all concentrated at :594–600 and
:864–1129 plus one at :1598 — i.e. roughly 13–15 tests genuinely need the
binary's log file. **Assumption to validate at implementation time:** ~13–18
of the 28 cases can be converted without losing an assertion.

**Proposed change.** Add `Rig::run_lib(&self, flags: &[&str])` that builds a
`RunCtx` through the existing `common::ctx_for(...)`, maps the few flags used
(`--scan`, `--repair-mode`, `--max-ram-bytes`, `--max-write-bytes`, `--include`,
`--fail-fast`) onto its public fields, calls `common::run(ctx)`, and returns
`(Summary, RunExit)`; read values via `summary_record(...).text()` plus the
existing `common::field` helper (the pattern already used by
`repair_writer.rs`/`repair_limits.rs`). Keep the ~10–13 log-asserting tests on
`Rig::run`; keep at least one binary case exercising the deprecated
`--io-threads` alias (:185–186).

**Coverage / trade-off.** All asserted numbers, exit codes, chunk-fetch sets,
on-disk bytes and scopes are identical. Traded: these specific cases no longer
exercise `--output json` / `--io-threads` wiring, which is separately pinned in
`cli_contract.rs`. **Risk:** low–medium (mechanical harness change).
**Scale effect:** removes ~20–25 process spawns, ~20–25 log files and their
`TempDir` traffic; the test count is unchanged unless cases are also merged.

#### A2 — DOWNSHIFT + MERGE: `repair_scan.rs` estimate/mode tests

**Location:** `tests/repair_scan.rs` — :194–259, :262–356, :359–429, :512–580,
:583–652, :655–760 (~14 binary spawns across six tests).

**Current state.** These spawn the binary and parse stdout with `field(...)`,
though the values asserted (`RunExit`, `summary.download.settled()`,
`plan_download_bytes`, the `--plan-out` JSON written by the library, chunk hit
sets) are all obtainable from `common::run(ctx)` plus `summary_record`. The
comments at :225–229 and :394–397 already justify avoiding a separate
`--print-plan` spawn — the same reasoning applies one level down.

**Proposed change.** Convert read-only/estimate cases to the library harness;
merge the two `scanned estimate` tests into one table over `{inplace, temp}`
with a shared estimate ↔ document ↔ real-download parity assertion. Keep one
binary run per test only where the assertion is genuinely process-level:
stdout negatives (`no PLAN`, `no CHECK`) and the stderr retry wording at
:728–744.

**Coverage / trade-off.** Exit-code gradations, disk immutability, plan-out
content and fetch sets are unchanged; the process-only stdout/stderr pins stay
on the binary. **Risk:** medium. **Scale effect:** ~12–15 fewer process spawns
and mock servers; 3 tests → 2.

#### A3 — MERGE (partial downshift): `repair_plan.rs` audio-plan cases

**Location:** `tests/repair_plan.rs:117–219`.

One test loops three cases, each spawning a fresh `MockServer` and binary. Only
`no_scope` asserts a binary-level fact (`audio_langs=en-us` in stdout, :189–195);
`scan_out`/`scan_in` only assert exit 0, a PLAN line and filesystem negatives —
all library-observable. Keep the binary spawn for `no_scope`; run the other two
through `ctx` with `print_plan = true` and the audio/scope fields set. **Risk:**
low. **Scale effect:** −2 spawns and mock servers, test count unchanged.

#### A4 — MERGE: `process.rs` phase-logging and env-seam clusters

**Location:** `tests/process.rs:449`, :549, :591 (phase logging) and :405, :432
(env seam).

Three near-identical tests each build the same install and spawn the binary to
read phase lines from a log file, covering `{scan, check-only, unscanned,
print-plan, purge-before}`; two more differ only in flag-vs-env precedence.
Fold into one `phase_logging` table keyed by `(mode flags, phases present,
phases absent)` and one two-row env table. **Risk:** low.
**Scale effect:** 5 tests → 2, ~4 fewer spawns/mock servers.

#### A5 — REPLACE_STRESS: `peak_heap.rs` measurement → scale-free invariant

**Location:** `tests/peak_heap.rs` (whole file, 357 lines); related unit test
`src/sophon/stream.rs:634–659`.

**Independently checked.** The file is an `#[ignore]`d, non-asserting
measurement: a custom `#[global_allocator]` (:88–133), a streaming HTTP server
(:223–266), 256 KiB incompressible payload (:198–210) and a printed MiB ratio
(:276–357). It is the sole reason the workspace carries an `unsafe` exemption
(`Cargo.toml:26–29`, file-level `#![allow(unsafe_code)]` at :60). The
correctness it protects — streamed decode equals buffered decode — is already
pinned scale-free by `the_streamed_decode_agrees_with_the_buffered_one` using
64 KiB and `futures_util::stream::iter`, no socket, no allocator.

**Proposed change.** Replace with an always-on invariant test using the
existing `ChunkFetch`/`ChunkSink` seam: a counting sink records each `write`
call while `stream_chunk_to` streams a multi-piece in-memory body; assert
piece count > 1, accumulated bytes equal the payload, and no single write
equals the whole payload (so reintroduced buffering fails). Delete the allocator,
TCP server, incompressible payload and `#[ignore]`; optionally drop the
workspace's `unsafe` exception and its documentation.

**Coverage / trade-off.** What is traded is the quantitative heap ratio (a
human-read number), replaced by a structural bound that fails on buffering.
**Risk:** medium — the invariant must not pass vacuously.
**Scale effect:** −357-line test file, −1 spawned server task, −256 KiB payload
and zstd encode, −1 log sink, −1 workspace `unsafe` exemption. No runtime win
(the test was ignored); this is pure complexity reduction.

#### A6 — REMOVE: duplicated process-level "removed capture" case

**Location:** `tests/process.rs:282–330` vs `tests/repair_purge.rs:257–307`.

The process test's own comment says it is "the same case as the library-level
case, through the real binary … exit 1 is what a caller sees", but it asserts
`code == 0` (:309); its stated process-only distinction does not exist. The
library test asserts the same disk state, empty plan, zero chunk hits and
`config_bumped=0`/`scoped=1`; CLAP acceptance is in `cli_contract.rs`. Remove
the process case, or keep a reduced one-line smoke that only checks the flag
combination exits 0 and prints `SUMMARY total=0`. **Risk:** medium if a flag
smoke is wanted. **Scale effect:** −1 test, −1 spawn/mock server, ~−5 file writes.

#### A7 — MERGE: `repair_purge.rs` plan-out and purge-only clusters

**Location:** `tests/repair_purge.rs` — :422/:455 (plan-out before/after) and
:310/:359/:492 (purge-only trio). Same fixture shape, largely overlapping
assertions. Two tables over `{purge_before, purge_after}` and
`{purge_only, purge_only+dry-run, purge_only+check_only}`. **Risk:** low.
**Scale effect:** ~2 tests merged, ~2 mock servers and temp dirs saved.

#### A with nothing new to report

`tests/run_lock.rs` / `tests/common/run_lock.rs` are already exhaustively
table-driven; `tests/artifact_server.rs` / `artifact_bridge.rs` are synthetic or
deliberately ignored hand-run cases; `tests/repair_limits.rs`,
`repair_writer.rs`, `repair_scope.rs` are already library-based with 2-file
fixtures; `tests/scripts/io-profile/*` are standalone on-demand benchmark crates
outside the test gate.

### B. `tools/capture` (`gircap`)

The dominant scale cost here is the **shared bundle**: each `BundleVariant` means
a separate `Mock::start` + spawned `gircap capture` process + full on-disk bundle
tree, published under a cross-process lock and fingerprinted on all of `src/**`
(`tests/common/mod.rs:58–85,111–169`). `bundle_source_key()` means any edit to
`src` forces a one-time recapture — friction to sequence around, not a scale
problem itself.

#### B1 — REMOVE / DOWNSHIFT: delete the `Ceiling` bundle variant

**Location:** `tests/schema.rs:298–303`
(`the_ceiling_variant_records_the_ceiling_it_used`),
`tests/common/mod.rs:64–65,73,82` (`BundleVariant::Ceiling`, key
`"ceiling123456"`, flag `--max-decompressed-bytes 123456`).

**Checked:** `BundleVariant::Ceiling` has exactly one consumer — the schema test
above. It captures a third complete bundle only to assert
`MANIFEST.json["max_decompressed_bytes"] == 123456`.

**Proposed change.** Delete the variant and the test; add a unit test in
`src/emit/index.rs` beside `the_manifest_is_whole_when_build_manifest_returns_it`
(~:1463) using the existing `Fixture::input()` (:1428): set
`max_decompressed_bytes = 123456` and assert `build_manifest(...)` carries it.
`build_manifest` (:72) is pure and copies the field directly (:158); the
flag→plan step is already pinned in `src/config/tests.rs:153–154`.

**Coverage / trade-off.** Only "a non-default value survives an actual process"
is lost — the same code path with a different integer. If that must stay, fold
the field check into `the_gzip_variant_records_its_encoding_decisions`
(`schema.rs:450`) by giving that variant the flag too. **Risk:** low.
**Scale effect:** removes 1 of 3 full captures (−1 mock, −1 process, −1 whole
bundle tree, −1 lock entry) and one `BundleVariant` arm.

#### B2 — DOWNSHIFT: `a_device_name_from_a_flag_is_still_refused` → config unit test

**Location:** `tests/capture_layout.rs:1739–1760`.

The case starts `Mock::start(World::game_only())` and spawns `gircap capture
--files GenshinImpact_Data/Resources/CON` — but `--files` values are rejected in
`config/mod.rs:348` via `validate_paths` → `paths::path_rule(..., allow_device =
false)` (`paths.rs:67–140`) **before any request is made**; the mock is never
contacted. The same rule is already unit-tested at `src/paths.rs:383`, and
`--files` refusal is already at `src/config/tests.rs:240–257`.

**Proposed change.** Add that path to the `bad[]` list in
`config/tests.rs:242–249`, delete the integration test. Keep the contrasting
`a_device_name_manifest_path_is_kept_and_escaped_on_disk`
(`capture_layout.rs:1630`) — that one genuinely needs the full round-trip.
**Risk:** low. **Scale effect:** −1 test, −1 process, −1 mock server.

#### B3 — REPLACE_STRESS / REDUCE_IO_SCALE: downscale the `m7_assembly` megabyte bodies

**Location:** `tests/measured_fixture.rs` helpers `measured()` (:1277–1288),
`seeded()` (:1292–1297), `hoyo_pair()` (:1321–1352), `assemble_measured()`
(:1354–1382); four tests at :1385, :1411, :1440, :1463.

**Checked.** Every one of the four calls `assemble_measured(&hoyo_pair())`,
which builds `690_719 + 1_626_019 + 23_526` bytes per side (~4.7 MB total,
~28 MB allocated + hashed across four tests). The large sizes exist only to pin
literal totals (`download_bytes == 274_426`,
`materialize_old/new_bytes == 2_340_264`, `n.bytes.len() == 2_340_264`,
`uncompressed_size == 690_719`). None of the assembly decisions (reuse,
A7–A12, patch-vs-direct byte equality, `must_not_fetch`) depend on the
magnitudes.

**Proposed change.** Parameterize `hoyo_pair` with the three lengths; keep one
test on the measured lengths to preserve the ground-truth pin, run the other
three with tiny lengths (e.g. 64/128/16) and express their size assertions as
scale-free relations (`download_bytes == sum(fetch chunk sizes)`,
`uncompressed_size == chunk.uncompressed_size`, …). No production change needed
— `assemble::assemble` already accepts `ChunkSource::from_memory`.

**Coverage / trade-off.** The structural identity that gives this file its
ground-truth value (real id prefixes, one chunk shared across both builds, two
different chunks at the same offset) is untouched. **Risk:** low–medium (verify
the rewrites equal the old literals at measured sizes). **Scale effect:** ~28 MB
→ <10 KB of bodies/hashing; 4 tests kept.

#### B4 — MERGE: the four "goes to the mock / no network" full captures → two

**Location:** `tests/capture_layout.rs` — :205
(`every_request_the_capture_made_went_to_the_mock`), :1092
(`the_request_log_rows_are_what_the_spec_says_they_are`), :1339
(`every_request_url_in_the_bundle_is_a_loopback_address`), :1368
(`the_chunk_cdn_base_comes_from_the_mock_not_from_the_region`).

Each independently proves "the run only talked to the loopback mock": path
prefixes (:205–216), recorded URLs (:1339–1361), and the stored CDN base
(:1368–1401). Loopback is guaranteed by construction (`Mock::start` binds
`127.0.0.1:0`, `common/mod.rs:1110–1112`); the product property worth pinning is
that the tool uses the bases it was given — exactly what :1368 asserts.

**Proposed change.** Keep :1368 and fold the distinct assertions from :205
(prefixes + `getGameBranches < getBuild` ordering) and :1092 (request-log row
shape) onto its one bundle; delete :1339 as redundant with the other two.
**Risk:** medium (check exact row/byte pins still hold on the shared capture —
same world ⇒ same requests). **Scale effect:** 4 tests → 2, −2 full captures,
−2 mock servers.

#### B5 — DOWNSHIFT: `the_gzip_variant_records_its_encoding_decisions` → 2-file emit test

**Location:** `tests/schema.rs:450–~510`, `tests/common/mod.rs:64,72,81`
(`BundleVariant::GzipThreshold`, key `"gzip1000"`).

The decision rule `hash::should_gzip` is already exhaustively tested at
`schema.rs:424–442` (both sides of the boundary). Replace the full-capture test
with a small disk test: two in-memory-sized files (one below, one above 1000 B)
through the existing `emit::index::digest_file(path, encoding)` (:226) plus
`should_gzip(len, threshold)`, asserting one `gzip` and one `identity` encoding.
**Coverage / trade-off.** The document-level fact on the default bundle remains;
what is lost is "a real capture wrote the encoding it decided" — confirm
`digest_file` is the path `emit::tree::write` uses, otherwise keep a thin e2e
smoke. **Risk:** medium (least certain of this group). **Scale effect:** removes
the second full capture — combined with B1, one capture for the whole suite.

#### B6 — STRATEGIC (scoped follow-up, not a first move): in-process capture seam

Across `tools/capture/tests/*.rs` there are ~15 `bin()`/`Command::new` helpers
and ~50 spawned `gircap` processes, usually with a hand-rolled `Mock::start`
TCP server. `run::capture/verify/materialize/list` are already public library
APIs returning `Outcome`/`Failure`, so the process spawn can be removed by
calling them directly and asserting on `Outcome.0` records instead of parsed
stdout. Removing the TCP mock too would need an injected HTTP transport trait
for `Fetcher` (today it hardcodes `reqwest::Client` in `Fetcher::new`,
`src/http/fetcher.rs`); the project's preferred injection pattern is
`Arc<dyn Sleep>` in `src/http/transport.rs:72–135`. This is the largest lever,
but wide and risky: current mocks intentionally pin byte-exact HTTP bodies and
request logs. B1–B5 capture most of the benefit at a fraction of the risk.

#### B — non-findings (skeptical checks)

`verify/tests.rs`, `select/tests.rs`, the `emit/index.rs` a16 family, and the
bundle/schema/offline-bundle suites (plus 3 ignored `bundle_shapes` cases) have
no new redundant cluster. `blobs/tests.rs:644–1463` has eight near-identical
async blob-measurement tests, but each pins a distinct state of a real
source-competition state machine — keep. `assemble/tests.rs`'s 35 tests are each
a distinct assertion. `common/mod.rs`'s source-hash friction is noted in B6's
sequencing, not "fixed".

### C. Root product unit tests and production code (`src/**`)

#### C1 — REPLACE_STRESS: `retry.rs` injectivity by sampling → coprimality proof

**Location:** `src/retry.rs:298–312`
(`jitter_is_injective_over_an_attempts_caller_range`), production `jitter` at
:138–146 with `RANGE`/`MULTIPLIER` at :139–140.

For 5 attempts the test calls `jitter(attempt, salt)` for `CALLERS = 4096`
salts and inserts each spread into a `HashSet` — 20,480 calls + hash inserts.
The production comment itself says the first real repeat is a salt difference
of `RANGE = 250_000_000`, so the test samples ~0.0016% of the property.

**Proposed change.** Lift `RANGE`, `MULTIPLIER` and the per-attempt constant
`0x9E37_79B9_7F4A_7C15` to module-level `const`s (no behavior change), then
replace the loop with a scale-free proof: for fixed `attempt`,
`jitter_nanos(salt) = (salt·M + c) mod RANGE`, a bijection over the full period
exactly when `gcd(M, RANGE) == 1`; assert that with a ~10-step Euclid loop
(`RANGE = 2^7·5^9`; `MULTIPLIER = 0x9E3779B1` is odd and not divisible by 5).
Keep a 16-salt empirical sanity loop for a concrete example.

**Coverage / trade-off.** The coprimality proof is **stronger** than the sample
— it proves injectivity over the whole period. If `jitter` became non-linear
the gcd check would no longer suffice, but `backoff_is_linear_with_a_bounded_spread`
(:284) still catches gross regressions. **Risk:** low.
**Scale effect:** ~20,480 hash inserts → ~50 calls + ~10 arithmetic steps.

#### C2 — PROD_CHANGE + REDUCE_IO_SCALE: inject the MD5 buffer, shrink 513 KiB files

**Location:** `src/util/md5.rs:664–676`
(`a_file_larger_than_one_block_is_scanned_across_the_boundary`) and :678–702
(`a_handle_digest_agrees_with_the_file_digest_at_any_position`); production
`scan_reader` at :211–281 (buffer at :233) and `md5_reader_digest` at :370–382
(buffer at :372).

To cross the 512 KiB internal buffer, test 1 writes `MD5_BUF + 1000` bytes (:669)
and test 2 writes one `MD5_BUF + 17` file plus two tiny ones (:681–688) — about
1 MB of file IO and two ~513 KiB heap buffers per run.

**Proposed change.** Extract private `scan_reader_with_buf(...)` /
`md5_reader_digest_with_buf(...)` that accept `&mut [u8]`; production functions
allocate `vec![0u8; MD5_BUF]` and delegate. Tests pass a 64-byte buffer with a
~100-byte file and keep the straddle assertion (`start = buf.len() - 10`,
len 20).

**Coverage / trade-off.** Identical loop/range arithmetic; crossing a 64-byte
boundary exercises the same `block_end`/`start`/`end` logic. Not an RFC vector
or a bench, so outside the protected set. **Risk:** low–medium (private
signature extraction, no behavior change). **Scale effect:** ~1 MB file IO
and allocation → <1 KB.

#### C3 — MERGE: `phases/plan.rs` layout verdict cluster

**Location:** `src/repair/phases/plan.rs:564–614` and :688–719.

Six near-identical `file_with(...).layout()` tests (:565, :574, :584, :592,
:600, :608) plus `the_cheap_tiling_check_agrees_with_the_layout` (:692), which
rebuilds the same six cases in a table to assert `is_tiled() == layout().tiled`.

**Proposed change.** One table of `(&str, PlannedFile, bool /* tiled */,
gaps, overlaps)` over all six cases; assert all three fields per row **plus**
`f.is_tiled() == layout.tiled`. Keep a two-line order-independence assertion
(or a reversed-chunks row). Delete the seven tests.

**Coverage / trade-off.** All exact verdicts retained and the `is_tiled`
agreement is now checked on every case — strictly stronger. **Risk:** low.
**Scale effect:** 7 tests → 1–2; 12 case constructions → 6.

#### C4 — REMOVE: `clock_support` meta-tests

**Location:** `src/lib.rs:357–374` (helper at :346–355).

`a_pre_epoch_fixture_skips_without_the_switch` (:362) merely calls
`skip_pre_epoch_decision(false, ...)` and asserts nothing;
`a_pre_epoch_fixture_fails_with_the_switch` (:369) `#[should_panic]`-tests the
same test-support helper. Both test a `cfg(test)` skip helper, not product
behavior — the same category the prior pass removed in `path.rs`/`ini.rs`.
Real clock behavior is covered at `lib.rs:246/301/312` and `inplace.rs:1950+`.

**Proposed change.** Delete the nested `mod tests` (357–374). **Risk:** low.
**Scale effect:** −2 tests, −1 module. Traded: a regression in the skip switch
itself would no longer be caught directly (negligible).

#### C5 — PROD_CHANGE + DOWNSHIFT: pure `DiskPlan::from_figures` seam

**Location:** `src/repair/scope/report.rs:557–589`, :632–654, :698–709,
:747–770; production `DiskPlan::new` at :80–143.

Four tests create four temp dirs and six files solely to drive `DiskPlan::new`,
which does two stats per file (`util::file_len` at :112 and `resolve_mode`'s
`symlink_metadata` at `file/mod.rs:688`). The pure single-file seam
`planned_on_disk` (:161–175) already exists.

**Proposed change.** Extract
`DiskPlan::from_figures(plan, impl Iterator<Item=(FileLen, bool /* regular */)>,
requested, jobs)`; `new()` becomes a thin I/O wrapper supplying the two facts.
Tests feed `FileLen::Present(128)` / `Absent` / `Unreadable` and a bool.
Do **not** change `resolve_mode` itself (shared with the repair path); just
move its decision into the injected input.

**Coverage / trade-off.** Identical arithmetic and saturation; only the
measurement source is injected. **Risk:** medium (preserve symlink/dir →
`temp` semantics). **Scale effect:** −4 temp dirs, −6 files, ~8 fewer stats.
A side benefit is production simplification (two stats → one).

#### C6 — MERGE / REDUCE_IO_SCALE: `post.rs` refusal-target cluster

**Location:** `src/repair/phases/post.rs:758–1007` and :1095–1282 — :963, :1017,
:1037, :1101.

:1017 already table-tests the pure `check_publish_target` name rule with zero
files; :963, :1037 and :1101 each create a temp dir + `Audio/cache` dir + a
cache-bytes file to drive the *same* refusal through `copy_publish_with` /
`move_audio_cache_with`. Fold the three file-backed cases into :1017's table
(keeping one end-to-end move test as representative) and add one
`audio_tree(name)` fixture helper for the remaining move tests. **Risk:** medium
(loses some end-to-end path assertions). **Scale effect:** −3 temp dirs, −3
files, ~100 lines of repeated setup collapsed.

#### C — non-findings

`config.rs`, `phase.rs`, `sophon/*`, `util/counter.rs`, `util/atomic.rs`,
`repair/purge.rs` and `report/tests.rs` have no new justified opportunity beyond
prior passes. `symlink_support.rs`'s fail-closed test (:289) and the md5
RFC-1321 vectors / two ignored throughput benches stay protected.

### D. Small crates and cross-cutting

The sled2redb and log crates hold the biggest remaining lever in this group;
`crates/tempdir` and cross-cutting hygiene are essentially done.

#### D1 — PROD_CHANGE: expose `Args` + conversion record from `sled2redb` lib

**Location:** `src/main.rs:34–50` (`struct Args`), :81–88 (inline
`Record::keyed("converted")…emit(args.output)`); `tests/convert.rs:438–481`
(3 spawns + 3 full conversions), :496–586 (2 spawns), :604–661 (4 spawns),
:768–796 (1 spawn). Enabling APIs already exist:
`tools/sync/src/report/mod.rs:91–96,136–153`, `record.rs:86,100`.

10 process spawns, 5 of them full sled→redb conversions, only to inspect a
rendered record that is a pure function of `(Converted, src, dst, OutputFormat)`.
Because `Record` construction lives in `main.rs`, tests can only reach it by
spawning.

**Proposed change.** Move `Args` and a
`pub fn conversion_record(&Converted, &Path, &Path) -> Record` plus a thin
`conversion_output(...)` built on `Report::render` into `sled2redb/src/lib.rs`;
`main.rs` calls them. Then assert JSON/text/default rendering in-process, keep
**one** spawn for the flag→record→exit-0 path, replace the byte-equality spawn
with in-process parse+render equality (both spellings already parse identically
in the 6-row table at :525–532), and assert 3 of 4 usage spellings via
`Args::try_parse_from`.

**Coverage / trade-off.** Pure render contract, all parse spellings, and the
binary's exit-code/`--help` wiring all remain. **Risk:** low–medium (`clap`
derive moves into lib; already a normal dependency at `Cargo.toml:32`; keep
`.bin_name(...)` in the help pin). **Scale effect:** spawns 10 → ~3–4, binary
conversions 5 → 2, two `build_sled_db` fixtures removed.

**Distinct from the earlier deleted F03 proposal:** that one merely deleted two
conversions whose properties were then unique; here a new seam makes the
properties assertable without converting at all.

#### D2 — PROD_CHANGE: split `convert_into` at the sled-read / redb-write boundary

**Location:** `src/convert.rs:191–264` — sled open/parse :192–226, `drop(old)`
:229, redb write/count/commit :231–254, cleanup :260–262.

`build_sled_db` (`tests/convert.rs:78–141`: sled open, meta + 3 rows, one with
3 algorithms, `flush()`/fsync, sidecars) is rebuilt 7 times (:200, :266, :335,
:380, :420, :442, :500); `sled_to_redb` is called 11 times. Only the round-trip
test needs real hex/JSON decoding and the 5-hash count.

**Proposed change.** Extract
`pub fn write_records(temp: &Path, case_sensitive: bool,
recs: &[(String, FileRec)]) -> Result<Converted>` from :231–262; `convert_into`
becomes read+parse then delegate. Keep one real sled DB for the round-trip and
one bad-row DB for the error path; rewrite count/force/atomicity tests (:270,
:341, :384, :424) to call `write_records` with 1–2 synthetic rows and assert
the same counters/file outcomes (adjust expected `Converted` counts).

**Coverage / trade-off.** Decoding still pinned by round-trip + bad-row cases;
counters, case routing, unknown-algorithm passthrough and cleanup still
asserted. **Risk:** medium (preserve case mode from meta, foreign-algo routing
and exact counters). **Scale effect:** ~5 of 7 sled fixture builds removed,
including 5 fsync-bearing conversions.

#### D3 — FIXTURE: "refuse to overwrite" needs a stale file, not a conversion

**Location:** `tests/convert.rs:262–275`, line :270.

A full conversion creates `dst` whose result is never inspected. `sled_to_redb`
only checks `dst.exists()` (`convert.rs:151`) and `dst.is_dir()` (:277) before
writing the temp sibling; `std::fs::write(&dst, b"stale")` exercises the same
refuse branch. Byte-for-byte "preserve a real cache under force" is already
pinned at :331–365. **Risk:** low. **Scale effect:** −1 sled fixture, −1
conversion.

#### D4 — MERGE: fold `a_successful_conversion_creates_the_destination_and_no_temporary`

**Location:** `tests/convert.rs:416–432` vs :196–260.

Both assert `(files, dirs) == (2,1)`, `dst.is_file()` and source untouched; the
only unique assertion in :416–432 is `entry_names(&work)` (no leftover temp) —
also present at :312–316 and :356–364. Add it to the round-trip test, delete
:416–432. **Risk:** low. **Scale effect:** −1 test, −1 sled fixture,
−1 conversion. **Hygiene:** `tests/convert.rs:256` `let _ = &log();` is dead —
remove it.

#### D5 — MERGE: table-drive `crates/log` `LogPlan::resolve` clusters

**Location:** `crates/log/src/plan.rs:384–613` — file resolution at :449, :468,
:492, :509; level/filter resolution at :483, :543, :521, :550, :570, :588.

All call `LogPlan::resolve` with small variations and assert 1–3 fields. Two
tables — `file_resolution_precedence` and `console_and_file_filters` — with a
per-row comment preserving each policy rationale. **Must keep** (named and
intact) `the_default_file_is_written_in_a_release_build_and_not_in_a_debug_one`
(:426–443), because `README.md:446–450` documents a `--release -p girpr-log`
gate run solely for it, and `a_bad_file_level_is_a_warning_and_not_a_failure`
(:600–612). **Risk:** low. **Scale effect:** 13 → ~6 tests (net −7); pure
functions, no IO change.

#### D6 — MERGE: `crates/log` `filter()` single-assertion pair

**Location:** `crates/log/src/filter.rs:70–83` —
`a_valid_spec_is_used_as_it_is` + `a_broken_spec_falls_back_to_the_projects_own_level`
→ one 2-row table. **Risk:** low. **Scale effect:** −1 test.

**Optional hygiene (no count effect):** the `Targets` `Layer`/`Arc<Mutex<Vec>>`
harness at `filter.rs:36–68` exists only for two upstream-`EnvFilter` tests
(:92–111); it could downshift to `EnvFilter::enabled` with constructed metadata,
removing ~30 lines of test harness.

#### D7 — MERGE: `crates/tempdir` `TempDir::at` cases

**Location:** `src/lib.rs:364–374` (`a_chosen_path_starts_clear`) and :379–387
(`a_file_at_the_chosen_path_is_cleared_too`) → one test covering both, keeping
all five assertions. **Risk:** low. **Scale effect:** −1 test, −1 fixture.
Low absolute value.

#### D8 — MAINTENANCE: sled2redb help pin duplicates production `Args`

**Location:** `tests/convert.rs:697–727` — `the_converter_command()` hand-rebuilds
a `clap::Command` with four `.help(...)` strings mirroring `main.rs:39–50`.

Once `Args` is exposed (D1), render the real command
`sled2redb::Args::command().bin_name(...)`, deleting the 30-line duplicate.
This makes the pin strictly stronger — it pins the command that ships, not a
copy that can drift. **Risk:** low. **No test-count change**; maintenance-scale.

#### D — non-findings / constraints

`crates/log/src/level.rs` is already table-driven in all four tests. In
`crates/tempdir/src/test_support.rs`, `pair_with` (:638–683) already pays the
expensive `cmd_update`/fsync only when `warmed == true` (and warmed callers use
`pair_mem`); `is_case_sensitive` (:205–211) has exactly one call site
(`tools/sync/tests/symlink.rs:882`); `wfile` (:92–98) is already the shared
file-creation seam. No new fixture-seam recommendation from it.

`tests/scripts/io-profile/*` are **not part of any gate** (the documented gate
in `README.md:437–462` never references them); their fixed iteration counts are
measurement resolution, not test scale — leave them alone. `zz_probe*` files are
not Cargo test targets. 12 `#[ignore]` tests: keep. The two `no_run` doctests
mirrored as tests in `crates/log/src/lib.rs:50-53` and `crates/tempdir/src/lib.rs:60-63`
exist because nextest cannot run doctests - one cheap test each, keep.

### E. `tools/sync` (`girsync`) — the largest single lever in the workspace

#### E0 — Reversal of the prior audit's "no safe cuts" verdict

The earlier audit concluded that no process tests could be downshifted because
"the library returns only an exit code; the JSON record stream exists only in the
process." That premise is false once production changes are allowed. **Verified
against the source:** `Report` is the single writer of stdout in the entire crate
— one `println!` at `src/report/mod.rs:140` — and `run()` returns only `i32`
while discarding the records (`src/commands/mod.rs:139–199`). Every process test
exists to capture that stream, the `anyhow` error string logged by `main`, or the
tracing subscriber. All three are reachable in-process with contained changes.

Measured process surface (call-site counts; ~100 spawns in ~90 tests):

| File | Helper | Spawns | Asserted |
|---|---|---:|---|
| `tests/sync.rs` | `run_binary` :67, `run_binary_raw` :102 | 30 | records, exit code, stderr error text |
| `tests/sync_plan/{table,plan_print,as_done,dry_run}.rs` | `run` in `main.rs:73` | 24 | records only |
| `tests/output.rs` | `run` :41 + 9 inline | 18 | records, log file, stderr |
| `tests/symlink.rs` | `sync_bin_raw` :35, `sync_bin` :53 | 11 | records + stderr error text |
| `tests/why/flags.rs` | `bin` :13 | 11 | rendered text/JSON + exit code |
| `tests/stat_trust/main.rs` | `bin` :64, `bin_mem` :77 | 4 | stderr warnings/refusals |
| `tests/rw_threads/main.rs` | `run_env` :206 | 2 | records |
| `compare.rs:718`, `compare_self.rs:161`, `hash_mode/end_to_end.rs:120` | inline | 3 | records + exit code |

#### E1 — PROD_CHANGE + DOWNSHIFT: injectable record sink + `run_captured`

**Locations:** `src/report/mod.rs:90–154`, `src/config/common.rs:40–68`
(`LogCtx`, `report()` at :66), `src/commands/mod.rs:139–199`, `src/main.rs:34–42`,
`src/commands/update.rs:143–149`, `src/report/record.rs:39,217–219`.

**Proposed change.**
1. `Report { format, sink }` with `enum ReportSink { Stdout, Capture(Arc<Mutex<Vec<Record>>>) }`;
   `emit` pushes into the mutex when capturing, otherwise prints exactly as today.
   The `Arc<Mutex<_>>` is **required**: `COPY` records are emitted from rayon
   workers under a separate `emit.lock()` (`apply/phases.rs:230–237`), so a
   thread-local would silently drop them. `Record` is already `Clone`.
2. Add `report_sink` to `LogCtx` (default `Stdout`); change `update.rs:149` from
   `.emit(log.output)` to `log.report().emit(...)` so `update` is captured too.
   All four commands already take `&LogCtx` and call `log.report()`, so no command
   signatures change.
3. Add `pub struct RunOutcome { code, records, error }` and
   `pub fn run_captured(cli, plan) -> Result<RunOutcome>`, dispatching the same
   four commands with a fresh capture sink and mirroring `main`'s `Err → 3`
   mapping with `format!("{:#}", e)` recorded as `error`. Parse-error tests can
   use the already-public `Cli::try_parse_from`.
4. Test side: replace `run_binary` / `sync_bin` / `bin` / `sync_plan::run` with a
   thin wrapper `Cli::try_parse_from → run_captured → records.map(json)`.

**Coverage / trade-off.** The captured values *are* the printed `Record`s (one
definition, both renderings), the exit code comes from the same
`report_diff`/`report_and_finish` functions, and the error string is the same one
`main` logs. Vocabulary, ordering, `why` tags, summaries and failure messages all
survive verbatim. Traded: the real process boundary (binary linking, clap→`main`
wiring, stdout flush, ANSI stderr) — keep ~3 smoke process tests (`compare`,
`sync`, `update`) plus the `output.rs` log tests as the process guard.
**Risk:** low (removing `Copy` from `Report` needs an audit of its three by-ref
`let report = log.report()` sites; `Arc<Mutex>` production contention is absent).
**Scale effect:** **~85 of ~100 spawns eliminated** (sync.rs 30, sync_plan 24,
7 of 18 in output.rs, symlink 11, why/flags 11, stat_trust 4, rw_threads 2,
compare/compare_self/hash_mode 3). Test count can stay flat or drop ~10 where a
process test already duplicates a library test.

#### E2 — PROD_CHANGE: scoped subscriber + injectable console writer for `output.rs`

**Locations:** `src/logging.rs:96–180` (`init_tracing` calls global `try_init()`
at :148–154; console writer hardwired to `std::io::stderr` at :105); nine tests
in `tests/output.rs` at :352, :438, :487, :536, :587, :627, :703, :915, :994
plus :306.

**Proposed change.** Extract
`pub fn subscriber_for(plan: &LogPlan, console_writer: impl MakeWriter + 'static)
-> impl Subscriber` out of `init_tracing`; tests use
`tracing::subscriber::with_default(subscriber_for(...), || run_captured(cli, plan))`
and keep the `LogGuard` alive for the closure. `Report::emit`'s
`tracing::enabled!(DEBUG)` gate respects the scoped default, so field-by-field
log assertions remain valid.

**Coverage / trade-off.** Identical layer stack, filters, JSON format and file
path — only installation scope and console writer differ. **Risk:** medium —
the non-blocking file writer's `WorkerGuard` must outlive the closure, and
`with_default` is thread-scoped, so rayon worker events would not inherit it;
verify that the `scan progress` / `effective done` events asserted at :703 are
main-thread before downshifting that one. **Scale effect:** ~9 more spawns
removed; combined with E1, `CARGO_BIN_EXE_girsync` shrinks to ~2–3 smoke tests.

#### E3 — PROD_CHANGE: return a richer `CompareOutcome`, delete duplicated pipelines

**Locations (six reimplementations of classify → open_side → plan_pairs →
resolve_side → diff_maps):** `tests/lazy/main.rs:96–163`,
`tests/hash_mode/main.rs:206–243` and :250–302, `tests/stat_trust/main.rs:173–211`,
`tests/helpers.rs:44–72`, `tests/compare.rs` (`effective_maps`).

`src/commands/compare.rs:97–175` computes exactly this pipeline but returns only
`i32`; `lazy/main.rs:92–95` says so explicitly. Each harness re-derives side
labels and re-threads `ScanMode`, `RwSide`, `serial_rw()`.

**Proposed change.** Factor the body into public
`compare_detailed(opts, log) -> Result<CompareOutcome>` with
`CompareOutcome { diff, src_stats, dst_stats, code }` (and a `compare_self_detailed`);
`cmd_compare` calls it and renders. Test harnesses become one shared module.

**Coverage / trade-off.** Assertions read the production computation instead of
a test copy — strictly stronger. The grouped suites still test genuinely
different mechanisms (`lazy/`, `hash_mode/`, `stat_trust/`, `why/`, `rw_threads/`,
`sync_plan/` are **not** one scenario repeated at layers — this confirms the
prior audit), so merge only their harness, not their cases. **Risk:** low–medium.
**Scale effect:** removes ~250–300 lines of duplicated test pipeline; enables a
shared helper and optionally table-izing `lazy/compare.rs`'s 15 near-identical
fixture/expected-record tests (~14 tests) — optional, not required.

#### E4 — PROD_CHANGE + REDUCE_IO_SCALE: parameterize the heartbeat period

**Location:** `tests/output.rs:703–902`, fixture at :710–719
(`TEST_HEARTBEAT_ENTRIES = 101`, loop creating 100 files + one excluded), seam at
:739–742 (`GIRSYNC_TEST_HEARTBEAT_ENTRIES` on the child); production `src/scan.rs:21`
(`HEARTBEAT_ENTRIES = 2000`), :31–47 (pure `heartbeat_entries_from`).

The env seam is child-process-only, so the test still writes 101 files and
spawns to read a log file. The comment at :704–709 admits the number comes from
a phase-A progress period of 100, not from the phenomenon under test.

**Proposed change.** Add `walk_live_with(root, max_depth, heartbeat_entries)`
(and thread a period into `scan_stat_only`/`open_folder_cache` via `ScanMode`,
defaulting to `HEARTBEAT_ENTRIES`); expose the two progress counters on a library
result rather than only log events. Test becomes an integration test over 2–3
files with period 2, asserting the same heartbeat/phase fields directly.
**Risk:** medium (if counters stay log-only, E2 is still required).
**Scale effect:** **~100 file creates removed**, 1 spawn removed, ~10 log-parse
assertions replaced with direct values.

#### E5 — PROD_CHANGE: replace ambient `GIRSYNC_TEST_*` hooks with injected `TestHooks`

**Locations:** read sites `src/config/common.rs:366–372`, `src/cache/mod.rs:61–93`,
`src/cache/write.rs:65–93`, `src/commands/sync/apply/copy.rs:79–140` (`FAIL_COPY`,
`FAIL_FLUSH`, `NO_FSYNC`, `OnceLock`-cached), `src/commands/sync/rename.rs:18–51`,
`src/commands/sync/apply/phases.rs:463–569` (`GATE_COPY`), `src/scan.rs:31–47`;
test users `tests/sync.rs:826,935,975,1024,1195,1425–1428,1526–1531,1678–1759`
and `tests/output.rs:740`.

These hooks are process-global and several are `OnceLock`-cached, so a test
wanting one must spawn a child — the main reason ~15 `sync.rs` spawns survive E1.

**Proposed change.** Group into `#[derive(Default)] pub struct TestHooks { cache,
commit, fail_copy, fail_rename, gate_copy, heartbeat_entries, … }`, add `hooks`
to `CommonOpts` (default = production env behaviour), read from `common.hooks`
instead of `std::env`, and let `run_captured` accept a `TestHooks`. Keep the env
path as a fallback so `.config/nextest-test-env` still works.
**Coverage / trade-off.** Same injected failure points and error strings.
**Risk:** medium-large diff across ~8 production files; the `OnceLock` in
`copy.rs` must be replaced carefully. Sequence after E1.
**Scale effect:** unlocks the remaining ~15 failure-path `sync.rs` spawns;
removes ~20 `env(...)` calls; makes the suite work under plain `cargo test`.

#### E6 — Smaller sync findings

- **Helper/meta-tests (−2 tests).** `tests/sync_plan/table.rs:209–240`
  (`the_fixture_covers_every_apply_phase`) reads only the test's own `cases()` table
  and `APPLY_ORDER` — fold into a `debug_assert` in `cases()` (`main.rs:342`).
  `tests/lazy/sync.rs:208–218` (`fixtures_classify_as_folders`) asserts a fixture
  helper — fold into `pair()`/`fixture()` as a `debug_assert`.
  `src/scan.rs:240–248` half-asserts that a test env var is unset — keep the
  constant pin, drop the harness pin (marginal).
- **Fixture-size trims (low value, already partly done).** `tests/rw_threads/main.rs:116–140`
  `PAD_COUNT = 16` (comment says it was already cut from 96) only needs to exceed
  the widest pool (8+1) — deriving it from the limit saves ~42 file writes.
  `tests/rw_threads/through_the_binary.rs:177–180` creates 40 files × 6 spellings
  (=240); derive from `limits_of_flags()` (e.g. `2 × pool + 4`) → ~96, saving
  ~144 writes.
- **`benches/tree_shapes.rs` — SKIP.** Its `TINY_FILES = 2_000`, `LARGE_FILES = 4 ×
  8 MiB`, `DEEP_LEVELS = 200` are load-bearing for *timing*, not a gate
  (:6–11 says it is a local measurement aid with no CI assertion). Lowering
  defaults via `GIRSYNC_BENCH_SCALE` would be a measurement-quality regression,
  not a test-scale win. Adding a scale-free truncation unit test would duplicate
  `scan.rs` coverage. Leave as-is.

## Cross-cutting synthesis and ranked action list

### The five recurring patterns

1. **A library already computes the value; the test spawns a process just to
   read its rendered form.** This is the single biggest lever: `tools/sync` E1/E2
   (~94 of ~100 spawns), root A1/A2/A3/A4 (~40), `tools/capture` B6 (~50, wide
   risk), `sled2redb` D1 (~6). The common fix is a capture sink / render function
   next to the existing `Report`/`Record`/`Summary` type, with a few smoke
   process tests retained for the binary boundary.
2. **Fixed-size buffers or thresholds force oversized fixtures.** Inject the
   knob: MD5 buffer C2 (~1 MB → <1 KB), heartbeat E4 (~100 files → 3),
   `DiskPlan` figures C5 (−4 temp dirs, −6 files), `measured_fixture` B3
   (~28 MB → <10 KB), `post.rs` refusal cluster C6 (−3 temp dirs).
3. **Sampling/benchmark proofs that can be scale-free invariants.** `retry` C1
   (20,480 hash inserts → a gcd proof), `peak_heap` A5 (drops allocator, TCP
   server, 256 KiB payload and the workspace's only `unsafe` exemption).
4. **Meta-tests and redundant case construction.** `clock_support` C4 (−2),
   `log` plan/filter D5/D6 (−8), `tempdir` D7 (−1), `plan.rs` layout C3 (−6),
   `sled2redb` D3/D4 (−2), `sync` E6 (−2), `capture` B4 (−2).
5. **Multiple full captures/fixtures for one field.** `capture` B1+B5 takes the
   shared-bundle variants from 3 → 1; `sled2redb` D2 removes ~5 of 7 sled DB
   builds; root A1 removes ~20 log files.

### Suggested sequencing (lowest risk / highest density first)

- **Wave 1 — pure, zero-coverage-loss, low risk:** C1, C3, C4, D3, D4, D5, D6,
  D7, B1, B2, B3, E6. Mostly table folds and test-only changes; net ~−25 tests,
  no production API changes (except lifting `retry` constants).
- **Wave 2 — narrow production seams, medium risk:** A1, A3, C2, C5, E4, B4,
  A7. Each removes tens of files/spawns behind a pure injectable helper.
- **Wave 3 — wider seams, highest payoff:** **E1 (then E2, E5)**, A2, A5, D1/D2,
  B6/E1-style in-process transport for capture. E1 alone removes ~85 spawns and
  is the highest-value item in the whole report; E5 is its prerequisite for the
  failure-path cases.
- **Sequence note:** capture's `bundle_source_key()` hashes all of `src/**`, so
  do B1 and B5 in the same change to avoid repeated recaptures. Logging seams
  (E2) should be verified against rayon event threading before downshifting
  `output.rs:703`.

### Constraints to preserve (do not violate when implementing)

- `README.md:446–450` documents a `--release -p girpr-log` gate run **solely**
  for `the_default_file_is_written_in_a_release_build_and_not_in_a_debug_one`;
  keep that test named and intact (D5 must not fold it away).
- The two `no_run` doctests mirrored by tests in `crates/log`/`crates/tempdir`
  exist because nextest cannot run doctests — keep.
- The 12 `#[ignore]` tests (artifact bridge, bundle shapes, md5 throughput,
  peak heap) are deliberate hand-run measurements/pre-release checks — keep
  unless A5 is implemented (it removes one).
- `tests/zz_probe*.rs` are not Cargo test targets; `tests/scripts/io-profile/*`
  are not part of any gate and their iteration counts are measurement
  resolution — leave both alone.
- `tools/sync`'s grouped suites (`lazy/`, `hash_mode/`, `stat_trust/`, `why/`,
  `rw_threads/`, `sync_plan/`) test genuinely different mechanisms — merge their
  harnesses (E3), not their cases.

### Assumptions to validate at implementation time

- Exact spawn/file counts are derived from static call-site counts; a small
  number will differ once harnesses are refactored.
- The ~13–18 convertible `repair_machinery.rs` cases (A1) and the ~85 convertible
  `tools/sync` cases (E1) are estimates based on which assertions are
  library-observable; confirm per-case by reading each test body.
- E2's rayon-thread caveat and B5's `digest_file` writer-path equivalence are
  the two findings most likely to need a fallback to a thin process smoke test.

### Headline conclusion

There is no large block of useless tests. The scale that can be removed is
mostly **fixture and process infrastructure**, not test intent: ~130–150 process
spawns and tens of MB of synthetic payloads across the workspace are created to
read values that the production library already computes. The highest-value
single change is `tools/sync` **E1** — one injectable record sink that turns
~85 process tests into in-process integration tests without losing a single
assertion. Doing nothing remains defensible for the low-risk merges, but E1/E5
offer a scale reduction far larger than any previous pass.
