# `REFACTOR_PLAN` — crate-split turnaround plan

This is the planning package for the crate-split turnaround research recorded in
`docs/09-crate-split-turnaround-research.md` (referred to everywhere below as
**the report**).

The goal of the work is to shrink **turnaround after a change**: Cargo recompiles
the changed package and every downstream dependent, then re-links the binaries
that depend on them. The way to shrink that set is to move code into smaller
crates and make the high-level consumers (binaries and integration tests)
depend on those crates **directly**, so a change invalidates fewer targets. The
report already holds the findings, the trade-offs, the affected code and the
assumptions. This package deliberately contains **only implementation plans and
the rules for carrying them out**; every task points at report sections instead
of copying them.

> This directory and the report are **deleted when the work is finished**.
> Implementation code, tests, comments, manifests, doc tables and commit
> messages must therefore never refer to `REFACTOR_PLAN/` or to
> `docs/09-crate-split-turnaround-research.md`. Every plan file points outward
> to the report only so an implementing session can find the goal; no shipped
> artifact depends on those files.

## Layout

```text
REFACTOR_PLAN/
  README.md                     this file — index, structure, how to read a task
  ORCHESTRATION.md              the orchestration plan (state machine, checkpoints)
  STATE.md                      the committed check-list state (single source of truth)
  tasks/
    T01-...md ... T19-...md      one implementation plan per task
  guidance/
    CONVENTIONS.md              commit/state conventions, pointer rules, skip/follow-up rules
    DO-NOT-TOUCH.md             findings that must stay; what is not a split task
    REVIEW-CHECKPOINTS.md       how tasks are grouped into review batches
```

## How to read a task file

Every `tasks/Txx-*.md` has the same shape:

- **Goal / pointers** — the exact report sections (§ and line numbers) that are
  the detailed brief, plus the assumptions that must be validated first.
- **Scope** — the packages/files named in the report. These are *hints*; the
  implementing session locates them in the current tree before editing.
- **Implementation plan** — a short outline of the intended split. It is not
  exhaustive and not carved in stone; the implementing session does the
  detailed research and validation.
- **Constraints / validation** — the caveats that must survive (direct-dependency
  payoff, gircap independence, byte stability, cycles, "keep" items, …).
- **Done when** — the acceptance check.

The implementing session is expected to validate every premise against the
current code. Report items are static claims and were never measured (§F41).
If a task is stale, contradicted by the code, would couple two programs, or
would not actually shrink the rebuild set, the session **stops** and the task is
recorded as `skipped` with a one-line reason. If a better optimization presents
itself, the session may improvise as long as the goal — faster turnaround by
splitting a crate boundary — is met.

**The central test for every task is report §A2 (lines 27–33):** a split pays off
only when a high-level consumer depends on the smaller crate directly. If the
old facade still depends on and re-exports both halves, the split adds a crate
without removing a rebuild edge — the implementing session must say so and
stop rather than produce a cosmetic split.

## Task order

Task IDs define the recommended pick-up order used by the orchestrator (T01
first). The order runs roughly: break the dependency inversion first, then
product (`src/`) splits from low to high level, then test-link reduction, then
tool (`tools/`) splits, then the remaining shared-crate splits. The order is a
recommendation, not a hard rule; `STATE.md` is authoritative for what remains.

## Lifecycle at a glance

1. Orchestrator reads `STATE.md`, picks the next `todo` task.
2. A single implementation session handles that one task only and passes
   pointers (report sections + task file), not a recitation of the plan.
3. In a **separate commit**, the orchestrator moves the task to `pending_review`.
4. When the accumulated `pending_review` set reaches a review checkpoint sized
   by amount of work, one review-and-fix session reviews the batch, fixes or
   reverts, and updates `STATE.md` (`completed` / `reverted` / `skipped`) and
   records any follow-ups.
5. After all tasks are settled, a planning review first decides how to batch
   follow-ups and which to drop, then alternate implement and review sessions
   over those batches.
6. Final cleanup deletes this directory and the report.

See `ORCHESTRATION.md` for the full state machine and `CONVENTIONS.md` for the
exact commit and state-update format.
