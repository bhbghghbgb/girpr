# Checklist state

Committed state for the temporary optimization work. The orchestrator owns this
file; implementers and reviewers must not edit it. Every change to this file is
its own commit (`chore(plan): ...`), separate from code commits.

Status vocabulary for tasks: `todo`, `in_progress`, `pending_review`,
`completed`, `reverted`, `blocked`, `skipped`. Review batches: `pending`,
`in_progress`, `done`. See `ORCHESTRATION.md` for transitions.

## Tasks

| ID | Title | Status | Commits | Batch |
|---|---|---|---|---|
| T01 | C1 - closed stdout is not a panic | todo | | RC-01 |
| T02 | C5 - reject oversized JSON numbers | todo | | RC-01 |
| T03 | C6 - harden meter subtraction | todo | | RC-01 |
| T04 | C7 - use a Windows path key, not Unicode lowercase | todo | | RC-01 |
| T05 | C3 - never publish a partial audio target | todo | | RC-02 |
| T06 | C4 - write `--plan-out` atomically | todo | | RC-02 |
| T07 | C8 - one temp-file naming rule | todo | | RC-02 |
| T08 | P1 - overlap metadata calls | todo | | RC-03 |
| T09 | P2 - fetch manifests concurrently | todo | | RC-03 |
| T10 | P4 - move blocking phases off workers; parallel check | todo | | RC-04 |
| T11 | P5 - size the runtime for `--jobs` | todo | | RC-04 |
| T12 | P6 - verify with one whole-file digest | todo | | RC-05 |
| T13 | P7 - reuse the temp source handle | todo | | RC-05 |
| T14 | P8 - remove the per-piece sink seek | todo | | RC-05 |
| T15 | P10 - share the local chunk map | todo | | RC-05 |
| T16 | P3 - consume `--scan` verdicts in repair | todo | | RC-06 |
| T17 | C2 + C9 - one lock for every mutating run | todo | | RC-07 |
| T18 | A1 - narrow the public surface | todo | | RC-07 |
| T19 | A2 - make `--jobs` the primary flag | todo | | RC-07 |
| T20 | A3 - exercise the post-phase in dry-run | todo | | RC-08 |
| T21 | A4 - one emitter and one progress driver | todo | | RC-08 |
| T22 | A5 - return an exit code and flush logs | todo | | RC-08 |
| T23 | P9 - buffer the file log sink | todo | | RC-09 |
| T24 | P11 - remove purge/scope per-pass allocations | todo | | RC-09 |
| T25 | P13 - fast-path unlimited meters | todo | | RC-09 |
| T26 | P12 - resolve `PathRoot` leaves safely (decision gate) | todo | | RC-09 |
| T27 | D1 - per-file facts live on the file | todo | | RC-10 |
| T28 | D3 - one atomic-write helper | todo | | RC-11 |
| T29 | D2 - one retry/backoff scaffold | todo | | RC-11 |
| T30 | D4 - one rel-path spelling helper | todo | | RC-12 |
| T31 | D5 - one per-range to per-chunk remap | todo | | RC-12 |
| T32 | D7 - reconcile documentation with the code | todo | | RC-13 |
| T33 | D6 - remove test-only production API | todo | | RC-13 |
| T34 | D8 - split the functions doing too much | todo | | RC-14 |
| T35 | D9 - replace parameter bags with structs | todo | | RC-15 |
| T36 | D10 - name every exit code | todo | | RC-15 |
| T37 | B1 - trim the tokio feature set | todo | | RC-16 |
| T38 | B2 - remove the `regex` dependency | todo | | RC-16 |
| T39 | B3 - use `futures-util` directly | todo | | RC-16 |
| T40 | B4 - build the product by default | todo | | RC-16 |
| T41 | B5 - tune the release profile | todo | | RC-16 |
| T42 | B6 - add a workspace lint policy | todo | | RC-16 |
| T43 | B7 - evaluate the md-5 upgrade (conditional) | todo | | RC-16 |
| T44 | T1 - regression tests for the fixed failure paths | todo | | RC-17 |
| T45 | T2 - log phase and per-file durations | todo | | RC-17 |
| T46 | T4 - pin the output-contract ordering | todo | | RC-18 |
| T47 | T3 - split the large integration test | todo | | RC-18 |

## Review batches

| Batch | Tasks | Status | Review commits | Notes |
|---|---|---|---|---|
| RC-01 | T01-T04 | pending | | |
| RC-02 | T05-T07 | pending | | |
| RC-03 | T08-T09 | pending | | |
| RC-04 | T10-T11 | pending | | |
| RC-05 | T12-T15 | pending | | |
| RC-06 | T16 | pending | | |
| RC-07 | T17-T19 | pending | | |
| RC-08 | T20-T22 | pending | | |
| RC-09 | T23-T26 | pending | | |
| RC-10 | T27 | pending | | |
| RC-11 | T28-T29 | pending | | |
| RC-12 | T30-T31 | pending | | |
| RC-13 | T32-T33 | pending | | |
| RC-14 | T34 | pending | | |
| RC-15 | T35-T36 | pending | | |
| RC-16 | T37-T43 | pending | | |
| RC-17 | T44-T45 | pending | | |
| RC-18 | T46-T47 | pending | | |

## Rework

Tasks the reviewer reverted, with the reason and the suggested correction. Pick
these before the follow-up phase.

| ID | Original task | Reason | Status |
|---|---|---|---|
| | | | |

## Follow-ups

Non-blocking items recorded by reviewers or implementers. Do not implement
until the main tasks and rework are done.

| ID | Source | Area | Description | Status |
|---|---|---|---|---|
| | | | | |

## Follow-up batches

Filled in by the follow-up batching session, then executed batch by batch.

| Batch | Follow-ups | Status | Notes |
|---|---|---|---|
| | | | |

## Notes

- `INDEX.md` defines the task map, dependencies, and review checkpoints.
- `completed` means the task and its review batch are done.
- `reverted` means the original work was undone and a rework row was added.
- `skipped` means the task was stale or no longer applicable; record why.
- Task ordering is the report's overall ordering; do not reorder the table.
