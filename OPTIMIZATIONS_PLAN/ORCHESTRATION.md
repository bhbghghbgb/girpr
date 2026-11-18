# Orchestration plan — test-scale reduction

This is the process document for the work planned in `tasks/Tnn-*.md` and
measured in `docs/09-test-scale-reduction-research.md` ("the report"). It is
written to be resumed by an orchestrator with **no prior conversation history**.

## 0. Lifecycle and hard rules

* This whole directory and the report are temporary. When everything is done,
  delete `OPTIMIZATIONS_PLAN/` and `docs/09-test-scale-reduction-research.md` in
  a final commit. Implementation must never leave a reference to them behind.
* The orchestration session may be replaced at any time. A replacement starts
  from `STATE.md` plus this document alone. **Record every decision, discovery
  and piece of context in `STATE.md`**; never rely on what was said in chat.
* No session-spawning mechanism, schedule, wall-clock time, session id, machine
  identity or historical change log is prescribed here. The user supplies the
  way to spawn sessions. Give sessions pointers, not prose.
* The report, `README.md`, this file and the task files are read-only. `STATE.md`
  is the single source of truth and is owned by the orchestrator and committed
  to version control after every status change.
* The goal is to reduce test **scale** (fewer functions, fixtures, files, spawns,
  payloads; downshifting layers; scale-free invariants) with no coverage lost.
  An optimization need not show a runtime win. The report says doing nothing is
  defensible for many low-risk items; implementers must not force a bad change
  to hit a number. Skipping a stale or risky task is a valid result.
* Production code changes are allowed where a task calls for a seam; they are
  not automatically in scope for merge-only tasks.

## 1. Sources of truth and who owns what

| Thing | Read-only? | Owner / updater |
|---|---|---|
| `docs/09-test-scale-reduction-research.md` | yes | nobody (reference; deleted at the end) |
| `OPTIMIZATIONS_PLAN/tasks/Tnn-*.md` | yes | nobody (implementation plans) |
| `OPTIMIZATIONS_PLAN/README.md`, `ORCHESTRATION.md` | yes | nobody (guidance) |
| `OPTIMIZATIONS_PLAN/STATE.md` | no | orchestrator (committed after each status change) |

Task order and checkpoint boundaries are seeded in `STATE.md`. Reordering or
moving a boundary is allowed only if recorded in `STATE.md` with a reason.

## 2. Roles

### Orchestrator
* Reads `STATE.md` to learn the current position; never re-derives it from chat.
* Picks the next task in seeded order and spawns **one** implementing session
  for it, waiting for that session to finish before doing anything else
  (strictly one at a time).
* Maintains `STATE.md`, makes the status commits, defines/adjusts review
  checkpoints and manages follow-ups (including pruning runaway ones).
* Hands sessions **pointers only**: the task file path, the report line ranges
  from that file's Pointers block, and the relevant role section of this
  document. For review sessions also pass the commit ranges from `STATE.md`.
  It does not recite or summarize the plan or the report, and it does not
  research code itself — research is delegated to the sessions it spawns.
* May spawn review sessions at any time to evaluate or reduce follow-ups when
  runaway is suspected; it does not do that research itself.

### Implementing session (one task at a time)
* Receives exactly one task file, the report pointers in its Pointers block and
  the "Implementing session" section of this document. It validates those
  pointers against the current code first: the report is a snapshot and line
  numbers, counts and even whole recommendations may be stale.
* If the task is stale, contradicted by the code, already done, or trouble
  arises, it **stops and reports `skipped` with a reason** — it does not force a
  bad change. It may improvise where that reaches the same optimization better;
  the plan steps and commit breakdown are guidance, not a contract.
* Only test/harness behavior changes unless the task explicitly flags product
  code; where product code is involved, validate before changing it.
* Reports follow-up ideas in its final message as **suggestions only** — it does
  not implement them. The reviewer decides if they are worth keeping; the
  orchestrator records the accepted ones in `STATE.md`.

### Review-and-fix session (one checkpoint batch at a time)
* Receives the report pointers, the relevant task-file pointers and **the commit
  ranges produced for that batch** (recorded in `STATE.md`).
* Checks each task against its goal, the current code and the batch's commits.
  It may **fix** a weak task or **revert** a bad one. It never implements
  non-blocking follow-ups; it only recommends them.
* Returns, per task: `completed`, `completed (fixed)`, `reverted`, or `skipped`
  (confirming an implementer's skip), plus follow-up recommendations —
  including removing existing follow-ups to prevent runaway.
* The orchestrator applies those verdicts to `STATE.md` and marks the checkpoint
  `done`.

### Follow-up batching review session
* Spawned before the follow-up implementation phase and at any time runaway is
  suspected. It evaluates the surviving follow-ups, decides how to batch them,
  may create new follow-ups and may delete existing ones to avoid runaway. It
  does not implement them; the orchestrator records its decisions in `STATE.md`.

## 3. Per-task loop (implementation phase)

Repeat until every task in `STATE.md` is terminal (`completed`, `completed
(fixed)`, `reverted`, `skipped`).

1. From `STATE.md`, pick the first task with status `todo`, in seeded order.
2. Set it to `in_progress` in `STATE.md` and commit that state change so a
   replacement knows what is underway.
3. Spawn **one** implementing session with only: the task file path, the report
   pointers from its Pointers block, the relevant section of this document and
   (when relevant) the constraints from the report's "Constraints to preserve".
4. Wait for it to finish. It either produces implementation commits, or reports
   `skipped` with a reason.
5. If skipped: record `skipped` + reason in `STATE.md` (commit). Record any
   suggested follow-ups as `proposed` for later review.
6. If implemented: **in a separate commit touching only `STATE.md`**, change the
   task from `in_progress` to `pending_review`, record the commit range for
   review and append its follow-up suggestions as `proposed`. Do not mix this
   state commit with implementation changes.
7. If the current checkpoint batch is now complete, run the review phase
   (section 4) before starting the next task. Otherwise go to step 1.

Never have two implementing sessions running at once. The next session starts
only after the previous one is done.

## 4. Review checkpoints (review-and-fix phase)

Checkpoints are seeded in `STATE.md` at natural crate/theme boundaries and
sized by amount of work — roughly 3–5 light tasks, fewer when tasks are large,
so a single review session is not overwhelmed. Large tasks (e.g. T16) sit alone.

| CP | Tasks | Theme |
|---|---|---|
| CP1 | T01–T04 | P0 root unit + small crates + sled2redb low-risk |
| CP2 | T05–T08 | P0 capture + sync small findings |
| CP3 | T09–T10 | `repair_machinery` + `repair_plan` downshifts |
| CP4 | T11–T12 | root-unit production seams (MD5, DiskPlan) |
| CP5 | T13–T14 | capture mock merge + `repair_purge` merges |
| CP6 | T15 | sync heartbeat parameterization |
| CP7 | T16 | sync record sink + `run_captured` |
| CP8 | T17–T18 | sync logging + `TestHooks` seams |
| CP9 | T19 | sync `CompareOutcome` pipeline dedup |
| CP10 | T20–T21 | `repair_scan` downshift + `peak_heap` invariant |
| CP11 | T22–T23 | sled2redb production seams |

The orchestrator may move a boundary if actual work demands it; record it in
`STATE.md`. For each checkpoint whose tasks are all `pending_review` (or
resolved skips):

1. Set the checkpoint to `in_progress` in `STATE.md` (commit).
2. Spawn one review-and-fix session with the relevant report pointers, the
   batch's task-file pointers and the commit ranges in `STATE.md`.
3. Apply its verdicts: `completed` / `completed (fixed)` for good or fixed
   tasks, `reverted` for bad ones, keep `skipped` where confirmed. Record its
   follow-up recommendations (`proposed`, or `removed` for existing ones).
4. Mark the checkpoint `done` in `STATE.md` (commit).

Do not start new implementation tasks inside a checkpoint while its review is
in progress.

## 5. Follow-ups and runaway control

* A follow-up is a possible optimization noticed during implementation or
  review, or seeded from the report's no/minimal-count-effect items, but out of
  the current task's scope. It is recorded in `STATE.md` with a status and short
  note. The report's `B6`, `D8` and E3's optional `lazy/compare.rs` table-izing
  are seeded there from the start.
* The reviewer evaluates proposed follow-ups and may add new ones or **remove
  existing ones** to prevent runaway growth. The orchestrator only records what
  the reviewer decides; it does not invent follow-ups by researching code.
* While tasks remain, follow-ups stay `proposed`/`scheduled` and are not
  implemented.
* If runaway is suspected at any time, spawn a review session (not a research
  session by the orchestrator) to evaluate and prune the follow-up list.
* **After all tasks are terminal and all checkpoints `done`**, handle follow-ups:
  1. Spawn a follow-up batching review session **first** to decide how to batch
     the surviving follow-ups. It may create new follow-ups and delete existing
     ones; the orchestrator records the resulting batches in `STATE.md`.
  2. For each batch, spawn one implementing session, then one review session,
     using the same per-task / checkpoint loop and the same separate state
     commits. Record each batch's status, commit range and verdict.
  3. Repeat until no actionable follow-ups remain.

## 6. Resumability

On (re)start: read `README.md`, this file and `STATE.md`. Continue from the
first non-terminal task and the first non-`done` checkpoint. Any `in_progress`
task or `in_progress`/`pending_review` checkpoint is the resume point; the
commit ranges in `STATE.md` tell a review session what to look at. Everything
needed to continue must already be written in `STATE.md`.

## 7. Completion

When every task is terminal, every checkpoint and follow-up batch is `done`,
and no actionable follow-ups remain, make a final commit deleting
`OPTIMIZATIONS_PLAN/` and `docs/09-test-scale-reduction-research.md`. Verify no
implementation file contains a path or reference to either. Do not leave the
plan, state or report files in the tree.
