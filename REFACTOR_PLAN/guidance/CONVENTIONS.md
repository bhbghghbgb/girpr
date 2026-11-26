# Conventions

## Pointers, not recitations

- To aim a session at a goal, pass **file paths + line ranges / section numbers**
  (e.g. `docs/08-third-party-delegation-findings.md §APP-2 lines 281–294` and a
  `REFACTOR_PLAN/tasks/T09-url-delegation.md` path). Do not paste the report or a
  task file into the session prompt as prose; the session reads the files itself.
- Line numbers come from the report as committed alongside this package. A session
  that finds a stale pointer should trust the code and say so.
- Report sections: Assumptions A1–A12 (lines 13–51), Cross-cutting X1–X4 (lines
  54–134), root R1–R8 (138–257), `app/*` APP-1–APP-12 (260–410), `crates/*`
  LEAF-1–LEAF-10 (413–535), `tools/sync*` SYNC-1–SYNC-9 (538–650),
  `tools/capture*` CAP-1–CAP-11 (653–791), non-findings (795–855). A task file names
  the exact sections and lines it needs.
- The report deliberately has no plan section; keepers and non-findings are recorded in
  `DO-NOT-TOUCH.md` as pointers only.

## Commits

- Implementation work for one task is committed by the implementing session. A
  complicated task may span several commits (e.g. a server rewrite, then golden-file
  regeneration, then manifest/lockfile changes); record all their refs as pointers.
- Every `STATE.md` change is its **own commit**, separate from implementation commits
  (this includes `todo → in_progress`, `→ pending_review`, review results and follow-up
  bookkeeping). Moving a task to `pending_review` must never be folded into the
  implementation commit.
- Commit messages must not reference `REFACTOR_PLAN/`, the report file, session
  identifiers, tool names, dates, or machine-specific data. Describe the code change
  itself.

## Validation before accepting a finding

- Report items are static claims taken on one tree (§A1, lines 15–16). Confirm the
  premise in the current tree before editing; line numbers and versions may have
  drifted (§A2, 17–18).
- Decide once whether `game_ids[]` is sent literally or as `%5B%5D` (§X2 lines 90–109)
  and follow the decision recorded in `STATE.md` Persistent context; do not let one
  product normalize differently from another unless the report's assumptions justify it.
- Behavior changes are acceptable (§A4 lines 22–23), but golden files must be
  regenerated, not emptied, and tests of intended function adapted, not deleted (§A5
  lines 26–28).
- Never enable `serde_json/preserve_order` (§A7 lines 33–34); use a local ordered
  representation where insertion order matters.
- Do not change a non-finding or a pinned contract in `DO-NOT-TOUCH.md` (report lines
  795–855) as a drive-by edit. Such a change belongs to a task or an endorsed
  follow-up; if a session believes one should change, it stops and suggests a
  follow-up.
- Verify the relevant crate still builds and its tests resolve — including
  regeneration of recorded request headers, URLs and golden bytes where the report
  says they will change spelling.

## `STATE.md` fields

- **Tasks**: `id`, `title`, `state`, `commit ref(s)`, `note`.
  States: `todo` → `in_progress` → `pending_review` → `completed` / `reverted` /
  `skipped`.
- `commit ref(s)` is only the pointer handed to a review session so it knows what to
  look at. It is not a chronological change log. Do not add dates, session/tool IDs,
  machine data, or a running narrative.
- `note` carries the one-line reason for `skipped`/`reverted`, or a decision that later
  tasks need (e.g. the `game_ids[]` spelling from T09). Keep it short and point to
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
- An implementing session may deviate from the written plan when it finds a safer or
  simpler delegation; record the deviation in the task's `note`. The goal is
  library-owned mechanics with local policy, not literal compliance with a static plan.
- Only a review session creates or removes follow-ups, and only the orchestrator writes
  them into `STATE.md`. The reviewer decides whether an implementer-suggested follow-up
  is worth doing.
- A task whose premise is pre-satisfied, stale, unsafe, low-payoff churn, or whose
  change would hit a keeper is skipped, not reported as a cosmetic success. This is
  especially expected for the optional/low-confidence findings (§R6, §LEAF-8, §SYNC-9,
  §APP-12, §CAP-11).

## No lingering references

Before the final cleanup, confirm no source, test, comment, manifest, doc table or
commit message in version control points at `REFACTOR_PLAN/` or at
`docs/08-third-party-delegation-findings.md`. Then delete `REFACTOR_PLAN/` and that
report file.
