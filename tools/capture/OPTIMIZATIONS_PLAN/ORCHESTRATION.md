# Orchestration plan

This document is for the orchestrator session. The user decides **how**
sessions are spawned; this plan does not assume a tool, command, or API. When
it says "spawn", use whatever mechanism the user supplied. If none was
supplied, ask before starting.

## Roles

- **Orchestrator** (this session): owns `STATE.md`, picks tasks, spawns one
  implementer per task, spawns reviewers at checkpoints, records results.
  It does not implement tasks itself.
- **Implementer**: one task per fresh session. Writes code/tests, commits, and
  reports SHAs. Never edits `STATE.md`.
- **Reviewer**: one batch per fresh session. Reviews, fixes, or reverts; never
  adds scope beyond fixing the batch. Returns verdicts and follow-ups.

## Files the orchestrator reads

Normally only:

- `OPTIMIZATIONS_PLAN/STATE.md` (the state machine);
- `OPTIMIZATIONS_PLAN/INDEX.md` (task map and checkpoints).

It reads a task file only when it needs to verify a return or resolve a
blocker. It **never** pastes the whole report or whole plan into a session
prompt. It passes the task file path and the exact report line ranges from that
task file. The implementer/reviewer reads the report file itself at those
lines.

## State file

`STATE.md` is committed to version control and is the single source of truth.

- Task statuses: `todo`, `in_progress`, `pending_review`, `completed`,
  `reverted`, `blocked`.
- Review batches: `pending`, `in_progress`, `done`.
- Every implementation task has a row with its commit SHAs.
- `completed` is only set after a review batch containing the task is `done`.
- `reverted` is terminal for that attempt. The orchestrator adds a rework entry
  to `## Rework` with the reviewer's reason; rework entries are scheduled like
  tasks.
- Follow-ups from reviews go to `## Follow-ups`. They are not implemented until
  the main task list is done.

## Main loop

1. **Start clean.** Confirm `git status` is clean. If `STATE.md` is not yet
   committed, commit it as `chore(plan): initialize checklist`. Do not start a
   task on a dirty tree.
2. **Pick the next task.** Take the first `todo` task in table order. If there
   is no `todo` task, take the first `## Rework` row whose status is `todo`.
   If a review batch is complete (all its tasks are `pending_review`) and that
   batch is still `pending`, do the review first (step 4). If the next task's
   batch has already been reviewed, that is a state error: stop and fix state.
   Rework is reviewed by the next batch reviewer, or by a targeted reviewer if
   no batch remains.
3. **Implement one task.**
   - Record the current `HEAD` as the task's base.
   - Set the task to `in_progress`, commit that state change separately
     (`chore(plan): start <task-id>`). This keeps the checklist truthful if the
     session dies.
   - Spawn an implementer with the template below.
   - On return: inspect `git log`/`git status` yourself. If there are no
     commits or the tree is dirty, do not accept the task; mark `blocked` with
     the reason and either retry once or ask the user.
   - Record the commit SHAs and set the task to `pending_review`. Commit this
     state change separately (`chore(plan): <task-id> pending review`).
   - Go to step 2. Do not spawn the next implementer until the previous return
     has been recorded.
4. **Review a batch.**
   - Only when all tasks in the batch are `pending_review`.
   - Set the batch to `in_progress`, commit separately.
   - Spawn a reviewer with the batch template. Give it the task ids, their
     commit SHAs, and the report line ranges. Do not give it the orchestrator's
     history or unrelated task files.
   - On return: apply the verdicts to `STATE.md`. For a task the reviewer
     fixed, add the reviewer's commit SHAs. For a reverted task, mark
     `reverted` and add a rework row. Record every non-blocking follow-up in
     `## Follow-ups`. Set the batch to `done`.
   - Commit the state update separately (`chore(plan): review <batch-id>`).
5. **Repeat** until every main task is `completed` (or `reverted` with a
   scheduled rework that is later completed) and every review batch is `done`.

## Review-fix exceptions

- A reviewer may leave a task `pending_review` only if it cannot decide; treat
  that as `blocked`, not as done.
- If a reviewer reverts a task, the orchestrator must not silently drop it.
  Add a `## Rework` row with the task id, the reason, and the reviewer's
  suggested correction, status `todo`. Rework rows are picked before follow-ups
  and are implemented like a normal task (one session, commit, then
  `pending_review`). They join the next review batch, or get a targeted review
  if no batch remains.
- A reviewer's fixes are part of the same batch. They do not need a second
  review unless the reviewer says the fix is risky; if so, schedule a targeted
  re-review of just that task/commit.
- Rework sessions get the original task file plus the reviewer's reason. They
  do not get the reviewer's transcript.

## Follow-up phase

When all main tasks and rework are done:

1. **Batch the follow-ups first.** Spawn an evaluation-only session with
   `## Follow-ups` from `STATE.md`, plus `INDEX.md`. Ask it to group follow-ups
   into small batches by area and risk, and to say which ones are not worth
   doing. It must not edit code. Record the result under
   `## Follow-up batches` in `STATE.md` and commit.
2. Pick one batch, spawn one implementer with the batch entries (same narrow
   context rule), record commits, mark `pending_review`.
3. Review follow-up batches using the same reviewer rules, but the reviewer
   may only fix/revert that batch. Record verdicts and any new follow-ups.
4. Repeat until every follow-up is `completed`, `reverted`, or explicitly
   `dropped` with a reason.

## Completion and cleanup

When nothing is `todo`, `in_progress`, `pending_review`, `blocked`, or
`reverted` without a rework:

1. Run the full relevant test suite in a clean tree. Record the command and
   result in the final state commit message.
2. Check the tree for references to the temporary files:
   `rg -n "OPTIMIZATIONS_(REPORT|PLAN)|P[0-9]-[0-9]{2}|RC-[0-9]"`.
   Remove or rewrite any that leaked into code, tests, comments, or permanent
   docs.
3. Delete `OPTIMIZATIONS_REPORT.md`, `OPTIMIZATIONS_PLAN/` (including
   `STATE.md`), and any temporary notes created during the work.
4. Commit the deletion separately. Confirm the tree is clean.

## Implementer prompt template

Replace `<...>`. Keep everything outside the task slice out of the prompt.

```text
You are implementing one task in the gircap crate.

Repository: <absolute working tree>
Task file: OPTIMIZATIONS_PLAN/tasks/<task-file>.md
Report excerpts: OPTIMIZATIONS_REPORT.md lines <ranges>. Read only those lines.
Read the task file in full, then the code and tests it names. Do not read other
task files, STATE.md, or the rest of the report.

Implement only this task, following its commit plan. Use the repository's
existing style and test patterns. Do not edit OPTIMIZATIONS_PLAN/STATE.md.
Do not reference OPTIMIZATIONS_REPORT.md, OPTIMIZATIONS_PLAN, task ids, or
review ids in code, comments, tests, or docs; these files are temporary.

Run: <task test commands>. Commit each planned commit with the task's message.
Return: task id, commit SHAs, tests and results, deviations, blockers, and
non-blocking follow-up observations (do not implement them).
```

## Reviewer prompt template

```text
You are reviewing a completed batch of gircap tasks.

Repository: <absolute working tree>
Batch: <RC-id>
Tasks and commits:
  <task-id> — <title> — commits <sha...>
  ...
Report excerpts for these tasks: OPTIMIZATIONS_REPORT.md lines <ranges>.
Task files: <paths>. Read those task files and only those report lines.

For each task, verify the acceptance criteria and that the change does not
break existing contracts. Run the relevant tests. You may fix a task or revert
its commits. Do not implement non-blocking improvements; record them as
follow-ups. Do not edit OPTIMIZATIONS_PLAN/STATE.md. Do not add references to
the report or plan in the code.

Return per task: verdict (completed / completed-with-fixes / reverted), commits
you added or reverted, a one-line reason for any revert, and non-blocking
follow-ups with the affected area and a suggested batch.
```

## Context-budget rules

- One implementer = one task. Never bundle two tasks to save a spawn.
- One reviewer = one checkpoint batch. If a batch returns unexpectedly large,
  split the *next* review checkpoint rather than giving one reviewer more.
- Pass file paths and line ranges, not excerpts, unless the user's spawn
  mechanism cannot share the filesystem.
- Never forward an implementer's full transcript to a reviewer; forward only
  the task ids, SHAs, and deviation notes.
- Never forward review transcripts to the next implementer; a new task gets
  only its own slice.
