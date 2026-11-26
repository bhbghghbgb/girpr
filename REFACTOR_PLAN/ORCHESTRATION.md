# Orchestration plan

This document is the operating procedure for the orchestrator. It defines a state
machine, not a history. **Read `STATE.md` first** — it is the single source of
truth. The orchestrating session may be replaced at any time; a replacement
starts from `STATE.md` and the files in this directory with no prior
conversation, so every decision that must survive is recorded there (see
"Persistent context contract").

The user will instruct the orchestrator how to spawn a session. No
session-spawning mechanism, tool, date/time, session ID, machine data or change
history is recorded here or in `STATE.md`. "Spawn a session" always means: give
that session the relevant **pointers** (report sections + task file paths +, for
a review, the commit refs from `STATE.md`) and let it read the files and the code
itself. Do **not** recite the report or a task file into a prompt; do not do the
research yourself.

## Persistent context contract

- `STATE.md` holds: the task checklist with states, the commit refs needed to
  hand a batch to a reviewer, the review ledger, the follow-up list, and a
  "Persistent context" block for anything else discovered during orchestration.
- Any validation result or decision that a later task depends on — an assumption
  that turned out false, a keeper that must stay, a pinned output or wire
  contract, a chosen spelling, a deviation an implementer chose — is written
  into that task's `note` or the Persistent context block, as a pointer to
  report sections where possible.
- The `game_ids[]` spelling decision (report §X2, lines 90–109; §APP-2, lines
  281–294) is made once in T09 and recorded in the Persistent context block
  before T27/T28 or any other URL-touching task is dispatched. A session that
  finds the recorded decision contradicted by the code records that in its
  task's `note` rather than silently diverging.
- Standing invariants from the report are pre-seeded in the Persistent context
  block so a replaced orchestrator does not lose them. A session that finds one
  contradicted by the code records that in the relevant task's `note` rather
  than silently proceeding.
- Never write dates, session/tool identifiers, local paths, or a chronological
  narrative. Commit refs are operational pointers only, not a change log.

## Phase 0 — Establish the check-list state

The committed `STATE.md` is the check list. It is pre-seeded with every task in
`todo` state, empty review and follow-up tables, and the standing invariants.
The orchestrator keeps it current: every state transition is committed in its
**own** commit, separate from implementation work.

## Phase 1 — Implement tasks one by one

Repeat while any task is `todo`:

1. **Pick** the next `todo` task in ID order from `STATE.md`. Read its
   `REFACTOR_PLAN/tasks/Txx-*.md` file and the report sections it points at only
   enough to know which pointers to pass on; do not summarize them.
2. **Mark** the task `in_progress` in `STATE.md` in its own commit (so an
   interrupted run can resume from the file).
3. **Spawn one implementation session for that task only.** Give it:
   - the path to its task file,
   - the report sections/line ranges named in that task file (goal, scope,
     assumptions, keep items),
   - `guidance/CONVENTIONS.md` and `guidance/DO-NOT-TOUCH.md`, and
   - any persistent-context pointer from `STATE.md` that its task names (e.g.
     the recorded `game_ids[]` spelling for URL tasks).

   Instruct it to validate every premise against the current code (the report
   is a static study of one tree, §A1 lines 15–16), to stop and report if the
   task is stale, contradicted by the code, rests on an assumption that does not
   hold, would change a non-finding or pinned contract in
   `guidance/DO-NOT-TOUCH.md` (report lines 795–855), would delete a test of
   intended behavior (§A5 lines 26–28), would enable `serde_json/preserve_order`
   (§A7 lines 33–34), or cannot be bounded safely. A validated skip is a
   legitimate outcome — do not force a library swap. The session may improvise
   toward the finding's goal, does not touch other tasks, and does not update
   `STATE.md`.
4. **On return**:
   - If the session says the task is stale/troublesome, unsafe, or would change
     a keeper → set the task to `skipped` with the one-line reason in `STATE.md`
     (own commit). It does not go to review. The implementing agent may suggest
     a follow-up; record it later only if a reviewer endorses it (the reviewer
     decides whether it is worth doing).
   - Otherwise record the implementation commit ref(s) and set the task to
     `pending_review` in `STATE.md`, in a **separate commit** from the
     implementation. Note any deviation or persistent context (e.g. the chosen
     `game_ids[]` spelling in T09, a golden file that was regenerated, an
     assumption that needed a workaround). A complicated task may have more than
     one implementation commit; record them all as pointers.
5. **Checkpoint**: when the accumulated `pending_review` set matches a batch in
   `REVIEW-CHECKPOINTS.md` (or has grown large enough that a reviewer would be
   overwhelmed), go to Phase 2. Otherwise pick the next `todo` task. Checkpoint
   sizes are chosen by amount of work, not by a fixed task count. Only one
   implementation session runs at a time; the next starts after the previous
   one is done.

## Phase 2 — Review and fix a batch

1. **Spawn one review-and-fix session** for the current `pending_review` batch.
   Give it:
   - the report sections and task files for those tasks,
   - the implementation commit refs from `STATE.md`, and
   - `guidance/DO-NOT-TOUCH.md` and `guidance/CONVENTIONS.md`.

   Instruct it to review only that batch, to **fix or revert** bad tasks, and to
   produce **non-blocking follow-up suggestions only** (it must not implement
   them). It returns final states and follow-ups; it does not edit `STATE.md`.
   It checks especially that:
   - no non-finding or pinned contract was changed (report lines 795–855 and
     `DO-NOT-TOUCH.md`), including the deliberately malformed raw-socket helper
     (§X1 line 84, §CAP-1 line 672), the `serde_json/preserve_order` rule (§A7
     lines 33–34), the `File::lock` decision (§A9 lines 40–41) and retry loops
     that are explicitly out of scope (§LEAF-6 line 493, §CAP-6 lines 735–736);
   - every assumption behind a swap really holds in the current tree, and
     library versions/features were re-checked (§A2, lines 17–18);
   - tests of intended function were adapted rather than deleted (§A5, lines
     26–28), golden files were regenerated rather than emptied, and assertions
     target library guarantees, not internal formatting quirks (§A12, lines
     48–50);
   - deterministic output, artifact bytes and fixed-order contracts survived
     where they are pinned (e.g. §SYNC-1 lines 540–549, §LEAF-4 lines 451–465,
     §CAP-2 lines 674–687);
   - shared leaves were not turned into behavior changes for unrelated
     consumers (§A8 lines 35–36; §LEAF-7 lines 496–505); and
   - no committed artifact references this package or the report.
2. **Apply** the result in `STATE.md` (own commit): each task becomes
   `completed`, `reverted` or `skipped`; write any revert/skip reason; record
   proposed follow-ups with new IDs and state `open`; mark the review `done`.
   Add any persistent context (e.g. a decision that later URL tasks rely on).

   Return to Phase 1 if `todo` tasks remain.

A reverted task is not automatically retried; a reviewer may turn it into a
follow-up if it is worth re-attempting differently. The reviewer evaluates
implementer-suggested follow-ups and decides whether each is worth doing; only
the orchestrator writes them into `STATE.md`.

## Phase 3 — Follow-ups

Only when every task is settled (`completed` / `reverted` / `skipped`):

1. **Spawn a follow-up planning review session first.** Give it the full
   follow-up list from `STATE.md` and the relevant report sections. It decides
   how to batch the follow-ups, which are not worth doing (recommend `dropped`),
   and may propose new follow-ups. Record its decisions in `STATE.md` (own
   commit). This guard exists to stop follow-ups from running away; reviewers
   may remove existing follow-ups to keep the list bounded.
2. For each approved batch: spawn **one implementation session** for the whole
   batch (unlike Phase 1, a batch is done together), then **one review session**
   to evaluate it. Apply results to `STATE.md` in separate commits exactly as in
   Phase 2. Mark follow-ups `batched` → `done` / `dropped`.
3. The orchestrator may spawn an extra review session at any time to evaluate
   and reduce the follow-up list when run-away is suspected, especially around
   the optional/low-confidence findings (§R6, §LEAF-8, §SYNC-9, §APP-12, §CAP-11).
   The orchestrator itself never researches the code — research belongs to
   implementation and review sessions.

## Phase 4 — Cleanup

When all tasks and follow-ups are settled:

1. Verify via a review session (or as the final review's checklist) that no
   source, test, comment, manifest, doc table or commit message in version
   control references `REFACTOR_PLAN/` or
   `docs/08-third-party-delegation-findings.md`.
2. Delete this entire directory and the report file, and commit the deletion.
   No implementation artifact may keep a reference to them.

## Invariants (never violate)

- One implementation session handles one task in Phase 1; sessions run one after
  another, never concurrently on overlapping work.
- Every `STATE.md` write is its own commit, separate from implementation work.
  In particular, moving a task to `pending_review` is its own commit.
- A `skipped` task is never sent to review as if it were done.
- Reviewers fix/revert and recommend; they never implement follow-ups and never
  expand a task's scope into a `DO-NOT-TOUCH.md` item.
- The orchestrator passes pointers, not prose summaries, and does no research of
  its own. A task that is unsafe, stale, or whose premise does not hold is
  skipped, not reported as a success.
- No dates, session/tool IDs, local paths or chronological narrative are stored
  anywhere in this package.
