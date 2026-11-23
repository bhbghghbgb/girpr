# Conventions

## Pointers, not recitations

- To aim a session at a goal, pass **file paths + line ranges / finding IDs** (e.g.
  `docs/OPTIMIZATION_REPORT.md §2 S-01 lines 109–125` and a
  `REFACTOR_PLAN/tasks/T01-*.md` path). Do not paste the report or a task file into
  the session prompt as prose; the session reads the files itself.
- Line numbers come from the report as committed alongside this package. A session
  that finds a stale pointer should trust the code and say so.
- Report sections: Summary (§1, lines 27–105), Scheduling (§2, 107–198), Disk I/O
  (§3, 200–293), Memory (§4, 295–345), Network (§5, 347–413), Correctness (§6,
  415–513), Logging (§7, 515–560), Micro (§8, 562–637), Dev tools (§9, 639–806),
  Build (§10, 808–829), Assumptions (§11, 832–888), Non-findings (§12, 890–905).
  A task file names the exact sections and lines it needs. The report deliberately
  has no plan section; keepers are recorded in `DO-NOT-TOUCH.md`.

## Commits

- Implementation work for one task is committed by the implementing session. A
  complicated task may span several commits (e.g. code, call-site rewrites, tests,
  docs); record all their refs as pointers.
- Every `STATE.md` change is its **own commit**, separate from implementation commits
  (this includes `todo → in_progress`, `→ pending_review`, review results and
  follow-up bookkeeping).
- Commit messages must not reference `REFACTOR_PLAN/`, the report file, session
  identifiers, tool names, dates, or machine-specific data. Describe the code change
  itself.

## Validation before accepting a finding

- Report items are static claims taken on one tree (lines 8–13). Confirm the premise
  in the current tree before editing; line numbers may have drifted.
- Confirm the finding's listed assumption (§11, lines 832–888) actually holds. An
  unconfirmed assumption means stop and record `skipped`, not guess.
- Do not change a non-finding (§12, lines 890–905), the tuned Cargo profiles (lines
  810–816), B-02 (lines 825–829), the data-plane output contract (lines 527–530),
  deterministic log/artifact ordering (lines 179–183; assumptions 14–17), or a pinned
  wire format (e.g. girsync's `postcard` digest vocabulary, T24).
- Do not make drive-by edits outside the task's scope. Findings that are trade-offs
  worth revisiting are tasks with their own validation, not incidental changes.
- Benchmark and behavior claims in the report are static reasoning, not measurements
  taken here. An implementer may improve the code on intuition, but must not claim a
  measured win that was not measured; a reviewer may revert changes whose stated
  premise does not hold.

## `STATE.md` fields

- **Tasks**: `id`, `title`, `state`, `commit ref(s)`, `note`.
  States: `todo` → `in_progress` → `pending_review` → `completed` / `reverted` /
  `skipped`.
- `commit ref(s)` is only the pointer handed to a review session so it knows what to
  look at. It is not a chronological change log. Do not add dates, session/tool IDs,
  machine data, or a running narrative.
- `note` carries the one-line reason for `skipped`/`reverted`, or a decision that later
  tasks need (e.g. a failed assumption, a renamed type, a chosen workaround). Keep it
  short and point to report sections where useful.
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
- An implementing session may deviate from the written plan when it finds a safer
  approach; record the deviation in the task's `note`. The goal is the finding's
  improvement, not literal compliance with a static plan.
- Only a review session creates or removes follow-ups, and only the orchestrator
  writes them into `STATE.md`. The reviewer decides whether an implementer-suggested
  follow-up is worth doing.
- A task whose premise is pre-satisfied, stale, unsafe, or whose change would merely
  add churn is skipped, not reported as a cosmetic success.

## No lingering references

Before the final cleanup, confirm no source, test, comment, manifest, doc table or
commit message in version control points at `REFACTOR_PLAN/` or at
`docs/OPTIMIZATION_REPORT.md`. Then delete `REFACTOR_PLAN/` and that report file.
