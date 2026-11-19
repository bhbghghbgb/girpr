# Conventions

## Pointers, not recitations

- To aim a session at a goal, pass **file paths + line ranges / section numbers**
  (e.g. `docs/08-hand-rolled-code-reduction-findings.md §B1 lines 351–372` and a
  `REFACTOR_PLAN/tasks/T01-*.md` path). Do not paste the report or a task file
  into the session prompt as prose; the session reads the files itself.
- Line numbers come from the report as committed alongside this package. A
  session that finds a stale pointer should trust the code and say so.
- Report sections: §A cross-cutting, §B root `girpr`, §C `crates/log`,
  §D `crates/tempdir`, §E `tools/sync`, §F `tools/capture`,
  §G `tools/sled2redb`, §H keep-as-is findings, §I assumptions to validate.

## Commits

- Implementation work for one task is committed by the implementing session.
- Every `STATE.md` change is its **own commit**, separate from implementation
  commits (this includes `todo → in_progress`, `→ pending_review`, review
  results and follow-up bookkeeping).
- Commit messages must not reference `REFACTOR_PLAN/`, the report file, session
  identifiers, tool names, dates, or machine-specific data. Describe the code
  change itself.

## `STATE.md` fields

- **Tasks**: `id`, `title`, `state`, `commit ref(s)`, `note`.
  States: `todo` → `in_progress` → `pending_review` → `completed` / `reverted` /
  `skipped`.
- `commit ref(s)` is only the pointer handed to a review session so it knows
  what to look at. It is not a chronological change log. Do not add dates,
  session/tool IDs, machine data, or a running narrative.
- `note` carries the one-line reason for `skipped`/`reverted`, or a decision that
  later tasks need. Keep it short and point to report sections where useful.
- **Reviews**: `id`, task IDs in the batch, `state` (`pending`/`done`), note.
- **Follow-ups**: `id`, description, `state` (`open`/`batched`/`done`/`dropped`),
  note. Follow-ups are non-blocking suggestions; the reviewer records them and
  never implements them.
- **Persistent context**: any validation result or decision that affects later
  tasks (e.g. "library X ruled out for §A4, see §I.2"). This is the only place to
  stash knowledge gained during orchestration, because the orchestrating session
  may be replaced. No dates, no session/tool IDs, no change history.

## Skip / improvise / follow-up

- A stopped task becomes `skipped` with the reason. Skipped tasks do not go to
  review. They may be reopened later as follow-ups if a reviewer says so.
- An implementing session may deviate from the written plan when it finds a
  better optimization; record the deviation in the task's `note`.
- Only a review session creates or removes follow-ups, and only the orchestrator
  writes them into `STATE.md`. Reviewers may prune follow-ups to avoid run-away.

## No lingering references

Before the final cleanup, confirm no source, test, comment, manifest or commit
message in version control points at this directory or the report. Then delete
`REFACTOR_PLAN/` and `docs/08-hand-rolled-code-reduction-findings.md`.
