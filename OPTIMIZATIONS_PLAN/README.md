# OPTIMIZATIONS_PLAN — index & how-to

Temporary planning workspace for reducing test runtime across the workspace.
This directory is **not** shipped code: once the work is complete, delete this
whole directory and `docs/08-test-runtime-optimization.md` in a final commit, and
make sure no source file refers to either of them.

This directory is the hand-off package for an orchestrator that may be replaced
between steps. A replacement starts from the files here, not from any prior
conversation.

## Files

| File | Role |
|---|---|
| `ORCHESTRATION.md` | The process: roles, the per-task loop, review checkpoints, follow-ups, guardrails, resumability and completion. **Read this first.** |
| `STATE.md` | The committed checklist and single source of truth. The orchestrator owns and updates it; task files and the report are read-only. |
| `tasks/Tnn-*.md` | One implementation plan per task. Each is read-only and may describe several commits. Hand exactly one task file to exactly one implementing session. |

Goal, measurements and the *why* live in the read-only report:
`docs/08-test-runtime-optimization.md` (referenced below as "the report"). The
order of the tasks is the report's **prioritized action list, section 8**
(P0 → P1 → P2), as seeded in `STATE.md`; do not reorder silently — record any
change in `STATE.md`.

## Task roster (recommended order — report §8)

| ID | Task | Crate | Priority | Checkpoint | Report |
|---|---|---|---|---|---|
| T01 | Lower heartbeat fixture via test hook | girsync | P0 | CP1 | §6.1 A1 |
| T02 | Default `run_binary_raw` to memory cache | girsync | P0 | CP1 | §6.1 B4 |
| T03 | Audit `opts()` → memory helpers in semantic suites | girsync | P0 | CP1 | §6.1 B5/C7 |
| T04 | Shrink rw_threads 96-file fixture | girsync | P0 | CP1 | §6.1 A3 |
| T05 | Shrink `as_done` 11 MB fixture | girsync | P0 | CP1 | §6.1 A2 |
| T06 | Consolidate gircap default captures + shared bundle | gircap | P0 | CP2 | §6.2 1 |
| T07 | Hand-build minimal bundles for "delete a doc" tests | gircap | P0 | CP2 | §6.2 2 |
| T08 | Cache gircap byte tables + shrink oversized chunk | gircap | P0 | CP2 | §6.2 2 |
| T09 | Cut sled2redb argument-spelling test 8 → 2–3 | sled2redb | P1 | CP3 | §6.4 1 |
| T10 | Drop sled2redb read-only flush + redundant reopens | sled2redb | P1 | CP3 | §6.4 2–3 |
| T11 | Replace girsync publish fixed sleeps with signals | girsync | P1 | CP3 | §6.1 C6 |
| T12 | Replace girpr 300 ms panic-ordering sleep | girpr | P1 | CP3 | §6.3 4 |
| T13 | Add gircap zero-backoff retry hook | gircap | P1 | CP3 | §6.2 4 |
| T14 | Reduce girpr trace-level logging | girpr | P1 | CP4 | §6.3 1 |
| T15 | Remove girpr `--help`/argument subprocess spawns | girpr | P1 | CP4 | §6.3 2 |
| T16 | Consolidate girpr multi-run parity tests | girpr | P1 | CP4 | §6.3 3 |
| T17 | Merge redundant girsync dry-run/hard-link groups | girsync | P1 | CP4 | §6.1 D8–D9 |
| T18 | Merge redundant gircap read/verify test groups | gircap | P1 | CP4 | §6.2 3/5 |
| T19 | Tidy ignored tests (bundle reuse, peak_heap, stale case) | girpr/girsync | P2 | CP5 | §6.3 5, §7 |
| T20 | Guardrail audit: hooks intact, no temp→`Z:` redirect | all | P2 | CP5 | §8 row 10, §9 |

## How a task is handed off

Give an implementing session **only**:

1. its task file path (`OPTIMIZATIONS_PLAN/tasks/Tnn-*.md`);
2. the report line pointers in that file's **Pointers** block;
3. the "Implementing session" section of `ORCHESTRATION.md`.

Do **not** paste the plan or the report into the session. The session reads the
pointers, validates them against the current code, and implements. See
`ORCHESTRATION.md` for the full loop, the separate `pending review` commit, the
review batches and the follow-up rules.
