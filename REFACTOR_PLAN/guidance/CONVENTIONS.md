# Conventions

## Pointers, not recitations

- To aim a session at a goal, pass **file paths + line ranges / section numbers** (e.g.
  `docs/shared-crate-ownership-findings.md §F3 lines 91–102` and a
  `REFACTOR_PLAN/tasks/T01-*.md` path). Do not paste the report or a task file into the
  session prompt as prose; the session reads the files itself.
- Line numbers come from the report as committed alongside this package. A session that
  finds a stale pointer should trust the code and say so.
- Report sections: Assumptions (A1–A7, lines 8–39), Findings (F1–F9, lines 43–177).
  A task file names the exact sections and lines it needs. The report deliberately has no
  plan or negative-findings section; keepers are recorded in `DO-NOT-TOUCH.md`.

## Commits

- Implementation work for one task is committed by the implementing session. A
  complicated task may span several commits (e.g. crate move, consumer rewrites, doc
  seams, lockfile); record all their refs as pointers.
- Every `STATE.md` change is its **own commit**, separate from implementation commits
  (this includes `todo → in_progress`, `→ pending_review`, review results and follow-up
  bookkeeping).
- Commit messages must not reference `REFACTOR_PLAN/`, the report file, session
  identifiers, tool names, dates, or machine-specific data. Describe the code change
  itself.

## Validation before accepting a finding

- Report items are static claims taken on one tree (§A1, lines 8–13). Confirm the premise
  in the current tree before editing; line numbers may have drifted.
- A candidate move is only worth making when the consumer map (§F1) shows a single owning
  family, the crate is not consumed by a second family (§F5, §F9), and every direct
  consumer edge survives (§F6). Dev-dependency edges count (§A3).
- A move is incomplete until every §A7/§F8 seam is updated: workspace `members`, path
  dependencies in consumer manifests, `Cargo.lock` package entries for the three renames,
  path text in the root README, `tools/sync/README.md`, `tools/sled2redb/README.md` and
  doc comments, and the `tests/common/mod.rs` shims where the crate identity changes.
- Lint tables follow the owner (§F7): crates under `tools/sync/` use the tools-only table;
  crates in the girpr subtree keep `[lints] workspace = true`.
- Do not "fix" the misleading `girpr-lock` manifest description or similar wording unless
  a task (T07) or an endorsed follow-up says so; they are finding material (§F9, lines
  175–177), not drive-by edits.

## `STATE.md` fields

- **Tasks**: `id`, `title`, `state`, `commit ref(s)`, `note`.
  States: `todo` → `in_progress` → `pending_review` → `completed` / `reverted` /
  `skipped`.
- `commit ref(s)` is only the pointer handed to a review session so it knows what to look
  at. It is not a chronological change log. Do not add dates, session/tool IDs, machine
  data, or a running narrative.
- `note` carries the one-line reason for `skipped`/`reverted`, or a decision that later
  tasks need (e.g. the chosen girpr subtree path from T04). Keep it short and point to
  report sections where useful.
- **Reviews**: `id`, task IDs in the batch, `state` (`pending`/`done`), note.
- **Follow-ups**: `id`, description, `state` (`open`/`batched`/`done`/`dropped`), note.
  Follow-ups are non-blocking suggestions; the reviewer records them and never
  implements them. Reviewers may prune follow-ups to avoid run-away.
- **Persistent context**: any validation result or decision that affects later tasks.
  This is the only place to stash knowledge gained during orchestration. No dates, no
  session/tool IDs, no change history.

## Skip / improvise / follow-up

- A stopped task becomes `skipped` with the reason. Skipped tasks do not go to review.
  They may be reopened later as follow-ups if a reviewer says so.
- An implementing session may deviate from the written plan when it finds a safer or more
  ownership-consistent boundary; record the deviation in the task's `note`. The goal is
  ownership/layout agreement, not literal compliance with a static plan.
- Only a review session creates or removes follow-ups, and only the orchestrator writes
  them into `STATE.md`. The reviewer decides whether an implementer-suggested follow-up
  is worth doing.
- A task whose premise is pre-satisfied, stale, unsafe, or whose change would move a
  shared crate or merely add churn is skipped, not reported as a cosmetic success.

## No lingering references

Before the final cleanup, confirm no source, test, comment, manifest, doc table or commit
message in version control points at `REFACTOR_PLAN/` or at
`docs/shared-crate-ownership-findings.md`. Then delete `REFACTOR_PLAN/` and that report
file.
