# Orchestration plan

This document is the operating procedure for the orchestrator. It defines a state
machine, not a history. **Read `STATE.md` first** — it is the single source of truth.
The orchestrating session may be replaced at any time; a replacement starts from
`STATE.md` and the files in this directory with no prior conversation, so every decision
that must survive is recorded there (see "Persistent context contract").

The user will instruct the orchestrator how to spawn a session. No session-spawning
mechanism, tool, date/time, session ID, machine data or change history is recorded here
or in `STATE.md`. "Spawn a session" always means: give that session the relevant
**pointers** (report sections + task file paths +, for a review, the commit refs from
`STATE.md`) and let it read the files and the code itself. Do **not** recite the report
or a plan into a prompt; do not do the research yourself.

## Persistent context contract

- `STATE.md` holds: the task checklist with states, the commit refs needed to hand a
  batch to a reviewer, the review ledger, the follow-up list, and a "Persistent context"
  block for anything else discovered during orchestration.
- Any validation result or decision that a later task depends on — a finding that turned
  out stale, a byte contract that must stay, a divergence that is a keeper, a boundary
  that turned out unsafe, a deviation an implementer chose — is written into that task's
  `note` or the Persistent context block, as a pointer to report sections where possible.
- Never write dates, session/tool identifiers, local paths, or a chronological narrative.
  Commit refs are operational pointers only, not a change log.
- Standing invariants from the report are pre-seeded in the Persistent context block so a
  replaced orchestrator does not lose them. A session that finds one contradicted by the
  code records that in the relevant task's `note` rather than silently proceeding.

## Phase 0 — Establish the check-list state

The committed `STATE.md` is the check list. It is pre-seeded with every task in `todo`
state, empty review and follow-up tables, and the standing invariants. The orchestrator
keeps it current: every state transition is committed in its **own** commit, separate
from implementation work.

## Phase 1 — Implement tasks one by one

Repeat while any task is `todo`:

1. **Pick** the next `todo` task in ID order from `STATE.md`. Read its
   `REFACTOR_PLAN/tasks/Txx-*.md` file and the report sections it points at only enough
   to know which pointers to pass on; do not summarize them.
2. **Mark** the task `in_progress` in `STATE.md` in its own commit (so an interrupted run
   can resume from the file).
3. **Spawn one implementation session for that task only.** Give it:
   - the path to its task file,
   - the report sections/line ranges named in that task file (goal, scope, assumptions,
     keep items), and
   - `guidance/CONVENTIONS.md` and `guidance/DO-NOT-TOUCH.md`.

   Instruct it to validate every premise against the current code (report items are
   static, host-specific claims — §A1, lines 22–28), to stop and report if the task is
   stale, contradicted by the code, would regress a deliberately kept policy (§A9,
   §N1–N16, `DO-NOT-TOUCH.md`), would change a published byte contract (§A5), could not be
   bounded at an acyclic leaf (§A12), or would be a facade that defeats the split (§A2).
   For low-confidence / "likely skip" / A4-forbidden findings, a validated skip is a
   legitimate outcome — do not force an abstraction. The session may improvise toward a
   safe shared boundary, does not touch other tasks, and does not update `STATE.md`.
4. **On return**:
   - If the session says the task is stale/troublesome, unsafe, or changes a keeper → set
     the task to `skipped` with the one-line reason in `STATE.md` (own commit). It does
     not go to review. The implementing agent may suggest a follow-up; record it later
     only if a reviewer endorses it (the reviewer decides whether it is worth doing).
   - Otherwise record the implementation commit ref(s) and set the task to
     `pending_review` in `STATE.md`, in a **separate commit** from the implementation.
     Note any deviation or persistent context in `note`. A complicated task may have
     more than one implementation commit; record them all as pointers.
5. **Checkpoint**: when the accumulated `pending_review` set matches a batch in
   `REVIEW-CHECKPOINTS.md` (or has grown large enough that a reviewer would be
   overwhelmed), go to Phase 2. Otherwise pick the next `todo` task. Checkpoint sizes
   are chosen by amount of work, not by a fixed task count. Only one implementation
   session runs at a time; the next starts after the previous one is done.

## Phase 2 — Review and fix a batch

1. **Spawn one review-and-fix session** for the current `pending_review` batch. Give it:
   - the report sections and task files for those tasks,
   - the implementation commit refs from `STATE.md`, and
   - `guidance/DO-NOT-TOUCH.md` and `guidance/CONVENTIONS.md`.

   Instruct it to review only that batch, to **fix or revert** bad tasks, and to produce
   **non-blocking follow-up suggestions only** (it must not implement them). It returns
   final states and follow-ups; it does not edit `STATE.md`. It checks especially that:
   no published byte/key order/flag condition drifted (§A5), gircap stayed independent
   and doc tables moved with any new leaf (§A4, §A14), no report plane or digest semantic
   was wrongly merged (§A6, §A8), verification stayed an independent re-derivation
   (§A13), no keeper from §A9/§N1–N16 was pulled in, and no re-exporting facade was left
   behind (§A2).
2. **Apply** the result in `STATE.md` (own commit): each task becomes `completed`,
   `reverted` or `skipped`; write any revert/skip reason; record proposed follow-ups with
   new IDs and state `open`; mark the review `done`. Add any persistent context (e.g.
   "§F11 blocked by file↔scan cycle until Mode is lifted — see T07").
3. Return to Phase 1 if `todo` tasks remain.

A reverted task is not automatically retried; a reviewer may turn it into a follow-up if
it is worth re-attempting differently. The reviewer evaluates implementer-suggested
follow-ups and decides whether each is worth doing; only the orchestrator writes them
into `STATE.md`.

## Phase 3 — Follow-ups

Only when every task is settled (`completed` / `reverted` / `skipped`):

1. **Spawn a follow-up planning review session first.** Give it the full follow-up list
   from `STATE.md` and the relevant report sections. It decides how to batch the
   follow-ups, which are not worth doing (recommend `dropped`), and may propose new
   follow-ups. Record its decisions in `STATE.md` (own commit). This guard exists to
   stop follow-ups from running away; reviewers may remove existing follow-ups to keep
   the list bounded.
2. For each approved batch: spawn **one implementation session** for the whole batch
   (unlike Phase 1, a batch is done together), then **one review session** to evaluate
   it. Apply results to `STATE.md` in separate commits exactly as in Phase 2. Mark
   follow-ups `batched` → `done` / `dropped`.
3. The orchestrator may spawn an extra review session at any time to evaluate and reduce
   the follow-up list when run-away is suspected. The orchestrator itself never
   researches the code — research belongs to implementation and review sessions.

## Phase 4 — Cleanup

When all tasks and follow-ups are settled:

1. Verify via a review session (or as the final review's checklist) that no source, test,
   comment, manifest, doc table or commit message in version control references
   `REFACTOR_PLAN/` or `docs/11-shared-crate-generalization-research.md`.
2. Delete this entire directory and the report file, and commit the deletion. No
   implementation artifact may keep a reference to them.

## Invariants (never violate)

- One implementation session handles one task in Phase 1; sessions run one after
  another, never concurrently on overlapping work.
- Every `STATE.md` write is its own commit, separate from implementation work.
- A `skipped` task is never sent to review as if it were done.
- Reviewers fix/revert and recommend; they never implement follow-ups and never expand a
  task's scope into a `DO-NOT-TOUCH.md` item.
- The orchestrator passes pointers, not prose summaries, and does no research of its own.
  A task that is unsafe, stale, or not a real generalization is skipped, not reported as
  a success.
- No dates, session/tool IDs, local paths or chronological narrative are stored anywhere
  in this package.
