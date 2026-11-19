# `REFACTOR_PLAN` — implementation-plan package

This directory is the planning package for the hand-rolled-code-reduction work
described in `docs/08-hand-rolled-code-reduction-findings.md` (referred to
everywhere below as **the report**).

It is deliberately **not** a research document and **not** an orchestration log.
It contains only implementation plans and the rules for carrying them out. The
report already holds the findings, the trade-offs, the affected tests and the
assumptions; this package points at those sections instead of copying them.

> This directory and the report are **deleted when the work is finished**.
> Implemented code, tests, comments and commit messages must therefore never
> refer to `REFACTOR_PLAN/` or to `docs/08-…`. Every plan file points outward to
> the report only so an implementing session can find the goal; no shipped
> artifact depends on those files.

## Layout

```text
REFACTOR_PLAN/
  README.md                     this file — index, structure, how to read a task
  ORCHESTRATION.md              the orchestration plan (state machine, checkpoints)
  STATE.md                      the committed check-list state (single source of truth)
  tasks/
    T01-...md ... T35-...md      one implementation plan per task
  guidance/
    CONVENTIONS.md              commit/state conventions, pointer rules, skip/follow-up rules
    DO-NOT-TOUCH.md             findings that must stay (pointers to report §H)
    REVIEW-CHECKPOINTS.md       how tasks are grouped into review batches
```

## How to read a task file

Every `tasks/Txx-*.md` has the same shape:

- **Goal / pointers** — the exact report sections (§ and line numbers) that are
  the detailed brief, plus the assumptions (report §I) that must be validated.
- **Scope** — the packages/files named in the report. These are *hints*; the
  implementing session locates them in the current tree before editing.
- **Implementation plan** — a short outline of the intended delegation. It is
  not exhaustive. The implementing session does the detailed research.
- **Constraints / validation** — the caveats that must survive (byte-stability,
  parity rules, "do not delegate this part", etc.).
- **Done when** — the acceptance check.

The implementing session is expected to validate every premise against the
current code, not to treat the report or the task file as instructions carved in
stone. If a task is stale, contradicted by the code, or would regress a
load-bearing property, the session **stops** and the task is recorded as
`skipped` with a one-line reason. If a better optimization presents itself, the
session may improvise as long as the goal (remove hand-rolled code by delegating
to a maintained library) is met.

## Task order

The task IDs define the pick-up order used by the orchestrator (T01 first). The
order roughly runs low-risk → shared infrastructure → larger/riskier, with the
open-question and optional items last. It is a recommendation, not a hard rule;
`STATE.md` is authoritative for what remains.

## Lifecycle at a glance

1. Orchestrator reads `STATE.md`, picks the next `todo` task.
2. A single implementation session handles that one task.
3. In a **separate commit**, the orchestrator moves the task to `pending_review`.
4. When a batch reaches a review checkpoint, one review-and-fix session reviews
   the batch, fixes or reverts, and updates `STATE.md` (`completed` / `reverted`
   / `skipped`) and records any follow-ups.
5. After all tasks are settled, follow-ups are evaluated, batched and handled by
   further implement + review sessions.
6. Final cleanup deletes this directory and the report.

See `ORCHESTRATION.md` for the full state machine and `CONVENTIONS.md` for the
exact commit and state-update format.
