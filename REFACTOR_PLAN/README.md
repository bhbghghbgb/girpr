# `REFACTOR_PLAN` — shared-crate extraction plan

This is the planning package for the shared-crate refactor research recorded in
`docs/08-shared-crate-refactor-research.md` (referred to everywhere below as
**the report**).

The goal of the work is to move duplicated implementation and mechanics that sit
*below* the existing policy line into shared leaf crates under `crates/`, without
coupling the programs to one another and without changing any byte-level or
behavioural contract. The report already holds the findings, the trade-offs, the
affected code and the assumptions. This package deliberately contains **only
implementation plans and the rules for carrying them out**; every task points at
report sections instead of copying them.

> This directory and the report are **deleted when the work is finished**.
> Implementation code, tests, comments, manifests and commit messages must
> therefore never refer to `REFACTOR_PLAN/` or to
> `docs/08-shared-crate-refactor-research.md`. Every plan file points outward to
> the report only so an implementing session can find the goal; no shipped
> artifact depends on those files.

## Layout

```text
REFACTOR_PLAN/
  README.md                     this file — index, structure, how to read a task
  ORCHESTRATION.md              the orchestration plan (state machine, checkpoints)
  STATE.md                      the committed check-list state (single source of truth)
  tasks/
    T01-...md ... T20-...md      one implementation plan per task
  guidance/
    CONVENTIONS.md              commit/state conventions, pointer rules, skip/follow-up rules
    DO-NOT-TOUCH.md             findings that must stay; what is not a shared-crate task
    REVIEW-CHECKPOINTS.md       how tasks are grouped into review batches
```

## How to read a task file

Every `tasks/Txx-*.md` has the same shape:

- **Goal / pointers** — the exact report sections (§ and line numbers) that are
  the detailed brief, plus the assumptions that must be validated first.
- **Scope** — the packages/files named in the report. These are *hints*; the
  implementing session locates them in the current tree before editing.
- **Implementation plan** — a short outline of the intended extraction. It is
  not exhaustive and not carved in stone; the implementing session does the
  detailed research.
- **Constraints / validation** — the caveats that must survive (gircap
  independence, byte stability, deliberately divergent policies, "do not merge
  this", etc.).
- **Done when** — the acceptance check.

The implementing session is expected to validate every premise against the
current code. If a task is stale, contradicted by the code, would couple two
programs, or would regress a load-bearing property, the session **stops** and the
task is recorded as `skipped` with a one-line reason. If a better optimization
presents itself, the session may improvise as long as the goal — removing
duplication by moving shared mechanics into a leaf crate — is met.

Adding a shared crate is never just a code move. See `guidance/CONVENTIONS.md`
for the standing rule that any new leaf dependency of gircap must update the
independence tables named in report §A2 (and the doc-table gap in §F30).

## Task order

Task IDs define the recommended pick-up order used by the orchestrator (T01
first). The order roughly runs: digest/identity primitives → path and filesystem
leaves → hashing/retry → logging/CLI/reporting → domain model → test
infrastructure. The order is a recommendation, not a hard rule; `STATE.md` is
authoritative for what remains.

## Lifecycle at a glance

1. Orchestrator reads `STATE.md`, picks the next `todo` task.
2. A single implementation session handles that one task only.
3. In a **separate commit**, the orchestrator moves the task to `pending_review`.
4. When the accumulated `pending_review` set reaches a review checkpoint sized
   by amount of work, one review-and-fix session reviews the batch, fixes or
   reverts, and updates `STATE.md` (`completed` / `reverted` / `skipped`) and
   records any follow-ups.
5. After all tasks are settled, follow-ups are evaluated by a planning review,
   batched, and handled by further implement + review sessions.
6. Final cleanup deletes this directory and the report.

See `ORCHESTRATION.md` for the full state machine and `CONVENTIONS.md` for the
exact commit and state-update format.
