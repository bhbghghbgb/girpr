# Test-run optimization research: assumptions and findings

Static research over the current tree, plus baseline measurements taken on this host
(Windows x86_64-msvc, 4 logical CPUs). No source was changed and no fix was applied.
Everything below is a claim to be validated when implementing; file/line references are
from this tree and may drift. The workspace already carries substantial prior test
optimization (leaf-crate extraction, shared test-support crates, no-fsync hooks,
`profile.test` tuning, ignored throughput/bridge suites). Those are treated as existing
facts, not candidates. The three previous research/refactor documents in git history
(`docs/08-…`, `docs/09-…`, `REFACTOR_PLAN/…`) are prior art, not instructions repeated here.

## 1. Measured baseline (this host, this tree)

| Measurement | Result |
| --- | --- |
| Tests listed by `cargo nextest list --workspace` | 2,183 in 78 suites |
| Full `cargo nextest run --workspace` | 171.2 s wall; 2,172 passed, 11 ignored/skipped |
| Cumulative test-seconds (sum of per-test times) | 679 s (~4x parallelism on 4 CPUs) |
| `cargo nextest run --workspace --no-run` (workspace test targets; deps cached) | 59.2 s |
| 14 root test binaries, clean build (cargo timing, prior run on this host) | 6.09–8.20 s each, ~98 s aggregate, ~50 s wall in 4 batches |
| Duplicate dependency versions in `cargo tree -d` | `base64` 0.22/0.23, `getrandom` 0.2/0.4, `syn` 2/3 |

Cumulative test-seconds by area (share of 679 s): sync side 311 s (46%), girpr side
195 s (29%), gircap side 172 s (25%). Highest-cost suites:

| Suite | Tests | Cumulative s | Slowest single test |
| --- | ---: | ---: | ---: |
| `gircap` (lib) | 641 | 91.2 | 1.32 s |
| `girsync::sync` | 46 | 87.9 | 5.70 s |
| `girpr-sync-cache` | 129 | 52.3 | 3.12 s |
| `girsync` (lib) | 190 | 31.3 | 1.24 s |
| `girsync::why` | 25 | 30.7 | 6.30 s |
| `gircap::capture_layout` | 32 | 24.8 | 2.51 s |
| `girsync::symlink` | 34 | 23.5 | 2.40 s |
| `girpr` (lib) | 231 | 23.4 | 0.29 s |

Interpretation: gircap's cost is mostly 641 Windows process spawns of ~0.14 s each;
girsync's cost is a much smaller number of long, real-binary, file-cache-backed cases
(`girsync::rw_threads::through_the_binary::sync_reports_the_same_actions_under_every_rw_setting`
8.17 s, `girsync::why::flags::why_does_not_change_the_exit_code` 6.30 s, several
`girsync::sync` cases at 4–6 s). The two areas need different remedies: process-count
reduction for gircap, expensive-setup reduction for girsync.

## 2. Assumptions

**A1 — nextest process model.** `cargo nextest run` runs one OS process per `#[test]`.
A process-wide `OnceLock`, static fixture, or shared server therefore helps only *within*
one test, never across tests. Merging test functions saves process spawns; adding
parameterized cases (e.g. `rstest`) adds them. Sources: `.config/nextest.toml:24-88`,
`README.md:610-624`, `tests/artifact_bridge.rs:331-340`,
`tools/capture/test-support/src/lib.rs:642-699` (a `OnceLock` cache explicitly noted as
useless under nextest).

**A2 — Rebuild unit.** Cargo fingerprints a package per (package, feature-set, profile,
target); editing any source file invalidates that target and all downstream dependents,
while upstream dependencies stay cached. `#[cfg(test)]` code is not in the shipped lib,
but a test-only edit still invalidates the file's fingerprint for every build. With
`profile.test: incremental = false`, an invalidated target is rebuilt from scratch.
Sources: `Cargo.toml:193-215`; prior research assumption A1.

**A3 — Splitting only helps when the dependent set shrinks.** If a low-level crate is
split out but a facade still re-exports both halves and every consumer depends on the
facade, the change adds a crate to compile without removing a rebuild edge. Consumers
must depend on the narrow crates directly. Source: prior research A2; current edges
`crates/util/src/lib.rs:24-28`, `tools/capture/hash/Cargo.toml:38`.

**A4 — Existing no-fsync and scratch policy is already in force.**
`.config/nextest-test-env` exports `GIRSYNC_TEST_DURABILITY=none`,
`GIRSYNC_TEST_NO_FSYNC=1`, `GIRPR_TEST_NO_FSYNC=1`; setup scripts copy them to every
test process. `crates/tempdir` gives one scratch-tree policy; `crates/test-fixtures`,
`crates/test-support`, `girpr-log`'s `test-support` feature and the tool-specific
support crates already deduplicate helpers behind thin shims. None of this is a new
candidate. Sources: `.config/nextest.toml`, `.config/nextest-test-env`, `README.md:656-671`.

**A5 — Fixtures are already tiny.** The standard repair fixture is 64-byte chunks; the
artifact-server mini-bundle is a few KB of JSON/zstd; capture's largest test blob is
32 KB and is pinned by assertions; sled2redb's fixtures are four rows over five bytes.
Scaling fixture bytes down further buys essentially nothing. Sources:
`crates/test-fixtures/src/lib.rs:625-627,666-669`,
`tools/capture/test-support/src/lib.rs:895-908`, `tools/sled2redb/tests/convert.rs:137-230`.

**A6 — No sleeps in the suites.** There is no `sleep` in root `tests/*.rs` or `src/`;
`crates/test-support/src/lib.rs:77-78` sets `BackoffPolicy::none()`/`ZeroSleep`, and
`repair_limits.rs:544-555,688-692` documents why the 125 s/15 s control cases are absent.
The only sleep-like code found on the sync side is a 5 ms contention hold at
`tools/sync/tests/rw_threads/limiter.rs:283`; the interval-commit test backdates instead
of sleeping (`crates/sync-cache/src/write.rs:600-619`). The 10 s guards in repair tests
wake on notify/channel and never fire on a passing run. Re-adding sleeps or "without
fail-fast" control runs is a regression.

**A7 — Host dependence.** This is a link-bound, 4-core HDD host. Linker settings, cache
opens (a file-backed redb open is recorded as ~0.5 s at
`tools/sync/tests/migration_lock.rs:86-99`), and Windows process creation dominate here;
other hosts will rank findings differently. Fixture-scale findings should be validated
on the CI host too.

**A8 — Toolchain drift.** `rust-toolchain.toml:4` pins `channel = "stable"`, not an exact
release. Every stable release changes `rustc`'s fingerprint and invalidates all cached
artifacts, including hundreds of third-party crates.

**A9 — Wiremock's dependency footprint.** `cargo tree -i`/`-d` shows `wiremock 0.6` is the
exclusive source of `regex`, `futures`, `deadpool`, `assert-json-diff` and `base64 0.22`
(plus their trees: regex-syntax, aho-corasick, futures-*, …). `base64 0.23` comes from
reqwest/hyper and is unaffected. A std-only `TcpListener` loopback server pattern is
already exercised in `crates/test-fixtures/src/lib.rs:886-939` and duplicated in the
capture HTTP tests.

**A10 — Nextest name filters are pinned.** `.config/nextest.toml` names exact packages,
binaries and fully-qualified test paths for skip-visibility overrides. Renaming, moving
or merging those tests requires editing those filters in the same change.

**A11 — Dev-dependency feature unification.** Edition 2024 implies resolver 3; per-package
dev-dependency features unify into that package's normal dependencies under test, so the
same crate may exist as two cached builds (normal and test-unified). A dev-dep removed
from one member pays off fully only in isolated `-p <crate>` builds; feature-unified
workspace runs may still compile it for another member.

**A12 — Interpretation of "resource usage."** Beyond wall time, the relevant resources
are process spawns, file-cache opens, temp trees/files, and `target/` disk. Each is
measured separately below; a change that improves one can worsen another (e.g. merging
tests saves processes but coarsens failure granularity).

## 3. Findings

Benefit/effort are estimates on this host. "High/L" means high estimated benefit and
large effort. Findings are grouped by owner area; cross-cutting items are listed first
because they are shared by several groups.

### 3.1 Cross-cutting: build and test infrastructure

**F1 — Consolidate test binaries; each binary is a full link of the workspace tree.**
Evidence: root `tests/` has 14 `*.rs` targets; 11 inline `mod common;` and link
`girpr-test-fixtures` → wiremock/prost/zstd/reqwest/tokio plus the `girpr` lib tree.
Clean-build cost 6.09–8.20 s each (~98 s aggregate); `run_lock.rs`, which does not link
the fixture tree, is 3.82 s. `tools/sync/tests/` has 10 `*.rs` targets plus 6–7 directory
targets (16–17 link steps). Cost: every `src/`/lib edit relinks all of them; every
`tests/common/mod.rs` edit relinks the 11 root binaries. Fix: `autotests = false` plus 3–4
`[[test]]` roots that `#[path]`-include the existing files (directory targets already use
this pattern, e.g. `tools/sync/tests/why/main.rs:55-61`); change per-file `mod common;`
to one crate-root module with `use crate::common::*;`. Under nextest, isolation is per
*test*, not per binary, so shared modules do not weaken `set_var`-based assumptions.
Benefit **High**, effort **M**. Validate module path resolution with a compile and
re-measure clean-build wall time.

**F2 — One std-only loopback HTTP mock could replace wiremock in every crate.**
Evidence: four independent copies — root `src/test_http.rs:15-16` (one test at
`src/repair/mod.rs:1403-1408`), `crates/fetch/src/test_http.rs:15-16` (six tests at
`crates/fetch/src/hyp.rs:655-733`, `crates/fetch/src/sophon/stream.rs:1055-1086`),
`tools/capture/src/http/tests.rs:48-136` (~8 tests) and
`tools/capture/http/src/tests.rs:670-758` (~14 tests). The capture files already contain
a std-only raw-socket mock for malformed replies (`src/http/tests.rs:138-210`,
`http/src/tests.rs:760-843`); `crates/test-fixtures/src/lib.rs:886-939` has another.
Cost: wiremock's five exclusive dependency trees per isolated `-p` build, plus a hyper
server per test. Fix: a tiny dev-only leaf crate (no product dependencies, so no cycle
with fetch/test-fixtures) exposing scripted status/body/headers, request journal and hit
counts; migrate the four sites and drop `wiremock` from root, `girpr-fetch`, `gircap` and
`gircap-http`. Keep at most one wiremock smoke if wire compatibility is considered part
of the contract. Benefit **Med–High** (single-crate builds), effort **M**. Note the leaf
must be dependency-free: `crates/test-fixtures` cannot currently be used by fetch
(`crates/test-fixtures/Cargo.toml:19` depends on fetch).

**F3 — Pin the stable toolchain to an exact release.**
Evidence: `rust-toolchain.toml:4 = "stable"`. Cost: a full-workspace rebuild cliff every
~6 weeks. Fix: pin e.g. `channel = "1.xx.0"` and bump deliberately; this is not an MSRV
floor (the workspace still refuses to support old compilers). Benefit **Med**, effort **S**.

**F4 — Trial a faster linker for the test profile; do not assume it works.**
Evidence: with `codegen-units=1`, `debug=0`, `incremental=false` already set, each heavy
test binary still costs 6–8 s, dominated by the MSVC link. `rust-lld.exe` ships in the
toolchain. Fix: trial `[target.x86_64-pc-windows-msvc] linker = "rust-lld"` in a scratch
config, measure clean `cargo nextest run` wall, and adopt only if all targets link.
Benefit **High if validated / 0 if not**, effort **S–M**, workspace-wide risk.

**F5 — `target/` has accumulated ~191 GB, ~97.5 GB of it stale PDBs.**
Evidence (measured during research, includes artifacts from earlier builds): `target/` =
191.34 GB / 218,099 files; `target/debug/build/` = 131.3 GB / 59,381 files; ~1,928 PDBs
totalling 97.53 GB there, including 37 stale `cli_contract-*.pdb` copies of 58–186 MB.
After `profile.test debug = 0` the newest PDBs are ~3.2 MB, so the large ones are
leftovers from older profiles. Cost: disk pressure and file-count overhead on the stated
HDD host. Fix: one-time `cargo clean` (or targeted removal of pre-`debug=0`
`out/*.pdb`) plus periodic cleanup; consider a faster `CARGO_TARGET_DIR`. Benefit **High
(disk)**, effort **S**. This is a maintenance finding, not a test-code change.

**F6 — Rebuild-locality edges still exist; cut them at the consumer.**
Evidence and fixes:
- `crates/util/src/lib.rs:24-28` declares five unrelated modules; `crates/audio`,
  `crates/fetch`, `crates/report` use disjoint slices and none uses `ini.rs` (1,437
  lines), yet they all rebuild on an `ini.rs` edit. Split into narrow crates (e.g.
  path/io/hash/ini) and have consumers depend directly; drop the re-export facade after
  migration. Keep the existing `girpr-util` test names until the pinned nextest filters
  are updated (A10). Benefit **Med–High (turnaround)**, effort **L**.
- `tools/capture/hash/Cargo.toml:38` depends on `gircap-http` solely for
  `impl BodyFile` (`tools/capture/hash/src/lib.rs:238`; trait at
  `tools/capture/http/src/lib.rs:97`). Move the one-method trait into `girpr-digest`
  (already a dependency of both) or a tiny `gircap-io` leaf. Benefit **Med**, effort **S–M**.
- `crates/log/Cargo.toml:20` bundles `logs_of_async` (tokio) into `test-support`; util's
  tests use only sync `logs_of` and have no tokio dev-dep, but must compile tokio anyway.
  Split into `test-support` / `test-support-async`, or move `logs_of_async` into its sole
  consumer `crates/test-fixtures/src/lib.rs:757`. Benefit **Med (isolated builds)**, effort **S**.
- `crates/report/Cargo.toml:28` compiles clap for one `#[derive(clap::ValueEnum)]`
  (`crates/report/src/record.rs:32-34`). Make it a feature enabled by consumers that
  need the CLI flag type. Benefit **Low**, effort **S**.
All four are turnaround wins, not wall-clock wins in a `--workspace` run where features
are already unified (A11).

**F7 — Do not add `rstest`; merge under nextest instead.**
Evidence: `rstest` is absent from all manifests and `Cargo.lock`; existing case tables
are already `for`-loop driven (`crates/util/src/ini.rs:599-620`,
`crates/util/src/path.rs:504-570`, `crates/fs-names/src/lib.rs:159-233`,
`crates/digest/src/stream.rs:247`). Under A1, `rstest` would turn each case into a new
process and add a proc-macro dev-dep. Fix: keep loop tables and assertion messages; fold
the tightest clusters into single tests where failure granularity is not important.
Benefit **Low wall / regression avoidance**, effort none for the no-change part.

**F8 — The one fully ignored test binary is still compiled.**
Evidence: `tools/capture/tests/bundle_shapes.rs:12-17,211,369,463` — all three tests are
`#[ignore]`d and hand-run. Fix: `[[test]] name = "bundle_shapes" required-features =
["manual-tests"]` with an empty feature and a crate-level cfg, documenting the manual
command. Benefit **Low–Med**, effort **S**.

**F9 — Root dev-dep hygiene: verify before deleting.**
Evidence/correction: the integration-group claim that root's `prost` dev-dep is unused is
**false** — `src/repair/phases/manifest.rs:290,305` uses it in unit tests. It can only be
dropped after those fixtures are frozen as bytes (F14). Tokio's root `test-util` feature
appears unused in root tests (the only `time::pause()` calls are
`crates/report/src/tests.rs:106,144`, and girpr-report declares its own dev feature);
verify with `cargo tree -e dev -p girpr` before removing. Benefit **Low**, effort **S**.

### 3.2 Root `src/` unit tests

**F10 — ~215 repair unit tests live in one lib test target.**
Evidence: `src/repair/mod.rs:1283` (6 tests), `phases/plan.rs:512` (18),
`phases/check.rs:391` (13), `phases/post.rs:741` (21), `phases/metadata.rs:496` (13),
`phases/manifest.rs:285` (8), `file/inplace.rs:1075` (25), `file/temp.rs:763` (14),
`scope/report.rs:452` (23), `scope/limits.rs:224` (11), `scope/filter.rs:123` (9),
`reuse.rs:194` (8), `purge.rs:535` (23), `scan.rs:524` (12). The lib test binary has 231
tests and 23.4 cumulative seconds — tiny wall cost, large rebuild cost: any repair edit
re-codegens all ~11k lines of test code. Fix: the leaf extraction in F6 (move the pure,
I/O-light modules `scope/report`, `phases/plan`, `reuse`, `scope/{limits,filter}` and
`cli.rs` into leaves with a shared model home for `RepairPlan`/`PlannedFile`/`Summary`).
Benefit **High (build)**, effort **L**.

**F11 — Two hand-rolled loopback HTTP servers duplicated in `src/`.**
Evidence: `src/repair/phases/manifest.rs:511-606` (`serve`, `handle_connection`,
`TestServer` with `Drop`-abort) and `src/repair/phases/metadata.rs:573-644` (near-identical
header parsing, routing, response writing) serve 5 and 9 tests. The metadata accept loop
has no `Drop`-abort, so each test leaves a listener/task until runtime teardown. Fix: one
shared `#[cfg(test)] mod test_origin` with a reply enum and `Drop`-abort driver; fold it
into F2. Benefit **Med (code/robustness)**, effort **M**.

**F12 — Eight manually built current-thread runtimes in sync tests.**
Evidence: `src/repair/phases/check.rs:519,652,700,784,1003`,
`src/repair/phases/post.rs:1288,1492`, `src/repair/mod.rs:1298` repeat
`Builder::new_current_thread().enable_all().build()`; `check.rs:513` already factors one
into `run_verify`. Fix: `#[tokio::test(flavor = "current_thread")]` except the one case
that deliberately asserts cross-thread progress (`check.rs:1002`). Benefit **Low**, effort **S**.

**F13 — Audio-move fixture repeated across 12 post-phase tests.**
Evidence: `src/repair/phases/post.rs:774-1320` — 12 cases each create `Audio/cache` and
write small bytes; `config()` at `:759` already dedups config. Fix: one
`audio_cache_fixture()` plus a table test for the pure `check_publish_target` alias cases
(`:987-1052`). ~12 temp dirs/24 tiny files per run. Benefit **Low**, effort **S**.

**F14 — Manifest bodies are re-encoded with zstd+prost per test.**
Evidence: `src/repair/phases/manifest.rs:300-311` (`manifest_body` =
`prost::encode` + `zstd::encode_all(level 3)` + md5), called at `:770,813,872,922,947-948`;
only three distinct deterministic bodies. Fix: freeze them as `const &[u8]` with pinned
checksums, then remove the root `prost`/`zstd` dev-deps (see F9 correction). Benefit **Low
(build-time hygiene)**, effort **S**.

**F15 — Hang guards are generous but never fire on a passing run.**
Evidence: `check.rs:403` and `repair/mod.rs:1320` use 10 s; `manifest.rs:740,843,908,937`
and `file/mod.rs:1074` use 5 s; all wake on notify/channel. Fix: lower to 2 s for faster
failure feedback. Benefit **Low**, effort **S**.

**F16 — Clap-arg boilerplate in 15 pure CLI tests.**
Evidence: `src/cli.rs:437-627`, five tests repeat
`Args::parse_from(["girpr", "--game-path", ".", "--biz", …])`. Fix: a `parse(extra)`
helper and one table test for jobs/alias/range cases (`:499-545`), combined with the
`cli.rs` leaf move in F10. Benefit **Low**, effort **S**.

### 3.3 Root integration tests (`tests/`)

**F17 — ~62 binary spawns; 10 in `repair_machinery` exist only to read log strings.**
Evidence: `Rig::run` (`tests/repair_machinery.rs:235-287`) spawns
`CARGO_BIN_EXE_girpr`; 10 cases use it while 16 use in-process `run_lib` (`:297-340`).
The assertions are implementation-detail log text: `ascending=false/true`
(`:1017,1038,1108`), `salvaged=20 of=23` (`:1066-1072`), trim/reuse messages
(`:1149,1154-1155,1188-1195,1222-1225`), chunk stats (`:690-699`) and per-chunk trace
download records (`:1672`). Cost: 10 `CreateProcess` + full pipeline runs, 10 log files,
10 JSON parses, 11 temp trees. Fix: expose a structured `RepairDiagnostics` alongside
`Summary` through the existing test-support surface (or a conditional diagnostics
record that leaves the `cli_contract` goldens unchanged), convert those 10 cases to
`run_lib`, keep one binary smoke for "log is a process artifact". Benefit **High (wall,
I/O)**, effort **L** (production API change).

**F18 — `cli_contract.rs` still spawns the binary ~24 times; 14 are log announcements.**
Evidence: the argument table collapse is already done (19 in-process cases + 1 spawn at
`:607-737`; `real_help()` is a `OnceLock` at `:476-486`). Remaining: four argument/exit
cases (`:592,736,806,823`), 14 logging cases (`:993,1006,1027,1092×4,1123,1171×2,
1280×4`), seven data-plane cases (`:2026,2040,2061,2098,2113,2135,2155`). Fix: extract
`src/main.rs:18-31`'s announcement into a pure helper unit-tested in-process (`init_tracing`
is already public at `src/lib.rs:87` and tested at `src/logging.rs:359-434`); keep one
binary log smoke. Point post-parse logs at a scratch `--log-file` instead of the
`logs/` dir beside the binary, which currently accumulates `girpr_<ts>.log` files inside
`target/`. Benefit **High / Med**, effort **M / S**.

**F19 — `cli_contract` JSON/text plane pairs duplicate mock+install setup.**
Evidence: `:2019` vs `:2093`, `:2035` vs `:2108`, `:2056` vs `:2130` use the same
`seed_one_damaged_one_clean` fixture (`:2003-2013`) and differ only in `--output`. Fix:
one test per pair running the binary twice against one mock/install. Benefit **Med**, effort **S**.

**F20 — `process.rs` phase/log tests repeat the same fixture five times.**
Evidence: `start_standard_install` (`:44-53`) runs per test; phase-duration assertions at
`:444,498,545,592,627` = 5 spawns, 4 mocks, 5 installs, 5 full-log reads. Base-precedence
tests `:400,:426` assert the same fact with two spawns. Fix: one table-driven phase test
over flag sets and a two-row precedence table. Net 11 tests → ~8, same 14 process-level
assertions, three fewer mocks/installs. Benefit **Med**, effort **S–M**.

**F21 — `artifact_server.rs`: 8 tests each build a mini-bundle and start four listeners.**
Evidence: `MiniBundle::build` (`:106-226`) writes ~10 small files; `Bridge::start` at
`:431,507,566,643,707,767,909,986` = 32 listeners plus real HTTP GETs. Fix: group by
bundle shape into ~4 tests (`:429,765,907` share one; `:980`, `:641`, `:564`, `:505,:705`),
keeping one full TCP end-to-end case — the TCP path is the contract under test. Do not
expect a cross-test fixture-sharing win (A1). Benefit **Med (resource) / Low (wall)**, effort **S**.

**F22 — Cross-file duplicate process-level assertions.**
Evidence: `process.rs:229-279` asserts REPORT/SUMMARY, repair, scope and `getBuild` hits
already asserted in-process by `repair_offline.rs:56-96` and `cli_contract.rs:2019-2064`;
`repair_post.rs:335`, `repair_scan.rs:374,618,739` and `repair_plan.rs:188` (inside a
2-row loop) duplicate in-process rigs. Fix: keep one binary smoke per flag path and assert
details in-process. Benefit **Med**, effort **M** (preserve process-only coverage).

**F23 — Repeated `build_fixture()` inside single tests (tiny, honest win).**
Evidence: 41 calls in `repair_writer.rs`, 28 in `repair_offline.rs`, 2 in `process.rs:45,47`.
Cost: kilobytes of allocation with today's fixture scale (A5). Fix: pass `&GameFixture`
instead of rebuilding. Benefit **Low**, effort **S**. Do not expect measurable wall savings.

**F24 — Stale build output under `tests/scripts/`.**
Evidence: `tests/scripts/io-profile/iot-bench/target/` ~27.5 MB of ignored artifacts
including a rustdoc tree. It is gitignored and not part of any cargo target; it only
slows tools that walk `tests/`. Fix: delete it. Benefit **Low (disk)**, effort **S**.

### 3.4 Core leaf crates

**F25 — `girpr-fetch` compiles wiremock for six canned-reply tests.** Evidence and fix in
F2; helper at `crates/fetch/src/test_http.rs:8-32,70-79`, call sites at
`crates/fetch/src/hyp.rs:562,655-733` and `crates/fetch/src/sophon/stream.rs:624,1055-1086`.
The helper already counts hits with `AtomicUsize`; only `received_requests()` needs the
wiremock API. Benefit **Med (High for isolated `-p girpr-fetch`)**, effort **M**.

**F26 — ~131 of 271 in-scope tests are tiny pure tests in their own process.**
Evidence: tightest clusters — `crates/util/src/atomic.rs:235-347` (four tests each
creating a `TempDir` to assert "refused before anything is written"), `crates/retry/src/lib.rs:187-233`
(three), `crates/fetch/src/retry.rs:187-297` (six), `crates/fs-names/src/lib.rs:133-258`
(six pure policy tests), `crates/report-core/src/lib.rs:146-178` (four),
`crates/fetch/src/sophon/stream.rs:951-1036` (five). Fix: fold closest cases into one test
with labelled subcase loops and preserved assertion messages. Do not merge tests named
in `.config/nextest.toml`, host-dependent skips, or log-capture users. Benefit **Low–Med
(wall)**, effort **S–M**.

**F27 — `report/ledger.rs` concurrency tests spawn 40 threads for exact sums.**
Evidence: `crates/report/src/ledger.rs:691-707,712-727,821-850,962-986` — 8 threads ×
100 iterations each, but only `:758-781` uses a `Barrier` to force a genuine race. Fix:
reduce the two barrier-free sum tests (`:691,:712`) to 4 × 25 or one thread; keep barrier
tests at 8 threads. Benefit **Low**, effort **S**.

**F28 — Redundant tag-6 byte assertion in three crates.**
Evidence: `crates/proto/src/lib.rs:81-96` pins `[0x30, 0x96, 0x01]` and round-trips
`unknown_field6 = 150`; `crates/fetch/tests/fetch_direct.rs:40-45` re-asserts the same
bytes; `crates/fetch/src/sophon/proto.rs:30-53` duplicates the round-trip but with value 0
so it does not actually pin tag 6. Fix: keep the integration binary for its
"consumable without the facade" compile check (`:1-8`) and the proto crate's authoritative
test; drop the duplicate byte assertion. Benefit **Low**, effort **S**.

**F29 — Other small, validated non-candidates.** `crates/util/src/md5.rs:647-782` (two
ignored 64 MiB benchmarks) never run; `crates/fetch/src/sophon/stream.rs:627-629`'s zstd
re-encodes would not be shared across nextest processes; `crates/tempdir/src/lib.rs:155`'s
`create_dir_all` runs once per process; `crates/fs-identity/src/lib.rs:190-216`'s own
scratch dir deliberately keeps that leaf dev-dep-free; `crates/report-core`'s `serde_json`
dev-dep is the actual serializer; `crates/util/src/junction_support.rs:166-173` spawns
`cmd.exe` only outside the injected command seam (`:502-533`). None worth changing.

### 3.5 Sync side (girsync, sync-cache, sync-leaves, sled2redb)

**F30 — ~44 in-process CLI cases force the file-backed redb cache; ~90 expensive opens.**
Evidence: `crates/sync-test-support/src/lib.rs:526-540` unconditionally sets
`GIRSYNC_TEST_CACHE=file` in `run_cli_with_hooks`, which all `run_args`/`run_text`/
`run_error_text` helpers funnel through. `tools/sync/src/commands/sync/mod.rs:262-263`
opens two folder caches per `sync` run; a fresh cache is a 1 MB create
(`crates/sync-cache/src/db/backend.rs:133-136`) and an open is ~0.5 s on this host
(`tools/sync/tests/migration_lock.rs:86-99`). Call sites asserting only records/FS state:
`tools/sync/tests/symlink.rs:398,431,456,523,548,572,600,629,662,706,745,920,1081,1089,1127,1171,1205,1237`
(~18) and `tools/sync/tests/sync.rs:321,333,355,415,433,464,522,582,642,647,846,927,955,1000,1049,1222,1696,1773,1787,1811,1907,1971,2037,2080,2104,2124`
(~26), plus `tests/compare.rs:682`, `tests/compare_self.rs:161`, `tests/sync_plan/*`,
`tests/rw_threads/main.rs:224`, `tests/why/flags.rs:14,267`. This is the dominant source
of the 311 sync-side test-seconds. Fix: add a `run_args_mem`/`run_cli_mem` seam setting
`GIRSYNC_TEST_CACHE=memory` and use it for record/FS-only cases; keep the file backend
for cases whose subject is the cache file (`sync.rs:2141+` backup/retain,
`migration_lock.rs`, cache-copy checks at `:1088-1160`). The in-memory registry is
path-keyed and shared within a process (`sync-test-support:519-524`), so audit
dry-then-real pairs (`sync.rs:321/333`, `symlink.rs:1081/1089`) and call the existing
forget helper when a cold cache is required. Benefit **High (wall, I/O)**, effort **M**.

**F31 — Row-reading helpers reopen redb after every command.**
Evidence: `tools/sync/tests/sync.rs:41-58` (`md5_in`, `md5_in_file`),
`crates/sync-test-support/src/lib.rs:293-306,313-321` (`recs_of`, `cache_at`, `edit_row`,
`strip_algo`, `poison_digest`). `migration_lock.rs:86-92` documents the cost and already
introduces a metadata-only `cache_snapshot`. Fix: largely falls out of F30 (memory entries
are found by `cache_at`); where a real file is needed, return rows on a test-only
`RunOutcome` instead of reopening. Benefit **Med**, effort **M**.

**F32 — 300 KB byte-at-a-time digest loop in one sync-cache unit test.**
Evidence: `crates/sync-cache/src/hash/tests.rs:1052-1084` builds 300,000 bytes and calls
`by_byte.update(from_ref(b))` for each of three algorithm sets = 900,000 `update` calls
in an unoptimized profile. The comment at `:1053-1055` only requires the body to be under
`CHUNK` (512 KiB, `hash/stream.rs:15`) and to cross digest block boundaries. Fix: reduce
to 4–8 KB. Benefit **Med (one test)**, effort **S**.

**F33 — Repeated `temp_root`/`src`/`dst` setup in `sync.rs`.**
Evidence: ~50 three-line sequences, e.g. `:161-166,207-211,232-238,272-279,315-318`.
A `pair` helper exists (`sync-test-support:399-458`) but cases are heterogeneous (mtime
pinning, one-sided shapes). Fix: a thin `dirs(tag) -> (TempDir, PathBuf, PathBuf)` helper
where semantics permit. Benefit **Low (source/compile only)**, effort **S**.

**F34 — Sync-side non-candidates.** sled2redb fixtures are tiny and already have a
no-sled path (`tools/sled2redb/tests/convert.rs:51-84,137-230`); its three converter
spawns (`:586,685,832`) assert process-boundary behavior. `output.rs` has three justified
spawns (`:42,560,578`). No `#[ignore]`d tests on the sync side. md-5/sha2/serde_json/clap
are production deps, so their dev-dep entries add no compilation. Do not lower
`GIRSYNC_CACHE_SIZE_MIB`: redb 5.0.0 treats it as a budget, not an up-front allocation.

### 3.6 Capture side (`gircap`) and its subcrates

**F35 — gircap's 641 lib tests are mostly process spawns; merge pure clusters.**
Evidence: gircap lib = 641 tests / 91.2 cumulative s (~0.14 s each);
`tools/capture/tests/measured_fixture.rs` has 29 tests each rebuilding the same fixture
via `game()` (`:54-279`), `hash/src/tests.rs` 23, `src/config/tests.rs` 39,
`src/select/tests.rs` 39. Fix: consolidate pure-function groups into single tests with
labelled assertion blocks, accepting coarser failure granularity; the
`capture_layout.rs:431-520` trio of near-identical "missing document" tests is one
candidate table. Benefit **Low–Med (wall)**, effort **M**. Measure nextest per-test
overhead before a broad rewrite.

**F36 — Full bundle captures are rebuilt per test process.**
Evidence: `tools/capture/test-support/src/lib.rs:642-699` caches generated blobs in a
per-process `OnceLock` (useless across tests, A1); `capture_ok`
(`tools/capture/tests/capture_layout.rs:228-235`) and `captured`
(`tools/capture/tests/matrix.rs:89-118`) build a fresh `World::all_shapes()`, zstd-encode
~25 blobs (one 32 KB at `test-support:895-908`) and write a bundle tree for ~20 + ~10
callers. `shared_bundle()` (`test-support:328-385`) is the read-only alternative already
used by several tests. Fix: switch read-only callers (e.g.
`capture_layout.rs:670`) to `shared_bundle`; do not change spawn-based or
flag-sensitive cases. Benefit **Low**, effort **M**.

**F37 — Shared-bundle cache key hashes every source file.**
Evidence: `tools/capture/test-support/src/lib.rs:240-258,271-287,329` walks all `.rs`
under `src/` plus `Cargo.toml`, so any edit (including logging/CLI/exit code) forces one
full capture on the next test process. The hash I/O itself is negligible. Fix: not
recommended without profiling — narrowing the key to output-affecting modules accepts a
stale-bundle risk the current comment deliberately avoids. Recorded as a considered and
rejected change.

**F38 — `fs2` is an unused dev-dependency of `gircap`.**
Evidence: `tools/capture/Cargo.toml:217` declares it with a comment about
`tests/common/mod.rs`, which is now just `pub use gircap_test_support::*;`
(`tools/capture/tests/common/mod.rs:1-12`). Actual `fs2` use is in
`tools/capture/test-support/src/lib.rs:57,345`; `cargo tree -i fs2` also shows sled. Fix:
delete the line; no dependency-tree change. Benefit **Low**, effort **S**.

## 4. Highest-value findings at a glance

| Finding | Benefit | Effort | Primary resource |
| --- | --- | --- | --- |
| F1 consolidate 14 root + ~16 girsync test binaries | High | M | build/link |
| F30 memory cache for record/FS-asserting girsync CLI cases | High | M | wall/I/O |
| F17 structured repair diagnostics to drop 10 log-only spawns | High | L | wall/process |
| F18 pure log-announcement helper to drop 14 cli_contract spawns | High/M | M/S | wall/process |
| F2 one std-only loopback HTTP mock, drop wiremock everywhere | Med–High | M | build |
| F4 trial rust-lld on this MSVC host | High if validated | S–M | build/link |
| F5 reclaim ~191 GB of stale `target/` output | High (disk) | S | disk |
| F6 narrow rebuild edges (util split, `BodyFile`, log features) | Med–High | S–L | build turnaround |
| F3 pin exact stable toolchain | Med | S | build turnaround |
| F32 shrink 300 KB byte-at-a-time digest fixture | Med | S | wall/CPU |

## 5. Explicitly not recommended (already done, or would regress)

- Do not scale down fixtures further (A5); do not re-add sleeps or the "without
  fail-fast"/"without terminal 404" control runs (A6).
- Do not try to share `OnceLock` fixtures across nextest tests (A1), and do not add
  `rstest` for existing loop tables (F7).
- Do not re-recommend no-fsync hooks, shared tempdir/test-support crates, leaf-crate
  extraction already landed, `profile.test debug = 0 / codegen-units = 1 /
  incremental = false`, or the ignored bridge/throughput suites.
- Do not drop root's `prost` dev-dep without first freezing the three manifest bodies
  (F9/F14 — it is used by `src/repair/phases/manifest.rs` unit tests).
- Do not lower `GIRSYNC_CACHE_SIZE_MIB` (F34) or rewrite capture's 32 KB blob (A5/F36).
- Do not remove `fs-identity`'s dev-dep-free scratch dir, `report-core`'s `serde_json`,
  or the narrow-consumer `fetch_direct.rs` integration binary (F29/F28).
- Do not delete any test named in `.config/nextest.toml` overrides without updating those
  filters (A10).

## 6. Validation notes for implementation

Every finding above is a hypothesis to be checked in the implementing change: confirm
clean-build and `--workspace` test wall time before/after, confirm that the semantic
assertion a test carries is still present after any merge/downshift, and confirm the
dependency-tree change with `cargo tree -e dev -p <crate>`. Line numbers come from this
tree. Items marked "isolated builds only" (F2, F6, F25) will not show up in a
feature-unified workspace run (A11); measure them with `cargo nextest run -p <crate>`.
