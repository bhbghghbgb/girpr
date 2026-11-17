# STATE — checklist (single source of truth)

Owned by the orchestrator. Committed to version control. This file is the only
thing a replacement orchestrator needs to resume; keep it current. See
`ORCHESTRATION.md` for the process and `README.md` for the file index.

Do not put session-spawning mechanisms, dates/times, session ids, machine data
or a historical change log in this file. Record context, decisions, status and
commit ranges (the functional review handoff data) only.

## Current position

* Next action: wait for the **T07** implementing session to finish, then apply
  its result (commit range / skip) per the per-task loop.
* Active batch: none. Phase: implementation (P0 tasks). CP1 and CP2 reviews done.
* Active checkpoint: CP3 (`open`). Status: T05 `pending_review`, T06 `pending_review`, T07 `in_progress`.
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
| T01 | Merge `cli_contract.rs` helper/layout/parser self-tests | completed | P0 | CP1 | impl `a0a45fc8`; CP1 review confirmed completed. 19 self-tests → 4 tables (49→34), all cases preserved (several assertions strengthened); no product/process/golden code touched. |
| T02 | Remove duplicated fixture-helper meta-tests | completed (fixed) | P0 | CP1 | impl `d6dfb324`, review fix `e4439122`. 7 duplicate meta-tests removed; reviewer confirmed the flagged assertions were only transitively covered and folded them verbatim into `a_broken_fixture_fails_the_case_through_the_shared_decision` (no test function added, count reduction preserved). |
| T03 | Consolidate remaining low-risk root-unit cases | completed (fixed) | P0 | CP2 | commits `5e8d593f..185cfa4c`, review fix `cd80083b`. All 3 bullets done (−9 tests); the four distinct salvage_order cases kept separate. Review fix derived `live_count` in the reuse table loop instead of storing a redundant column, retaining the all-dead explicit check. No product logic changed. |
| T04 | Table-drive sync hash dedup/order asserts | completed | P0 | CP2 | impl `6f6a47af` (test-only); CP2 review confirmed completed. Four cases → `dedup_keeps_first_seen_order` table (41→38), 8 assertions preserved, parse-vs-parse_any via fn pointer. Review flags judged not worth changing (nested grouping keeps one comment per regression; named struct unnecessary for 4 fixed rows). |
| T05 | Remove capture no-op test; merge same-function pairs | pending_review | P0 | CP3 | commits `0e8496e9..30e15ae4` (test-only). No-op removed + 4 pairs merged (670→665 tests, −5); manifest/proto.rs equality assertion kept, only misleading doc dropped. REVIEW FLAG: the 5th pair `config/tests.rs:411/483` was SKIPPED per guardrail - 411 is end-to-end rejection across bad tags, 483 checks the 'device' diagnostic content via tag_shape_problem; implementer says a reviewer could fold 483 into 411 by adding a `.contains("device")` assertion. Reviewer must confirm skip or fix. |
| T06 | sled2redb removes + usage-error parametrization | pending_review | P0 | CP3 | impl `e72f5859` (test-only, tools/sled2redb/tests/convert.rs). Removed subsumed help + temp-name tests, parametrized 4 usage-error cases (integration 16→11, −5). Strict help pin, src/convert.rs:410, and convert.rs:515 conversions (F03) left intact. No flags. |
| T07 | `crates/log` + `tempdir` low-risk merges | in_progress | P0 | CP3 | |
| T08 | `repair_plan` scope table + optional root-integration merges | todo | P0 | CP3 | |
| T09 | Consolidate `run_lock.rs` case clusters | todo | P1 | CP4 | |
| T10 | Consolidate `purge.rs` classifier early tests | todo | P1 | CP4 | |
| T11 | Trim `migration_lock.rs` clear duplicates | todo | P1 | CP5 | |
| T12 | Capture: remove redundant process test; fold assertion-free test | todo | P1 | CP5 | |
| T13 | Cross-binary log-file policy reduction | todo | P1 | CP5 | |

## Review checkpoints

| CP | Tasks | Status | Notes |
|---|---|---|---|
| CP1 | T01–T02 | done | root CLI self-tests + root-unit fixture dedup; T01 completed, T02 completed (fixed e4439122) |
| CP2 | T03–T04 | done | remaining root-unit merges + sync hash table; T03 completed (fixed cd80083b), T04 completed |
| CP3 | T05–T08 | open | capture, sled2redb, small crates, root-integration merges |
| CP4 | T09–T10 | open | `run_lock.rs` + `purge.rs` (verification-required) |
| CP5 | T11–T13 | open | `migration_lock.rs`, capture medium, cross-binary log policy |

## Commit ranges for review

Filled by the orchestrator when a task moves to `pending_review`; consumed by
the review-and-fix session. Keep each range until its checkpoint is `done`.

| Task | Commit range | Review verdict |
|---|---|---|
| T01 | a0a45fc8 (single commit) | completed |
| T02 | d6dfb324 + fix e4439122 | completed (fixed) |
| T03 | 5e8d593f..185cfa4c + fix cd80083b | completed (fixed) |
| T04 | 6f6a47af (single commit) | completed |
| T05 | 0e8496e9..30e15ae4 (2 commits) | — |
| T06 | e72f5859 (single commit) | — |
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
| F04 | ~~Consider merging the measured-fixture logic-name duplicates in `tools/capture/tests/measured_fixture.rs` (§E.5, L192–199) — only if an explicit size target exists; report recommends KEEP.~~ | report | removed | CP1 reviewer pruned: conditional on a non-existent size target, MEDIUM risk, report recommends KEEP; idea stays in report §E.5. |
| F05 | If later tasks add more `catch_unwind` assertions in other integration-test binaries, move the `panic_message` helper into `tests/common/mod.rs` to avoid duplicating the panic-downcast logic per test binary. | T01 implementer | proposed | Only if duplication actually arises; test-only. |
| F06 | Audit remaining `#[should_panic]` tests in other root integration-test files for single-helper clusters that could be folded into tables the same way; merge only where multiple cases exercise the same helper. | T01 implementer | proposed | CP1 reviewer kept; skip `run_lock.rs` clusters (T09 folds those). Do not force. |
| F07 | ~~Fold `the_runtime_keeps_a_worker_beyond_every_file_window` (main.rs) into T03's `worker_threads_arithmetic` table using exact-value rows; saves one test.~~ | T03 implementer | removed | CP2 reviewer pruned: saves only 1 test while replacing a general `> jobs` headroom invariant (already covered by exact-value + saturation rows) with exact examples; runaway risk. |
| F08 | If more sled2redb usage-error shapes with distinct assertions are added later, replace the parametrized test's `is_version` boolean with a per-row assertion closure so special-case checks stay data-driven rather than becoming a widening `if` chain. | T06 implementer | proposed | Conditional on future additions; test-only. Reviewer may prune as speculative. |

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
