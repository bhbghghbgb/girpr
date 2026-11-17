# STATE — checklist (single source of truth)

Owned by the orchestrator. Committed to version control. This file is the only
thing a replacement orchestrator needs to resume; keep it current. See
`ORCHESTRATION.md` for the process and `README.md` for the file index.

Do not put session-spawning mechanisms, dates/times, session ids, machine data
or historical change logs in this file. Record context, decisions, status and
commit ranges only.

## Current position

* Next action: wait for implementing session on T08; after it finishes, CP2 is complete — run the CP2 review phase.
* Active checkpoint: CP2 (`open`); T06, T07 `pending_review`, T08 `in_progress`.
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
| T06 | Consolidate gircap captures + shared bundle | pending_review | CP2 | 5 commits be7314ff..9096e118; see review range |
| T07 | Hand-build minimal bundles for delete-a-doc | pending_review | CP2 | 3 commits 7e4ca155..81988899; see review range |
| T08 | Cache gircap byte tables + shrink chunk | in_progress | CP2 | |
| T09 | Cut sled2redb spelling test 8 → 2–3 | todo | CP3 | |
| T10 | Drop sled2redb flush + redundant reopens | todo | CP3 | |
| T11 | Replace girsync publish fixed sleeps | todo | CP3 | |
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
| CP2 | T06–T08 | open | gircap capture consolidation & fixtures |
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
| T06 | `575c7ed0..9096e118` | — |
| T07 | `a1de5165..81988899` | — |

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
| F10 | Add the gircap binary's fingerprint to the shared-bundle key so a product-code change to capture output also invalidates the cached bundle (today only the test-side world source does). | T06 implementer | proposed | Reviewer decides. |
| F11 | Unit test asserting the shared-bundle key is stable across repeated calls and changes when `common/mod.rs` changes, guarding the invalidation contract. | T06 implementer | proposed | Reviewer decides. |
| F12 | When T07 lands, confirm the two schema key-deletion tests don't regress to fresh captures (they already use the shared bundle as copy source). | T06 implementer | proposed | Verification item for T07; reviewer decides. |
| F13 | Split `the_bundle_has_the_shape_the_schema_documents` (layout) and `admission_is_cheapest_first...` / `audio_candidates_are_attributed...` (matrix) so document reads can move onto the shared fixture; summary assertions stay. | T06 implementer | proposed | Reviewer decides. |
| F14 | Consolidate the new `minimal_bundle` test helper with the private `write_minimal_bundle` in `src/verify/tests.rs:380` into one shared test-support target to avoid drift (integration tests cannot reach `#[cfg(test)]` lib code). | T07 implementer | proposed | Reviewer decides. |

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
