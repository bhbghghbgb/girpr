# STATE — checklist (single source of truth)

Owned by the orchestrator. Committed to version control. This file is the only
thing a replacement orchestrator needs to resume; keep it current. See
`ORCHESTRATION.md` for the process and `README.md` for the file index.

Do not put session-spawning mechanisms, dates/times, session ids, machine data
or historical change logs in this file. Record context, decisions, status and
commit ranges only.

## Current position

* Next action: pick the next `todo` task (T04) and spawn its implementing session.
* Active checkpoint: CP1 (`open`); T01, T02 `pending_review`, T03 `skipped`.
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
| T01 | Lower heartbeat fixture via test hook | pending_review | CP1 | implemented in 833f6d6a; see review range below |
| T02 | Default `run_binary_raw` to memory cache | pending_review | CP1 | implemented in eb6afaf2; see review range below |
| T03 | Audit `opts()` → memory helpers | skipped | CP1 | stale: R1 commits already landed the audit; no changes. See context. |
| T04 | Shrink rw_threads 96-file fixture | todo | CP1 | |
| T05 | Shrink `as_done` 11 MB fixture | todo | CP1 | |
| T06 | Consolidate gircap captures + shared bundle | todo | CP2 | |
| T07 | Hand-build minimal bundles for delete-a-doc | todo | CP2 | |
| T08 | Cache gircap byte tables + shrink chunk | todo | CP2 | |
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
| CP1 | T01–T05 | open | girsync fixtures & cache backend |
| CP2 | T06–T08 | open | gircap capture consolidation & fixtures |
| CP3 | T09–T13 | open | sled2redb conversions & fixed sleeps |
| CP4 | T14–T18 | open | girpr spawns, logging & redundant merges |
| CP5 | T19–T20 | open | ignored tests & final guardrail audit |

## Commit ranges for review

Filled by the orchestrator when a task moves to `pending_review`; consumed by
the review-and-fix session. Keep the range until the checkpoint is `done`.

| Task | Commit range | Review verdict |
|---|---|---|
| T01 | `ae60730d..833f6d6a` | — |
| T02 | `c1e2fb04..eb6afaf2` | — |

## Follow-ups

Reviewer-controlled. Record proposed ideas here; only implement after all tasks
are done and a review session has decided the batches.

| ID | Description | Source | Status | Note |
|---|---|---|---|---|
| F01 | Add a test-only hook for phase A's 100-file progress threshold in `effective/phase_a.rs` (optionally emit on final entry like phase C), then lower the T01 fixture to ~8 files. Product-code change; out of T01 scope. | T01 implementer | proposed | Would reach the report's literal ~99% IO cut; current 101-file fixture already ~20x smaller. |
| F02 | Unit test in `scan.rs` locking `heartbeat_entries() == 2000` with no hook set, to guard the production default. | T01 implementer | proposed | Env-reading test carries mild fragility; implementer left it out. |
| F03 | Named constant (e.g. `FILE_CACHE_ENV`) for the `("GIRSYNC_TEST_CACHE", "file")` opt-out repeated across three tests in `tests/sync.rs`. | T02 implementer | proposed | Minor readability; reviewer decides. |
| F04 | Debug assertion in `run_binary`/`run_binary_raw` rejecting a redundant `GIRSYNC_TEST_CACHE` in `envs`, to keep "one clear default" enforced against regressions. | T02 implementer | proposed | Enforces the helper invariant; reviewer decides. |
| F05 | On next report regeneration, account for already-landed R1 memory-cache work so §6.1 B5/C7 are not re-flagged as outstanding. | T03 implementer | proposed | Report is temporary and deleted at completion; reviewer decides if moot. |
| F06 | One-line note in `common/mod.rs` near `opts()` pointing readers to the `*_mem` variants, to avoid re-introducing file backends in semantic suites. | T03 implementer | proposed | Documentation/readability only; reviewer decides. |

## Context / decisions

Append-only notes for a replacement orchestrator: stale-report findings,
deliberate deviations from task files, why a task was skipped/reverted,
checkpoint boundary changes, and anything else needed to resume.

* T01 deviation (implementer): report suggested an 8-file fixture, but phase A emits its progress event only every 100 kept files (`effective/phase_a.rs:483`, inside the file branch), while phase C emits on its final entry. With one excluded file, the smallest period preserving the phase-A progress assertion is 101; 8 or 100 yielded an empty `phase_a_progress`. Implementer chose 101 (~1,900 fewer file creates; ~7 s -> ~0.25 s) rather than add a second product hook in `phase_a.rs`. Approved follow-ups F01/F02 recorded above; reviewer to confirm.
* T02 note: implementer also flagged the adjacent P0 `opts()` -> `opts_mem()` audit in `tests/common/mod.rs`/semantic suites; that is already task T03 on the roster, so no new follow-up was created. T02 added file-cache opt-outs only where on-disk bytes are asserted; `GIRSYNC_TEST_NO_FSYNC=0` durability hooks untouched.
* T03 skipped (implementer): audit target no longer exists — R1 commits already in the tree (`989b618c`, `1eafe53b`, `0ba1d130`, `8914668b`, `28678736`) implemented the `opts()` -> `*_mem` migration. Memory helpers (`opts_mem`, `pair_mem`, `sync_mem`, `compare_mem`, `update_mem`, `asking_mem`, `recs_of` memory-first lookup) are used throughout semantic suites; all `pair()` calls use `warmed: false`. Remaining `opts()` (File) sites are intentional cache-byte/on-disk assertions or harnesses overriding to Memory in their own `common`. Report's `helpers.rs` is a misdiagnosis (unit tests with no `cmd_*` calls). Full `cargo test -p girsync` passes (19 binaries, 0 failures). No code changed; reviewer to confirm skip.
