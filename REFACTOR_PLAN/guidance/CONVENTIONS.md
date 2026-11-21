# Conventions

## Pointers, not recitations

- To aim a session at a goal, pass **file paths + line ranges / section numbers**
  (e.g. `docs/10-test-optimization-research.md §F1 lines 131–143` and a
  `REFACTOR_PLAN/tasks/T01-*.md` path). Do not paste the report or a task file into the
  session prompt as prose; the session reads the files itself.
- Line numbers come from the report as committed alongside this package. A session
  that finds a stale pointer should trust the code and say so.
- Report sections: §1 baseline, §A assumptions (A1–A12), §F findings (F1–F38), §4
  highest-value table, §5 explicit exclusions, §6 validation notes. A task file names
  the exact sections and lines it needs.

## Commits

- Implementation work for one task is committed by the implementing session. A
  complicated task may span several commits; record all their refs as pointers.
- Every `STATE.md` change is its **own commit**, separate from implementation commits
  (this includes `todo → in_progress`, `→ pending_review`, review results and follow-up
  bookkeeping).
- Commit messages must not reference `REFACTOR_PLAN/`, the report file, session
  identifiers, tool names, dates, or machine-specific data. Describe the code change
  itself.

## Validation before accepting a finding

- Report items are static claims measured on one host (§1, §6). Confirm the premise in
  the current tree; re-measure clean-build and `--workspace` wall time before/after a
  change and confirm the semantic assertion a test carries still exists after a merge
  or downshift.
- Under nextest, one process runs per test (§A1). A change that only adds a shared
  cache or helper does not save spawns; merging tests does. Do not add `rstest` to
  existing loop tables (F7).
- Dependency-tree claims must be checked with `cargo tree -e dev -p <crate>`; a dev-dep
  removed from one member may still be compiled for another under feature unification
  (§A11). Isolated payoff is measured with `cargo nextest run -p <crate>`.
- Findings marked "isolated builds only" (F2, F6, F25) will not show up in a
  feature-unified workspace run; measure them at package scope.

## `STATE.md` fields

- **Tasks**: `id`, `title`, `state`, `commit ref(s)`, `note`.
  States: `todo` → `in_progress` → `pending_review` → `completed` / `reverted` /
  `skipped`.
- `commit ref(s)` is only the pointer handed to a review session so it knows what to
  look at. It is not a chronological change log. Do not add dates, session/tool IDs,
  machine data, or a running narrative.
- `note` carries the one-line reason for `skipped`/`reverted`, or a decision that later
  tasks need (e.g. "F30 seam made dry-then-real pairs share state — forget helper
  required"). Keep it short and point to report sections where useful.
- **Reviews**: `id`, task IDs in the batch, `state` (`pending`/`done`), note.
- **Follow-ups**: `id`, description, `state` (`open`/`batched`/`done`/`dropped`), note.
  Follow-ups are non-blocking suggestions; the reviewer records them and never
  implements them.
- **Persistent context**: any validation result or decision that affects later tasks.
  This is the only place to stash knowledge gained during orchestration. No dates, no
  session/tool IDs, no change history.

## Skip / improvise / follow-up

- A stopped task becomes `skipped` with the reason. Skipped tasks do not go to review.
  They may be reopened later as follow-ups if a reviewer says so.
- An implementing session may deviate from the written plan when it finds a better
  optimization; record the deviation in the task's `note`. The goal is cheaper test
  execution or rebuild, not literal compliance with a static plan.
- Only a review session creates or removes follow-ups, and only the orchestrator
  writes them into `STATE.md`. Reviewers may prune follow-ups to avoid run-away.
- A task whose premise is pre-satisfied, stale, or whose change does not actually
  reduce the relevant cost is skipped, not reported as a cosmetic success.

## No lingering references

Before the final cleanup, confirm no source, test, comment, manifest, doc table or
commit message in version control points at `REFACTOR_PLAN/` or at
`docs/10-test-optimization-research.md`. Then delete `REFACTOR_PLAN/` and that report
file.
