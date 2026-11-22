# `REFACTOR_PLAN` — shared-crate generalization plan

This is the planning package for the shared-crate generalization research recorded in
`docs/11-shared-crate-generalization-research.md` (referred to everywhere below as
**the report**).

The goal of this round is **further generalization with high-risk changes**: decide which
remaining cross-program duplication justifies a new shared crate or a deeper abstraction,
and what makes that change high-risk. The report already holds the assumptions (§A1–A15),
the candidate boundaries (§F1–F53) and the negative findings (§N1–N16). It deliberately
contains **no task list, sequencing or implementation steps**. This package adds only the
implementation plans and the rules for carrying them out; every task points back to report
sections instead of copying them.

> This directory and the report are **deleted when the work is finished**.
> Implementation code, tests, comments, manifests, doc tables and commit messages must
> therefore never refer to `REFACTOR_PLAN/` or to
> `docs/11-shared-crate-generalization-research.md`. Every plan file points outward to the
> report only so an implementing session can find the goal; no shipped artifact depends on
> those files.

## Layout

```text
REFACTOR_PLAN/
  README.md                     this file — index, structure, how to read a task
  ORCHESTRATION.md              the orchestration plan (state machine, checkpoints)
  STATE.md                      the committed check-list state (single source of truth)
  tasks/
    T01-...md ... T48-...md      one implementation plan per candidate boundary
  guidance/
    CONVENTIONS.md              commit/state conventions, pointer rules, skip/follow-up rules
    DO-NOT-TOUCH.md             findings that must stay; what is not a task
    REVIEW-CHECKPOINTS.md       how tasks are grouped into review batches
```

## How to read a task file

Every `tasks/Txx-*.md` has the same shape:

- **Goal / pointers** — the exact report sections (§ and line numbers) that are the
  detailed brief, plus the assumptions that must be validated first and any keep items.
- **Scope** — the packages/files and the proposed leaf named in the report. These are
  *hints*; the implementing session locates them in the current tree before editing.
- **Implementation plan** — a short outline of the intended change. It is not exhaustive
  and not carved in stone; the implementing session does the detailed research and
  validation. A task may be delivered in more than one commit.
- **Constraints / validation** — the caveats that must survive (byte contracts, gircap
  independence, report planes, digest semantics, keepers, cycles, …).
- **Done when** — the acceptance check, including what to do when validation fails.

The implementing session is expected to validate every premise against the current code.
Report items are static claims taken on a particular tree (§1, §A1 lines 22–28); line
numbers may have drifted. If a task is stale, contradicted by the code, would regress a
deliberately kept policy (§A9, §N1–N16, `DO-NOT-TOUCH.md`), would change published bytes
(§A5), or cannot be bounded safely, the session **stops** and the task is recorded as
`skipped` with a one-line reason. Several findings are explicitly low-confidence or
"A4-forbidden"; for those the correct outcome is often a validated skip, not a forced
abstraction. If a better generalization presents itself, the session may improvise as long
as the goal — a safe, worthwhile shared boundary — is met.

Recurring constraints drawn from §1 that apply to nearly every task:

- **A2 (lines 29–36):** editing a file invalidates its target and downstream dependents;
  a split only pays off when high-level consumers depend on the narrow crate directly. A
  re-exporting facade that keeps both halves is not a win.
- **A4 (lines 48–56):** gircap stays independent — no `girpr` dependency, only logic-free
  leaves; adding a leaf forces synchronized updates to the gircap §2 table, manifest
  comment block, README inventory and bridge spec.
- **A5 (lines 57–67):** byte contracts are load-bearing — key order, casing, spacing,
  wording and flag conditions must not drift even when tests pass.
- **A14 (lines 129–135):** new crates move through a fixed set of seams (workspace,
  README, gircap tables/manifest comments, nextest filters) together.

## Task order

Task IDs define the recommended pick-up order used by the orchestrator (T01 first). The
order front-loads the self-contained chunk-level leaves, then the cycle-sensitive product
repair internals (dependency prerequisites first), then gircap-side and girsync-side
candidates, then shared-crate/test-infrastructure candidates. Findings that are
low-confidence, "likely re-confirm the skip", or forbidden by a standing invariant are
placed later. This is a recommendation, not a hard rule; `STATE.md` is authoritative for
what remains.

## Lifecycle at a glance

1. Orchestrator reads `STATE.md`, picks the next `todo` task.
2. A single implementation session handles that one task only and receives pointers
   (report sections + its task file), not a recitation of the plan.
3. In a **separate commit**, the orchestrator moves the task to `pending_review`.
4. When the accumulated `pending_review` set reaches a review checkpoint sized by amount
   of work, one review-and-fix session reviews the batch, fixes or reverts, and the
   orchestrator updates `STATE.md` (`completed` / `reverted` / `skipped`) and records any
   non-blocking follow-ups.
5. After all tasks are settled, a planning review first decides how to batch follow-ups
   and which to drop, then alternate implement and review sessions over those batches.
6. Final cleanup deletes this directory and the report; no reference may survive.

See `ORCHESTRATION.md` for the full state machine, `CONVENTIONS.md` for the exact commit
and state-update format, and `DO-NOT-TOUCH.md` before editing anything.
