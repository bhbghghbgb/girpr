# Orchestration plan

This document is the operating procedure for the orchestrator. It defines a
state machine, not a history. **Read `STATE.md` first** — it is the single source
of truth. The orchestrating session may be replaced at any time; a replacement
starts from `STATE.md` and the files in this directory with no prior
conversation, so every decision that must survive is recorded there.

The user will instruct the orchestrator how to spawn a session. No session
spawning mechanism, tool, date/time, session ID, machine data or change history
is recorded here or in `STATE.md`. "Spawn a session" always means: give that
session the relevant **pointers** (report sections + task file paths +, for a
review, the commit refs from `STATE.md`) and let it read the files and the code
itself. Do **not** recite the report or a plan into a prompt.

## Persistent context contract

- `STATE.md` holds: the task checklist with states, the commit refs needed to
  hand a batch to a reviewer, the review ledger, the follow-up list, and a
  "Persistent context" block for anything else discovered during orchestration.
- Any validation result or decision that a later task depends on (e.g. a library
  ruled out, a behavior that must stay, a deviation an implementer chose) is
  written into the task's `note` or the Persistent context block — as a pointer
  to report sections where possible.
- Never write dates, session/tool identifiers, local paths, or a chronological
  narrative. Commit refs are operational pointers only.

## Phase 1 — Implement tasks one by one

Repeat while any task is `todo`:

1. **Pick** the next `todo` task in ID order from `STATE.md`. Read its
   `REFACTOR_PLAN/tasks/Txx-*.md` file and the report sections it points at
   only enough to know which pointers to pass on; do not summarize them.
2. **Mark** the task `in_progress` in `STATE.md` in its own commit (optional but
   recommended if a run can be interrupted).
3. **Spawn one implementation session for that task only.** Give it:
   - the path to its task file, and
   - the report sections/line ranges named in that task file (goal + assumptions).

   Instruct it to validate against the current code, to stop and report if the
   task is stale or trouble arises, and that it may improvise toward the goal.
   It does not touch other tasks and does not update `STATE.md`.
4. **On return**:
   - If the session says the task is stale/troublesome → set the task to
     `skipped` with the one-line reason in `STATE.md` (own commit). It does not
     go to review.
   - Otherwise record the implementation commit ref(s) and set the task to
     `pending_review` in `STATE.md`, in a **separate commit** from the
     implementation. Note any deviation or persistent context in `note`.
5. **Checkpoint**: when the accumulated `pending_review` set matches a batch in
   `REVIEW-CHECKPOINTS.md` (or has grown large enough to review), go to Phase 2.
   Otherwise pick the next `todo` task.

## Phase 2 — Review and fix a batch

1. **Spawn one review-and-fix session** for the current `pending_review` batch.
   Give it:
   - the report sections and task files for those tasks, and
   - the implementation commit refs from `STATE.md`.

   Instruct it to review only that batch, to **fix or revert** bad tasks, and to
   produce **non-blocking follow-up suggestions only** (it must not implement
   them). It returns final states and follow-ups; it does not edit `STATE.md`.
2. **Apply** the result in `STATE.md` (own commit): each task becomes
   `completed`, `reverted` or `skipped`; write any revert/skip reason; record
   proposed follow-ups with new IDs and state `open`; mark the review `done`.
   Add any persistent context (e.g. "§A4 byte-stability confirmed — keep
   renderer") to the context block.
3. Return to Phase 1 if `todo` tasks remain.

A reverted task is not automatically retried; a reviewer may turn it into a
follow-up if it is worth re-attempting differently.

## Phase 3 — Follow-ups

Only when every task is settled (`completed` / `reverted` / `skipped`):

1. **Spawn a follow-up planning review session first.** Give it the full
   follow-up list from `STATE.md` and the relevant report sections. It decides
   how to batch the follow-ups, which are not worth doing (recommend `dropped`),
   and may propose new follow-ups. Record its decisions in `STATE.md` (own
   commit). This guard exists to stop follow-ups from running away.
2. For each approved batch: spawn **one implementation session** for the whole
   batch (unlike Phase 1, a batch is done together), then **one review session**
   to evaluate it. Apply results to `STATE.md` in separate commits exactly as
   in Phase 2. Mark follow-ups `batched` → `done` / `dropped`.
3. The orchestrator may spawn an extra review session at any time to evaluate
   and reduce the follow-up list when run-away is suspected. The orchestrator
   itself never researches the code — research belongs to implementation and
   review sessions.

## Phase 4 — Cleanup

When all tasks and follow-ups are settled:

1. Verify via a review session (or as the final review's checklist) that no
   source, test, comment, manifest or commit message in version control
   references `REFACTOR_PLAN/` or `docs/08-hand-rolled-code-reduction-findings.md`.
2. Delete this entire directory and the report file, and commit the deletion.
   No implementation artifact may keep a reference to them.

## Invariants (never violate)

- One implementation session handles one task in Phase 1; sessions run one after
  another, never concurrently on overlapping work.
- Every `STATE.md` write is its own commit, separate from implementation work.
- A `skipped` task is never sent to review as if it were done.
- Reviewers fix/revert and recommend; they never implement follow-ups and never
  expand a task's scope into report §H / `DO-NOT-TOUCH.md` items.
- The orchestrator passes pointers, not prose summaries, and does no research of
  its own.
