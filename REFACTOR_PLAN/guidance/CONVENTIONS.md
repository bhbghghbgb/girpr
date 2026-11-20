# Conventions

## Pointers, not recitations

- To aim a session at a goal, pass **file paths + line ranges / section numbers**
  (e.g. `docs/08-shared-crate-refactor-research.md §F3 lines 125–136` and a
  `REFACTOR_PLAN/tasks/T08-*.md` path). Do not paste the report or a task file
  into the session prompt as prose; the session reads the files itself.
- Line numbers come from the report as committed alongside this package. A
  session that finds a stale pointer should trust the code and say so.
- Report sections: §A assumptions (A1–A12), §F findings (F1–F30), §N negative
  findings (N1–N10). A task file names the exact sections and lines it needs.

## Commits

- Implementation work for one task is committed by the implementing session.
- Every `STATE.md` change is its **own commit**, separate from implementation
  commits (this includes `todo → in_progress`, `→ pending_review`, review
  results and follow-up bookkeeping).
- Commit messages must not reference `REFACTOR_PLAN/`, the report file, session
  identifiers, tool names, dates, or machine-specific data. Describe the code
  change itself.

## New shared crates

- Convention from §A1: shared code lives in `crates/`, is named `girpr-<policy>`,
  carries a one-line `description = "The shared … policy …"`, opts into
  `[lints] workspace = true`, and holds policy/mechanics rather than program
  logic. A crate that holds a program's own data model needs an explicit
  justification and must still be logic-free.
- **gircap boundary (§A2):** no dependency on the `girpr` crate, no invoking its
  binary, and only leaf crates that hold no program logic. Adding any third
  shared leaf dep requires updating `tools/capture/docs/00-spec.md:42` (the
  table currently names two) plus the other locations in §A2:
  `README.md:389-391`, `tools/sync/README.md:8-10`,
  `tools/capture/README.md:3-4`, `docs/06-artifact-bridge-spec.md:707-709`.
  Update these in the same work that adds the dependency; do not leave a doc
  gap (§F30 notes the existing gap for `girpr-filter`).
- A shared crate's tests must not make one program's test depend on another's.
  Keep test-only inversions visible (§A11: `girpr-tempdir` depends on girsync and
  serde_json for `test_support`).

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
  tasks (e.g. "three digest semantics confirmed distinct, see §A7"). This is the
  only place to stash knowledge gained during orchestration. No dates, no
  session/tool IDs, no change history.

## Skip / improvise / follow-up

- A stopped task becomes `skipped` with the reason. Skipped tasks do not go to
  review. They may be reopened later as follow-ups if a reviewer says so.
- An implementing session may deviate from the written plan when it finds a
  better optimization; record the deviation in the task's `note`.
- Only a review session creates or removes follow-ups, and only the orchestrator
  writes them into `STATE.md`. Reviewers may prune follow-ups to avoid run-away.

## No lingering references

Before the final cleanup, confirm no source, test, comment, manifest, doc table
or commit message in version control points at this directory or at
`docs/08-shared-crate-refactor-research.md`. Then delete `REFACTOR_PLAN/` and
that report file.
