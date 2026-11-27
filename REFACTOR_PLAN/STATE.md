# Refactor checklist state

Single source of truth for orchestration. Updated by the orchestrator in commits
separate from implementation commits. See `ORCHESTRATION.md` for the protocol and
`README.md` for ordering, checkpoints and coverage.

Initial state: no task started, no reviews done, no follow-ups.

## Tasks

Status values: `todo`, `in progress`, `pending review`, `completed`, `reverted`, `skipped`.

| ID | Task file | Status | Commits | Notes |
| --- | --- | --- | --- | --- |
| T01 | `tasks/T01-chunk-cost-arithmetic.md` | todo | — | |
| T02 | `tasks/T02-chunk-layout.md` | todo | — | depends on T01 |
| T03 | `tasks/T03-xxh64-hex-boundary.md` | todo | — | |
| T04 | `tasks/T04-digest-internals.md` | todo | — | depends on T03 |
| T05 | `tasks/T05-retryable-status.md` | todo | — | |
| T06 | `tasks/T06-async-retry-driver.md` | todo | — | depends on T05 |
| T07 | `tasks/T07-bounded-join.md` | todo | — | |
| T08 | `tasks/T08-ledger-cas-helpers.md` | todo | — | |
| T09 | `tasks/T09-whole-file-verify.md` | todo | — | |
| T10 | `tasks/T10-bounded-byte-io.md` | todo | — | |
| T11 | `tasks/T11-clear-readonly.md` | todo | — | |
| T12 | `tasks/T12-request-url.md` | todo | — | |
| T13 | `tasks/T13-line-and-bom-readers.md` | todo | — | |
| T14 | `tasks/T14-claim-publish.md` | todo | — | |
| T15 | `tasks/T15-fs-path-helpers.md` | todo | — | |
| T16 | `tasks/T16-global-cli-and-format.md` | todo | — | before T18 |
| T17 | `tasks/T17-subscriber-and-log-internals.md` | todo | — | |
| T18 | `tasks/T18-report-core.md` | todo | — | after T16 |
| T19 | `tasks/T19-test-fixture-helpers.md` | todo | — | |
| T20 | `tasks/T20-test-http-servers.md` | todo | — | |
| T21 | `tasks/T21-hyp-wire.md` | todo | — | |
| T22 | `tasks/T22-capture-emit-stores.md` | todo | — | |
| T23 | `tasks/T23-girsync-heartbeat.md` | todo | — | |
| T24 | `tasks/T24-workspace-inheritance.md` | todo | — | |

## Reviews

| Checkpoint | Reviews | Status | Outcome |
| --- | --- | --- | --- |
| CP1 | T01–T04 | open | |
| CP2 | T05–T07 | open | |
| CP3 | T08–T11 | open | |
| CP4 | T12–T14 | open | |
| CP5 | T15–T16 | open | |
| CP6 | T17–T18 | open | |
| CP7 | T19–T20 | open | |
| CP8 | T21–T22 | open | |
| CP9 | T23–T24 | open | |

## Follow-ups

Status values: `proposed`, `accepted`, `declined`, `done`.

| ID | Source task | Recommendation | Status |
| --- | --- | --- | --- |
| — | — | none recorded | — |

## Persistent context

- Report: `docs/09-shared-code-refactor-findings.md` (temporary, deleted at finish).
- Every finding is mapped to a task in `README.md`; non-candidates are listed there.
- LF-14's cross-reference to X14 is a pointer, not a grouping; LF-14 is in T17.
- Findings are static readings; implementers must validate against current code.
- These plan files and the report must not be referenced from code or permanent docs.
- No task is started; no commits have been made for this refactor.
