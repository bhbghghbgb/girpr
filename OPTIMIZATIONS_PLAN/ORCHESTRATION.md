# Orchestration plan

This file is for the orchestrator session. The user decides **how** sessions
are spawned. This plan does not assume a tool, command, API, or agent
framework. When it says "spawn", use whatever mechanism the user supplied. If
none was supplied, ask before starting.

## Roles

- **Orchestrator** (this session): owns `STATE.md`, picks tasks, spawns one
  implementer per task, spawns reviewers at checkpoints, records results, and
  schedules follow-ups. It does not implement tasks itself.
- **Implementer**: one task per fresh session. Reads only its task file and the
  report slices named there, writes code/tests, commits, and returns SHAs.
  Never edits `STATE.md`.
- **Reviewer**: one checkpoint batch per fresh session. Reviews, fixes, or
  reverts the batch. Never adds scope beyond fixing that batch. Returns
  verdicts and non-blocking follow-ups.

## Files the orchestrator reads

Normally only:

- `OPTIMIZATIONS_PLAN/STATE.md` - the state machine;
- `OPTIMIZATIONS_PLAN/INDEX.md` - task map, dependencies, checkpoints.

Read a task file only when verifying a return or resolving a blocker. Never
paste the whole report or the whole plan into a session prompt. Pass the task
file path and the exact report line ranges that the task names; the session
reads the report itself.

## State file

`STATE.md` is committed to version control and is the single source of truth.

- Task statuses: `todo`, `in_progress`, `pending_review`, `completed`,
  `reverted`, `blocked`, `skipped`.
- Review batches: `pending`, `in_progress`, `done`.
- Every task row carries its commit SHAs once it has work.
- `completed` is set only after a review batch containing the task is `done`.
- `reverted` is terminal for that attempt; add a `## Rework` row with the
  reviewer's reason. Rework rows are scheduled like tasks.
- Follow-ups from reviewers and implementers go to `## Follow-ups`. They are
  not implemented until the main tasks and rework are done.
- Every change to `STATE.md` is its own commit (`chore(plan): ...`), separate
  from implementation work.

## Main loop

1. **Start clean.**
   - Confirm `git status` is clean. If it is not, stop and ask the user; do not
     build on top of someone else's uncommitted work.
   - If `STATE.md` is not yet committed, commit it as
     `chore(plan): initialize checklist`.
   - Do not start a task on a dirty tree.

2. **Choose the next unit of work.**
   - If the current checkpoint's remaining tasks are all `pending_review` or
     `skipped`, do the review first (step 4).
   - Otherwise take the first `todo` task in `INDEX.md` order whose
     dependencies are `completed` or `skipped`.
   - If there is no such task and `## Rework` has a `todo` row, take that.
   - If no main task or rework is available and some task is `blocked`, stop
     and resolve the blocker with the user.
   - Do not skip ahead to a later checkpoint to "keep busy"; the ordering is
     the plan.

3. **Implement one task.**
   - Record the current `HEAD` as the task's base.
   - Set the task to `in_progress` and commit that state change separately
     (`chore(plan): start <task-id>`). This keeps the checklist truthful if the
     session dies.
   - Spawn an implementer with the template below. Pass only:
     - the absolute working-tree path;
     - the task file path;
     - the report line ranges from the task file;
     - the commands the task asks for.
   - On return, inspect `git log` and `git status` yourself. If there are no
     commits or the tree is dirty, do not accept the work: mark `blocked` with
     the reason, retry once, or ask the user.
   - Record the commit SHAs and set the task to `pending_review`. Commit this
     state change separately (`chore(plan): <task-id> pending review`).
   - Do not spawn the next implementer until the return is recorded.
   - If the implementer reports the task is stale or impossible, mark it
     `skipped` or `blocked` (with the reason) and do **not** force a change.

4. **Review a batch at its checkpoint.**
   - A checkpoint is reached when every task in the batch is
     `pending_review` or `skipped`; skipped tasks normally have no commits to
     review, but the reviewer should see their reason.
   - Set the batch to `in_progress` and commit separately.
   - Spawn a reviewer with the batch template. Give it the task ids, their
     commit SHAs, the task file paths, and the union of report line ranges. Do
     not give it the orchestrator's history or unrelated tasks.
   - On return:
     - `completed` / `completed-with-fixes`: add any reviewer fix SHAs, set the
       task to `completed`.
     - `reverted`: set the task to `reverted` and add a `## Rework` row with
       the reason and suggested correction.
     - `blocked`: set the task to `blocked` and stop to resolve it.
     - Record every non-blocking follow-up in `## Follow-ups`.
     - Set the batch to `done`.
   - Commit the state update separately (`chore(plan): review <batch-id>`).
   - If a reviewer fixed a task and says the fix is risky, schedule a targeted
     re-review of just that task/commit before marking the batch done.

5. **Repeat** until every main task is `completed`, `skipped`, or has no
   remaining work, and every review batch is `done`. Rework rows are picked
   before follow-ups and join the next review batch, or get a targeted review
   if no batch remains.

## Review-fix exceptions

- A reviewer may leave a task `pending_review` only if it cannot decide; treat
  that as `blocked`, not as done.
- If a reviewer reverts a task, do not silently drop it. Add a `## Rework` row
  with the task id, the reason, and the suggested correction, status `todo`.
- A rework session gets the original task file plus the reviewer's reason. It
  does not get the reviewer's transcript.
- Fixes inside a review are part of the same batch. They need a second review
  only when the reviewer says the fix is risky.

## Follow-up phase

When all main tasks and rework are done:

1. **Evaluate batching first.** Spawn an evaluation-only session with
   `## Follow-ups` from `STATE.md` and `INDEX.md`. Ask it to group follow-ups
   into small batches by area and risk, and to say which ones are not worth
   doing. It must not edit code. Record the result under
   `## Follow-up batches` in `STATE.md` and commit.
2. Pick one batch and spawn one implementer per batch, using the same narrow
   context rule. Record commits and set the batch to `pending_review`.
3. Review follow-up batches with the same reviewer rules, but the reviewer may
   only fix/revert that batch. Record verdicts and any new follow-ups.
4. Repeat until every follow-up is `completed`, `reverted`, or explicitly
   `dropped` with a reason.

A follow-up that turns out to be a main-task-sized change is scheduled as its
own batch; do not let one session swallow the whole follow-up list.

## Completion and cleanup

When nothing is `todo`, `in_progress`, `pending_review`, `blocked`, or
`reverted` without rework:

1. Run the full relevant test suite in a clean tree. Record the command and
   result in the final state commit message.
2. Search the tree for leaked temporary references:
   `rg -n "OPTIMIZATIONS_(REPORT|PLAN)|T[0-9]{2}-|RC-[0-9]{2}"`.
   Remove or rewrite any that leaked into code, tests, comments, or permanent
   docs.
3. Delete `OPTIMIZATIONS_REPORT.md`, `OPTIMIZATIONS_PLAN/` (including
   `STATE.md`), and any temporary notes created during the work.
4. Commit the deletion separately. Confirm the tree is clean and the final test
   command still passes after the deletion.

## Implementer prompt template

Replace `<...>`. Keep everything outside the task slice out of the prompt. The
implementation session must not read the orchestrator's transcript.

```text
You are implementing one task in the girpr crate.

Repository: <absolute working tree>
Task file: OPTIMIZATIONS_PLAN/tasks/<task-file>.md
Report excerpts: OPTIMIZATIONS_REPORT.md lines <ranges>. Read only those lines.
Read the task file in full, then the code and tests it names. Do not read other
task files, STATE.md, ORCHESTRATION.md, or the rest of the report.

Validate the task against the current code before editing. If the functions,
types, flags, or tests it names no longer exist or no longer have the described
shape, stop and report what changed; do not force the change.

Implement only this task, following its commit plan. Use the repository's
existing style and test patterns. Do not edit OPTIMIZATIONS_PLAN/STATE.md. Do
not reference OPTIMIZATIONS_REPORT.md, OPTIMIZATIONS_PLAN, task ids, or review
ids in code, comments, tests, docs, or commit messages; these files are
temporary.

Run: <task test commands>. Commit each planned commit with the task's message.
Return: task id, commit SHAs, tests and results, deviations, blockers, and
non-blocking follow-up observations (do not implement them).
```

## Reviewer prompt template

```text
You are reviewing a completed batch of girpr tasks.

Repository: <absolute working tree>
Batch: <RC-id>
Tasks and commits:
  <task-id> - <title> - commits <sha...>
  ...
Report excerpts for these tasks: OPTIMIZATIONS_REPORT.md lines <ranges>.
Task files: <paths>. Read those task files and only those report lines.

For each task, verify the acceptance criteria and that the change does not
break existing contracts. Run the relevant tests. Confirm the implementer
validated the task against the current code rather than applying a stale plan.
You may fix a task or revert its commits. Do not implement non-blocking
improvements; record them as follow-ups. Do not edit
OPTIMIZATIONS_PLAN/STATE.md. Do not add references to the report or plan in
the code.

Return per task: verdict (completed / completed-with-fixes / reverted /
blocked), commits you added or reverted, a one-line reason for any revert, and
non-blocking follow-ups with the affected area and a suggested batch.
```

## Context-budget rules

- One implementer = one task. Never bundle two tasks to save a spawn.
- One reviewer = one checkpoint batch. If a batch returns unexpectedly large,
  split the *next* checkpoint rather than giving one reviewer more.
- Pass file paths and line ranges, not excerpts, unless the user's spawn
  mechanism cannot share the filesystem.
- Never forward an implementer's full transcript to a reviewer; forward only
  task ids, SHAs, and deviation notes.
- Never forward review transcripts to the next implementer; a new task gets
  only its own slice.
- Keep the orchestrator's own reading to `STATE.md` and `INDEX.md`; read a task
  file only to verify a specific return.
