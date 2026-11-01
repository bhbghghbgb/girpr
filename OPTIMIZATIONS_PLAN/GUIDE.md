# Guide for implementers, reviewers, and the orchestrator

These rules apply to every session spawned for this plan. The orchestrator
passes this file, the assigned task file, and the report line ranges named by
that task. Nothing else is needed.

## For an implementer

1. **Read narrowly.** Read your task file, then only the report line ranges it
   names. Do not read other task files, `STATE.md`, `ORCHESTRATION.md`, or the
   rest of the report. Read the code and tests the task names, plus their
   direct callers.
2. **Validate before editing.** The report and plan were written against a
   particular snapshot. Confirm that the functions, types, flags, and tests
   the task names still exist and still have the described shape.
3. **Stop instead of forcing.** If the task is stale, already fixed, or its
   premise no longer holds, do not invent a change. Stop and return a short
   report: what you found, where, and why the task cannot proceed. The
   orchestrator marks it `skipped` or `blocked`. The same applies if a task
   requires breaking one of the deliberate choices in the report.
4. **Implement only your task.** Do not pick up the next task, do not fix
   neighbouring findings, and do not implement a follow-up you noticed. Record
   the observation in your return instead.
5. **Keep the tree honest.** Check `git status` and `git log` before editing.
   Do not overwrite or delete unfamiliar work. Never edit `STATE.md`; the
   orchestrator owns it.
6. **Follow the commit plan.** A task may have several commits. Commit code and
   tests only. Use the repository's style and the task's suggested messages.
   Do not amend, rebase, or force-push existing commits. A deliberate rollback
   is `git revert`.
7. **Run the tests the task names**, then the broader command it requires. If a
   test cannot run on this host, say exactly why. Do not add timing assertions
   to CI; performance tasks are measured manually or via a narrow benchmark.
8. **No temporary references.** Do not put `OPTIMIZATIONS_REPORT.md`,
   `OPTIMIZATIONS_PLAN`, a task id, a review id, or "see the report" into code,
   comments, tests, docs, or commit messages. The plan is deleted; the change
   must stand on its own.
9. **Return a compact result:** task id, commit SHAs, commands run and results,
   deviations from the plan, blockers, and non-blocking follow-up observations.

## For a reviewer

1. Read the task files in the batch and only the report line ranges they name.
   Read the commits the orchestrator gives you (`git show` / `git log -p`).
2. Verify each task against its acceptance criteria, the report's stated goal,
   and the repository's existing contracts. Run the relevant tests. Confirm the
   implementer validated the code rather than blindly applying a stale plan.
3. You may **fix** a bad task or **revert** it. Fixes are new commits and must
   stay inside the task's scope. Reverts preserve history; if a later task in
   the batch depends on the reverted commit, fix or revert the dependent work
   too, and say so.
4. Do **not** implement non-blocking improvements. List them as follow-ups with
   the affected area and a suggested batch.
5. Do not edit `STATE.md`. Return a structured result the orchestrator can
   apply: per-task verdict (`completed`, `completed-with-fixes`, `reverted`,
   or `blocked`), commits added or reverted, tests run, one-line reasons, and
   follow-ups.
6. Do not add references to the report or plan, even in a committed review
   comment.

## For the orchestrator

Follow `ORCHESTRATION.md`. Keep your own context narrow: read `STATE.md` and
`INDEX.md`; read a task file only to verify a return or resolve a blocker. Do
not paste the report or the whole plan into a session prompt.

## Status vocabulary

Tasks: `todo`, `in_progress`, `pending_review`, `completed`, `reverted`,
`blocked`, `skipped`.

- `completed` is set only after the review batch containing the task is `done`
  and the reviewer's verdict is `completed` or `completed-with-fixes`.
- `reverted` is terminal for that attempt. Add a `## Rework` row with the
  reviewer's reason; rework is scheduled like a task.
- `skipped` means the task was stale or no longer applicable, with a reason.
  It is not a failure and does not need rework.
- `blocked` means the session could not proceed for a reason the orchestrator
  must resolve (missing dependency, ambiguous decision, broken environment).

Review batches: `pending`, `in_progress`, `done`.

## Commit conventions

- Implementation: `opt(<task-id>): <imperative summary>`.
- Review fixes: `review(<batch-id>): <summary>`.
- Orchestrator state: `chore(plan): <summary>`.
- A state change is always a separate commit from implementation work, so a
  reviewer's commit range never mixes code and checklist changes.
- The initial checklist is committed as `chore(plan): initialize checklist`.

## Test baseline

The repository's normal gate for this crate is:

```text
cargo fmt --all -- --check
cargo test -p girpr
```

The workspace is configured for `cargo nextest run`; use it if the host has
`cargo-nextest`, otherwise `cargo test -p girpr` is equivalent for the
checks these tasks need. A task that changes only build configuration still
must compile and run the crate's tests. A task that changes `tools/` build
membership must also prove the tools still compile explicitly.
