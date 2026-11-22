# Conventions

## Pointers, not recitations

- To aim a session at a goal, pass **file paths + line ranges / section numbers** (e.g.
  `docs/11-shared-crate-generalization-research.md §F4 lines 177–190` and a
  `REFACTOR_PLAN/tasks/T01-*.md` path). Do not paste the report or a task file into the
  session prompt as prose; the session reads the files itself.
- Line numbers come from the report as committed alongside this package. A session that
  finds a stale pointer should trust the code and say so.
- Report sections: §1 assumptions (A1–A15), §2 findings (F1–F53, grouped §2.1–§2.6),
  §3 negative findings (N1–N16). A task file names the exact sections and lines it needs.

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

- Report items are static claims taken on a particular tree (§A1, lines 22–28). Confirm
  the premise in the current tree before editing; line numbers may have drifted.
- A candidate is only worth extracting if it meets the tests in §1: a clean boundary at
  an acyclic leaf or whole SCC (§A12, lines 114–121), direct consumption by high-level
  callers rather than a re-exporting facade (§A2, lines 29–36), preservation of published
  bytes and key order (§A5, lines 57–67), gircap independence if gircap is a caller (§A4,
  lines 48–56), and no collapse of intentionally divergent policies (§A9, lines 88–97).
- A new shared crate moves through every doc seam in §A14 (lines 129–135): workspace
  membership, README crate table and prose, the gircap §2 table and manifest comment if
  gircap depends on it, and relevant `.config/nextest.toml` filters if tests move.
- Feature unification can grow the number of compiled variants of a shared crate (§A11,
  lines 106–113); do not assume a consolidated `test-support` surface is cheaper.
- Under nextest one process runs per test (§A10, lines 98–105): moving a test changes its
  fully-qualified filter, and a process-global fixture helps only within one test.

## `STATE.md` fields

- **Tasks**: `id`, `title`, `state`, `commit ref(s)`, `note`.
  States: `todo` → `in_progress` → `pending_review` → `completed` / `reverted` /
  `skipped`.
- `commit ref(s)` is only the pointer handed to a review session so it knows what to look
  at. It is not a chronological change log. Do not add dates, session/tool IDs, machine
  data, or a running narrative.
- `note` carries the one-line reason for `skipped`/`reverted`, or a decision that later
  tasks need (e.g. "§F11 blocked until Mode lifted — landed in T07"). Keep it short and
  point to report sections where useful.
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
  valuable boundary; record the deviation in the task's `note`. The goal is a worthwhile,
  safe generalization, not literal compliance with a static plan.
- Only a review session creates or removes follow-ups, and only the orchestrator writes
  them into `STATE.md`. The reviewer decides whether an implementer-suggested follow-up
  is worth doing.
- A task whose premise is pre-satisfied, stale, unsafe, forbidden by a standing
  invariant, or whose change would merely add a facade is skipped, not reported as a
  cosmetic success. For low-confidence findings (e.g. §F20, §F26, §F28, §F29, §F41, §F44,
  §F48, §F52) a validated skip is the expected default; implement only on evidence of a
  safe boundary.

## No lingering references

Before the final cleanup, confirm no source, test, comment, manifest, doc table or commit
message in version control points at `REFACTOR_PLAN/` or at
`docs/11-shared-crate-generalization-research.md`. Then delete `REFACTOR_PLAN/` and that
report file.
