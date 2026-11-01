# How to use a task file

## For an implementer session

1. Read only your assigned task file and the report line ranges it names.
   Do not read other task files, `STATE.md`, or the rest of the report. The
   orchestrator deliberately gives you a narrow slice.
2. Read the code the task names, plus its direct callers and tests. Prefer
   existing patterns over new abstractions.
3. Implement only that task. Do not fix neighbouring findings, even obvious
   ones; report them as follow-up observations instead.
4. Follow the task's commit plan. Commit code and tests only. **Never edit
   `OPTIMIZATIONS_PLAN/STATE.md`** - the orchestrator owns it and commits it
   separately.
5. Run the tests named in the task, plus any broader command the task requires.
   If a test cannot run in this environment, say exactly why.
6. **Do not keep references to this plan or the report.** No task ids, no
   "see OPTIMIZATIONS_REPORT" comments, no links to `OPTIMIZATIONS_PLAN`. The
   rationale must stand on its own in code, tests, or the project's permanent
   docs. These files are deleted when the work ends.
7. Return: task id, commit SHAs, tests run and results, deviations from the
   plan, blockers, and non-blocking follow-up observations.

## For a reviewer session

1. Read the task files in the batch and the report line ranges they name.
   Read the commits named by the orchestrator (use `git show`/`git log -p`).
2. Verify each task against its acceptance criteria, the report's stated goal,
   and the repository's existing contracts. Run the relevant tests.
3. You may **fix** a bad task or **revert** it. You may not implement
   non-blocking improvements. Record those as follow-ups.
4. If you fix a task, commit the fix; keep it within the task's scope. If you
   revert, revert the task's commits (do not revert the orchestrator's state
   commits) and say so.
5. Return a per-task verdict (`completed`, `completed-with-fixes`, `reverted`),
   the commit SHAs you added or reverted, tests run, and follow-ups. The
   orchestrator records the verdict.

## For the orchestrator

Follow `ORCHESTRATION.md`. Keep your own context narrow: read `STATE.md` and
`INDEX.md`, not every task file and not the whole report.

## Task file shape

Each task file contains only:

- report pointers (line ranges, not a copy of the report);
- dependencies and the intended commit breakdown;
- the code locations to start from;
- ordered implementation steps;
- tests and acceptance criteria;
- risks/decisions the implementer must respect.

If a task needs a product decision that the report leaves open (A3, B1, B2),
the task says so and requires the implementer to choose the option that
preserves current artifacts unless the report explicitly says otherwise. If no
safe default exists, stop and return the decision to the orchestrator instead
of guessing.

## Commit conventions

- Task commits: `opt(<task-id>): <imperative summary>`.
- Review fixes: `review(<batch-id>): <summary>`.
- Orchestrator state updates: `chore(plan): <summary>`.
- A state update is always a separate commit from implementation work, so a
  reviewer's commit range never mixes code and checklist changes.
