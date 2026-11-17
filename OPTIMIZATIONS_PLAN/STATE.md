# STATE — checklist (single source of truth)

Owned by the orchestrator. Committed to version control. This file is the only
thing a replacement orchestrator needs to resume; keep it current. See
`ORCHESTRATION.md` for the process and `README.md` for the file index.

Do not put session-spawning mechanisms, dates/times, session ids, machine data
or historical change logs in this file. Record context, decisions, status and
commit ranges only.

## Current position

* Next action: wait for implementing session on T01, then advance per the loop.
* Active checkpoint: CP1 (`open`); T01 is `in_progress`.
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
| T01 | Lower heartbeat fixture via test hook | in_progress | CP1 | |
| T02 | Default `run_binary_raw` to memory cache | todo | CP1 | |
| T03 | Audit `opts()` → memory helpers | todo | CP1 | |
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
| — | — | — |

## Follow-ups

Reviewer-controlled. Record proposed ideas here; only implement after all tasks
are done and a review session has decided the batches.

| ID | Description | Source | Status | Note |
|---|---|---|---|---|
| — | — | — | — | — |

## Context / decisions

Append-only notes for a replacement orchestrator: stale-report findings,
deliberate deviations from task files, why a task was skipped/reverted,
checkpoint boundary changes, and anything else needed to resume.

* (none yet)
