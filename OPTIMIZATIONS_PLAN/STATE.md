# STATE — checklist (single source of truth)

Owned by the orchestrator. Committed to version control. This file is the only
thing a replacement orchestrator needs to resume; keep it current. See
`ORCHESTRATION.md` for the process and `README.md` for the file index.

Do not put session-spawning mechanisms, dates/times, session ids, machine data
or historical change logs in this file. Record context, decisions, status and
commit ranges only.

## Current position

* Next action: pick the next `todo` task (T12) and spawn its implementing session.
* Active checkpoint: CP3 (`open`); T09, T10, T11 `pending_review`.
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
| T09 | Cut sled2redb spelling test 8 → 2–3 | pending_review | CP3 | 37d11fb5; see review range |
| T10 | Drop sled2redb flush + redundant reopens | pending_review | CP3 | 2 commits fea93769..f342d32e; product flush removal + test reopens |
| T11 | Replace girsync publish fixed sleeps | pending_review | CP3 | 2 commits fa3546dd..f7eb3995; see review range |
| T12 | Replace girpr panic-ordering sleep | todo | CP3 | |
| T13 | Add gircap zero-backoff retry hook | todo | CP3 | |
| T14 | Reduce girpr trace-level logging | todo | CP4 | |
| T15 | Remove girpr `--help`/argument spawns | todo | CP4 | |
| T16 | Consolidate girpr multi-run parity tests | todo | CP4 | |
| T17 | Merge girsync dry-run/hard-link groups | todo | CP4 | |
| T18 | Merge redundant gircap read/verify groups | todo | CP4 | |
| T19 | Tidy ignored tests | todo | CP5 | |
| T20 | Guardrail audit (hooks, no temp→`Z:`) | todo | CP5 | |

## Review checkpoints

| CP | Tasks | Status | Notes |
|---|---|---|---|
| CP1 | T01–T05 | done | girsync fixtures & cache backend |
| CP2 | T06–T08 | done | gircap capture consolidation & fixtures |
| CP3 | T09–T13 | open | sled2redb conversions & fixed sleeps |
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
| T09 | `0c7e0b20..37d11fb5` | — |
| T10 | `aa9c1add..f342d32e` | — |
| T11 | `fa0a7687..f7eb3995` | — |

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
| F18 | In-process spelling check relies on the hand-built `the_converter_command()` mirror; if the binary's `Args` derive drifts from the mirror without affecting `--help`, the pin won't catch it. An authoritative in-process parse would expose/move `Args` from `main.rs` into the library (product change). | T09 implementer | proposed | Out of T09 scope; reviewer decides. |
| F19 | Removing the one remaining ~200 ms reopen in `convert_sled_dir_to_redb_file` would require exposing written records or accepting a caller-provided handle in the converter's product API. | T10 implementer | proposed | Recommend leaving it as the contract test; reviewer decides. |
| F20 | Capture actual before/after nextest timing for the sled2redb suite to confirm the reopen savings (report's P1 estimate bundled this with T09). | T10 implementer | proposed | Optional measurement; reviewer decides. |
| F21 | Extend the T11 one-shot gate to carry a "claim created" notification so `wait_for_published` becomes fully event-driven instead of polling 100x10 ms (not dead time, but returns eagerly). | T11 implementer | proposed | Minor; reviewer decides. |
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
