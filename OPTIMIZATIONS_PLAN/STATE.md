# STATE — checklist (single source of truth)

Owned by the orchestrator. Committed to version control. This file is the only
thing a replacement orchestrator needs to resume; keep it current. See
`ORCHESTRATION.md` for the process and `README.md` for the file index.

Do not put session-spawning mechanisms, dates/times, session ids, machine data
or a historical change log in this file. Record context, decisions, status and
commit ranges (the functional review handoff data) only.

## Current position

* Next action: spawn implementing session for **T02** (first `todo`), handing its
  task-file path, its Pointers block and the "Implementing session" section of
  `ORCHESTRATION.md`.
* Active batch: none. Phase: implementation (P0 tasks).
* Active checkpoint: CP1 (`open`). Status: T01 `pending_review`, T02 `todo`.
* Blocked on user: no.

## Status vocabulary

* Tasks: `todo` → `in_progress` → `pending_review` → `completed` /
  `completed (fixed)` / `reverted`; or `todo`/`in_progress` → `skipped`.
* Checkpoints: `open` → `in_progress` → `done`.
* Follow-ups: `proposed` → `scheduled` → `in_progress` → `pending_review` →
  `completed` / `removed`. Follow-up batches reuse the per-task / checkpoint
  loop and its separate state commits (ORCHESTRATION.md §5).

## Tasks

Seeded order: P0 safe merges/removes first, then P1 verification-required work.

| ID | Title | Status | Priority | Checkpoint | Notes |
|---|---|---|---|---|---|
| T01 | Merge `cli_contract.rs` helper/layout/parser self-tests | pending_review | P0 | CP1 | impl commit `a0a45fc8`; 15 test functions removed (49→34), no coverage lost per implementer |
| T02 | Remove duplicated fixture-helper meta-tests | todo | P0 | CP1 | |
| T03 | Consolidate remaining low-risk root-unit cases | todo | P0 | CP2 | |
| T04 | Table-drive sync hash dedup/order asserts | todo | P0 | CP2 | |
| T05 | Remove capture no-op test; merge same-function pairs | todo | P0 | CP3 | |
| T06 | sled2redb removes + usage-error parametrization | todo | P0 | CP3 | |
| T07 | `crates/log` + `tempdir` low-risk merges | todo | P0 | CP3 | |
| T08 | `repair_plan` scope table + optional root-integration merges | todo | P0 | CP3 | |
| T09 | Consolidate `run_lock.rs` case clusters | todo | P1 | CP4 | |
| T10 | Consolidate `purge.rs` classifier early tests | todo | P1 | CP4 | |
| T11 | Trim `migration_lock.rs` clear duplicates | todo | P1 | CP5 | |
| T12 | Capture: remove redundant process test; fold assertion-free test | todo | P1 | CP5 | |
| T13 | Cross-binary log-file policy reduction | todo | P1 | CP5 | |

## Review checkpoints

| CP | Tasks | Status | Notes |
|---|---|---|---|
| CP1 | T01–T02 | open | root CLI self-tests + root-unit fixture dedup |
| CP2 | T03–T04 | open | remaining root-unit merges + sync hash table |
| CP3 | T05–T08 | open | capture, sled2redb, small crates, root-integration merges |
| CP4 | T09–T10 | open | `run_lock.rs` + `purge.rs` (verification-required) |
| CP5 | T11–T13 | open | `migration_lock.rs`, capture medium, cross-binary log policy |

## Commit ranges for review

Filled by the orchestrator when a task moves to `pending_review`; consumed by
the review-and-fix session. Keep each range until its checkpoint is `done`.

| Task | Commit range | Review verdict |
|---|---|---|
| T01 | a0a45fc8 (single commit) | — |
| T02 | — | — |
| T03 | — | — |
| T04 | — | — |
| T05 | — | — |
| T06 | — | — |
| T07 | — | — |
| T08 | — | — |
| T09 | — | — |
| T10 | — | — |
| T11 | — | — |
| T12 | — | — |
| T13 | — | — |

## Follow-ups

Reviewer-controlled. Record proposed ideas here; only implement after all tasks
are done and a review session has decided the batches. The first four are seeded
from the report's cross-cutting / optional items (no or minimal test-count
effect) so the final follow-up phase is meaningful.

| ID | Description | Source | Status | Note |
|---|---|---|---|---|
| F01 | Extract the byte-identical `tests/common/mod.rs` shared by `tools/sync` and `tools/sled2redb` into the `girpr-tempdir` crate (report cross-cutting, L226–233). Saves maintenance, not test count. | report | proposed | Test-only refactor; LOW–MEDIUM risk. |
| F02 | Remove the stale doc note in `tools/sync/DESIGN-NOTES.md:907` referencing an ignored test that no longer exists (report cross-cutting, L239–240). Documentation cleanup only. | report | proposed | No test-count effect. |
| F03 | Drop the two redundant full binary conversions in `tools/sled2redb/tests/convert.rs:515` (report cross-cutting, L248–250). Saves runtime, not test count. | report | proposed | LOW risk. |
| F04 | Consider merging the measured-fixture logic-name duplicates in `tools/capture/tests/measured_fixture.rs` (§E.5, L192–199) — only if an explicit size target exists; report recommends KEEP. | report | proposed | MEDIUM risk; ground-truth constants are the value. |
| F05 | If later tasks add more `catch_unwind` assertions in other integration-test binaries, move the `panic_message` helper into `tests/common/mod.rs` to avoid duplicating the panic-downcast logic per test binary. | T01 implementer | proposed | Only if duplication actually arises; test-only. |
| F06 | Audit remaining `#[should_panic]` tests in other root integration-test files for single-helper clusters that could be folded into tables the same way; merge only where multiple cases exercise the same helper. | T01 implementer | proposed | Follows T01's pattern; do not force. |

## Follow-up batches

After all tasks and checkpoints are done, a review session decides how to batch
the surviving follow-ups (and may add/remove). Record the result here; implement
one batch at a time with one implementing session then one review session, using
the same separate state commits as the task/checkpoint loop. Statuses: `open` →
`in_progress` → `pending_review` → `done`.

| Batch | Follow-ups | Status | Commit range | Verdict | Scope / pointers | Notes |
|---|---|---|---|---|---|---|

## Known constraints from the report (persistent)

* The report is a snapshot: line numbers, even whole recommendations, may be
  stale. Implementers always validate against the current code before editing.
* No large block of useless tests: most tests pin a named defect. **Doing
  nothing is a defensible outcome**; do not force changes to hit a number.
* 12 `#[ignore]`d tests are KEEP (skipped by default; removing them would not
  reduce the running count). `tools/zz_probe*.rs` contain `#[test]` but are not
  Cargo test targets — leave them in place.
* "Leave alone": §C.6, §D.6, §E.6, `repair_*` integration invariants, capture
  mutation-driven HTTP/verify/emit suites, sync race/guard suites.
* §E.5 measured-fixture merges are optional (recommend KEEP).
* Do-not-merge pairs in §B.4: `repair_post.rs:512/596`,
  `repair_purge.rs:421/454`, `repair_scope.rs:212/342`,
  `repair_plan.rs:477/503` (complementary, not duplicate).
* Area overlap is counted once: all log-file-policy CLI tests (§A, §B.1 4–5,
  §D.1, §D.3) belong to **T13**. **T04** covers only the sync hash dedup table
  (§D.2). **T12** covers only capture `cli_contract.rs:214` + `closeout.rs:1000`.
  Do not create duplicate work across these tasks.

## Persistent context / decisions

Append-only notes for a replacement orchestrator: stale-report findings,
deliberate deviations from a task file, why a task was skipped/reverted,
checkpoint boundary changes, and anything else needed to resume. Write these as
short, context-only statements — no session ids, dates, machine data or
chronological change log.
