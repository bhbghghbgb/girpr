# Orchestration plan

This document describes how the refactor is driven. The orchestrator does not
research code itself; it reads `REFACTOR_PLAN/STATE.md`, dispatches sessions,
and keeps the state file current. Every rule here is independent of any
particular session-spawning tool: the operator supplies the mechanism.

## 1. Source of truth

`REFACTOR_PLAN/STATE.md` is committed and updated by the orchestrator. It holds:

- the task checklist (id, task file, status, commits, notes),
- the follow-up list (id, source task, recommendation, status),
- a persistent-context section (decisions, discovered constraints, skipped
  reasons, checkpoint records),
- review records (which checkpoints are done).

An orchestrator that has lost all conversation history starts by reading
`STATE.md`, `README.md`, and the referenced task/report line ranges. No other
memory is assumed. Everything that must survive a handoff is written into the
state file before the next session is dispatched.

Never record in the plan files or state file: dates/times, session identifiers,
commit hashes, local machine paths, or a log of which session did what. The
state file records **task status and commits by subject/scope**, not history
metadata.

## 2. State vocabulary

Task statuses:

| Status | Meaning |
| --- | --- |
| `todo` | Not started, in index order. |
| `in progress` | An implementing session is working on it. |
| `pending review` | Implementation commits exist; awaiting the checkpoint reviewer. |
| `completed` | Reviewed and kept. |
| `reverted` | Reviewed and undone (record why). |
| `skipped` | Never implemented: stale, already done, or blocked (record why). |

Follow-up statuses: `proposed`, `accepted`, `declined`, `done`.

Review status per checkpoint: `open` or `done`.

## 3. Main loop

Repeat until every task is `completed`, `reverted`, or `skipped` and no follow-up
remains active:

1. **Read state.** Open `STATE.md`. Determine the next task: the first `todo`
   task in index order whose recorded dependencies are resolved.
2. **Brief the implementing session.** Give it only:
   - the path to its task file (`REFACTOR_PLAN/tasks/T##-*.md`),
   - the exact report line ranges named in that task file,
   - the path to `STATE.md` and the rule that it must not edit the state file,
   - the standing session rules below.
   Do **not** paste the report or the whole plan into the brief. Do not recite
   findings; the agent reads the named sections itself.
3. **Wait for the session to finish.** It validates against current code,
   implements, and commits its work. It may propose follow-ups; it does not
   record them itself.
4. **Update state in a separate commit.** If implementation succeeded, set the
   task to `pending review`, record the commit scope/subjects and any proposed
   follow-ups. If the session reported the task stale/already done/blocked, set
   it to `skipped` with the reason and do not create a review item. This state
   change is its own commit, never mixed with implementation commits.
5. **At a checkpoint boundary** (see `README.md`), run section 4 before
   dispatching the next task.
6. **When all tasks are terminal**, run section 5 for follow-ups.

If more than one task is blocked, keep moving to the next `todo` task rather than
waiting on the blocked one; a skipped task is terminal.

## 4. Checkpoint review-and-fix sessions

At each checkpoint, dispatch one review session for the batch since the previous
checkpoint. Brief it with:

- the task files for the batch (by path) and their named report line ranges,
- the commit subjects/scopes recorded in `STATE.md` for the batch,
- the rule that it reviews only this batch.

The reviewer may:

- fix defects in the batch,
- revert a bad task entirely,
- record non-blocking follow-ups as recommendations (it does not implement
  them),
- mark the checkpoint `done` by reporting the outcome to the orchestrator.

The reviewer does not implement follow-ups and does not decide new work outside
the batch. It may note that a task is mis-scoped; the orchestrator records that
as persistent context.

After the reviewer finishes, the orchestrator updates `STATE.md` in its own
commit: each task becomes `completed` or `reverted`, the review record becomes
`done`, and recommended follow-ups are appended with `proposed` status. A
reverted task is terminal; its commits are undone in the review batch.

A checkpoint may be inserted early when `STATE.md` shows unusually large batches
or a growing follow-up list; the orchestrator does not inspect code to decide
this, it only reads state.

## 5. Follow-up treatment

After all tasks are terminal:

1. Dispatch a **batching review session**. Brief it with the entire follow-up
   list and the relevant report/plan line ranges only. It decides which
   follow-ups are worth doing, groups them into batches, may create new
   follow-ups, and may remove existing ones to avoid runaway growth. It does not
   implement anything.
2. The orchestrator records the decisions in `STATE.md` (`accepted`, `declined`,
   removed entries deleted with a one-line reason in persistent context).
3. For each accepted batch, dispatch one implementing session following the
   same rules as section 3, then a review-and-fix session as in section 4.
4. If the follow-up list keeps growing after a review, dispatch another
   batching review session to shrink it. The orchestrator never researches the
   code to make that judgement itself.

Follow-ups are bounded: a follow-up that merely restates a non-candidate in
`README.md` is declined without implementation.

## 6. Standing rules for implementing sessions

These go into every implementing brief:

- The plan is a hypothesis. Validate every line reference against the current
  tree. If the task is stale, already implemented, or blocked, **stop** and
  report instead of guessing; the orchestrator will mark it skipped.
- You may improvise. The objective is the optimization described by the goal,
  not literal reproduction of the proposed commit breakdown. If a different
  boundary is better, take it and explain in your report.
- Keep program policy at call sites: published bytes, `--help` output, exit
  codes, field order, golden URLs, error wording and test seams are contracts.
- Suggest follow-ups instead of implementing them.
- Do not edit `STATE.md` or other orchestration files.
- Do not add comments, tests, or docs that reference these plan files or the
  report; both are deleted when the work finishes.

## 7. Standing rules for review sessions

- Review only the assigned batch and its commits.
- Fix or revert; do not expand scope into other tasks.
- Recommendations are non-blocking follow-ups only.
- Report which tasks are kept, fixed, or reverted so state can be updated.

## 8. Replacement resilience

Because the orchestrator may be replaced at any point, every decision that
affects later work is committed in `STATE.md` before the next session starts:

- why a task was skipped or reverted,
- constraints discovered during implementation that change later tasks,
- dependency changes,
- follow-up accept/decline/removal decisions and their reasons,
- checkpoint outcomes.

A new orchestrator reads the state file and resumes; it does not need the prior
conversation.

## 9. Finishing

When all tasks are terminal and no follow-up remains:

1. Verify no code or permanent documentation references the plan files or the
   report (search for `REFACTOR_PLAN` and `09-shared-code-refactor-findings`).
2. Delete `REFACTOR_PLAN/` and `docs/09-shared-code-refactor-findings.md` in a
   final commit, together with any leftover references found in step 1.
3. Stop orchestration. Implementation work must stand on its own without the
   temporary documents.
