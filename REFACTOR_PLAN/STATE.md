# Refactor state

Single source of truth for orchestration. The orchestrating session may be replaced at
any time; all durable knowledge lives here. See `ORCHESTRATION.md` for the procedure
and `CONVENTIONS.md` for the format.

States: `todo` → `in_progress` → `pending_review` → `completed` / `reverted` /
`skipped`. Every change to this file is its own commit. Commit refs are pointers for
reviewers only — no dates, session/tool IDs, machine data or change log.

## Tasks

| ID | Title | State | Commit ref(s) | Note |
|----|-------|-------|---------------|------|
| T01 | Consolidate root integration test binaries | completed | 080a0d45 | 4 root targets; kept artifact_bridge & repair_post standalone (pinned filters/host-skip/log-capture) so no nextest.toml edit; consolidated FQNs gained module prefix (repair::/contracts::); run_lock uses crate::common |
| T02 | Memory cache seam for girsync record/FS cases | completed | b54115fd, 60382ea2 | 2 commits (seam + migration); file opens 302→183 (~39%), sync wall 62.6→41-46s; dry-run pairs skip cache writes so no forget call needed (wrapper exported); kept 4 cache-file-subject cases on file backend; F31 falls out for migrated cases; cold-cache pattern = rw_threads/limiter.rs drop_caches() |
| T03 | Structured repair diagnostics to drop log-only spawns | completed | aa970db2, 02ddeaaa | 2 commits (surface + migration); 13 spawns→1 (one process smoke kept); premise was 11 cases/13 calls not 10/10; structured diagnostics are thread-local, take_repair_diagnostics() needs current-thread runtime; Rig::logs now lazy OnceLock; no nextest filter change; CLI goldens/shipped bytes unchanged |
| T04 | Pure log-announcement helper to drop cli_contract spawns | completed | e41a134a, a627d4fd | 2 commits (helper + migration); 14 log-case spawns→1 smoke (separate from T03 repair smoke); contracts binary nextest 2.141s→1.235s; cases were not purely formatting — process-boundary assertions preserved via open_or_warn unit test, girpr_log::test_support::logs_of, pure plan assertions; no nextest filter change; announcement bytes/goldens unchanged; 0 girpr_<ts>.log under target/ |
| T05 | Replace wiremock with a std-only loopback HTTP mock | completed | b5eba55d, c2fb8120, 6a375e6b, 5e7b2735, 8a8d077e, e1dee308, 80053a67, ab98c622 (review fix) | 7 commits: dependency-free leaf + 4 site migrations + repair-server fold-in + manifest cleanup; wiremock gone from all 4 manifests; unique deps girpr 220→208, fetch 167→152, gircap 222→210, gircap-http 161→146; cargo tree -d 229→199; no wiremock smoke kept (wire compat not contractual); metadata.rs serve returns (TestServer,String), tests bind _origin to keep Drop-abort alive; capture malformed/probe/hyp/test-fixtures servers untouched (distinct contracts) |
| T06 | Trial rust-lld for the test profile | completed | c603441b | Adopted: 1 config line in .cargo/config.toml (linker = rust-lld for x86_64-pc-windows-msvc) + comment; A/B/A/B clean nextest wall 436.4s→408.5s (~4-7%); all workspace targets linked, 2175 passed/11 skipped; profile.test debug/codegen untouched; host-dependent per §F4/§A7, revalidate per host/CI |
| T07 | Pin the stable toolchain to an exact release | completed | 08d799c5 | 1 commit; channel=1.100.0 (exact rustc already in use, no compiler change, only one-time cache invalidation); README, tests/scripts/io-profile/README.md and rust-toolchain.toml comment updated; build + nextest 2175 passed/11 skipped; no MSRV floor; Cargo.toml [package] "stable" comment kept (accurate, avoids manifest-body change); .cargo/config.toml untouched; host-specific pointer §F3/§A8 |
| T08 | Narrow `girpr-util` rebuild edges | completed | 3b502ee5, 70083a25, 33931270, 1f5dff56, 2e5b1ea9 | 5 commits: split into girpr-util-path/hash/ini/counter, facade deleted; ini.rs edit now leaves audio/fetch/report/lock Fresh (warmed state per §A11); isolated leaves 82 passed/2 skipped; full 2175 passed/11 skipped; fetch uses hash+path (report's "disjoint" claim slightly off); atomic lives in path, counter its own leaf; junction fixtures + FixtureRefusal behind girpr-util-path test-support, md5 slices behind girpr-util-hash test-support — preserve these seams; 3 visibility promotions for ini fixture helpers; nextest filters + doc tables updated |
| T09 | Narrow the small rebuild-locality edges | completed | db43c5a6, 02081be2, e9039e69, 1aa50d2c (review fix) | 3 commits, all 3 sub-items implemented: BodyFile moved to girpr-digest (re-export kept as gircap_http::BodyFile — removing it later needs gircap fetcher import + README update); logs_of_async behind test-support-async feature (2 consumers, report's "sole consumer" claim stale; util-ini no tokio); clap ValueEnum behind clap feature in girpr-report (kebab-case preserved, root enables); gircap-hash no gircap-http dep; 2175 passed/11 skipped; no nextest filter change |
| T10 | Gate the fully-ignored `bundle_shapes` binary | completed | 1ed90f21 | 1 commit; empty manual-tests feature + crate-level cfg guard + docs; normal gircap build/test lists 0 bundle_shapes, manual nextest command runs all 3 ignored tests; no nextest filter named it (no change needed); artifact-bridge feature-gating rejection left untouched |
| T11 | Root dev-dependency hygiene and frozen manifest bodies | completed | e57340a9 | 1 commit; froze 3 deterministic bodies as const &[u8] with pinned length/body MD5/manifest checksum; removed root prost dev-dep + tokio test-util feature (unused); zstd KEPT — premise contradicted, zstd_bytes() in tests/artifact_server.rs compiled via contracts binary; prost/zstd already in normal tree via girpr-fetch so declaration hygiene, not crate-count win; 2176 passed/8 skipped |
| T12 | Root `src/` unit-test cleanups | completed | 710c29a3, 0b494593, 9cb2fc97 | 3 commits: hand-rolled runtimes→#[tokio::test] (one cross-thread case kept manual); guards→2s except manifest.rs:537 kept 5s (deadline, not signal wait); CLI parse(extra)+table test (432→431 tests); audio-fixture sub-item SKIPPED — alias cases already loop-table and per-test temp dirs mean no cost saved; no sleeps, loaded-run guard check clean, no byte/production change |
| T13 | Collapse duplicated root integration fixtures | completed | 81344b48, 7e312de9, 37610881 | 3 commits: F19 JSON/text pairs→3 merged tests with reseed_install helper (config bump contaminates second plane otherwise); F20 phase/precedence tables (process tests 11→8, premise partly pre-satisfied); F23 Clone derives at call sites (30 duplicate builds removed, no wall saving expected); process spawns/messages/bytes preserved; no pinned filter affected; 425 passed/6 skipped |
| T14 | Group `artifact_server.rs` bundle tests | completed | fe0fea6b | 1 commit; 5 with_shapes tests→1 grouped test (one bundle + one bridge), 3 other shape tests untouched; tests 8→4, bundles 8→4, listeners 32→16; all 87 assertions preserved verbatim incl HTTP status/body/zstd; full TCP end-to-end case unchanged; no nextest filter named them; no OnceLock/sleeps/fixture growth; zstd_bytes intact; contracts 44 passed |
| T15 | Remove cross-file duplicate process-level assertions | completed | a94eac24 | 1 commit; removed 1 duplicate process case after folding its only process-only assertion (getBuild via endpoint overrides) into base_precedence_and_env_fallback smoke; spawns 14→13, process tests 8→7; coverage verified in-process first (REPORT/SUMMARY via cli_contract goldens, scope via repair_scope); no nextest filter change; pinned repair_post + host-skips untouched; 420 passed/6 skipped; F22 premises partly stale (already reduced by earlier work) |
| T16 | Fold tiny pure-test clusters | completed | 4e8d728c, 97a7cd34, 5588b71f, f2ae25b8, 09b8f468, 0301c15c | 6 commits (one per cluster), none skipped; measured report-core isolated 0.070s→0.043s (~38% saved by 3 fewer spawns); 45→24 test fns (21 fewer spawns); every assertion/message preserved with labelled subcases; pinned junction/symlink, clock and artifact-bridge cases untouched; no nextest filter change; 79 passed |
| T17 | Leaf-crate test cleanups | completed | f90cb5ef, 98740650 | 2 commits: ledger barrier-free exact-sum tests 8×100→4×25 (sums 700/1100 retained), Barrier race test kept at 8, ceiling-overshoot race tests untouched; redundant tag-6 byte assertion removed from fetch_direct (binary + compile check kept) and value-0 proto_roundtrip duplicate removed; authoritative girpr-proto byte pin unchanged; no nextest filter change; report 40/40, fetch 34/34, proto 1/1 |
| T18 | Reduce row-reading redb reopens | completed | aba631af | 1 commit; step-1 confirmed T02 pre-satisfied the bulk, then addressed 4 residual file-subject reopens via test-only thread-local row observer + path-keyed in-memory row mirror (instead of RunOutcome.rows); opens 21→14, sync pair 3.204→2.531s, migration pair 1.642→0.801s; assertions/helper semantics/file backend unchanged; no nextest filter change; 585 passed |
| T19 | Shrink the sync-cache digest fixture and add `dirs()` | completed | cc069d9b, 44991aed | 2 commits: digest body 300,000→8192 bytes (still crosses 128×64-byte blocks, under CHUNK, assertion intact, 900k→24.5k updates); dirs(tag) helper applied to 32 homogeneous sequences, 17 left alone (14 mtime-pinned + 3 heterogeneous), _t guards keep TempDir alive; hygiene only, no wall claim; no nextest filter change; sync 46/46, sync-cache 129/129 |
| T20 | Merge pure gircap test clusters | pending_review | bc115d8a, 9ad944de, 30c38788 | 3 commits: config 39→1 (0.370→0.020s), select 39→1 (0.384→0.053s), measured_fixture 29→1 (0.981→0.83-0.86s); hash cluster SKIPPED (~1s real CPU/test, merging loses parallelism 0.712→0.884s), capture_layout trio SKIPPED (shapes differ + spawns binary); gircap lib 641→565, 10.8-11.2s→8.0-9.1s; bodies byte-identical (only #[test] removed), labelled case helper; no nextest filter change, independence held; 751 passed |
| T21 | Use `shared_bundle` for read-only capture tests; drop `fs2` | in_progress | | |

## Reviews

| ID | Batch (task IDs) | State | Note |
|----|------------------|-------|------|
| A | T01 | done | |
| B | T02 | done | |
| C | T03, T04 | done | |
| D | T05 | done | Review fix ab98c622 added missing gircap leaf-crate doc table entries |
| E | T06, T07 | done | |
| F | T08 | done | |
| G | T09, T10, T11 | done | Review fix 1aa50d2c removed stale gircap-http edge from hash crate docs |
| H | T12, T13 | done | |
| I | T14, T15 | done | |
| J | T16, T17 | done | |
| K | T18, T19 | done | T18 had genuine residual work beyond T02's memory seam |
| L | T20, T21 | pending | |

## Follow-ups

Follow-ups are non-blocking suggestions from review sessions; the orchestrator records
them here. A pruning review reduced the list to those below (12 dropped, FU10 merged
into FU09). Phase 3 planning decides final batching and any further drops.

| ID | Description | State | Note |
|----|-------------|-------|------|
| FU01 | Re-measure clean-build wall time on CI with a fresh target dir | dropped | Host-specific; link reduction verified structurally |
| FU02 | Merging repair_post into repair needs pinned overrides updated | dropped | Restates T01 decision; merge explicitly deferred |
| FU03 | Narrow blanket allow(dead_code, unused_imports) in tests/common/mod.rs | dropped | Cosmetic |
| FU04 | Exercise or remove the unused forget_memory_cache wrapper | dropped | No cold-cache need; helper kept for F30 |
| FU05 | F31 residual redb reopens (md5_in/md5_in_file, cache-copy keepers) | dropped | Duplicate of active task T18 |
| FU06 | Remove ignored _extra env parameter in rw_threads/main.rs::run_env | dropped | Pre-existing vestigial nit |
| FU07 | Check a_copied_file_carries_the_sources_timestamp for cache reuse across its two runs | open | Real semantic gap: path-keyed cache may let run 2 skip the copy |
| FU08 | Restore in-place precondition in T03 a_scanned_repair_and_an_unscanned_repair_agree parity case | open | None==None could pass |
| FU09 | Name-overstates-coverage hygiene in T04 log cases | open | Absorbed FU10; keep one item for this class |
| FU10 | "created" in an_explicit_log_file_is_echoed_and_created is aspiration | dropped | Merged into FU09 (same class) |
| FU11 | Stale T03 comment at tests/repair_machinery.rs:1603 re LogLevel debug | dropped | Comment only |
| FU12 | Strengthen T04 level-sink assertion beyond plan filter directives to rendered records | dropped | Covered by smoke + girpr-log unit tests |
| FU13 | Record exact cargo tree command behind T05 dependency-count figures | dropped | Host/cargo-specific; deltas confirmed |
| FU14 | Harden loopback-mock ScriptedServer Drop-abort against a stalled read_line | open | Latent test-hang risk |
| FU15 | Document a deliberate toolchain-bump procedure (trigger/cadence/owner) | dropped | Process-doc nit |
| FU16 | Revalidate the rust-lld linker win on the CI host | dropped | Caveat already in T06 note; near-duplicate of FU01 |
| FU17 | Re-trial and scope linker config for other MSVC triples if support widens | dropped | Conditional future scope |
| FU18 | Narrow the root's private mod util glob alias to per-leaf aliases | open | Clarity only; no rebuild-graph effect — likely drop at Phase 3 |
| FU19 | Clarify the 3 promoted junction test-helper visibilities as test-only | open | No behavior/byte change; already behind test-support feature — likely drop |
| FU20 | Strengthen frozen-body pin test to verify body↔checksum relation via zstd decode + md5 | open | Byte contract self-containment; functional tests already cover it |
| FU21 | Consider dropping the unused gircap_http::BodyFile compatibility re-export | open | Needs out-of-tree caller check; trait canonical path is girpr_digest::BodyFile |
| FU22 | Optional merge of the 12 post.rs audio-move tests into one table test | open | Conditional on per-test spawn being material; F26/F35 want measurement first |
| FU23 | Reseed install between rows of the phase table if state-dependent assertions are added | open | Future-conditional; current assertions are flag-only |
| FU24 | Extend T13 reseed_install if the binary persists new local-state files | open | Maintenance note; currently resets config.ini + two fixture files |
| FU25 | Finish F22 in-process rig dedup in repair_post/scan/plan | open | Constraint-blocked: repair_post host-skip/log-capture cases must stay untouched |
| FU26 | Consider splitting the ~324-line merged T14 test | open | Only if failure triage proves painful; labels/messages already preserved |
| FU27 | Fold remaining tiny pure-test clusters beyond the six named | open | Broad rewrite, modest sampled benefit; re-measure per-test overhead first |
| FU28 | Remove label/match-arm duplication in retry_driver_contracts | open | Cosmetic only |
| FU29 | Derive row-mirror case sensitivity from captured cache meta, not hardcoded true | open | Future case-insensitive file-subject case would disagree |
| FU30 | Use a drop-guard so with_observed_rows clears ROW_CAPTURE on panic | open | Harmless under nextest; robustness for in-process sequential runs |

## Persistent context

Standing invariants from the report (pre-seeded so they survive a replacement
orchestrator; a session that finds one contradicted by the code should record that in
the relevant task's `note` instead of silently proceeding):

- **Reports are static, host-specific claims (§1, lines 12–42; §6, lines 503–510).**
  Every finding is a hypothesis to validate against the current code; confirm
  before/after wall and clean-build times and that the semantic assertion survives a
  merge. Line numbers may have drifted; trust the code.
- **One process per test under nextest (§A1, lines 46–52).** A process-wide cache,
  `OnceLock` or static fixture helps only within one test. Merging test functions
  saves spawns; parameterized cases add spawns. Do not try to share fixtures across
  tests.
- **Rebuild and split payoffs (§A2, lines 54–59; §A3, lines 61–65).** Editing a file
  invalidates its target and downstream dependents. A split pays off only when a
  high-level consumer depends directly on the smaller crate; a re-exporting facade is
  not a win. Several findings are turnaround wins invisible in a feature-unified
  workspace run (§A11, lines 112–116) — measure them with `-p <crate>`.
- **Already-done policy is not a candidate (§A4, lines 67–73; §5, lines 486–501).**
  No-fsync and scratch policy, tiny fixtures, leaf extraction already landed, and
  `profile.test` tuning are existing facts. Do not re-add sleeps or the "without
  fail-fast"/"without terminal 404" control runs; do not scale fixtures down further.
- **No sleeps (§A6, lines 82–89).** The suites contain no sleeps; guards wake on
  notify/channel and never fire on a passing run. Adding sleeps is a regression.
- **Host dependence (§A7, lines 91–95).** Linker, cache opens and Windows process
  creation dominate on the research host; other hosts rank findings differently. A
  local measurement is a hypothesis for CI, not a universal result.
- **Wiremock's footprint (§A9, lines 101–106).** wiremock 0.6 exclusively pulls
  `regex`, `futures`, `deadpool`, `assert-json-diff` and `base64 0.22`. A std-only
  loopback mock pattern already exists in the tree; keep the new mock dependency-free
  so `girpr-fetch` and `girpr-test-fixtures` can use it without a cycle.
- **nextest name filters are pinned (§A10, lines 108–110).** Renaming, moving or
  merging a named binary or fully-qualified test path requires editing
  `.config/nextest.toml` in the same change.
- **Explicitly not recommended (§5, lines 486–501) and the non-candidate findings
  (F7, F29, F34, F37) are out of scope.** See `guidance/DO-NOT-TOUCH.md`. Disk
  maintenance (F5, F24) is not tracked as a task; it changes no version-controlled
  artifact.
