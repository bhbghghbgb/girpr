# STATE — checklist (single source of truth)

Owned by the orchestrator. Committed to version control. This file is the only
thing a replacement orchestrator needs to resume; keep it current. See
`ORCHESTRATION.md` for the process and `README.md` for the file index.

Do not put session-spawning mechanisms, dates/times, session ids, machine data
or historical change logs in this file. Record context, decisions, status and
commit ranges only.

## Current position

* Next action: wait for implementing session on T17, then advance per the loop.
* Active checkpoint: CP4 (`open`); T14, T15, T16 `pending_review`, T17 `in_progress`.
* Phase: implementation.
* Blocked on user: no.

## Status vocabulary

* Tasks: `todo` → `in_progress` → `pending_review` → `completed` /
  `completed (fixed)` / `reverted`; or `todo`/`in_progress` → `skipped`.
* Checkpoints: `open` → `in_progress` → `done`.
* Follow-ups: `proposed` → `scheduled` → `completed` / `removed`.

## Tasks

| ID | Title | Status | Checkpoint | Notes |
|---|---|---|---|---|
| T01 | Lower heartbeat fixture via test hook | completed | CP1 | implemented in 833f6d6a; CP1 review: completed |
| T02 | Default `run_binary_raw` to memory cache | completed | CP1 | implemented in eb6afaf2; CP1 review: completed |
| T03 | Audit `opts()` → memory helpers | skipped | CP1 | stale: R1 commits already landed the audit; CP1 review: skip confirmed |
| T04 | Shrink rw_threads 96-file fixture | completed | CP1 | implemented in c7b9c1c0; CP1 review: completed |
| T05 | Shrink `as_done` 11 MB fixture | completed | CP1 | implemented in a2ddea53; CP1 review: completed |
| T06 | Consolidate gircap captures + shared bundle | completed | CP2 | CP2 review: completed |
| T07 | Hand-build minimal bundles for delete-a-doc | completed | CP2 | CP2 review: completed |
| T08 | Cache gircap byte tables + shrink chunk | completed | CP2 | CP2 review: completed (partial scope accepted; old.blk shrink deferred to F15) |
| T09 | Cut sled2redb spelling test 8 → 2–3 | completed | CP3 | CP3 review: completed |
| T10 | Drop sled2redb flush + redundant reopens | completed | CP3 | CP3 review: completed (product flush removal validated) |
| T11 | Replace girsync publish fixed sleeps | completed | CP3 | CP3 review: completed |
| T12 | Replace girpr panic-ordering sleep | completed | CP3 | CP3 review: completed |
| T13 | Add gircap zero-backoff retry hook | completed | CP3 | CP3 review: completed |
| T14 | Reduce girpr trace-level logging | pending_review | CP4 | 32d12ebe; see review range |
| T15 | Remove girpr `--help`/argument spawns | pending_review | CP4 | ac507a7c; see review range |
| T16 | Consolidate girpr multi-run parity tests | pending_review | CP4 | b51e0395; see review range |
| T17 | Merge girsync dry-run/hard-link groups | in_progress | CP4 | |
| T18 | Merge redundant gircap read/verify groups | todo | CP4 | |
| T19 | Tidy ignored tests | todo | CP5 | |
| T20 | Guardrail audit (hooks, no temp→`Z:`) | todo | CP5 | |

## Review checkpoints

| CP | Tasks | Status | Notes |
|---|---|---|---|
| CP1 | T01–T05 | done | girsync fixtures & cache backend |
| CP2 | T06–T08 | done | gircap capture consolidation & fixtures |
| CP3 | T09–T13 | done | sled2redb conversions & fixed sleeps |
| CP4 | T14–T18 | open | girpr spawns, logging & redundant merges |
| CP5 | T19–T20 | open | ignored tests & final guardrail audit |

## Commit ranges for review

Filled by the orchestrator when a task moves to `pending_review`; consumed by
the review-and-fix session. Keep the range until the checkpoint is `done`.

| Task | Commit range | Review verdict |
|---|---|---|
| T01 | `ae60730d..833f6d6a` | completed |
| T02 | `c1e2fb04..eb6afaf2` | completed |
| T04 | `f84883a7..c7b9c1c0` | completed |
| T05 | `e1ef69fb..a2ddea53` | completed |
| T06 | `575c7ed0..9096e118` | completed |
| T07 | `a1de5165..81988899` | completed |
| T08 | `66ac441b..6bcd4b87` | completed (partial accepted) |
| T09 | `0c7e0b20..37d11fb5` | completed |
| T10 | `aa9c1add..f342d32e` | completed |
| T11 | `fa0a7687..f7eb3995` | completed |
| T12 | `326acea9..c33dfe0e` | completed |
| T13 | `fbf8d14c..1fe87a27` | completed |
| T14 | `49457928..32d12ebe` | — |
| T15 | `f7392a76..ac507a7c` | — |
| T16 | `aeefde5c..b51e0395` | — |

## Follow-ups

Reviewer-controlled. Record proposed ideas here; only implement after all tasks
are done and a review session has decided the batches.

| ID | Description | Source | Status | Note |
|---|---|---|---|---|
| F01 | Add a test-only hook for phase A's 100-file progress threshold in `effective/phase_a.rs` (optionally emit on final entry like phase C), then lower the T01 fixture to ~8 files. Product-code change; out of T01 scope. | T01 implementer | removed | CP1 review: only ~0.23 s remains after the 101-file fix; not worth a product-code change. |
| F02 | Unit test in `scan.rs` locking `heartbeat_entries() == 2000` with no hook set, to guard the production default. | T01 implementer | proposed | CP1 review: KEEP — backs T01's acceptance criterion. |
| F03 | Named constant (e.g. `FILE_CACHE_ENV`) for the `("GIRSYNC_TEST_CACHE", "file")` opt-out repeated across three tests in `tests/sync.rs`. | T02 implementer | removed | CP1 review: purely cosmetic constant. |
| F04 | Debug assertion in `run_binary`/`run_binary_raw` rejecting a redundant `GIRSYNC_TEST_CACHE` in `envs`, to keep "one clear default" enforced against regressions. | T02 implementer | removed | CP1 review: as stated would reject valid `"file"` opt-outs, conflicting with intended API. |
| F05 | On next report regeneration, account for already-landed R1 memory-cache work so §6.1 B5/C7 are not re-flagged as outstanding. | T03 implementer | removed | CP1 review: moot — report deleted at completion. |
| F06 | One-line note in `common/mod.rs` near `opts()` pointing readers to the `*_mem` variants, to avoid re-introducing file backends in semantic suites. | T03 implementer | removed | CP1 review: pointer already exists in `opts_mem()`'s doc comment. |
| F07 | In `through_the_binary.rs`, reuse one directory across passes of `update_hashes_the_same_set_at_every_limit` instead of 6 fresh 40-file trees — needs per-pass cache isolation to avoid leaking cache rows. | T04 implementer | removed | CP1 review: per-pass isolation risky; T04 captured the dominant win. |
| F08 | Reduce per-pad byte size below 512 B in the rw_threads fixture (pads only need stat-equality + identical content). | T04 implementer | removed | CP1 review: negligible savings; count was the dominant cost. |
| F09 | Debug assertion/comment tying `PAD_COUNT` to the max `pool_size()` in `settings()` to guard the "more files than workers" invariant against future worker increases. | T04 implementer | proposed | CP1 review: KEEP — cheap invariant guard for T04's core claim. |
| F10 | Add the gircap binary's fingerprint to the shared-bundle key so a product-code change to capture output also invalidates the cached bundle (today only the test-side world source does). | T06 implementer | proposed | CP2 review: KEEP — real stale-cache hardening. |
| F11 | Unit test asserting the shared-bundle key is stable across repeated calls and changes when `common/mod.rs` changes, guarding the invalidation contract. | T06 implementer | removed | CP2 review: key is a deterministic pure function of a file read; real gap is F10. |
| F12 | When T07 lands, confirm the two schema key-deletion tests don't regress to fresh captures (they already use the shared bundle as copy source). | T06 implementer | removed | CP2 review: resolved — schema key-deletion tests now use `minimal_bundle`, no `copy_tree`. |
| F13 | Split `the_bundle_has_the_shape_the_schema_documents` (layout) and `admission_is_cheapest_first...` / `audio_candidates_are_attributed...` (matrix) so document reads can move onto the shared fixture; summary assertions stay. | T06 implementer | proposed | CP2 review: KEEP — further mixed-test consolidation. |
| F14 | Consolidate the new `minimal_bundle` test helper with the private `write_minimal_bundle` in `src/verify/tests.rs:380` into one shared test-support target to avoid drift (integration tests cannot reach `#[cfg(test)]` lib code). | T07 implementer | proposed | CP2 review: KEEP — dedupe the two minimal-bundle writers. |
| F15 | If the `old.blk` shrink is still wanted: refresh the two pinned rows in `capture_layout.rs` (:1133 manifest checksum, :1157 chunk id + `bytes_in`) and keep `old.blk` above `eager.blk`'s ~14 KB so the over-budget test still refuses `removed`. | T08 implementer | proposed | CP2 review: KEEP — the deferred old.blk shrink. |
| F16 | Cache `World::archive()` (common/mod.rs:1435) which also zstd-encodes + MD5s per `Mock::start`; naive `(tag, field)` keying goes stale for mutated worlds — a content-derived key could make it safe. | T08 implementer | proposed | CP2 review: KEEP — relevant to CPU-bound gircap. |
| F18 | In-process spelling check relies on the hand-built `the_converter_command()` mirror; if the binary's `Args` derive drifts from the mirror without affecting `--help`, the pin won't catch it. An authoritative in-process parse would expose/move `Args` from `main.rs` into the library (product change). | T09 implementer | removed | CP3 review: drift gap narrow (help pin + in-process asserts + 2 end-to-end conversions); moving Args into library disproportionate. |
| F19 | Removing the one remaining ~200 ms reopen in `convert_sled_dir_to_redb_file` would require exposing written records or accepting a caller-provided handle in the converter's product API. | T10 implementer | removed | CP3 review: conflicts with accepted T10 design (one reopen is the round-trip contract); not worth product-API change. |
| F20 | Capture actual before/after nextest timing for the sled2redb suite to confirm the reopen savings (report's P1 estimate bundled this with T09). | T10 implementer | removed | CP3 review: moot — report deleted at completion; speedup already demonstrated. |
| F21 | Extend the T11 one-shot gate to carry a "claim created" notification so `wait_for_published` becomes fully event-driven instead of polling 100x10 ms (not dead time, but returns eagerly). | T11 implementer | removed | CP3 review: polling is bounded, non-dead-time; marginal. |
| F22 | T12's `FILE_PANIC_GATE` is a `thread_local` relying on `#[tokio::test]`'s current-thread runtime; a multi_thread flavor would hide the gate. Consider a runtime-flavor-independent hook if such a change is anticipated. | T12 implementer | removed | CP3 review: speculative runtime-flavor change; gate fails loudly (5 s timeout) if flipped, so no silent gap. |
| F23 | T12's `after 5 attempts` assertion hard-codes the attempt cap as a string, decoupled from `sophon::stream::ATTEMPTS`; exposing the constant or reusing the `serve_status` hit-counting helper would remove the magic number. | T12 implementer | removed | CP3 review: literal is established across hyp.rs/stream.rs/retry.rs; exposing private const marginal. |
| F24 | Make T13's zero-backoff self-verifying: have the child emit a marker when `GIRCAP_TEST_ZERO_BACKOFF` is active and assert it, so a refactor can't silently reintroduce the 500 ms wait. | T13 implementer | proposed | CP3 review: KEEP — guards a real acceptance criterion (broken hook would still pass, just 500 ms slower). |
| F25 | Small unit test for `GIRCAP_TEST_ZERO_BACKOFF` parsing/default-off and for `ZeroSleep::sleep` returning immediately, to lock in the contract. | T13 implementer | proposed | CP3 review: KEEP — cheap unit test for production-safety criterion. |
| F26 | Add a `run_no_log()` variant to repair_machinery that omits `--log-file`/`--log-file-level` entirely for tests that never read the log, eliminating log-creation IO (debug builds write no log file without `--log-file`). | T14 implementer | proposed | Reviewer decides. |
| F27 | Assert chunk outcomes from the binary's `--output json` NDJSON records (`download_bytes`, etc.) instead of parsing log text in `downloaded_chunks`/`in_place_chunk_stats`, reducing coupling to the log format. | T14 implementer | proposed | Reviewer decides. |
| F28 | Use a `warn` default log level in repair_machinery, with the ~8 log-asserting tests explicitly requesting `debug`; marginal gain given kilobyte fixtures. | T14 implementer | proposed | Reviewer decides. |
| F29 | The ~8 remaining post-parse jobs-boundary spawns in cli_contract assert `"usage error: --jobs must be <= 1024"` stderr text already unit-tested in `src/cli.rs`; consider whether the spelling-parity test's process cost is worth it. | T15 implementer | proposed | Implementer left intact to honour "assertions unchanged"; reviewer decides. |
| F30 | `GIRPR_TEST_NO_LOG` hook idea: skip default log file creation for argument-only runs; low-value since debug builds (what nextest runs) write no default log file anyway. | T15 implementer | proposed | Reviewer decides. |
| F31 | Comprehensive T16 site still uses 3 spawns because the unscanned-estimate upper-bound check needs a separate `--print-plan` process; collapsing would require exposing the unscanned estimate as a parseable field (product/test-hook change). | T16 implementer | proposed | Out of scope; reviewer decides. |
| F32 | T16's `chunk_hits == [chunk_p, chunk_t]` assertion also exists in `repair_writer.rs::the_ordered_tier_keeps_one_direction...` with the same fixture; consider dropping from one site for strict single-assertion-per-fact. | T16 implementer | proposed | Reviewer decides. |
| F33 | Merge the three repaired `repair_scan.rs` tests into one table-driven test over modes (inplace/temp/ordered) centralizing "scanned estimate == real download" via `--plan-out`; higher coverage risk. | T16 implementer | proposed | Reviewer decides. |
| F17 | Narrow T06's shared-bundle fingerprint to just the world-generating section so harness-only edits (e.g. T08's cache) don't force a rebuild — only if staleness can be ruled out (current whole-file key is conservative/correct). | T08 implementer | removed | CP2 review: risks stale bundles for marginal rebuild savings; whole-file key is correct. |

## Context / decisions

Append-only notes for a replacement orchestrator: stale-report findings,
deliberate deviations from task files, why a task was skipped/reverted,
checkpoint boundary changes, and anything else needed to resume.

* T01 deviation (implementer): report suggested an 8-file fixture, but phase A emits its progress event only every 100 kept files (`effective/phase_a.rs:483`, inside the file branch), while phase C emits on its final entry. With one excluded file, the smallest period preserving the phase-A progress assertion is 101; 8 or 100 yielded an empty `phase_a_progress`. Implementer chose 101 (~1,900 fewer file creates; ~7 s -> ~0.25 s) rather than add a second product hook in `phase_a.rs`. Approved follow-ups F01/F02 recorded above; reviewer to confirm.
* T02 note: implementer also flagged the adjacent P0 `opts()` -> `opts_mem()` audit in `tests/common/mod.rs`/semantic suites; that is already task T03 on the roster, so no new follow-up was created. T02 added file-cache opt-outs only where on-disk bytes are asserted; `GIRSYNC_TEST_NO_FSYNC=0` durability hooks untouched.
* T03 skipped (implementer): audit target no longer exists — R1 commits already in the tree (`989b618c`, `1eafe53b`, `0ba1d130`, `8914668b`, `28678736`) implemented the `opts()` -> `*_mem` migration. Memory helpers (`opts_mem`, `pair_mem`, `sync_mem`, `compare_mem`, `update_mem`, `asking_mem`, `recs_of` memory-first lookup) are used throughout semantic suites; all `pair()` calls use `warmed: false`. Remaining `opts()` (File) sites are intentional cache-byte/on-disk assertions or harnesses overriding to Memory in their own `common`. Report's `helpers.rs` is a misdiagnosis (unit tests with no `cmd_*` calls). Full `cargo test -p girsync` passes (19 binaries, 0 failures). No code changed; reviewer to confirm skip.
* CP1 review done (no fix/revert commits; tree clean at `db3928cc`): verdicts T01/T02/T04/T05 `completed`, T03 `skipped` confirmed. Review re-ran T01 test (1/1), `sync` (48/48), `rw_threads` (15/15), `sync_plan` (19/19) — all green. Follow-ups kept: F02, F09. Removed: F01, F03, F04, F05, F06, F07, F08 (reasons in Follow-ups table). No new follow-ups.
* T06 implemented: shared read-only bundle fixture in `tests/common/mod.rs` under an `fs2` cross-process file lock, published by atomic rename under `girpr-test/`, keyed by a fingerprint of `common/mod.rs` (so T08's World change auto-invalidates); `fs2` added as dev-dep. schema.rs: 19 doc tests -> 1 grouped default test + gzip/ceiling variants; layout/matrix: 8+2 doc reads moved to shared bundle. Deletion tests kept for T07. Verified capture_layout+schema+matrix 53 pass (was 68 in 24.3s -> ~9.5-13.9s), full gircap 868 pass/3 skip, clippy clean. No product code changed.
* T07 implemented: added `minimal_bundle(name)` in `tests/common/mod.rs` (smallest directory `gircap::verify::Bundle::open` accepts, built from crate value types). Converted 5 capture_layout delete-a-doc tests (error_shapes x2, error_responses x2, rejections x1) and 2 schema missing-field tests off full capture/copy_tree; assertions unchanged; removed unused `copy_tree` helper. capture_layout 37/37, schema 6/6, clippy clean. Coordinated with T06 (reused common/mod.rs). NOTE: task/report said "six" delete-a-doc tests but only five exist (no path_map.json failure test) — reviewer to confirm none lost. T07 implementer says F12 is resolved (schema key-deletion tests no longer copy the shared tree). A minimal bundle has an empty matrix, so verifying one unmodified would fail reverify — future "verify passes" tests should use offline_bundle.rs::Bundle::build.
* T08 partial (commit 6bcd4b87): DONE — per-process `OnceLock<Mutex<HashMap<(u64,usize),Blob>>>` memo in `blob()` so each blob is encoded/MD5'd once per process (per-process only; doc points to T06 for cross-process). SKIPPED — shrinking the 32 KB `old.blk` chunk: `the_request_log_rows_are_what_the_spec_says_they_are` (capture_layout.rs:1157) pins its id/digest + `bytes_in=32778`, and matrix relies on `removed` being dearest; a trial shrink broke that test, so it was reverted and a why-not comment added. T06 fingerprint (whole common/mod.rs) invalidated correctly on the edit. Full gircap 868 pass/3 skip, clippy -D warnings clean. Reviewer to judge whether the partial scope is acceptable.
* CP2 review done (no fix/revert commits; tree clean at `9d968559`): verdicts T06/T07 `completed`, T08 `completed` (partial scope accepted; old.blk shrink deferred to F15). Review re-ran gircap 53/53 both cached and after deleting all shared bundles to force fresh parallel builds (race-free confirmed); clippy -D warnings clean. Confirmed the task's "six" delete-a-doc tests was a miscount (five exist; none lost). Follow-ups kept: F02, F09, F10, F13, F14, F15, F16. Removed: F11, F12, F17 (reasons in table). No new follow-ups.
* T09 implemented: `the_flags_are_accepted_on_either_side_of_the_paths` now checks all 6 spellings in-process via a `parse_in_process()` helper parsing against the existing `the_converter_command()` clap mirror, and keeps 2 full conversions (down from 8) — one asserting the full record, one preserving the byte-for-byte `--output json` == `--output=json` equality. convert suite 16/16, clippy clean. No product code changed. Report §6.4 item 3 (read-only old.flush() in src/convert.rs:228) is a separate P1 product item, left for a different task.
* T10 implemented (both parts): PRODUCT code — removed no-op `old.flush()` in `src/convert.rs` after validating `old` is opened read-only and only read-only `old.iter()` uses it; `drop(old)` releases the sled lock `replace_destination` needs. No output-format/durability change. TEST code — reduced file-backed `open_db` reopens 3 -> 1; kept `convert_sled_dir_to_redb_file` as the single round-trip contract test, dropped reopens from the two force tests (assert byte-identity/change + converter counts instead), updated `entry_names` expectations (lock sidecar came from the test's own open_db). Full sled2redb 16/16 + 1 unit, nextest 17/17. Product change in fea93769 — reviewer to scrutinize.
* T11 implemented: in `cache/open.rs`, replaced `PUBLISH_PAUSED` thread-local + 500 ms sleep in `pause_publish()` with a test-only `#[cfg(test)]` one-shot mpsc gate; the four publish tests release the sender only after observing the claim (dropped sender on panic prevents hangs). In `cache/write.rs`, replaced the 150 ms interval sleep with a `#[cfg(test)]` `backdate_last_commit` hook. Production behavior untouched. The 5 affected tests passed 10/10 consecutive runs (~0.2-0.3 s each, down from ~2 s); full 24-test cache filter passed. Report's 300 ms girpr panic sleep (T12) and 500 ms gircap retry sleep (T13) are separate tasks, untouched.
* T12 implemented (commit c33dfe0e): both sleeps validated present first. In `src/repair/file/mod.rs`, replaced the 300 ms tokio sleep in `a_panic_is_observed_before_a_slower_earlier_task` with a `#[cfg(test)]` oneshot gate — the join loop signals on observing a panicked task, task 0 waits; 5 s timeout + dropped sender prevent hangs. In `src/repair/phases/manifest.rs`, removed the 5 ms `ShortSleep` struct/impl and unused `SleepFuture` import, switched test to `NoSleep`, added an "after 5 attempts" error-chain assertion. Only test-only timing paths in product files changed. 2 affected tests 10/10, full girpr --lib 453 pass/0 fail/2 ignored, clippy -D warnings clean. Product change touches test-only paths — reviewer to scrutinize.
* T13 implemented (2 commits): validated code matched report. Added `ZeroSleep` sleeper in `http/transport.rs` (exported from `http/mod.rs`) returning an immediately-ready future; default-off test-only env hook `GIRCAP_TEST_ZERO_BACKOFF=1` read in `run::capture` selects `Fetcher::with_sleeper(..., ZeroSleep)` instead of `Fetcher::new(..., RealSleep)` (deliberately not cfg(test) so it works on the spawned binary). Retry test sets the hook only on its own child via new `run_with_env`/`capture_with_env` helpers; 503/retry + request-log assertions intact. Production RetryPolicy (500 ms) and Fetcher::new untouched. capture_layout 37/37, fmt clean. Product code changed (test-only default-off hook) — reviewer to scrutinize.
* CP3 review done (no fix/revert commits; tree clean at `7308c6f1`): verdicts T09–T13 all `completed`. Product changes in T10 (flush removal), T11/T12 (test-only cfg hooks), T13 (default-off env hook) all validated: production behavior/backoff/durability unchanged, deterministic gates, no flakiness (repeated runs). Suites re-run: convert 17/17, sled2redb 17/17, cache 89/89, girpr --lib 453/453, capture_layout 37/37. Follow-ups kept: F02, F09, F10, F13, F14, F15, F16, F24, F25. Removed: F18, F19, F20, F21, F22, F23 (reasons in table). No new follow-ups.
* T14 implemented (commit 32d12ebe): only `tests/repair_machinery.rs` changed. Added `LogLevel` enum (Debug/Trace); split `Rig::run()` into a default-debug wrapper + `run_at_level(level, args)`. Determined from source that only `the_trace_logs_download_records_agree_with_the_servers_request_log` reads chunk-level log records — that one opts into trace; other log assertions are debug/info. Made log read-back lazy via `Run.log_path` + `OnceLock<String>` and `.log()`, so runs that never inspect the log skip the read; all log-content assertions kept verbatim, no `snapshot()`/hash assertion touched. repair_machinery 28/28, clippy/fmt clean. DEVIATION: report named ~3 trace tests but only one actually reads chunk-level logs; report's "MD5 of the log" premise also stale (logs in separate TempDir, not in snapshot walk) — reviewer to confirm.
* T15 implemented (commit ac507a7c): only `tests/cli_contract.rs`. Added `real_help()` that renders --help in-process by replaying clap's `DisplayHelp` error with the real binary name as argv[0] (verified byte-identical to binary --help; naive `Args::command().render_help()` was NOT — rejected), cached in a OnceLock; added `usage_rejection()` classifying bad args via `Args::try_parse_from` or `Args::into_ctx`. Converted 5 --help cases + the 17-case `argument_errors_exit_one_not_two` loop to in-process; kept real binary cases for exit-code contract (`help_exits_zero`, one bad-flag case, all post-parse/log-file/data-plane cases). Argument/help spawns ~31 -> ~10 (~68% cut); all 49 cli_contract tests pass, no clippy warnings. No product logging/lock code touched; no GIRPR_TEST_NO_LOG hook needed.
* T16 implemented (commit b51e0395): stale report line numbers; implementer mapped actual parity relations across repair_scan/repair_machinery/repair_writer. Genuine mode-vs-mode/tier-vs-tier relations already each asserted in exactly one place (repair_machinery.rs); repair_writer.rs flagged tests are single-spawn/single-mode with unique assertions — left untouched per "keep both when relations differ". Only real redundancy was in repair_scan.rs: two scanned-estimate-vs-real-download tests each spawned twice (--print-plan + repair) — consolidated to one `--scan --plan-out` run each, comparing per-file est_download_bytes sum to SUMMARY download_bytes; chunk_hits assertions kept. 2 spawns cut (16->14 run_binary calls). repair_scan/machinery/writer 64/64, clippy/fmt clean. No product code.
