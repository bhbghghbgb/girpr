# Orchestration plan — test-suite reduction

This is the process document for the work described in `tasks/Tnn-*.md` and
measured in `docs/08-test-suite-reduction-audit.md` ("the report"). It is written
to be resumed by an orchestrator with **no prior conversation history**.

## 0. Lifecycle and hard rules

* This whole directory and the report are temporary. When everything is done,
  delete `OPTIMIZATIONS_PLAN/` and `docs/08-test-suite-reduction-audit.md` in a
  final commit. Implementation must never leave a reference to them behind.
* The orchestration session may be replaced at any time. A replacement starts
  from `STATE.md` plus this document alone. **Record every decision, discovery
  and piece of context in `STATE.md`**; never rely on what was said in chat.
* No session-spawning mechanism, schedule, wall-clock time, session id, machine
  identity or historical change log is prescribed here. The user supplies the
  way to spawn sessions. Pass pointers, not prose.
* Only task files and the report are read-only. `STATE.md` is the single source
  of truth and is owned by the orchestrator and committed to version control.
* The goal is a smaller test count **with no coverage lost**. The report states
  that doing nothing is a defensible outcome and that no change justifies
  large-scale deletion. Implementers must not force a bad change to hit a
  number; skipping a stale or risky task is a valid result.

## 1. Sources of truth and who owns what

| Thing | Read-only? | Owner / updater |
|---|---|---|
| `docs/08-test-suite-reduction-audit.md` | yes | nobody (reference; deleted at the end) |
| `OPTIMIZATIONS_PLAN/tasks/Tnn-*.md` | yes | nobody (implementation plans) |
| `OPTIMIZATIONS_PLAN/README.md`, `ORCHESTRATION.md` | yes | nobody (guidance) |
| `OPTIMIZATIONS_PLAN/STATE.md` | no | orchestrator (committed after each status change) |

Task order is seeded in `STATE.md` (P0 safe merges/removes first, then P1
verification-required work). Reordering is allowed only if recorded in `STATE.md`
with a reason.

## 2. Roles

### Orchestrator
* Reads `STATE.md` to learn the current position; never re-derives it from chat.
* Picks the next task **in order**, spawns one implementing session for it, and
  waits for that session to finish before doing anything else (strictly one at
  a time).
* Maintains `STATE.md`, makes the status commits, defines/adjusts review
  checkpoints, and manages follow-ups (including pruning runaway ones).
* May spawn review sessions at any point to evaluate or reduce follow-ups when
  runaway is suspected. **It does not research code itself** — it delegates
  research to a review session.
* Hands sessions pointers only (task file path + report line ranges + the
  relevant section of this document). It does not recite the plan or report.

### Implementing session (one task at a time)
* Receives exactly one task file and the report pointers in its Pointers block.
* Validates the task against the current code first. The report is a snapshot;
  line numbers and even whole recommendations may be stale. If the task is
  stale, contradicted by the code, or trouble arises, it **stops and marks the
  task skipped** with a note — it does not force a bad change.
* Implements toward the goal; the plan steps and commit breakdown are guidance.
  It may improvise if that reaches the same optimization better. It may also
  find the work already done and report a skip.
* Only test/harness behavior changes unless the task explicitly flags product
  code; where product code is involved, validate before changing it.
* Reports any follow-up ideas in its final message (as suggestions only — it
  does not implement them). The reviewer decides if they are worth keeping;
  the orchestrator records the accepted ones in `STATE.md`.

### Review-and-fix session (one checkpoint batch at a time)
* Receives the report pointers, the relevant task-file pointers, and **the
  commit ranges produced for that batch** (recorded in `STATE.md`).
* Checks each task against its goal, the current code and the batch's commits.
  It may **fix** a weak task or **revert** a bad one. It never implements
  non-blocking follow-ups; it only recommends them.
* Returns, per task: `completed`, `completed (fixed)`, `reverted`, or
  `skipped` (confirming an implementer's skip), plus any follow-up
  recommendations (including removing existing follow-ups to avoid runaway).
* The orchestrator applies those results to `STATE.md` and marks the checkpoint
  `done`.

## 3. Per-task loop (implementation phase)

Repeat until every task in `STATE.md` is terminal (`completed`, `reverted`,
`skipped`).

1. From `STATE.md`, pick the first task with status `todo`, in seeded order.
2. Set it to `in_progress` in `STATE.md` and commit that state change so a
   replacement knows what is underway.
3. Spawn **one** implementing session with only: the task file path, the report
   pointers from its Pointers block, and the "Implementing session" section of
   this document.
4. Wait for it to finish. It either produces implementation commits, or reports
   `skipped` with a reason.
5. If skipped: record `skipped` + reason in `STATE.md` (commit). Record any
   suggested follow-ups as `proposed` (the reviewer evaluates them later).
6. If implemented: **in a separate commit** that only touches `STATE.md`,
   change the task from `in_progress` to `pending_review`, record the commit
   range for review, and append its follow-up suggestions as `proposed`. Do not
   mix this state commit with implementation changes.
7. If the current checkpoint batch is now complete, run the review phase
   (section 4) before starting the next task. Otherwise go to step 1.

Never have two implementing sessions running at once. The next session starts
only after the previous one is done.

## 4. Review checkpoints (review-and-fix phase)

Checkpoints are seeded in `STATE.md` at natural crate/theme boundaries. Each
batch is sized by the amount of work (roughly 3–5 tasks, or fewer when the tasks
are large) so a single review session is not overwhelmed:

| Checkpoint | Tasks | Theme |
|---|---|---|
| CP1 | T01–T02 | root CLI self-tests + root-unit fixture dedup |
| CP2 | T03–T04 | remaining root-unit merges + sync hash table |
| CP3 | T05–T08 | capture, sled2redb, small crates, root-integration merges |
| CP4 | T09–T10 | `run_lock.rs` + `purge.rs` (verification-required) |
| CP5 | T11–T13 | `migration_lock.rs`, capture medium, cross-binary log policy |

The orchestrator may move a boundary or add a checkpoint if the actual amount
of work demands it; record the change in `STATE.md`.

For each checkpoint whose tasks are all `pending_review` (or resolved skips):

1. Set the checkpoint to `in_progress` in `STATE.md` (commit).
2. Spawn one review-and-fix session with the relevant report pointers, the
   batch's task-file pointers, and the commit ranges recorded in `STATE.md`.
3. Apply its verdicts: `completed` / `completed (fixed)` for good or fixed
   tasks, `reverted` for bad ones, keep `skipped` where confirmed. Record its
   follow-up recommendations (`proposed`, or `removed` for existing ones).
4. Mark the checkpoint `done` in `STATE.md` (commit).

Do not start new implementation tasks inside a checkpoint while its review is
in progress.

## 5. Follow-ups and runaway control

* A follow-up is a possible optimization noticed during implementation or
  review, or seeded from the report's no/minimal-count-effect items, but out of
  the current task's scope. It is recorded in `STATE.md` with
  `status: proposed | scheduled | in_progress | pending_review | completed |
  removed` and a short note.
* The reviewer evaluates proposed follow-ups and may add new ones or **remove
  existing ones** to prevent runaway growth. The orchestrator only records what
  the reviewer decides; it does not invent follow-ups by researching code.
* While tasks remain, follow-ups stay `proposed`/`scheduled` and are not
  implemented.
* If runaway is suspected at any time, spawn a review session (not a research
  session by the orchestrator) to evaluate and prune the follow-up list.
* **After all tasks are terminal and all checkpoints `done`**, handle follow-ups:
  1. Spawn a review session first to decide how to batch the surviving
     follow-ups. It may create new follow-ups and delete existing ones.
  2. Record the resulting batches in `STATE.md`.
  3. For each batch, spawn one implementing session, then a review session,
     using the same per-task / checkpoint loop and the same state commits.
  4. Repeat until no actionable follow-ups remain.

## 6. Resumability

On (re)start: read `README.md`, this file and `STATE.md`. Continue from the
first non-terminal task and the first non-`done` checkpoint. Any `in_progress`
task or `in_progress`/`pending_review` checkpoint is the resume point; the
commit ranges recorded in `STATE.md` tell a review session what to look at.
Everything needed to continue must already be written in `STATE.md`.

## 7. Completion

When every task is terminal, every checkpoint is `done`, and no actionable
follow-ups remain, make a final commit that deletes `OPTIMIZATIONS_PLAN/` and
`docs/08-test-suite-reduction-audit.md`. Verify that no implementation file
contains a path or reference to either. Do not leave the plan, state or report
files in the tree.
