# Orchestration plan

The orchestrator owns the queue, the committed state file, and the review
checkpoints. It does not implement tasks and it does not review them.

## 0. Before the first task: settle the spawn mechanism

**Do not assume any session-spawning tool exists.** The user will tell the
orchestrator how to spawn a session. Ask once, before doing anything else:

> How should I start each implementing/reviewing session? Give me the exact
> command or tool call, including how to pass a prompt and a working
> directory. I will not assume one.

Record the answer in the state file under `spawn:` (a short description, not a
secret). Until it is recorded, `phase` stays `blocked-on-spawn` and no task is
started.

The orchestrator must also confirm:

- the working directory is `tools/sync` (or the repo root with that path);
- the current branch is the intended work branch, and `git status` is clean
  except for this plan directory and the report;
- whether the user wants the orchestrator to run `cargo test` itself or leave
  it to the sessions. Default: the orchestrator runs the cheap full check at
  each checkpoint, the sessions run it per task.

## 1. State file

`STATE.md` is committed to version control. It is the only source of truth for
progress. Only the orchestrator edits it.

Transitions are committed **separately** from task commits:

```text
chore(sync): record T03 pending review
chore(sync): record review B done
chore(sync): record follow-ups F2-F4
```

A task session must never touch `STATE.md`.

State fields:

- `phase`: `blocked-on-spawn`, `implement`, `review`, `followups-evaluate`,
  `followups-implement`, `followups-review`, `finalize`, `done`.
- `active_batch`: `A` … `G`, or a follow-up batch id.
- `tasks[]`: `id`, `title`, `plan`, `batch`, `report`, `status`, `commits`,
  `review`, `note`.
- `reviews[]`: `batch`, `status`, `tasks`, `base`, `head`, `result`,
  `followups`.
- `followups[]`: `id`, `source`, `description`, `status`, `batch`, `commits`.
- `spawn`: the user-provided mechanism description.
- `last_updated`: ISO-8601 UTC.

Task statuses:

| status | meaning |
| --- | --- |
| `pending` | not started |
| `in_progress` | a session is running; records its id if known |
| `pending_review` | the session finished and committed; awaiting the batch review |
| `completed` | reviewed and kept |
| `reverted` | reviewed and rolled back |
| `skipped` | deliberately not done; `note` has the reason |
| `blocked` | cannot proceed; `note` says why |

Review statuses: `pending`, `in_progress`, `done`.

## 2. Main loop

Repeat until the queue has no `pending` task:

1. **Select.** The first task in queue order whose status is `pending`. If an
   `in_progress` task exists, do not skip it: inspect the working tree. If the
   session died without commits, reset the task to `pending` (after checking
   that no partial work must be preserved) or ask the user. If it left
   commits, treat it as `pending_review` and record the hashes.
2. **Spawn one implementing session** with the prompt in §3. Pass the task
   file path and its report line ranges, not the text of either.
3. **Verify the hand-off.** The session must return commit hashes and a clean
   `git status`. If it returned `none` or left uncommitted work, do not mark
   it reviewed: ask the session to commit or report the blocker.
4. **Record `pending_review`** in `STATE.md`, with commit hashes, in its own
   commit.
5. **Checkpoint?** If this task is the last of its batch, go to §4. Otherwise
   go to 1.

If a session reports `blocked`, record `blocked` with the reason and ask the
user. Do not silently skip a task.

## 3. Implementing-session prompt

Pass a prompt file, not a paraphrase. Keep it short; the task file is the
detail.

```text
You are implementing one task in tools/sync (crate girsync).

Read first, in this order:
1. tools/sync/OPTIMIZATIONS_PLAN/GUIDELINES.md
2. <the exact task file path, e.g. tools/sync/OPTIMIZATIONS_PLAN/tasks/T01-by-name-digest-lookup.md>
3. tools/sync/DESIGN-NOTES.md sections the task names
4. only these ranges of tools/sync/OPTIMIZATIONS_REPORT.md: <line ranges>

Then implement only that task. Follow its commit plan. Do not touch
tools/sync/OPTIMIZATIONS_PLAN/STATE.md. Do not mention the plan or the report
in any commit, comment, doc, or test. Run the verifications the task names.
Return: task id, commit hashes, commands run with results, deviations,
candidate follow-ups.
```

Replace `<line ranges>` with the numeric ranges from the task's metadata, for
example `L59-L87` and `L572-L575`. Do not paste the report sections. Do not
summarise the task in the prompt beyond the file path.

The orchestrator reads only the task's metadata block (first ~20 lines) to
build the prompt. It must not read the whole report.

## 4. Review checkpoint

A checkpoint covers exactly the batch's tasks that are `pending_review`.

1. Record `phase: review`, `active_batch: <batch>`, review `in_progress`, and
   the base commit (the first commit's parent) and head commit.
2. Spawn **one review-and-fix session** with the prompt in §5.
3. The reviewer returns a structured result:
   - per task: `kept`, `fixed`, `reverted`, or `blocked`;
   - fix/revert commit hashes;
   - non-blocking follow-ups, each one sentence with its source task;
   - commands run and results.
4. **Apply the result** to `STATE.md`:
   - `kept` or `fixed` → `completed`;
   - `reverted` → `reverted`, with the revert commit;
   - `blocked` → `blocked`, ask the user;
   - append follow-ups to `followups[]` with status `pending`;
   - mark the review `done` with its result summary.
5. Commit the state change separately. Then set `phase: implement`, advance
   `active_batch` to the next batch, and continue.

If the reviewer fixed a task, the fix commits belong to the batch and are
recorded in that task's `commits` list. If it reverted a task that later tasks
depended on, it must also revert or adapt those later tasks; the state records
which ones changed.

### Review scope

The reviewer receives:

- the batch's task files (paths only) and their report line ranges;
- the commit range `<base>..<head>` plus any fix commits it makes;
- `GUIDELINES.md`;
- `STATE.md` for status only (read-only).

It does **not** receive the whole report, the whole plan, or the other
batches' task files. That keeps the review focused and its context small.

## 5. Review-session prompt

```text
You are reviewing a batch of completed tasks in tools/sync (crate girsync).
You may fix or revert; you may not implement follow-ups.

Read first:
1. tools/sync/OPTIMIZATIONS_PLAN/GUIDELINES.md
2. tools/sync/DESIGN-NOTES.md (the invariants section)
3. these task files: <paths>
4. only these report ranges: <ranges>
5. git log --oneline <base>..<head> and git diff <base>..<head>

For each task, check its acceptance criteria and the crate invariants. Fix
with new commits, or revert with git revert. Do not rewrite history. Run the
tests. Do not edit STATE.md. Do not mention the plan/report in committed
text.

Return exactly:
- per task: kept | fixed (<hashes>) | reverted (<hashes>) | blocked (<why>)
- non-blocking follow-ups: one sentence each, tagged with its source task
- commands run and results
```

## 6. Follow-ups

After every task is `completed` or `reverted`:

1. If `followups[]` is empty, go to §7.
2. Set `phase: followups-evaluate`. Spawn **one evaluation session** with the
   follow-up list from `STATE.md` and the file paths they touch. Its only job
   is to group them into batches:
   - same file or same subsystem together;
   - low-risk documentation/test-only items may share a batch;
   - do not put two behaviour changes that interact in one batch unless the
     evaluator explains why they must land together;
   - each batch should be small enough for one review session (roughly 3–5
     follow-ups or one medium change).
   The evaluator returns a proposed grouping with ids and a one-line reason.
3. The orchestrator records the grouping in `followups[].batch` and commits
   the state.
4. For each follow-up batch: set `phase: followups-implement`, spawn one
   implementing session with the batch's follow-ups and the same rules as §3.
   Mark its items `pending_review`, then `phase: followups-review` and run one
   review session as in §4–§5. Follow-up fixes that generate new follow-ups
   are appended again; do not loop forever — if a follow-up produces another
   follow-up of the same kind, record it as `skipped` with the reason.
5. Repeat until no follow-up is `pending` or `pending_review`.

Follow-ups are non-blocking. The orchestrator may choose to mark a follow-up
`skipped` with a reason rather than delay completion, but it must record the
decision.

## 7. Finalize

1. Run the final gate:
   ```text
   cargo fmt --all -- --check
   cargo test -p girsync
   cargo test -p girsync --features convert
   ```
2. Search for leftover references:
   ```text
   rg -n "OPTIMIZATIONS_(REPORT|PLAN)|T[0-9]{2}-|review checkpoint" tools/sync
   ```
   The plan and report themselves will match; everything else must not.
3. Set `phase: done` in `STATE.md` as the last state edit. The file is about
   to be deleted, so this edit exists only to make the intent explicit in the
   diff.
4. Delete `tools/sync/OPTIMIZATIONS_PLAN/` (including `STATE.md`) and
   `tools/sync/OPTIMIZATIONS_REPORT.md`.
5. Commit the deletion:
   ```text
   chore(sync): remove completed optimization plan
   ```
6. Stop. Do not create a new state file. The final commit message records
   completion; `git log` is the durable record.

## 8. Failure handling

- **Session crashes / no commits:** inspect `git status`; if the tree is
  clean, reset the task to `pending` and retry once with the same prompt. If
  the tree is dirty, ask the user before discarding anything.
- **Tests fail repeatedly:** mark the task `blocked`, record the failing
  command and the first error, and ask the user. Do not lower a test's
  standard to make it pass.
- **Reviewer wants a broad change:** it must not make it. It records a
  follow-up; the orchestrator decides later.
- **A task conflicts with a later task:** finish the current batch, let the
  reviewer adapt or revert, and record the dependent task's new state. Do not
  reorder the queue silently.
- **User changes the spawn mechanism mid-run:** update `spawn:` and continue;
  do not restart completed tasks.

## 9. Context discipline

The orchestrator's job is to pass **paths and line ranges**, not content. It
should:

- read `STATE.md` and the first metadata block of the selected task file;
- pass the task file path, report ranges, and the guidelines path;
- never paste the report or the full plan into a prompt;
- never summarise the report's rationale in its own words;
- keep its own messages short: state transition, session id, commit hashes,
  next action.

This is deliberate: a long orchestrator context is the easiest way for the
plan's intentions to drift from the code.
